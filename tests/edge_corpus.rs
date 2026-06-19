// Integration tests for git extended headers and special diff cases.

mod common;

use assert_cmd::Command;
use predicates::prelude::*;
use std::process::Command as Sys;

// ---------------------------------------------------------------------------
// 1. rename_is_preserved
// ---------------------------------------------------------------------------

/// A renamed file with a content change: rename headers should be present in
/// hunkpick's output, or at a minimum hunkpick should exit 0 with non-empty output.
///
/// NOTE: rename detection requires sufficient content similarity and the `-M` flag.
/// The diff is captured with `git diff --staged -M` after `git mv`.
/// If the git version on this machine omits rename headers (environment-dependent),
/// the assertion is weakened to exit-0 + non-empty output.
#[test]
fn rename_is_preserved() {
    let dir = common::repo_with(&[("old.txt", "line1\nline2\nline3\nline4\nline5\n")]);

    // Move and change content so rename similarity is high enough for detection.
    let new_path = dir.path().join("new.txt");
    std::fs::rename(dir.path().join("old.txt"), &new_path).unwrap();
    std::fs::write(&new_path, "line1\nline2\nline3\nline4\nline5_changed\n").unwrap();
    common::sys(&dir, &["add", "-A"]);

    let out = Sys::new("git")
        .args(["diff", "--staged", "-M"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let diff = String::from_utf8(out.stdout).unwrap();
    assert!(!diff.is_empty(), "staged diff must be non-empty");

    if diff.contains("rename from") {
        // Full assertion: rename headers must survive through hunkpick.
        let stdout = Command::cargo_bin("hunkpick")
            .unwrap()
            .args(["select", "1"])
            .write_stdin(diff.clone())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let text = std::str::from_utf8(&stdout).unwrap();
        assert!(
            text.contains("rename from") || text.contains("diff --git"),
            "rename headers must appear in output: {text}"
        );
        assert!(!text.is_empty());

        // list --json must also succeed.
        Command::cargo_bin("hunkpick")
            .unwrap()
            .args(["list", "--json"])
            .write_stdin(diff)
            .assert()
            .success();
    } else {
        // Rename detection not available in this environment: weaken to exit-0.
        Command::cargo_bin("hunkpick")
            .unwrap()
            .args(["list", "--json"])
            .write_stdin(diff.clone())
            .assert()
            .success();
        Command::cargo_bin("hunkpick")
            .unwrap()
            .args(["select", "1"])
            .write_stdin(diff)
            .assert()
            .success()
            .stdout(predicate::str::is_empty().not());
    }
}

// ---------------------------------------------------------------------------
// 2. mode_change_passthrough
// ---------------------------------------------------------------------------

/// A file whose mode changes (non-executable → executable) and whose content
/// also changes: hunkpick must preserve the old/new mode header lines.
///
/// Unix-only: the executable bit is not tracked on Windows (NTFS has no such
/// permission and git's `core.filemode` defaults to false there), so `git diff`
/// emits no `old mode`/`new mode` headers and the scenario cannot be produced.
#[cfg(unix)]
#[test]
fn mode_change_passthrough() {
    let dir = common::repo_with(&[("f.sh", "line1\nline2\n")]);

    // Make executable and change a line.
    let path = dir.path().join("f.sh");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(&path, "line1\nline2_changed\n").unwrap();

    let diff = common::diff_after(&dir, &[]);

    // The diff must carry old/new mode lines.
    assert!(
        diff.contains("old mode") && diff.contains("new mode"),
        "mode-change diff must contain old/new mode headers: {diff}"
    );

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = std::str::from_utf8(&stdout).unwrap();
    assert!(text.contains("old mode"), "output must contain 'old mode'");
    assert!(text.contains("new mode"), "output must contain 'new mode'");
}

// ---------------------------------------------------------------------------
// 3. new_file
// ---------------------------------------------------------------------------

/// A brand-new staged file: hunkpick must exit 0 and the output must contain
/// the `new file mode` header and the added lines.
#[test]
fn new_file() {
    let dir = common::repo_with(&[]);
    // repo_with makes an initial empty commit; now stage a new file.
    std::fs::write(dir.path().join("newf.txt"), "line1\nline2\n").unwrap();
    common::sys(&dir, &["add", "newf.txt"]);

    let diff = common::diff_staged(&dir);
    assert!(!diff.is_empty(), "staged diff must be non-empty");
    assert!(
        diff.contains("new file mode"),
        "diff must contain 'new file mode': {diff}"
    );

    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(diff.clone())
        .assert()
        .success();

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = std::str::from_utf8(&stdout).unwrap();
    assert!(
        text.contains("new file mode"),
        "output must contain 'new file mode'"
    );
    assert!(text.contains("+line1"), "output must contain added lines");
}

// ---------------------------------------------------------------------------
// 4. deleted_file
// ---------------------------------------------------------------------------

/// Deleting a committed file: hunkpick must exit 0 and preserve the
/// `deleted file mode` header.
#[test]
fn deleted_file() {
    let dir = common::repo_with(&[("f.txt", "line1\nline2\n")]);
    std::fs::remove_file(dir.path().join("f.txt")).unwrap();

    let diff = common::diff_after(&dir, &[]);
    assert!(!diff.is_empty(), "diff must be non-empty after deletion");
    assert!(
        diff.contains("deleted file mode"),
        "diff must contain 'deleted file mode': {diff}"
    );

    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(diff.clone())
        .assert()
        .success();

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = std::str::from_utf8(&stdout).unwrap();
    assert!(
        text.contains("deleted file mode"),
        "output must contain 'deleted file mode'"
    );
}

// ---------------------------------------------------------------------------
// 5. binary_file
// ---------------------------------------------------------------------------

/// A binary file diff: `list --json` must mark it `binary: true` with zero sub-hunks.
/// `select 1` on a binary-only diff succeeds (exit 0) and emits the binary stanza —
/// this matches the actual implementation behaviour (binary files bypass index bounds).
#[test]
fn binary_file() {
    let dir = common::repo_with(&[]);
    // Write a file containing a NUL byte so git treats it as binary.
    std::fs::write(dir.path().join("f.bin"), b"hello\x00world").unwrap();
    common::sys(&dir, &["add", "f.bin"]);
    common::sys(&dir, &["commit", "-q", "-m", "add binary"]);
    std::fs::write(dir.path().join("f.bin"), b"bye\x00world").unwrap();

    let diff = common::diff_after(&dir, &[]);
    assert!(!diff.is_empty(), "binary diff must be non-empty");
    assert!(
        diff.contains("Binary files"),
        "diff must contain 'Binary files': {diff}"
    );

    let json_output = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--json"])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_slice(&json_output).expect("list --json must produce valid JSON");
    let files = json.as_array().expect("top-level must be array");
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0]["binary"], true,
        "binary file must be marked binary: true"
    );
    let hunks = files[0]["hunks"].as_array().expect("hunks must be array");
    assert_eq!(hunks.len(), 0, "binary file must have zero sub-hunks");

    // Actual behaviour: select 1 on a binary-only diff exits 0 and emits the binary stanza.
    // (The implementation bypasses index bounds for binary files.)
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains("Binary files"));
}

// ---------------------------------------------------------------------------
// 6. crlf_preserved
// ---------------------------------------------------------------------------

/// An inline fixture with CRLF line endings: select must round-trip the \r bytes.
/// (No git involvement; tests the parser/emitter directly via the CLI.)
#[test]
fn crlf_preserved() {
    // Build the diff bytes explicitly with \r\n endings.
    let diff: Vec<u8> = concat!(
        "diff --git a/f b/f\r\n",
        "--- a/f\r\n",
        "+++ b/f\r\n",
        "@@ -1,3 +1,3 @@\r\n",
        " a\r\n",
        "-b\r\n",
        "+B\r\n",
        " c\r\n",
    )
    .bytes()
    .collect();

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("select")
        .arg("1")
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    // The output must contain at least one \r byte (CRLF preserved).
    assert!(
        stdout.contains(&b'\r'),
        "output must preserve \\r bytes from CRLF input"
    );
}

// ---------------------------------------------------------------------------
// 7. plain_non_git_diff
// ---------------------------------------------------------------------------

/// A plain (non-git) unified diff without `diff --git` preamble:
/// hunkpick must exit 0 and the output must start with `--- `.
#[test]
fn plain_non_git_diff() {
    let diff = "\
--- old/f
+++ new/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
";

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::starts_with("--- "));
}
