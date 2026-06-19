// Integration tests for hunkpick CLI behaviour using inline fixtures.

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
