// Integration tests for multi-file diffs: a single `git diff file1 file2 ... fileN`
// produces one diff spanning several files, and `path:` selectors address sub-hunks
// per file. Driven through the CLI against real `git diff` output.

mod common;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

/// Run hunkpick, assert success, return stdout bytes.
fn run_ok(args: &[&str], stdin: &str) -> Vec<u8> {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(args)
        .write_stdin(stdin.to_string())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

/// Content id of the sub-hunk at `path:index` (1-based) from `list --json`.
fn id_of(diff: &str, path: &str, index: u64) -> String {
    let out = run_ok(&["list", "--json"], diff);
    let v: Value = serde_json::from_slice(&out).unwrap();
    for f in v.as_array().unwrap() {
        if f["path"].as_str() == Some(path) {
            for h in f["hunks"].as_array().unwrap() {
                if h["index"].as_u64() == Some(index) {
                    return h["id"].as_str().unwrap().to_string();
                }
            }
        }
    }
    panic!("no sub-hunk {path}:{index} in listing");
}

/// `git diff` over three changed files yields one multi-file diff; selecting sub-hunks
/// from two of the three files emits only those, skips the third, and the combined
/// result applies cleanly via git.
#[test]
fn select_across_three_files_picks_named_only_and_applies() {
    let dir = common::repo_with(&[
        ("src/a.rs", "a\nb\nc\n"),
        ("src/b.rs", "p\nq\nr\n"),
        ("src/c.rs", "x\ny\nz\n"),
    ]);
    // Change all three files -> one diff spanning file1 file2 fileN.
    let diff = common::diff_after(
        &dir,
        &[
            ("src/a.rs", "A\nb\nc\n"),
            ("src/b.rs", "P\nq\nr\n"),
            ("src/c.rs", "X\ny\nz\n"),
        ],
    );
    assert!(
        diff.contains("a/src/a.rs") && diff.contains("a/src/b.rs") && diff.contains("a/src/c.rs"),
        "diff spans all three files:\n{diff}"
    );
    // Revert the working tree so the result can be checked with `git apply`.
    common::sys(&dir, &["checkout", "--", "."]);

    // Output carries only the two named files' changes.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "src/a.rs:1", "src/c.rs:1"])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .stdout(predicate::str::contains("+A"))
        .stdout(predicate::str::contains("+X"))
        .stdout(predicate::str::contains("+P").not())
        .stdout(predicate::str::contains("src/b.rs").not());

    // And the multi-file result applies cleanly via git.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            "src/a.rs:1",
            "src/c.rs:1",
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(diff)
        .assert()
        .success();
}

/// `path:*` addresses every sub-hunk of one file within a multi-file diff, leaving the
/// other files untouched.
#[test]
fn path_star_selects_one_file_of_many() {
    let dir = common::repo_with(&[("src/a.rs", "a\nb\nc\nd\ne\n"), ("src/b.rs", "p\nq\nr\n")]);
    let diff = common::diff_after(
        &dir,
        &[("src/a.rs", "A\nb\nC\nd\ne\n"), ("src/b.rs", "P\nq\nr\n")],
    );

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "src/a.rs:*"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains("+A"))
        .stdout(predicate::str::contains("+C"))
        .stdout(predicate::str::contains("src/b.rs").not());
}

/// Content ids work across a multi-file diff. The file path is part of the id, so the
/// same edit in different files gets different ids; `select @id` therefore addresses the
/// change in its own file and leaves the others untouched, and the result applies.
#[test]
fn select_by_id_addresses_its_own_file_in_multi_file_diff() {
    let dir = common::repo_with(&[
        ("src/a.rs", "a\nb\nc\n"),
        ("src/b.rs", "p\nq\nr\n"),
        ("src/c.rs", "x\ny\nz\n"),
    ]);
    // The same first-line edit in all three files.
    let diff = common::diff_after(
        &dir,
        &[
            ("src/a.rs", "Z\nb\nc\n"),
            ("src/b.rs", "Z\nq\nr\n"),
            ("src/c.rs", "Z\ny\nz\n"),
        ],
    );
    let id_a = id_of(&diff, "src/a.rs", 1);
    let id_c = id_of(&diff, "src/c.rs", 1);
    assert_ne!(
        id_a, id_c,
        "the same edit in different files must get different ids (path is hashed)"
    );
    common::sys(&dir, &["checkout", "--", "."]);

    // @id emits only the c.rs change; a.rs and b.rs are absent from the output.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", &format!("@{id_c}")])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .stdout(predicate::str::contains("src/c.rs"))
        .stdout(predicate::str::contains("src/a.rs").not())
        .stdout(predicate::str::contains("src/b.rs").not());

    // And the single-file result applies cleanly via git.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            &format!("@{id_c}"),
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(diff)
        .assert()
        .success();
}

/// A bare (no `path:`) selector is ambiguous across a multi-file diff and is rejected
/// with exit code 2.
#[test]
fn bare_selector_on_multi_file_diff_is_rejected() {
    let dir = common::repo_with(&[("a", "a\nb\n"), ("b", "p\nq\n")]);
    let diff = common::diff_after(&dir, &[("a", "A\nb\n"), ("b", "P\nq\n")]);

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .failure()
        .code(2);
}
