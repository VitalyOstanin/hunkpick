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
