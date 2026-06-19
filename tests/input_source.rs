// Tests for reading the diff from a file (`-i/--input`) and the input size limit
// (`--max-input-bytes`).

use assert_cmd::Command;
use predicates::prelude::*;

const DIFF: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
";

/// `select 1 -i FILE` reads the diff from the file instead of stdin.
#[test]
fn reads_from_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changes.diff");
    std::fs::write(&path, DIFF).unwrap();

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .arg("-i")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("+B"));
}

/// `--input -` explicitly selects stdin.
#[test]
fn dash_input_means_stdin() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1", "--input", "-"])
        .write_stdin(DIFF)
        .assert()
        .success()
        .stdout(predicate::str::contains("+B"));
}

/// Input larger than the configured limit is rejected with exit code 2.
#[test]
fn exceeds_limit_exits_2() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--max-input-bytes", "10"])
        .write_stdin(DIFF) // well over 10 bytes
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("exceeds limit"));
}

/// A limit of 0 disables the size check.
#[test]
fn zero_limit_is_unlimited() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--max-input-bytes", "0"])
        .write_stdin(DIFF)
        .assert()
        .success()
        .stdout(predicate::str::contains("[1]"));
}

/// Input of exactly the limit size is accepted (boundary).
#[test]
fn exactly_limit_is_accepted() {
    let limit = DIFF.len().to_string();
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--max-input-bytes", &limit])
        .write_stdin(DIFF)
        .assert()
        .success();
}

/// A nonexistent input file is an I/O error (exit code 74).
#[test]
fn missing_file_exits_74() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "-i", "/nonexistent/hunkpick/path.diff"])
        .assert()
        .failure()
        .code(74);
}

/// The size limit also applies to file input.
#[test]
fn file_input_respects_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changes.diff");
    std::fs::write(&path, DIFF).unwrap();

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--max-input-bytes", "10"])
        .arg("-i")
        .arg(&path)
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("exceeds limit"));
}
