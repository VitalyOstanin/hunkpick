//! Test-only helpers for the throwaway git repositories the unit tests apply diffs in.
//!
//! Several modules check their output by feeding it to `git apply --check` against a seeded
//! working tree. Keeping the repository setup here means the tests state what they assert, not
//! how a temporary repository is built, and the isolation from the developer's own git
//! configuration is applied in one place.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// A `git` invocation in `dir`, insulated from the ambient git configuration.
///
/// The global and system configuration files are pointed at paths that do not exist (git reads
/// a missing file as empty), and the repository-locating variables are dropped, so a developer's
/// own settings cannot change what a test sees.
fn git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir);
    cmd.env("GIT_CONFIG_GLOBAL", dir.join("absent-global-gitconfig"));
    cmd.env("GIT_CONFIG_SYSTEM", dir.join("absent-system-gitconfig"));
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_CEILING_DIRECTORIES",
    ] {
        cmd.env_remove(var);
    }
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
pub(crate) fn apply_check(diff_bytes: &[u8], dir: &Path) -> bool {
    let mut child = git(dir)
        .arg("apply")
        .arg("--check")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(diff_bytes).unwrap();
    child.wait().unwrap().success()
}

/// True if `diff_bytes` applies to a file `f` seeded with `content` in a fresh repository.
pub(crate) fn applies_to_file(diff_bytes: &[u8], content: &str) -> bool {
    let dir = repo_with_file(content);
    apply_check(diff_bytes, dir.path())
}
