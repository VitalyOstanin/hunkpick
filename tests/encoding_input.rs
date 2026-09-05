// Tests for the byte-oriented core (encoding-agnostic round-trip) and for
// input validation that rejects non-diff / binary input.

use assert_cmd::assert::Assert;

mod common;

use common::{run, run_ok};

// ---------------------------------------------------------------------------
// Encoding: non-UTF-8 content round-trips byte-for-byte
// ---------------------------------------------------------------------------

/// A diff whose changed lines contain a lone 0xE9 byte (latin-1 'é', invalid as
/// standalone UTF-8). `select 1` must succeed and preserve the exact byte.
#[test]
fn non_utf8_content_round_trips() {
    let mut input = Vec::new();
    input.extend_from_slice(b"--- a/f\n+++ b/f\n@@ -1 +1 @@\n-caf");
    input.push(0xE9);
    input.push(b'\n');
    input.extend_from_slice(b"+CAF");
    input.push(0xE9);
    input.push(b'\n');

    let out = run_ok(&["select", "1"], input);

    // The raw 0xE9 byte must survive into the output (two occurrences: -/+ lines).
    assert_eq!(
        out.iter().filter(|&&b| b == 0xE9).count(),
        2,
        "both 0xE9 bytes must be preserved in output: {out:?}"
    );
}

// ---------------------------------------------------------------------------
// Input validation: reject binary / non-diff, no-op on empty
// ---------------------------------------------------------------------------

/// What `hunkpick list` answers when handed `stream`: exit 2 and a diagnosis on stderr, which
/// the caller states with a predicate — the form the rest of the suite weighs stderr in.
fn the_diagnosis_for(stream: impl AsRef<[u8]>) -> Assert {
    run(&["list"], stream).failure().code(2)
}

/// Binary input containing a NUL byte is rejected with exit code 2, and the diagnosis names the
/// NUL byte: exit 2 alone is what every refusal in this file has, so it does not tell one
/// refusal from another.
#[test]
fn nul_byte_input_exits_2() {
    the_diagnosis_for(vec![0u8, 1, 2, b'h', b'i']).stderr(predicates::str::contains("NUL byte"));
}

/// Plain text with no diff markers at all is rejected with exit code 2, and the diagnosis says
/// what is missing rather than calling the text binary.
#[test]
fn non_diff_text_exits_2() {
    the_diagnosis_for("hello world\nthis is not a diff\n")
        .stderr(predicates::str::contains("no diff markers found"));
}

/// Empty input is a no-op (exit 0, empty output) for `list`.
#[test]
fn empty_input_list_is_noop() {
    run(&["list"], "").success().stdout(predicates::ord::eq(""));
}

/// Empty input is a no-op (exit 0, empty output) for `select`, even with a selector.
#[test]
fn empty_input_select_is_noop() {
    run(&["select", "1"], "")
        .success()
        .stdout(predicates::ord::eq(""));
}

/// Whitespace-only input is treated the same as empty (no-op, exit 0).
#[test]
fn whitespace_only_input_is_noop() {
    run(&["select", "1"], "  \n\t\n")
        .success()
        .stdout(predicates::ord::eq(""));
}

/// A hunk header separated by a non-ASCII space is refused rather than rewritten. git reads such
/// a header as a corrupt patch; before the fix hunkpick parsed it and emitted `@@ -1,3 +1,3 @@`
/// with a plain space, turning a diff git rejects into one git accepts, at exit 0.
///
/// The diagnosis is stated as well as the exit code: every refusal in this file exits 2, so the
/// code alone does not tell one refusal from another.
#[test]
fn a_non_ascii_space_in_the_hunk_header_exits_2() {
    let input = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3\u{a0}+1,3 @@\n a\n-b\n+B\n c\n";

    run(&["select", "*"], input)
        .failure()
        .code(2)
        .stderr(predicates::str::contains("malformed hunk header"));
}

/// A diff whose lines end with `ending`, as UTF-16 in the byte order `to_bytes` writes.
fn a_utf16_diff(ending: &str, to_bytes: fn(u16) -> [u8; 2]) -> Vec<u8> {
    let text = format!(
        "diff --git a/f b/f{ending}--- a/f{ending}+++ b/f{ending}\
         @@ -1 +1 @@{ending}-old{ending}+new{ending}"
    );

    text.encode_utf16().flat_map(to_bytes).collect()
}

/// A UTF-16 stream without a byte-order mark is named as an encoding problem, not reported as
/// binary input: `iconv -t UTF-16LE` and `UnicodeEncoding($false, $false)` both write one, and
/// "binary input" sends the reader looking for a binary file in the pipeline instead.
#[test]
fn a_utf16_diff_without_a_bom_names_the_encoding() {
    the_diagnosis_for(a_utf16_diff("\n", u16::to_le_bytes))
        .stderr(predicates::str::contains("UTF-16LE"));
}

/// The big-endian half of the same case: the NUL bytes fall on the even positions instead.
#[test]
fn a_utf16be_diff_without_a_bom_names_the_encoding() {
    the_diagnosis_for(a_utf16_diff("\n", u16::to_be_bytes))
        .stderr(predicates::str::contains("UTF-16BE"));
}

/// The stream the check exists for, written whole the way PowerShell 5.1 writes it: UTF-16LE
/// without a byte-order mark and a carriage return before every line break — asked of the binary
/// end to end rather than of the reading alone.
///
/// The return does not decide this answer: every line counted here carries a path past its
/// marker, so it would be read the same with the return gone. What weighing the return decides
/// is left to the cases on bare lines, where nothing else stands in the tail.
#[test]
fn a_utf16_diff_written_with_carriage_returns_names_the_encoding() {
    the_diagnosis_for(a_utf16_diff("\r\n", u16::to_le_bytes))
        .stderr(predicates::str::contains("UTF-16LE"));
}

/// Genuinely binary input keeps the binary diagnosis: the UTF-16 heuristic must not claim
/// every stream that happens to carry a NUL byte.
#[test]
fn binary_input_is_still_reported_as_binary() {
    the_diagnosis_for(vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
    ])
    .stderr(predicates::str::contains("binary input"));
}

/// A path in git's quoted form survives the round trip byte for byte, and the selector that
/// names it is matched against its decoded bytes rather than against the quoted spelling.
#[test]
fn a_quoted_path_round_trips_and_is_addressable_by_its_bytes() {
    let input = concat!(
        "diff --git \"a/\\303\\251.txt\" \"b/\\303\\251.txt\"\n",
        "--- \"a/\\303\\251.txt\"\n",
        "+++ \"b/\\303\\251.txt\"\n",
        "@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n",
    );

    let out = run_ok(&["select", "\u{e9}.txt:1"], input);

    assert_eq!(String::from_utf8_lossy(&out), input);
}
