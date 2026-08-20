// Tests for the byte-oriented core (encoding-agnostic round-trip) and for
// input validation that rejects non-diff / binary input.

use assert_cmd::Command;

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

    let out = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

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

/// Binary input containing a NUL byte is rejected with exit code 2.
#[test]
fn nul_byte_input_exits_2() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(vec![0u8, 1, 2, b'h', b'i'])
        .assert()
        .failure()
        .code(2);
}

/// Plain text with no diff markers at all is rejected with exit code 2.
#[test]
fn non_diff_text_exits_2() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin("hello world\nthis is not a diff\n")
        .assert()
        .failure()
        .code(2);
}

/// Empty input is a no-op (exit 0, empty output) for `list`.
#[test]
fn empty_input_list_is_noop() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin("")
        .assert()
        .success()
        .stdout(predicates::ord::eq(""));
}

/// Empty input is a no-op (exit 0, empty output) for `select`, even with a selector.
#[test]
fn empty_input_select_is_noop() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin("")
        .assert()
        .success()
        .stdout(predicates::ord::eq(""));
}

/// Whitespace-only input is treated the same as empty (no-op, exit 0).
#[test]
fn whitespace_only_input_is_noop() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin("  \n\t\n")
        .assert()
        .success()
        .stdout(predicates::ord::eq(""));
}

/// A hunk header separated by a non-ASCII space is refused rather than rewritten. git reads such
/// a header as a corrupt patch; before the fix hunkpick parsed it and emitted `@@ -1,3 +1,3 @@`
/// with a plain space, turning a diff git rejects into one git accepts, at exit 0.
#[test]
fn a_non_ascii_space_in_the_hunk_header_exits_2() {
    let input = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3\u{a0}+1,3 @@\n a\n-b\n+B\n c\n";

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "*"])
        .write_stdin(input)
        .assert()
        .failure()
        .code(2);
}

/// A UTF-16 stream without a byte-order mark is named as an encoding problem, not reported as
/// binary input: `iconv -t UTF-16LE` and `UnicodeEncoding($false, $false)` both write one, and
/// "binary input" sends the reader looking for a binary file in the pipeline instead.
#[test]
fn a_utf16_diff_without_a_bom_names_the_encoding() {
    let text = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n+new\n";
    let mut utf16 = Vec::new();
    for unit in text.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }

    let assert = Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(utf16)
        .assert()
        .failure()
        .code(2);
    assert.stderr(predicates::str::contains("UTF-16LE"));
}

/// The big-endian half of the same case: the NUL bytes fall on the even positions instead.
#[test]
fn a_utf16be_diff_without_a_bom_names_the_encoding() {
    let text = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n+new\n";
    let mut utf16 = Vec::new();
    for unit in text.encode_utf16() {
        utf16.extend_from_slice(&unit.to_be_bytes());
    }

    let assert = Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(utf16)
        .assert()
        .failure()
        .code(2);
    assert.stderr(predicates::str::contains("UTF-16BE"));
}

/// Genuinely binary input keeps the binary diagnosis: the UTF-16 heuristic must not claim
/// every stream that happens to carry a NUL byte.
#[test]
fn binary_input_is_still_reported_as_binary() {
    let assert = Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
        ])
        .assert()
        .failure()
        .code(2);
    assert.stderr(predicates::str::contains("binary input"));
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

    let out = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "\u{e9}.txt:1"])
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(String::from_utf8_lossy(&out), input);
}
