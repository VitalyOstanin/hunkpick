use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use hunkpick::cli::{Cli, ColorMode, Command, InputOpts, VerifyOpts};
use hunkpick::error::AppError;
use hunkpick::{emit, list, model, parser, select, split, validate};

fn main() -> ExitCode {
    // The flush belongs here, not at the end of `run`: stdout is line-buffered, so whatever
    // follows the last newline sits in the buffer until the runtime drops it at exit — and that
    // flush discards its error. A full disk or a closed device would then truncate the output
    // while the process reported success.
    match run().and_then(|()| flush_out()) {
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
        Command::List { json, color, input } => run_list(json, color, &input),
        Command::Select {
            selectors,
            input,
            verify,
        } => run_select(&selectors, &input, &verify),
        Command::Split {
            hunk,
            at,
            input,
            verify,
        } => run_split(&hunk, &at, &input, &verify),
    }
}

fn run_list(json: bool, color: ColorMode, input: &InputOpts) -> Result<(), AppError> {
    let Some(patch) = load_and_parse(input)? else {
        return Ok(());
    };
    let use_color = hunkpick::cli::resolve_color(color);
    let text = if json {
        list::list_json(&patch)
    } else {
        list::list_human(&patch, use_color)
    };
    write_out(text.as_bytes())?;
    // Both forms end in a newline: the JSON document is a line of a stream as much as the human
    // listing is, and a reader splitting on newlines (`| jq`, `while read`) would otherwise be
    // handed a last line that never terminates.
    if !text.ends_with('\n') {
        write_out(b"\n")?;
    }
    Ok(())
}

fn run_select(
    selectors: &[OsString],
    input: &InputOpts,
    verify: &VerifyOpts,
) -> Result<(), AppError> {
    let Some(patch) = load_and_parse(input)? else {
        return Ok(());
    };
    let sels = select::parse_selectors(selectors).map_err(usage)?;
    let out = select::select(&patch, &sels).map_err(usage)?;
    emit_verified(&out, verify)
}

fn run_split(
    hunk: &OsStr,
    at: &[u32],
    input: &InputOpts,
    verify: &VerifyOpts,
) -> Result<(), AppError> {
    // Split rewrites one hunk in place: the parsed diff is owned here, so there is no
    // reason to hold a second copy of the whole patch on the heap.
    let Some(mut patch) = load_and_parse(input)? else {
        return Ok(());
    };
    let (fi, hi) = select::resolve_hunk(&patch, hunk).map_err(usage)?;
    // `resolve_hunk` already rejected binary files, so the target is always text. The splice
    // and the trailer bookkeeping it forces live together in `split_file_hunk`.
    split::split_file_hunk(&mut patch.files[fi], hi, at).map_err(usage)?;
    // Same rule as `select`: the result owns its new-side anchors. A diff carved out
    // of a larger one keeps anchors describing a file this result does not produce,
    // and `git apply` searches from the new-side position (see `renumber`).
    hunkpick::renumber::renumber_new_side(&mut patch);
    emit_verified(&patch, verify)
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
    // A defect of the input diff is a usage error (exit 2), not a verification failure of
    // hunkpick's own result (exit 70): the latter would report a broken input as a broken tool.
    validate::validate_input(&patch).map_err(|e| AppError::Usage(format!("input diff: {e}")))?;
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
            // A terminal on stdin means the caller forgot the pipe (`hunkpick list` instead of
            // `git diff | hunkpick list`). Reading on is the right behaviour — that is what a
            // filter does, and a diff typed or pasted by hand must still work — but doing it
            // silently is indistinguishable from a hang. One line to stderr says which it is.
            if stdin.is_terminal() {
                eprintln!(
                    "hunkpick: reading a diff from the terminal; pipe one in or use -i FILE (Ctrl-D ends the input)"
                );
            }
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

/// The encoding named by a leading byte-order mark, for the marks whose text hunkpick cannot
/// read. A UTF-8 BOM is not one of them: it survives the round-trip in the preamble and git
/// accepts the result, so it is left alone.
fn utf16_or_32_bom(input: &[u8]) -> Option<&'static str> {
    // UTF-32 first: its little-endian mark starts with the UTF-16LE one.
    match input {
        [0xFF, 0xFE, 0x00, 0x00, ..] => Some("UTF-32LE"),
        [0x00, 0x00, 0xFE, 0xFF, ..] => Some("UTF-32BE"),
        [0xFF, 0xFE, ..] => Some("UTF-16LE"),
        [0xFE, 0xFF, ..] => Some("UTF-16BE"),
        _ => None,
    }
}

/// Reject input that is clearly not a unified diff: text in an encoding hunkpick does not read,
/// binary data (a NUL byte), or text that has no diff marker line at all. Empty / whitespace
/// input is handled by the caller.
fn reject_non_diff(input: &[u8]) -> Result<(), AppError> {
    // Checked before the NUL guard: a UTF-16 diff is ASCII interleaved with NUL bytes, so the
    // guard would fire first and send the reader looking for a binary file. `git diff > x.diff`
    // in Windows PowerShell 5.1 writes exactly that. Re-encoding it here is not an option —
    // the diff is passed through byte for byte (ADR 0005) — so say what to fix.
    if let Some(encoding) = utf16_or_32_bom(input) {
        return Err(AppError::Usage(format!(
            "input starts with a {encoding} byte-order mark; hunkpick reads a UTF-8 (or any \
             ASCII-compatible) byte stream — re-encode the diff, e.g. `iconv -f {encoding} -t \
             UTF-8`"
        )));
    }
    if input.contains(&0) {
        return Err(AppError::Usage(
            "binary input: NUL byte found, expected a unified diff".into(),
        ));
    }
    const MARKERS: [&[u8]; 5] = [b"diff --git ", b"--- ", b"+++ ", b"@@ ", b"Binary files "];
    // A combined diff counts as a marker here so the parser gets to reject it by name. Its
    // `---`/`+++` pair is omitted for a file resolved the same way in both parents, and without
    // this the guard would report "no diff markers found" — which points at the pipe rather
    // than at the format.
    let has_marker = input.split(|&b| b == b'\n').any(|line| {
        MARKERS.iter().any(|m| line.starts_with(m)) || hunkpick::parser::is_combined_marker(line)
    });
    if !has_marker {
        return Err(AppError::Usage(
            "input does not look like a unified diff (no diff markers found)".into(),
        ));
    }
    Ok(())
}

fn write_out(bytes: &[u8]) -> Result<(), AppError> {
    match std::io::stdout().write_all(bytes) {
        Ok(()) => Ok(()),
        // A reader that went away (`hunkpick list | head`) ends this filter's work normally.
        // Rust ignores SIGPIPE, so the write surfaces as EPIPE here; reporting it as an I/O
        // failure would fail the whole pipeline under `set -o pipefail`.
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(AppError::Io(e.to_string())),
    }
}

/// Push out what line buffering held back, classifying a failure the way `write_out` does: a
/// reader that left is a normal end, anything else is an I/O error the caller must see.
fn flush_out() -> Result<(), AppError> {
    match std::io::stdout().flush() {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(AppError::Io(e.to_string())),
    }
}

fn usage<E: std::fmt::Display>(e: E) -> AppError {
    AppError::Usage(format!("{e}"))
}

/// Verify the result diff (internal check by default, optional git check) then emit it.
fn emit_verified(out: &model::Patch, verify: &VerifyOpts) -> Result<(), AppError> {
    if !verify.no_verify_result_diff_internal {
        validate::validate_internal(out)
            .map_err(|e| AppError::Verify(format!("internal consistency check failed: {e}")))?;
    }
    let bytes = emit::emit(out);
    if verify.verify_result_diff_git {
        let dir = verify.dir.clone().unwrap_or_else(|| PathBuf::from("."));
        // Only a verdict from git says anything about the result diff. A git that would not
        // start, or a failure while talking to it, means the check never happened — reporting
        // that as exit 70 would blame the output for a broken environment.
        validate::validate_with_git(&bytes, &dir).map_err(|e| match e {
            validate::GitCheckError::Rejected(_) => AppError::Verify(e.to_string()),
            validate::GitCheckError::WriterPanicked => AppError::Internal(e.to_string()),
            validate::GitCheckError::Spawn(_) | validate::GitCheckError::Io(_) => {
                AppError::Io(e.to_string())
            }
        })?;
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
