// Integration tests for hunkpick CLI behaviour using inline fixtures.

mod common;

use assert_cmd::Command;
use predicates::prelude::*;

/// A unified diff with two separate single-line changes in one hunk,
/// separated by a context line — produces two auto-split sub-hunks.
const TWO_CHANGES: &str = "\
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
        .write_stdin(TWO_CHANGES)
        .assert()
        .success()
        .stdout(predicate::str::contains("+B"))
        .stdout(predicate::str::contains("+D").not());

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "2"])
        .write_stdin(TWO_CHANGES)
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
        .write_stdin(TWO_CHANGES)
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
        .write_stdin(TWO_CHANGES)
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
        .write_stdin(TWO_CHANGES)
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
        .write_stdin(TWO_CHANGES)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = std::str::from_utf8(&stdout).unwrap();
    // Count header lines, not `@@` occurrences: a header opens and closes with `@@`.
    let hunk_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("@@")).collect();
    assert_eq!(
        hunk_lines.len(),
        2,
        "expected 2 @@ hunk header lines, got: {text}"
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
        .write_stdin(TWO_CHANGES)
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
        .write_stdin(TWO_CHANGES)
        .assert()
        .failure()
        .code(2);
}

#[test]
fn out_of_range_index_exits_2() {
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "9"])
        .write_stdin(TWO_CHANGES)
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
        .write_stdin(TWO_CHANGES)
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
        .write_stdin(TWO_CHANGES)
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

// ---------------------------------------------------------------------------
// input validation
// ---------------------------------------------------------------------------

/// A hunk header that disagrees with its body is a defect of the INPUT diff: it must be
/// reported as a usage error (exit 2) in prose, not as a verification failure of hunkpick's
/// own result (exit 70) with a Debug dump of internal fields.
#[test]
fn inconsistent_input_header_is_a_usage_error() {
    let diff = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
";
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("sub-hunk 1"))
        .stderr(predicate::str::contains("hunk_index").not())
        .stderr(predicate::str::contains("CountMismatch").not());
}

/// Line numbers close to u32::MAX come straight from the input header. Adding them up must
/// not overflow: a debug build panicked with exit 101, outside the documented exit codes.
#[test]
fn huge_line_numbers_do_not_overflow() {
    let diff = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -4294967295,2 +4294967295,2 @@
 a
-b
+B
";
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "@@ -4294967295,2 +4294967295,2 @@",
        ));
}

/// A closed downstream reader (`hunkpick list | head`) is the normal end of a filter's work.
/// It must not be reported as an I/O failure: that breaks `set -o pipefail` pipelines.
#[test]
fn closed_downstream_pipe_is_not_an_error() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command as Sys, Stdio};

    // Larger than a pipe buffer, so the write cannot complete before the reader goes away.
    let mut diff = String::from("diff --git a/f b/f\n--- a/f\n+++ b/f\n");
    for i in 0..4000 {
        diff.push_str(&format!(
            "@@ -{n},1 +{n},1 @@\n-a{i}\n+b{i}\n",
            n = i * 10 + 1
        ));
    }

    let mut child = Sys::new(assert_cmd::cargo::cargo_bin("hunkpick"))
        .arg("list")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(diff.as_bytes());
    });

    // Read one line, then drop the pipe — the child's next write gets EPIPE.
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    drop(reader);

    let out = child.wait_with_output().unwrap();
    let _ = writer.join();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "exit status {:?}, stderr: {stderr}",
        out.status.code()
    );
    assert!(
        !stderr.contains("Broken pipe"),
        "no I/O diagnostic expected: {stderr}"
    );
}

/// Auto-splitting a hunk with many change runs must stay linear in their number. Recomputing
/// the prefix tally per sub-hunk made this quadratic; at 20 000 runs the difference is between
/// a fraction of a second and the test profile's slow-test timeout.
#[test]
fn many_change_runs_split_without_quadratic_blowup() {
    const RUNS: usize = 20_000;
    let mut diff = format!(
        "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,{n} +1,{n} @@\n",
        n = RUNS * 2
    );
    for i in 0..RUNS {
        diff.push_str(&format!(" ctx{i}\n-old{i}\n+new{i}\n"));
    }

    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--json"])
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(stdout).unwrap();
    assert_eq!(
        text.matches("\"index\"").count(),
        RUNS,
        "one sub-hunk per change run"
    );
}

/// `split` and `select` must treat new-side anchors the same way: a diff carved out of a
/// larger one carries anchors of that larger diff, and both commands recompute them.
#[test]
fn split_recomputes_new_side_anchors_like_select() {
    let diff = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -17,5 +18,5 @@
 a
-b
+B
 c
-d
+D
 e
";
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["split", "1", "--at", "20"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains("@@ -17,3 +17,3 @@"))
        .stdout(predicate::str::contains("+18,").not());

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1"])
        .write_stdin(diff)
        .assert()
        .success()
        .stdout(predicate::str::contains("@@ -17,3 +17,3 @@"));
}
