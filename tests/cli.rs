// Integration tests for hunkpick CLI behaviour using inline fixtures.

mod common;

use assert_cmd::Command;
use predicates::prelude::*;

/// A unified diff with two separate single-line changes in one hunk,
/// separated by a context line — produces two auto-split sub-hunks.
const TWO_CHANGE_DIFF: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,5 +1,5 @@
 a
-b
+B
 c
-d
+D
 e
";

// ---------------------------------------------------------------------------
// select tests
// ---------------------------------------------------------------------------

#[test]
fn select_emits_chosen_subhunk_only() {
    // Sub-hunk 1 contains +B; sub-hunk 2 contains +D.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .success()
        .stdout(predicate::str::contains("+B"))
        .stdout(predicate::str::contains("+D").not());

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "2"])
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .success()
        .stdout(predicate::str::contains("+D"))
        .stdout(predicate::str::contains("+B").not());
}

#[test]
fn select_range() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1-2"])
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .success()
        .stdout(predicate::str::contains("+B"))
        .stdout(predicate::str::contains("+D"));
}

// ---------------------------------------------------------------------------
// list tests
// ---------------------------------------------------------------------------

#[test]
fn list_human_shows_indices() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .success()
        .stdout(predicate::str::contains("[1]"))
        .stdout(predicate::str::contains("[2]"))
        .stdout(predicate::str::contains("f"));
}

#[test]
fn list_json_is_valid() {
    let output = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--json"])
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_slice(&output).expect("stdout must be valid JSON");
    let files = json.as_array().expect("top-level must be an array");
    assert_eq!(files.len(), 1, "expected one file entry");
    assert_eq!(files[0]["path"], "f");
    let hunks = files[0]["hunks"]
        .as_array()
        .expect("hunks must be an array");
    assert_eq!(hunks.len(), 2, "expected two sub-hunks for file f");
    assert_eq!(hunks[0]["index"], 1);
    assert_eq!(hunks[1]["index"], 2);
}

// ---------------------------------------------------------------------------
// split tests
// ---------------------------------------------------------------------------

#[test]
fn split_replaces_hunk_with_pieces() {
    // New-file line 3 is the context line "c"; cutting there splits the hunk in two.
    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["split", "1", "--at", "3"])
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = std::str::from_utf8(&stdout).unwrap();
    let at_count = text.matches("@@").count();
    // Each @@ appears twice per hunk header line (opening and closing @@), so two hunks = 4.
    // But `@@` also ends the header: count distinct @@ -... +... @@ occurrences instead.
    let hunk_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("@@")).collect();
    assert_eq!(
        hunk_lines.len(),
        2,
        "expected 2 @@ hunk header lines, got: {at_count}"
    );
}

// ---------------------------------------------------------------------------
// error / validation tests
// ---------------------------------------------------------------------------

#[test]
fn bad_selector_exits_2() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "nope:x"])
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .failure()
        .code(2);
}

#[test]
fn empty_selection_exits_2() {
    // No selectors → EmptySelection → Usage → exit 2.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("select")
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .failure()
        .code(2);
}

#[test]
fn out_of_range_index_exits_2() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "9"])
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .failure()
        .code(2);
}

#[test]
fn dash_c_requires_git_flag() {
    // clap: -C requires --verify-result-diff-git.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1", "-C", "."])
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("--verify-result-diff-git"));
}

#[test]
fn no_verify_internal_flag_accepted() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1", "--no-verify-result-diff-internal"])
        .write_stdin(TWO_CHANGE_DIFF)
        .assert()
        .success()
        .stdout(predicate::str::contains("+B"));
}

// ---------------------------------------------------------------------------
// changed-line selector (INDEX@L<set>) end-to-end tests
// ---------------------------------------------------------------------------

/// A file-creation diff: four added lines, one atomic addition-only sub-hunk.
const NEW_FILE_DIFF: &str = "\
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,4 @@
+l1
+l2
+l3
+l4
";

#[test]
fn select_changed_lines_first_part() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1@L1,2"])
        .write_stdin(NEW_FILE_DIFF)
        .assert()
        .success()
        .stdout(predicate::str::contains("+l1"))
        .stdout(predicate::str::contains("+l2"))
        .stdout(predicate::str::contains("+l3").not())
        .stdout(predicate::str::contains("+l4").not());
}

#[test]
fn select_changed_lines_out_of_range_is_usage_error() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1@L1-99"])
        .write_stdin(NEW_FILE_DIFF)
        .assert()
        .failure()
        .stderr(predicate::str::contains("out of range"));
}

#[test]
fn removed_lo_hi_range_form_is_friendly_usage_error() {
    // The old `@lo-hi` added-line range form was removed. Using it must fail with exit 2 and a
    // message that steers the caller to `@L`, not a bare "bad selector".
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1@1-2"])
        .write_stdin(NEW_FILE_DIFF)
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("@lo-hi"))
        .stderr(predicate::str::contains("@L"));
}

#[test]
fn changed_lines_split_new_file_first_part_stages_only_those_lines() {
    let dir = common::repo_with(&[]); // empty initial commit
    std::fs::write(dir.path().join("new.txt"), "l1\nl2\nl3\nl4\n").unwrap();
    common::sys(&dir, &["add", "-N", "new.txt"]); // intent-to-add: diff shows file creation
    let diff = {
        let out = std::process::Command::new("git")
            .args(["diff"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };

    let part1 = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1@L1,2"])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let mut apply = std::process::Command::new("git")
        .args(["apply", "--cached"])
        .current_dir(dir.path())
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    apply.stdin.take().unwrap().write_all(&part1).unwrap();
    assert!(apply.wait().unwrap().success(), "first apply failed");

    let staged = common::diff_staged(&dir);
    assert!(
        staged.contains("+l1") && staged.contains("+l2"),
        "staged: {staged}"
    );
    assert!(!staged.contains("+l3"), "l3 must not be staged: {staged}");
}

#[test]
fn select_whole_and_lineset_of_same_subhunk_exits_2() {
    // A whole sub-hunk plus an `@L` subset of the same sub-hunk is a selector error (exit 2),
    // reported before emission. `--no-verify-result-diff-internal` disables only the result-diff
    // self-check; it must NOT turn this into a silent success that emits a corrupt diff.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "--no-verify-result-diff-internal", "1", "1@L1,2"])
        .write_stdin(NEW_FILE_DIFF)
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("sub-hunk 1"));
}
