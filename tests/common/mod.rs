// Shared helpers for integration tests that work with real git repositories.
// Not every test binary uses every helper, so suppress per-binary dead-code warnings.
#![allow(dead_code)]

use assert_cmd::Command as Cli;
use std::io::Write;
use std::process::{Command as Sys, Stdio};
use tempfile::TempDir;

/// A `git` invocation in `dir`, insulated from the ambient git configuration.
///
/// The global and system configuration files are pointed at paths that do not exist (git reads
/// a missing file as empty) and the repository-locating variables are dropped, so a developer's
/// own settings — `diff.noprefix`, `core.autocrlf`, `diff.mnemonicPrefix` and the like — cannot
/// change the diffs these tests assert on.
pub fn git(dir: &TempDir) -> Sys {
    let mut cmd = Sys::new("git");
    cmd.current_dir(dir.path());
    cmd.env(
        "GIT_CONFIG_GLOBAL",
        dir.path().join("absent-global-gitconfig"),
    );
    cmd.env(
        "GIT_CONFIG_SYSTEM",
        dir.path().join("absent-system-gitconfig"),
    );
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

/// Initialise a git repo in a temp directory with the given files committed.
/// Pass an empty slice to create a repo with only an empty initial commit.
pub fn repo_with(old_files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    sys(&dir, &["init", "-q"]);
    sys(&dir, &["config", "user.email", "t@t"]);
    sys(&dir, &["config", "user.name", "t"]);
    // Pin line-ending handling so committed content and `git diff` output are
    // byte-identical across platforms. GitHub's Windows runners set
    // `core.autocrlf=true` globally, which would rewrite LF→CRLF on add and
    // perturb the diffs these tests assert on. `core.filemode` is left at the
    // platform default: the mode-change test relies on it (Unix-only), and
    // Windows reports false regardless.
    sys(&dir, &["config", "core.autocrlf", "false"]);
    if old_files.is_empty() {
        sys(&dir, &["commit", "-q", "-m", "init", "--allow-empty"]);
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

/// Run a git command in `dir` and assert it succeeded.
pub fn sys(dir: &TempDir, args: &[&str]) {
    let ok = git(dir).args(args).status().unwrap().success();
    assert!(ok, "git {args:?} failed");
}

/// Run a git command in `dir` and return its stdout as text.
pub fn git_output(dir: &TempDir, args: &[&str]) -> String {
    String::from_utf8(git_output_bytes(dir, args)).unwrap()
}

/// Like [`git_output`], for output that need not be valid UTF-8 (a diff over a file whose
/// name holds arbitrary bytes).
pub fn git_output_bytes(dir: &TempDir, args: &[&str]) -> Vec<u8> {
    git(dir).args(args).output().unwrap().stdout
}

/// Write new file contents then capture `git diff`; returns the diff text.
pub fn diff_after(dir: &TempDir, new_files: &[(&str, &str)]) -> String {
    for (p, c) in new_files {
        std::fs::write(dir.path().join(p), c).unwrap();
    }
    git_output(dir, &["diff"])
}

/// Capture `git diff --staged` output.
pub fn diff_staged(dir: &TempDir) -> String {
    git_output(dir, &["diff", "--staged"])
}

/// Revert the working tree to the last commit.
pub fn revert(dir: &TempDir) {
    sys(dir, &["checkout", "--", "."]);
}

/// Stage `diff` into the index of the repo in `dir` (`git apply --cached`).
pub fn apply_cached(dir: &TempDir, diff: &[u8]) {
    let mut child = git(dir)
        .args(["apply", "--cached"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(diff).unwrap();
    assert!(child.wait().unwrap().success(), "git apply --cached failed");
}

/// Run hunkpick with `args` and `stdin`, assert success, return stdout bytes.
pub fn run_ok(args: &[&str], stdin: &str) -> Vec<u8> {
    Cli::cargo_bin("hunkpick")
        .unwrap()
        .args(args)
        .write_stdin(stdin.to_string())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

/// Like [`run_ok`], for the common case of a textual result.
pub fn run_ok_text(args: &[&str], stdin: &str) -> String {
    String::from_utf8(run_ok(args, stdin)).unwrap()
}
