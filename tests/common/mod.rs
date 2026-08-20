// Shared helpers for integration tests that work with real git repositories.
// Not every test binary uses every helper, so suppress per-binary dead-code warnings.
#![allow(dead_code)]

use assert_cmd::Command as Cli;
use std::process::Command as Sys;
use tempfile::TempDir;

/// A `git` invocation in `dir`, insulated from the ambient git configuration: a developer's
/// settings — `diff.noprefix`, `core.autocrlf`, `diff.mnemonicPrefix` and the like — must not
/// change the diffs these tests assert on, and the repository-locating variables must not point
/// git at another tree. Both lists come from the crate, so the tests insulate exactly what the
/// tool does.
pub fn git(dir: &TempDir) -> Sys {
    let mut cmd = Sys::new("git");
    cmd.current_dir(dir.path());
    hunkpick::gitenv::insulate_config(&mut cmd, dir.path());
    hunkpick::gitenv::insulate_repo_location(&mut cmd);
    cmd
}

/// Run `git` in `dir` with `args`, feeding `stdin_bytes` on stdin, and return its output.
/// The feeding is `gitenv::feed_and_wait`, the same one the tool itself uses: these tests
/// generate diffs large enough for git to fill a pipe before the input ends.
pub fn git_with_stdin(dir: &TempDir, args: &[&str], stdin_bytes: &[u8]) -> std::process::Output {
    let mut cmd = git(dir);
    cmd.args(args);
    hunkpick::gitenv::feed_and_wait(&mut cmd, stdin_bytes)
        .unwrap_or_else(|e| panic!("git {args:?} in {}: {e}", dir.path().display()))
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
    apply_diff(dir, &["apply", "--cached"], diff, "git apply --cached");
}

/// Apply `diff` with `args` in `dir`, asserting success and reporting git's own diagnosis —
/// the stderr is what tells a rejected patch apart from an unusable repository.
pub fn apply_diff(dir: &TempDir, args: &[&str], diff: &[u8], what: &str) {
    let out = git_with_stdin(dir, args, diff);
    assert!(
        out.status.success(),
        "{what} failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
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

/// The `list --json` listing of `diff`, parsed. Every caller that reads the listing needs both
/// halves of this — run the binary, parse its stdout — and none of them needs anything else.
pub fn list_json(diff: &str) -> serde_json::Value {
    serde_json::from_slice(&run_ok(&["list", "--json"], diff)).unwrap()
}

/// Like [`run_ok`], for the common case of a textual result.
pub fn run_ok_text(args: &[&str], stdin: &str) -> String {
    String::from_utf8(run_ok(args, stdin)).unwrap()
}

/// Run `hunkpick select` over `diff` with the git check enabled against `dir`'s working tree,
/// and return the assertion so a caller can add expectations of its own.
///
/// The five-argument invocation is what most of these tests are made of; spelled out per test it
/// buries which selector is being exercised under identical scaffolding.
pub fn select_checked(dir: &TempDir, diff: &str, selectors: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Cli::cargo_bin("hunkpick").unwrap();
    cmd.arg("select");
    cmd.args(selectors);
    cmd.args([
        "--verify-result-diff-git",
        "-C",
        dir.path().to_str().unwrap(),
    ]);
    cmd.write_stdin(diff.to_string()).assert()
}

/// [`select_checked`] for the usual case: the selection must succeed. Returns its stdout.
pub fn select_checked_ok(dir: &TempDir, diff: &str, selectors: &[&str]) -> String {
    let out = select_checked(dir, diff, selectors)
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}
