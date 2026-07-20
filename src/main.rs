use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use hunkpick::cli::{Cli, Command, InputOpts, VerifyOpts};
use hunkpick::error::AppError;
use hunkpick::{emit, list, model, parser, select, split, validate};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("hunkpick: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

fn run() -> Result<(), AppError> {
    let cli = Cli::parse();

    match cli.command {
        Command::List { json, color, input } => {
            let Some(patch) = load_and_parse(&input)? else {
                return Ok(());
            };
            let use_color = hunkpick::cli::resolve_color(color);
            let text = if json {
                list::list_json(&patch)
            } else {
                list::list_human(&patch, use_color)
            };
            write_out(text.as_bytes())?;
            if !json && !text.ends_with('\n') {
                write_out(b"\n")?;
            }
            Ok(())
        }
        Command::Select {
            selectors,
            input,
            verify,
        } => {
            let Some(patch) = load_and_parse(&input)? else {
                return Ok(());
            };
            let sels = select::parse_selectors(&selectors).map_err(usage)?;
            let out = select::select(&patch, &sels).map_err(usage)?;
            emit_verified(&out, &verify)
        }
        Command::Split {
            hunk,
            at,
            input,
            verify,
        } => {
            let Some(patch) = load_and_parse(&input)? else {
                return Ok(());
            };
            let (fi, hi) = select::resolve_hunk(&patch, &hunk).map_err(usage)?;
            let mut out = patch.clone();
            if let model::FileContent::Text(hunks) = &mut out.files[fi].content {
                let pieces = split::split_hunk_at(&hunks[hi], &at).map_err(usage)?;
                hunks.splice(hi..=hi, pieces);
            }
            emit_verified(&out, &verify)
        }
    }
}

/// Read the input (file or stdin, enforcing the size limit), validate it, and parse it.
/// Returns `Ok(None)` for empty / whitespace-only input (a no-op). The raw input buffer is
/// dropped when this function returns, so it does not co-exist with the result diff on the
/// heap during `select` / `split` / `emit`.
fn load_and_parse(opts: &InputOpts) -> Result<Option<model::Patch>, AppError> {
    let input = read_source(opts)?;
    // Empty / whitespace-only input is a no-op (exit 0) for every subcommand.
    if input.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    reject_non_diff(&input)?;
    let patch = parser::parse(&input).map_err(|e| AppError::Usage(format!("parse error: {e}")))?;
    Ok(Some(patch))
}

/// Read the diff bytes from the configured source: a file (`--input FILE`) or stdin
/// (default, or `--input -`). Enforces `max_input_bytes` (0 disables the limit).
fn read_source(opts: &InputOpts) -> Result<Vec<u8>, AppError> {
    match opts.input.as_deref() {
        Some(path) if path != Path::new("-") => {
            let file =
                File::open(path).map_err(|e| AppError::Io(format!("{}: {e}", path.display())))?;
            read_limited(file, opts.max_input_bytes)
        }
        _ => {
            let stdin = std::io::stdin();
            read_limited(stdin.lock(), opts.max_input_bytes)
        }
    }
}

/// Read all bytes from `r`, rejecting input larger than `limit` bytes (0 = unlimited).
fn read_limited<R: Read>(r: R, limit: u64) -> Result<Vec<u8>, AppError> {
    let mut buf = Vec::new();
    if limit == 0 {
        let mut r = r;
        r.read_to_end(&mut buf)
            .map_err(|e| AppError::Io(e.to_string()))?;
        return Ok(buf);
    }
    // Read one byte past the limit so an exactly-`limit` input is accepted but anything
    // larger is detected without buffering the whole oversized stream. `saturating_add`
    // guards the degenerate `limit == u64::MAX`: `limit + 1` would wrap to 0 (release) or
    // panic (debug), reading nothing; saturating keeps the whole stream readable.
    r.take(limit.saturating_add(1))
        .read_to_end(&mut buf)
        .map_err(|e| AppError::Io(e.to_string()))?;
    if buf.len() as u64 > limit {
        return Err(AppError::Usage(format!(
            "input exceeds limit of {limit} bytes (override with --max-input-bytes)"
        )));
    }
    Ok(buf)
}

/// Reject input that is clearly not a unified diff: binary data (a NUL byte) or text
/// that has no diff marker line at all. Empty / whitespace input is handled by the caller.
fn reject_non_diff(input: &[u8]) -> Result<(), AppError> {
    if input.contains(&0) {
        return Err(AppError::Usage(
            "binary input: NUL byte found, expected a unified diff".into(),
        ));
    }
    const MARKERS: [&[u8]; 5] = [b"diff --git ", b"--- ", b"+++ ", b"@@ ", b"Binary files "];
    let has_marker = input
        .split(|&b| b == b'\n')
        .any(|line| MARKERS.iter().any(|m| line.starts_with(m)));
    if !has_marker {
        return Err(AppError::Usage(
            "input does not look like a unified diff (no diff markers found)".into(),
        ));
    }
    Ok(())
}

fn write_out(bytes: &[u8]) -> Result<(), AppError> {
    std::io::stdout()
        .write_all(bytes)
        .map_err(|e| AppError::Io(e.to_string()))
}

fn usage<E: std::fmt::Display>(e: E) -> AppError {
    AppError::Usage(format!("{e}"))
}

/// Verify the result diff (internal check by default, optional git check) then emit it.
fn emit_verified(out: &model::Patch, verify: &VerifyOpts) -> Result<(), AppError> {
    if !verify.no_verify_result_diff_internal {
        validate::validate_internal(out)
            .map_err(|e| AppError::Verify(format!("internal consistency check failed: {e:?}")))?;
    }
    let bytes = emit::emit(out);
    if verify.verify_result_diff_git {
        let dir = verify.dir.clone().unwrap_or_else(|| PathBuf::from("."));
        validate::validate_with_git(&bytes, &dir).map_err(AppError::Verify)?;
    }
    write_out(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_limited_accepts_input_at_max_limit() {
        // `limit == u64::MAX` must not wrap `limit + 1` to 0 (which would read nothing
        // and silently treat any input as empty). The whole input must be returned.
        let data = b"diff --git a/f b/f\n";
        let got = read_limited(&data[..], u64::MAX).unwrap();
        assert_eq!(got, data);
    }

    #[test]
    fn read_limited_rejects_oversized_input() {
        let data = b"0123456789";
        let err = read_limited(&data[..], 4).unwrap_err();
        assert!(matches!(err, AppError::Usage(_)));
    }
}
