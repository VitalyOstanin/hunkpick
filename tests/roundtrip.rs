// End-to-end integration tests that run hunkpick against a real git repository
// to verify that selected sub-hunks actually apply cleanly.

mod common;

use assert_cmd::Command;
use common::{diff_after, repo_with, revert};

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

/// A diff whose empty context line lost its trailing space in transport (mail clients and
/// paste buffers routinely strip it) must still be parsed in full: every change stays
/// addressable and the emitted result applies.
#[test]
fn context_line_stripped_of_its_trailing_space_still_applies() {
    let dir = repo_with(&[("f", "a\nb\nc\nd\n\nx\n")]);
    let diff = diff_after(&dir, &[("f", "a\nB\nc\nD\n\nX\n")]);
    revert(&dir);

    // The context line for the empty source line is a lone space; strip it.
    let stripped = diff.replace("\n \n", "\n\n");
    assert_ne!(diff, stripped, "input must actually lose the space marker");

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "*",
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(stripped.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        text.contains("-x\n+X\n"),
        "the change after the empty line must survive: {text}"
    );

    // All three changes stay addressable in the listing.
    let listed = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--json"])
        .write_stdin(stripped)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let listed = String::from_utf8(listed).unwrap();
    assert_eq!(
        listed.matches("\"index\"").count(),
        3,
        "three sub-hunks expected: {listed}"
    );
}

/// A blank line between two hunks (pasted diffs and mail transports add them) must not be
/// emitted ahead of the first hunk: git rejects such a reordered diff as garbage.
#[test]
fn blank_line_between_hunks_does_not_break_the_result() {
    let before: String = (1..=30).map(|i| format!("l{i}\n")).collect();
    let after: String = (1..=30)
        .map(|i| match i {
            2 => "CHANGED2\n".to_string(),
            25 => "CHANGED25\n".to_string(),
            _ => format!("l{i}\n"),
        })
        .collect();
    let dir = repo_with(&[("f", &before)]);
    let diff = diff_after(&dir, &[("f", &after)]);
    revert(&dir);
    let hunks = diff.lines().filter(|l| l.starts_with("@@ ")).count();
    assert_eq!(hunks, 2, "two hunks expected");

    // Insert a blank line right before the second hunk header.
    let cut = diff.rfind("@@ -").unwrap();
    let with_blank = format!("{}\n{}", &diff[..cut], &diff[cut..]);

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "*",
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(with_blank)
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

/// `-C DIR` must stay the only thing that selects the repository for the git check, whatever
/// git variables the caller had set. As it stands `git apply --check` reads the working tree
/// and ignores them; this pins that the check does not start depending on the environment.
#[test]
fn git_check_ignores_inherited_git_dir() {
    let target = repo_with(&[("f", "a\nb\nc\n")]);
    let diff = diff_after(&target, &[("f", "a\nB\nc\n")]);
    revert(&target);

    // A second, unrelated repository whose content the diff does not match.
    let other = repo_with(&[("other.txt", "x\n")]);

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "1",
            "--verify-result-diff-git",
            "-C",
            target.path().to_str().unwrap(),
        ])
        .env("GIT_DIR", other.path().join(".git").to_str().unwrap())
        .env("GIT_WORK_TREE", other.path().to_str().unwrap())
        .write_stdin(diff)
        .assert()
        .success();
}

/// A diff whose last sub-hunk deletes the tail of the file: the deletion carries no new-side
/// lines, so its header number is the last line the previous sub-hunk produced. Comparing the
/// raw header numbers reads that as an overlap and rejects a diff git itself writes, so this
/// pins that `select '*'` emits it and that git applies the result.
#[test]
fn a_trailing_deletion_does_not_read_as_an_overlap() {
    let dir = repo_with(&[("f", "a\nb\nc\nd\ne\n")]);
    let diff = diff_after(&dir, &[("f", "a\nc\n")]);
    revert(&dir);

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "*",
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(diff)
        .assert()
        .success();
}
