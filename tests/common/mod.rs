// Shared helpers for integration tests that work with real git repositories.

use std::process::Command as Sys;
use tempfile::TempDir;

/// Initialise a git repo in a temp directory with the given files committed.
/// Pass an empty slice to create a repo with only an empty initial commit.
pub fn repo_with(old_files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    sys(&dir, &["init", "-q"]);
    sys(&dir, &["config", "user.email", "t@t"]);
    sys(&dir, &["config", "user.name", "t"]);
    if old_files.is_empty() {
        sys_ok(&dir, &["commit", "-q", "-m", "init", "--allow-empty"]);
    } else {
        for (p, c) in old_files {
            let full = dir.path().join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, c).unwrap();
        }
        sys(&dir, &["add", "."]);
        sys(&dir, &["commit", "-q", "-m", "init"]);
    }
    dir
}

/// Like `sys` but uses allow-empty semantics; used internally.
fn sys_ok(dir: &TempDir, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(dir.path())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?} failed");
}

pub fn sys(dir: &TempDir, args: &[&str]) {
    let ok = Sys::new("git")
        .args(args)
        .current_dir(dir.path())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?} failed");
}

/// Write new file contents then capture `git diff`; returns the diff text.
pub fn diff_after(dir: &TempDir, new_files: &[(&str, &str)]) -> String {
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

/// Capture `git diff --staged` output.
pub fn diff_staged(dir: &TempDir) -> String {
    let out = Sys::new("git")
        .args(["diff", "--staged"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap()
}
