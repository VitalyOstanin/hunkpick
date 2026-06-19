// End-to-end integration tests that run hunkpick against a real git repository
// to verify that selected sub-hunks actually apply cleanly.

use assert_cmd::Command;
use std::process::Command as Sys;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Initialise a git repo in a temp directory with the given files committed.
fn repo_with(old_files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    sys(&dir, &["init", "-q"]);
    sys(&dir, &["config", "user.email", "t@t"]);
    sys(&dir, &["config", "user.name", "t"]);
    for (p, c) in old_files {
        let full = dir.path().join(p);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, c).unwrap();
    }
    sys(&dir, &["add", "."]);
    sys(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn sys(dir: &TempDir, args: &[&str]) {
    let ok = Sys::new("git")
        .args(args)
        .current_dir(dir.path())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Write new file contents then capture `git diff`; returns the diff text.
fn diff_after(dir: &TempDir, new_files: &[(&str, &str)]) -> String {
    for (p, c) in new_files {
        std::fs::write(dir.path().join(p), c).unwrap();
    }
    let out = Sys::new("git")
        .args(["diff"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}

/// Revert the working tree to the last commit.
fn revert(dir: &TempDir) {
    sys(dir, &["checkout", "--", "."]);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Select a single sub-hunk from a two-change diff and verify it applies
/// against the old working-tree state via `git apply --check`.
#[test]
fn select_subset_applies_to_old_state() {
    let dir = repo_with(&[("f", "a\nb\nc\nd\ne\n")]);
    let diff = diff_after(&dir, &[("f", "a\nB\nc\nD\ne\n")]);
    revert(&dir);

    // Select only sub-hunk 1 (b→B); the old working tree still has the original content.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "1",
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(diff.clone())
        .assert()
        .success();

    // Select both sub-hunks together; they must be non-overlapping and apply together.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "1-2",
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(diff)
        .assert()
        .success();
}

/// Three separate single-line changes; selecting all three at once must apply cleanly.
#[test]
fn select_all_applies() {
    let dir = repo_with(&[("f", "a\nb\nc\nd\ne\nf\ng\n")]);
    // Three separate changes, each separated by at least one context line.
    let diff = diff_after(&dir, &[("f", "a\nB\nc\nD\ne\nF\ng\n")]);
    revert(&dir);

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "1-3",
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(diff)
        .assert()
        .success();
}

/// A diff with a corrupted context line is rejected by git apply --check → exit 70.
#[test]
fn tampered_diff_fails_git_check() {
    let dir = repo_with(&[("f", "a\nb\nc\nd\ne\n")]);
    let diff = diff_after(&dir, &[("f", "a\nB\nc\nD\ne\n")]);
    revert(&dir);

    // Replace the context line " c" with " X" so the patch no longer matches.
    let tampered = diff.replace(" c\n", " X\n");
    assert_ne!(diff, tampered, "tampering must change the diff");

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "1",
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(tampered)
        .assert()
        .failure()
        .code(70);
}
