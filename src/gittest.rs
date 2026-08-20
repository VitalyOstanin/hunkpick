//! Test-only helpers for the throwaway git repositories the unit tests apply diffs in.
//!
//! Several modules check their output by feeding it to `git apply --check` against a seeded
//! working tree. Keeping the repository setup here means the tests state what they assert, not
//! how a temporary repository is built, and the isolation from the developer's own git
//! configuration is applied in one place.

use std::path::Path;
use std::process::Command;

/// A `git` invocation in `dir`, insulated from the ambient git configuration: neither the
/// developer's settings nor the variables that point git at another repository reach it.
fn git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir);
    crate::gitenv::insulate_config(&mut cmd, dir);
    crate::gitenv::insulate_repo_location(&mut cmd);
    cmd
}

/// A fresh git repository whose working tree holds a single file `f` with `content`.
pub(crate) fn repo_with_file(content: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f"), content).unwrap();
    let status = git(dir.path()).arg("init").arg("-q").status().unwrap();
    assert!(status.success(), "git init failed in {:?}", dir.path());
    dir
}

/// True if `diff_bytes` applies to the working tree in `dir` (`git apply --check`).
///
/// Runs through [`crate::gitenv::feed_and_wait`], as [`crate::validate::validate_with_git`]
/// does: a rejected diff is precisely what this helper is asked about, and git reports such a
/// diff line by line while it is still arriving.
pub(crate) fn apply_check(diff_bytes: &[u8], dir: &Path) -> bool {
    let mut cmd = git(dir);
    cmd.arg("apply").arg("--check");
    // An environment failure — a full disk, a git that will not start — is not the answer this
    // helper was asked for, and reporting it as "the diff does not apply" would send the reader
    // of a failing test after the diff.
    crate::gitenv::feed_and_wait(&mut cmd, diff_bytes)
        .unwrap_or_else(|e| panic!("git apply --check in {}: {e}", dir.display()))
        .status
        .success()
}

/// True if `diff_bytes` applies to a file `f` seeded with `content` in a fresh repository.
pub(crate) fn applies_to_file(diff_bytes: &[u8], content: &str) -> bool {
    let dir = repo_with_file(content);
    apply_check(diff_bytes, dir.path())
}
