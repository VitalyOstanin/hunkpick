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
    // `resolve_hunk` already rejected binary files, so the target is always text. The splice,
    // the trailer bookkeeping it forces and the trailing-newline rule live together in
    // `split_patch_hunk` — the last of the three needs the whole patch, not one file entry.
    split::split_patch_hunk(&mut patch, fi, hi, at).map_err(usage)?;
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
                    "hunkpick: reading a diff from the terminal; pipe one in or use -i FILE \
                     (Ctrl-D ends the input)"
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

/// How many opening bytes are examined when looking for UTF-16 that carries no byte-order mark.
/// Wide enough for the mail headers of a `git format-patch` input to pass before the diff marker
/// arrives, and for a commit message in another script to sit between them; what lies past the
/// window costs nothing, so a diff whose later content is not ASCII is read no further than this.
const UTF16_SNIFF_BYTES: usize = 8192;

/// The encoding of a UTF-16 stream that carries no byte-order mark, when the input reads like
/// one: two lines of the opening window read, as ASCII paired with NUL, as lines a diff writes
/// — a marker carrying what it promises, an extended header carrying what stands after it, or
/// one of the four headers a path follows, which is read on its own head because the path may
/// be in any script. `iconv -t UTF-16LE` and `UnicodeEncoding($false, $false)` write that, and
/// without this the NUL guard would call such a diff binary input and send the reader looking
/// for a binary file instead of at the encoding of their own patch.
///
/// The evidence is the shape of the lines, not the shape of the stream around them: which
/// header a mail writes first, whether the patch opens with a blank line, and what script the
/// commit subject or the paths are written in are all free, and every one of them was once a
/// reason to answer a UTF-16 patch as binary input. Binary data that happens to hold a
/// NUL-padded marker is refused by how many such lines it holds rather than by how one of them
/// reads: a lone `--- x` is as much an accident of the bytes a PNG carries as it is a patch,
/// and no reading of that one line tells the two apart.
fn utf16_without_bom(input: &[u8]) -> Option<&'static str> {
    /// Stands in for a unit that is not ASCII paired with NUL. It opens no marker and ends no
    /// line, so the text keeps its line boundaries and no marker is read across one.
    const NOT_ASCII: u8 = 0xFF;

    /// What the line says before its first unit that is not ASCII. A marker is evidence only
    /// where it was read whole; a path or a subject in another script after it is the marker's
    /// business no more than the bytes past the window are.
    fn read_as_ascii(line: &[u8]) -> &[u8] {
        let end = line
            .iter()
            .position(|&b| b == NOT_ASCII)
            .unwrap_or(line.len());
        &line[..end]
    }

    /// Whether `line` counts as a line a diff writes, read no further than its first unit that
    /// is not ASCII. Named apart from [`parser::reads_as_a_diff_line`], which it asks: what the
    /// count of two rests on here is the stricter of the two questions.
    ///
    /// [`parser::reads_as_a_diff_line`] takes a header a path follows on its own head: it is
    /// handed the line already read, and past such a header stands a path, which may be in any
    /// script and is not there to be read. Here the line is still whole, so the path is required
    /// to stand in it — counted, not read. A rename whose paths are in another script keeps both
    /// its lines: the units the path is written in stand there, and a unit that is not ASCII is
    /// not whitespace. A window holding nothing but the header twice keeps neither, which is the
    /// half of the count of two that binary data was free to make up.
    fn counts_as_a_diff_line(line: &[u8]) -> bool {
        parser::reads_as_a_diff_line(read_as_ascii(line))
            && parser::the_header_a_path_follows_in(line)
                .is_none_or(|header| parser::carries_something_past(line, header))
    }

    // Truncated to a whole number of code units.
    let len = input.len().min(UTF16_SNIFF_BYTES) & !1;
    let head = &input[..len];
    for (encoding, text_at) in [("UTF-16LE", 0usize), ("UTF-16BE", 1usize)] {
        let nul_at = 1 - text_at;
        // Every unit of the window is read, and one that is not ASCII in this order is kept as
        // NOT_ASCII rather than ending the reading: a commit subject or message in another
        // script sits between the mail headers and the diff marker, and stopping at it would
        // leave the marker unseen and the mail answered as binary input.
        let text: Vec<u8> = head
            .chunks_exact(2)
            .map(|u| {
                if u[nul_at] == 0 && matches!(u[text_at], b'\t' | b'\n' | b'\r' | 0x20..=0x7E) {
                    u[text_at]
                } else {
                    NOT_ASCII
                }
            })
            .collect();
        // Two lines, not one: a single NUL-padded diff line is as much an accident of the bytes
        // a PNG holds as it is evidence of a patch, and no reading of that one line tells the
        // two apart. A diff writes its lines in company — an entry carries a header and a hunk,
        // a rename carries the lines that name both paths — so the second line costs a real
        // patch nothing and is what binary data does not have.
        if text
            .split(|&b| b == b'\n')
            .filter(|line| counts_as_a_diff_line(line))
            .nth(1)
            .is_some()
        {
            return Some(encoding);
        }
    }
    None
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
             ASCII-compatible) byte stream -- re-encode the diff, e.g. `iconv -f {encoding} -t \
             UTF-8`"
        )));
    }
    // Also checked before the NUL guard, and for the same reason as the mark above: a UTF-16
    // diff written without one is still a diff with an encoding problem, not binary data.
    if let Some(encoding) = utf16_without_bom(input) {
        return Err(AppError::Usage(format!(
            "input looks like {encoding} without a byte-order mark; hunkpick reads a UTF-8 (or \
             any ASCII-compatible) byte stream -- re-encode the diff, e.g. `iconv -f {encoding} \
             -t UTF-8`"
        )));
    }
    if input.contains(&0) {
        return Err(AppError::Usage(
            "binary input: NUL byte found, expected a unified diff".into(),
        ));
    }
    if !parser::looks_like_a_diff(input) {
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
/// The working tree `git apply --check` is to run in: the value of `-C DIR`, or the current
/// directory.
///
/// A `-C DIR` that does not name a directory is rejected here rather than left to `spawn`,
/// which reports a missing directory with the same `NotFound` as a missing `git` binary. The
/// caller typed this path a moment ago; sending them after their git installation instead of
/// after their own typo costs more than the check does. A bad argument value is a usage error
/// (exit 2, ADR 0013), not an environment failure.
fn check_dir(dir: Option<&Path>) -> Result<PathBuf, AppError> {
    let Some(dir) = dir else {
        return Ok(PathBuf::from("."));
    };
    match std::fs::metadata(dir) {
        Ok(m) if m.is_dir() => Ok(dir.to_path_buf()),
        Ok(_) => Err(AppError::Usage(format!(
            "-C {}: not a directory",
            dir.display()
        ))),
        // The directory can still disappear between this call and `spawn`; that race lands in
        // `GitCheckError::Spawn`, which names the directory too.
        Err(e) => Err(AppError::Usage(format!("-C {}: {e}", dir.display()))),
    }
}

fn emit_verified(out: &model::Patch, verify: &VerifyOpts) -> Result<(), AppError> {
    if !verify.no_verify_result_diff_internal {
        validate::validate_internal(out)
            .map_err(|e| AppError::Verify(format!("internal consistency check failed: {e}")))?;
    }
    let bytes = emit::emit(out);
    if verify.verify_result_diff_git {
        let dir = check_dir(verify.dir.as_deref())?;
        // Only a verdict from git says anything about the result diff. A git that would not
        // start, a failure while talking to it, or a git that gave up on its own means the
        // check never happened — reporting that as exit 70 would blame the output for a
        // broken environment.
        validate::validate_with_git(&bytes, &dir).map_err(|e| match e {
            validate::GitCheckError::Rejected(_) => AppError::Verify(e.to_string()),
            validate::GitCheckError::WriterPanicked => AppError::Internal(e.to_string()),
            validate::GitCheckError::Spawn { .. }
            | validate::GitCheckError::Io(_)
            | validate::GitCheckError::Failed { .. } => AppError::Io(e.to_string()),
        })?;
    }
    write_out(&bytes)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn read_limited_accepts_input_at_max_limit() {
        // `limit == u64::MAX` must not wrap `limit + 1` to 0 (which would read nothing
        // and silently treat any input as empty). The whole input must be returned.
        let data = b"diff --git a/f b/f\n";
        let got = read_limited(&data[..], u64::MAX).unwrap();
        assert_eq!(got, data);
    }

    /// The four marks, each named exactly. The order of the arms is what makes this work:
    /// a UTF-32LE stream opens with the UTF-16LE mark, so a rearranged `match` would call it
    /// UTF-16LE and hand the user an `iconv -f UTF-16LE` that cannot restore their file. A test
    /// asserting only that the message says "UTF-16" passes on exactly that mistake.
    #[test]
    fn every_byte_order_mark_is_named_by_its_own_encoding() {
        let cases: [(&[u8], Option<&str>); 6] = [
            (&[0xFF, 0xFE, 0x00, 0x00, b'-'], Some("UTF-32LE")),
            (&[0x00, 0x00, 0xFE, 0xFF, b'-'], Some("UTF-32BE")),
            (&[0xFF, 0xFE, b'-', 0x00], Some("UTF-16LE")),
            (&[0xFE, 0xFF, 0x00, b'-'], Some("UTF-16BE")),
            (b"diff --git a/f b/f", None),
            (&[0xEF, 0xBB, 0xBF, b'd'], None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                utf16_or_32_bom(input),
                expected,
                "byte-order mark {:02X?}",
                &input[..input.len().min(4)]
            );
        }
    }

    /// A `git format-patch` mail is a supported input, and its first lines are the mail headers:
    /// the diff marker only turns up after them, past the commit message and the diffstat.
    /// Sniffing a fixed 256 bytes stopped short of it, and a mail written in UTF-16 was answered
    /// with "binary input" — the very message this sniffing exists to replace.
    ///
    /// The mail is sized from what such a mail carries, not from [`UTF16_SNIFF_BYTES`]: a window
    /// narrowed back towards the value the defect returns at then fails this test, whereas a
    /// fixture built from the constant would follow it down and keep passing.
    #[test]
    fn a_utf16_mail_is_named_by_its_encoding_and_not_called_binary() {
        const MARKER: &str = "diff --git ";
        // The window the mail was called binary input at, in the bytes the constant is written
        // in. Below this the defect is back whatever the fixture says.
        const WINDOW_THE_DEFECT_RETURNS_AT: usize = 256;

        let mut mail = String::from("From: Someone <someone@example.invalid>\n");
        mail.push_str("Subject: [PATCH] a change across a good part of the tree\n");
        mail.push_str("Date: Mon, 1 Sep 2026 00:00:00 +0300\n\n");
        mail.push_str("The commit message, and then the diffstat git writes before the diff:\n\n");
        for i in 0..40 {
            mail.push_str(&format!(
                " src/a/rather/long/path/file{i:02}.rs | 12 ++++++------\n"
            ));
        }
        mail.push_str(" 40 files changed, 240 insertions(+), 240 deletions(-)\n\n");
        mail.push_str(MARKER);
        mail.push_str("a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n");

        // One code unit per ASCII character, so the offset in the encoded stream is twice this.
        let marker_at = mail.find(MARKER).expect("the mail carries a diff") * 2;
        assert!(
            marker_at > WINDOW_THE_DEFECT_RETURNS_AT,
            "the marker has to sit past the window this test is about: {marker_at} bytes"
        );
        assert!(
            marker_at < UTF16_SNIFF_BYTES,
            "the window is narrower than a format-patch mail needs: the marker is {marker_at} \
             bytes in, the window is {UTF16_SNIFF_BYTES}"
        );

        let utf16: Vec<u8> = mail.bytes().flat_map(|b| [b, 0]).collect();
        assert_eq!(utf16_without_bom(&utf16), Some("UTF-16LE"));
    }

    /// A `@@ ` with bytes after it that are no hunk header carries a marker and nothing the
    /// marker promises. It is the case the reading of a hunk header is here for: what follows
    /// the marker is not whitespace, so every rule that asked only whether the line says
    /// something past its marker answered such a run of NUL-padded bytes with an
    /// `iconv -f UTF-16LE` for a stream that is not UTF-16 at all.
    #[test]
    fn a_hunk_marker_without_a_header_after_it_is_not_called_utf16() {
        assert_eq!(utf16_without_bom(b"@\x00@\x00 \x00\xff\xff"), None);
    }

    /// A commit whose subject or message is not in English is an ordinary one, and the mail
    /// `git format-patch` writes for it is UTF-16 like any other when written that way. The
    /// scan reaches the diff marker past such text, rather than ending at the first unit that
    /// is not ASCII and leaving the mail to be answered as binary input.
    #[test]
    fn a_utf16_mail_with_non_ascii_headers_is_named_by_its_encoding() {
        let mut mail = String::from("From: Someone <someone@example.invalid>\n");
        mail.push_str("Subject: [PATCH] Исправление разбора хвоста\n");
        mail.push_str("Date: Mon, 1 Sep 2026 00:00:00 +0300\n\n");
        mail.push_str("Запись без ханков читается как доходящая до своей последней строки.\n\n");
        mail.push_str("diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n");

        let utf16 = as_utf16le(&mail);
        assert_eq!(utf16_without_bom(&utf16), Some("UTF-16LE"));
    }

    /// A rename, a mode change and a binary file carry no `---`, `+++` or `@@` line, so the
    /// `diff --git` line is the only marker such an entry has — and it holds the paths, which
    /// are the part of a diff most likely not to be ASCII. Read as "the whole line decoded",
    /// a path in another script leaves the entry with no marker line at all and the patch is
    /// answered as binary input.
    #[test]
    fn a_utf16_rename_of_a_path_in_another_script_is_named_by_its_encoding() {
        let rename = "diff --git a/файл.md b/новый.md\n\
                      similarity index 100%\n\
                      rename from файл.md\n\
                      rename to новый.md\n";

        let utf16 = as_utf16le(rename);
        assert_eq!(utf16_without_bom(&utf16), Some("UTF-16LE"));
    }

    /// A diff written as UTF-16 is UTF-16 from its first byte. Binary data that happens to hold
    /// a NUL-padded marker line somewhere inside it — here a PNG signature, a marker line, and
    /// the signature again — is not, and answering it with an `iconv` names an encoding the
    /// stream is not in.
    #[test]
    fn a_marker_line_inside_binary_data_is_not_called_utf16() {
        let png_then_a_marker_then_png =
            b"\x89\x50\x0a\x00@\x00@\x00 \x00-\x001\x00\x0a\x00\x89\x50";
        assert_eq!(utf16_without_bom(png_then_a_marker_then_png), None);
    }

    /// The same binary data with `--- ` in place of `@@ `. A hunk header states its shape in
    /// full and a path states nothing, so a rule that reads one line and asks what its marker
    /// promises refuses the first and accepts the second — while both are the same accident of
    /// a NUL-padded run inside a PNG.
    #[test]
    fn a_path_marker_inside_binary_data_is_not_called_utf16() {
        let png_then_a_marker_then_png =
            b"\x89\x50\x0a\x00-\x00-\x00-\x00 \x00x\x00\x0a\x00\x89\x50";
        assert_eq!(utf16_without_bom(png_then_a_marker_then_png), None);
    }

    /// The headers of a mail come in no fixed order, and the first of them may be the subject —
    /// which is where a commit written in another script carries that script. Judged by whether
    /// its first line reads as ASCII, such a mail is answered as binary input for no reason
    /// beyond the order git happened to write two headers in.
    ///
    /// Written with each of [`LINE_ENDINGS`].
    #[test]
    fn a_utf16_mail_whose_first_header_is_not_ascii_is_named_by_its_encoding() {
        for ending in LINE_ENDINGS {
            let mail = format!(
                "Subject: [PATCH] Исправление разбора хвоста{ending}\
                 From: Someone <someone@example.invalid>{ending}{ending}\
                 diff --git a/f b/f{ending}--- a/f{ending}+++ b/f{ending}\
                 @@ -1 +1 @@{ending}-a{ending}+b{ending}"
            );

            let utf16 = as_utf16le(&mail);
            assert_eq!(utf16_without_bom(&utf16), Some("UTF-16LE"), "{ending:?}");
        }
    }

    /// A stream that opens with a line break is text whose first line is empty, not a stream
    /// that fails to open as text: `git format-patch` output pasted into a file and a diff
    /// written after a blank line both start this way.
    ///
    /// Written with each of [`LINE_ENDINGS`].
    #[test]
    fn a_utf16_diff_that_opens_with_a_blank_line_is_named_by_its_encoding() {
        for ending in LINE_ENDINGS {
            let diff = format!(
                "{ending}diff --git a/f b/f{ending}--- a/f{ending}+++ b/f{ending}\
                 @@ -1 +1 @@{ending}-a{ending}+b{ending}"
            );

            let utf16 = as_utf16le(&diff);
            assert_eq!(utf16_without_bom(&utf16), Some("UTF-16LE"), "{ending:?}");
        }
    }

    /// Every marker the parser knows, as the bare marker and as a line carrying what that
    /// marker promises. The cases below are asserted against each pair rather than against one
    /// marker written out by hand: every revision of this check so far has read one marker
    /// closely and the rest loosely, and passed a corpus that only ever asked about the marker
    /// it read closely.
    ///
    /// That the pairs are every marker is asserted rather than stated, by
    /// [`every_marker_the_parser_knows_has_a_pair`]: the example line beside each marker has to
    /// be written by hand, so the set cannot be read off the parser outright, and a list that
    /// only claims to be complete falls behind the one it stands for.
    const MARKER_LINES: [(&str, &str); 8] = [
        ("diff --git ", "diff --git a/f b/f"),
        ("--- ", "--- a/f"),
        ("+++ ", "+++ b/f"),
        ("@@ ", "@@ -1 +1 @@"),
        ("Binary files ", "Binary files a/f and b/f differ"),
        ("diff --cc ", "diff --cc f"),
        ("diff --combined ", "diff --combined f"),
        ("@@@", "@@@ -1,1 -1,1 +1,1 @@@"),
    ];

    /// A marker the parser learns and the pairs above do not is a gap in every case asserted
    /// over them: the corpus keeps passing while nothing ever asks about the new marker. The
    /// two sets are compared whole rather than counted, because a marker paired twice would
    /// otherwise stand in for one that is missing.
    #[test]
    fn every_marker_the_parser_knows_has_a_pair() {
        let paired: BTreeSet<&[u8]> = MARKER_LINES
            .iter()
            .map(|(bare, _)| bare.as_bytes())
            .collect();
        assert_eq!(paired, parser::markers().collect::<BTreeSet<_>>());
    }

    /// The lengths [`parser::headers_ascii_follows`] and [`parser::headers_a_path_follows`]
    /// are expected to have. Both lists are private to their own module, so the length is
    /// written here rather than taken; named once rather than at each of the two cases that
    /// hand the list in, and named at all for the reason [`weighed_list`] weighs it. The count of
    /// markers needs no constant: [`MARKER_LINES`] holds it.
    const HEADERS_ASCII_FOLLOWS_LEN: usize = 7;
    const HEADERS_A_PATH_FOLLOWS_LEN: usize = 4;

    /// The ways a line of the window may end. Every rule these cases weigh reads the tail of a
    /// line, and the stream the check exists for — what PowerShell 5.1 writes — ends its lines
    /// with a carriage return, so each rule is asked with that return standing in the tail and
    /// without it. Named once rather than written out at each case: spelled out, the set
    /// answered for whichever cases happened to spell it and was free to stay that way.
    const LINE_ENDINGS: [&str; 2] = ["\n", "\r\n"];

    /// `text` as UTF-16, in the byte order `to_bytes` writes.
    fn as_utf16(text: &str, to_bytes: fn(u16) -> [u8; 2]) -> Vec<u8> {
        text.encode_utf16().flat_map(to_bytes).collect()
    }

    /// `text` as UTF-16LE, the order the cases that name one byte order are written in: it is
    /// what `iconv -t UTF-16LE` and `UnicodeEncoding($false, $false)` write.
    fn as_utf16le(text: &str) -> Vec<u8> {
        as_utf16(text, u16::to_le_bytes)
    }

    /// `lines` as UTF-16 between two code units that are not ASCII: the shape binary data takes
    /// when it happens to carry a NUL-padded run of text. `to_bytes` writes the byte order.
    fn between_binary_noise(lines: &str, to_bytes: fn(u16) -> [u8; 2]) -> Vec<u8> {
        const NOISE: [u8; 2] = [0x89, 0x50];
        let mut window = NOISE.to_vec();
        window.extend(as_utf16("\n", to_bytes));
        window.extend(as_utf16(lines, to_bytes));
        window.extend(NOISE);
        window
    }

    /// A window holding `line` twice and nothing else that reads, each ended with `ending`: the
    /// shape both halves of the count of two are asked on — the lines that say nothing and the
    /// lines that say something are the same window with a different line in it.
    ///
    /// The byte order is named rather than taken, as [`as_utf16le`] names it: the answer is the
    /// same in either order, and these cases weigh the line in the one `iconv -t UTF-16LE`
    /// writes.
    fn two_lines_le(line: &str, ending: &str) -> Vec<u8> {
        between_binary_noise(&format!("{line}{ending}{line}{ending}"), u16::to_le_bytes)
    }

    /// The lines of `lines` as text. The list is weighed against `how_many` — the length it is
    /// expected to have — before any of its lines is read.
    ///
    /// The cases handing a list in differ in that list and in nothing else, and every list
    /// answers the same on a bare line, so one list handed in twice would leave another unasked
    /// and the set green. The lists are of different lengths, so naming the length catches that.
    /// A length of none is refused outright: a list that turned up empty would ask nothing at
    /// all and pass, which is what naming the length exists to catch.
    fn weighed_list(lines: impl Iterator<Item = &'static [u8]>, how_many: usize) -> Vec<String> {
        assert!(how_many > 0, "the length handed in");

        let lines: Vec<_> = lines
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect();
        assert_eq!(lines.len(), how_many, "the list handed in");
        lines
    }

    /// Every line of `lines`, written twice and nothing else, says nothing about the stream it
    /// was read out of: two lines that were never read are two lines binary data holds as
    /// readily as a patch does.
    ///
    /// Asked with each of [`LINE_ENDINGS`], since every rule these lists are weighed by reads
    /// the tail of the line, and a carriage return standing there is what a patch written under
    /// Windows leaves. Asked of two lines rather than one, since a single line is refused by the
    /// count whatever the reading of its tail says. The list itself is weighed by [`weighed_list`].
    fn two_bare_lines_say_nothing(lines: impl Iterator<Item = &'static [u8]>, how_many: usize) {
        let lines = weighed_list(lines, how_many);

        for ending in LINE_ENDINGS {
            for line in &lines {
                let window = two_lines_le(line, ending);
                assert_eq!(
                    utf16_without_bom(&window),
                    None,
                    "two `{line}` and {ending:?}"
                );
            }
        }
    }

    /// Every line of `lines` with each of `tails` written past it, twice and nothing else, names
    /// the stream: a line carrying what stands after it is a line that was read, and two of them
    /// are the count. The other half of [`two_bare_lines_say_nothing`], asked the same way and
    /// weighing its list the same way.
    ///
    /// A tail stands for the content of a real patch, which the rule does not weigh: what is
    /// required past the header is something that is not whitespace, so a single character says
    /// as much as an object name or a mode does. Where the content is read further — a path,
    /// which is required to be in the line — the tails say what may stand there. No tails at all
    /// would ask nothing, for the reason the length of the list is weighed, so a tail is
    /// required.
    fn two_lines_carrying_a_tail_name_the_encoding(
        lines: impl Iterator<Item = &'static [u8]>,
        how_many: usize,
        tails: &[&str],
    ) {
        let lines = weighed_list(lines, how_many);
        assert!(!tails.is_empty(), "the tails handed in");

        for ending in LINE_ENDINGS {
            for line in &lines {
                for tail in tails {
                    let window = two_lines_le(&format!("{line}{tail}"), ending);
                    assert_eq!(
                        utf16_without_bom(&window),
                        Some("UTF-16LE"),
                        "two `{line}{tail}` and {ending:?}"
                    );
                }
            }
        }
    }

    /// One diff line is what binary data holds by accident, whichever marker opens it. A rule
    /// that accepts a stream on the strength of a single line answers a PNG with an `iconv` for
    /// an encoding it is not in.
    ///
    /// Written with each of [`LINE_ENDINGS`] to keep the shape of its neighbours, not because
    /// the ending decides: what refuses this window is the count of two.
    #[test]
    fn a_single_diff_line_inside_binary_data_is_not_called_utf16() {
        for ending in LINE_ENDINGS {
            for (_, line) in MARKER_LINES {
                let window = between_binary_noise(&format!("{line}{ending}"), u16::to_le_bytes);
                assert_eq!(
                    utf16_without_bom(&window),
                    None,
                    "a lone `{line}` and {ending:?}"
                );
            }
        }
    }

    /// A line that is the marker and nothing else says nothing beyond the marker, and two such
    /// lines say it twice. Counting lines rather than reading them accepts exactly this.
    ///
    /// What the line ending has to do with it is stated where the window is built, at
    /// [`two_bare_lines_say_nothing`]: counting a carriage return as content past the marker
    /// would let every bare marker line through.
    #[test]
    fn bare_marker_lines_are_not_called_utf16() {
        two_bare_lines_say_nothing(parser::markers(), MARKER_LINES.len());
    }

    /// The edge of the count, stated so that moving it is a decision rather than an accident: a
    /// patch whose paths are not ASCII, written without the `a/` `b/` prefixes and without an
    /// `index ` line, has one line left to read. Its `---` and `+++` are read no further than
    /// the unit where the path changes script, which leaves them bare, and the hunk header is
    /// all that carries what it promises. One line is what binary data holds by accident, so
    /// the stream is answered as binary input — the cost of the count, paid by a patch git
    /// writes this way only when told to.
    ///
    /// Written with each of [`LINE_ENDINGS`].
    #[test]
    fn a_utf16_patch_with_one_line_left_to_read_is_not_named() {
        for ending in LINE_ENDINGS {
            let diff = format!(
                "--- \u{444}{ending}+++ \u{444}{ending}\
                 @@ -1 +1 @@{ending}-a{ending}+b{ending}"
            );

            let utf16 = as_utf16le(&diff);
            assert_eq!(utf16_without_bom(&utf16), None, "{ending:?}");
        }
    }

    /// Two headers that carry nothing are two lines that were never read, and binary data holds
    /// a NUL-padded `index ` as readily as a patch does. Counting lines is evidence only where
    /// each line counted said something.
    ///
    /// Asked of every header the parser reads that way rather than of the three that were
    /// written out here: the answer is the same for all of them, which is exactly what let a
    /// corpus naming three keep passing while a header the parser learned went untried. The
    /// window these headers are weighed in is built by [`two_bare_lines_say_nothing`].
    #[test]
    fn bare_extended_headers_are_not_called_utf16() {
        two_bare_lines_say_nothing(parser::headers_ascii_follows(), HEADERS_ASCII_FOLLOWS_LEN);
    }

    /// The other half for the same headers: one carrying what stands after it counts, and two of
    /// them name the stream. Asked by traversing the list, as the bare half is — the half that
    /// passes stood on one header written out by hand, so a header the parser learnt would have
    /// been weighed bare and never in company. What the tail stands for is written at
    /// [`two_lines_carrying_a_tail_name_the_encoding`].
    #[test]
    fn extended_headers_carrying_something_are_named_by_their_encoding() {
        two_lines_carrying_a_tail_name_the_encoding(
            parser::headers_ascii_follows(),
            HEADERS_ASCII_FOLLOWS_LEN,
            &["1"],
        );
    }

    /// A header a path follows is read on its own head by the parser, which is handed the line
    /// already read; here the line is still whole, so the path is required to stand in it. Two
    /// bare headers are two lines that name nothing, and binary data holds a NUL-padded
    /// `rename from ` as readily as a patch does.
    ///
    /// What the path is required to be is something past the header that is not whitespace, so
    /// the carriage return [`two_bare_lines_say_nothing`] writes into the tail is what this case
    /// turns on: read as a unit like any other, that return is a path of one character.
    #[test]
    fn bare_headers_a_path_follows_are_not_called_utf16() {
        two_bare_lines_say_nothing(parser::headers_a_path_follows(), HEADERS_A_PATH_FOLLOWS_LEN);
    }

    /// The other half of the same rule, so that requiring the path cannot be tightened into
    /// refusing the patch it was written for: where the path stands in the line, the header
    /// counts, whatever script the path is in. These two lines are all a rename of a path in
    /// another script has past its `diff --git` line, and the count of two rests on them.
    ///
    /// Asked with the carriage return in the tail as well as without it, by
    /// [`two_lines_carrying_a_tail_name_the_encoding`]: a rule refusing what stands before that
    /// return would refuse this patch entirely.
    #[test]
    fn headers_a_path_follows_are_named_by_their_encoding_where_the_path_stands_in_the_line() {
        two_lines_carrying_a_tail_name_the_encoding(
            parser::headers_a_path_follows(),
            HEADERS_A_PATH_FOLLOWS_LEN,
            &["\u{444}", "f"],
        );
    }

    /// A diff writes its lines in company: whatever marker opens an entry, another line of the
    /// diff follows it. That second line is what a patch has and binary data does not, and it
    /// is read the same way in both byte orders.
    ///
    /// Written with each of [`LINE_ENDINGS`].
    #[test]
    fn a_diff_line_in_company_is_named_by_its_encoding() {
        for ending in LINE_ENDINGS {
            for (_, line) in MARKER_LINES {
                let lines = format!("{line}{ending}index 111..222 100644{ending}");
                for (to_bytes, encoding) in [
                    (u16::to_le_bytes as fn(u16) -> [u8; 2], "UTF-16LE"),
                    (u16::to_be_bytes as fn(u16) -> [u8; 2], "UTF-16BE"),
                ] {
                    let window = between_binary_noise(&lines, to_bytes);
                    assert_eq!(
                        utf16_without_bom(&window),
                        Some(encoding),
                        "`{line}` with a header, {encoding} and {ending:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn read_limited_rejects_oversized_input() {
        let data = b"0123456789";
        let err = read_limited(&data[..], 4).unwrap_err();
        assert!(matches!(err, AppError::Usage(_)));
    }
}
