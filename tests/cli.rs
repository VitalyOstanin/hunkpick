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
    let diff = common::git_output(&dir, &["diff"]);

    let part1 = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1@L1,2"])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    common::apply_cached(&dir, &part1);

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

/// The "no final newline" flag belongs to one line of the input — its last — and so to one file:
/// the last one. A selection that emits only an earlier file ends somewhere else entirely, and
/// carrying the flag there would truncate a line nobody asked to change. The flag used to live in
/// a variable the emit loop overwrote per file, correct only because the last iteration wrote
/// last; this holds the property itself.
#[test]
fn only_a_selection_reaching_the_last_file_inherits_the_missing_newline() {
    let diff = concat!(
        "diff --git a/a b/a\n",
        "--- a/a\n",
        "+++ b/a\n",
        "@@ -1,3 +1,3 @@\n",
        " x\n",
        "-y\n",
        "+Y\n",
        " z\n",
        "diff --git a/b b/b\n",
        "--- a/b\n",
        "+++ b/b\n",
        "@@ -1,2 +1,2 @@\n",
        " p\n",
        "-q\n",
        "+Q", // the input ends here, without a newline
    );

    let from_the_first = common::run_ok_text(&["select", "a:1"], diff);
    assert!(
        from_the_first.ends_with('\n'),
        "a selection that stops before the last file ends on a terminated line: {from_the_first:?}"
    );

    let from_the_last = common::run_ok_text(&["select", "b:1"], diff);
    assert!(
        !from_the_last.ends_with('\n'),
        "a selection reaching the last line of the input reproduces its ending: {from_the_last:?}"
    );

    // Both files: the result still ends where the input did.
    let both = common::run_ok_text(&["select", "a:1", "b:1"], diff);
    assert!(
        !both.ends_with('\n'),
        "the whole selection ends where the input did: {both:?}"
    );
}

/// A device that cannot take the output is an I/O failure, and has to be reported as one: the
/// caller of `hunkpick select ... > file` on a full filesystem must not get a truncated file at
/// exit 0. `/dev/full` accepts an `open` and answers every write with ENOSPC, which is that
/// situation without needing a filesystem to fill.
///
/// The explicit flush at the end of `main` exists for the same contract from the other side —
/// the buffered tail of a result diff that ends without a newline. That path is not reachable
/// here: `LineWriter` writes through as far as the last newline, so a device failing every write
/// fails one of those first, and a closed reader is met by `write_out` for the same reason.
#[test]
#[cfg(unix)]
fn output_that_cannot_be_written_is_reported_not_swallowed() {
    use std::process::{Command as Sys, Stdio};

    let Ok(full) = std::fs::OpenOptions::new().write(true).open("/dev/full") else {
        // Not a Linux runner: the device is absent, and nothing here can be asserted.
        return;
    };

    let out = Sys::new(assert_cmd::cargo::cargo_bin("hunkpick"))
        .args(["select", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(TWO_CHANGES.as_bytes())?;
            child.wait_with_output()
        })
        .unwrap();

    assert_eq!(out.status.code(), Some(74), "an I/O failure exits 74");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No space left") || stderr.contains("os error 28"),
        "the diagnostic must name what went wrong: {stderr}"
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

    let started = std::time::Instant::now();
    let stdout = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--json"])
        .write_stdin(diff)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let elapsed = started.elapsed();
    let text = String::from_utf8(stdout).unwrap();
    assert_eq!(
        text.matches("\"index\"").count(),
        RUNS,
        "one sub-hunk per change run"
    );
    // The nextest profile's slow-test timeout is the usual guard, but the documented fallback
    // (`cargo test -- --test-threads=4`) has none: a quadratic regression would hang there
    // instead of failing. A linear split of this input takes a fraction of a second even in a
    // debug build, so a minute is unreachable without a change in complexity, and loaded
    // machines do not trip it.
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "listing {RUNS} change runs took {elapsed:?}; the split is no longer linear"
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

/// `split` replaces one hunk with several, so every line recorded after a later hunk moves down
/// by the pieces the split added. Without that shift the `-- ` signature `git format-patch`
/// writes after the last hunk lands between the pieces, and `git apply` rejects the result with
/// `patch fragment without header` while hunkpick itself still exits 0.
#[test]
fn split_keeps_trailing_lines_after_the_last_hunk() {
    let diff = "\
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
-- 
2.53.0
";
    // New-file line numbers: a=1 B=2 c=3 D=4 e=5. Cutting at context line 3 yields two pieces.
    let out = common::run_ok_text(&["split", "1", "--at", "3"], diff);
    assert_eq!(out.matches("@@ -").count(), 2, "two pieces expected: {out}");
    let last_hunk = out.rfind("@@ -").expect("hunk header in output");
    let signature = out.find("\n-- \n").expect("signature in output");
    assert!(
        signature > last_hunk,
        "the signature must stay after the last piece: {out}"
    );

    let dir = common::repo_with(&[("f", "a\nb\nc\nd\ne\n")]);
    common::apply_cached(&dir, out.as_bytes());
}

/// A diff that arrived without its final newline keeps that property only when the result still
/// ends on the same line. `select` was taught this; `split` was not, and a cut that drops the
/// trailing context ends the output on a line that is not the input's last — with its newline
/// removed, so the line is truncated. hunkpick's own check does not see it (it compares counts,
/// order and anchors, not the bytes after the last hunk) and exits 0, while `git apply` reads
/// the result as a corrupt patch.
#[test]
fn split_that_drops_the_tail_keeps_the_final_newline() {
    // No newline after ` e`: the input ends mid-line, and ` e` is the line it ends on.
    let diff = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,5 +1,5 @@
 a
-b
+B
 c
 d
 e";
    // New-file line numbers: a=1 B=2 c=3 d=4 e=5. Cutting at context line 3 leaves one piece
    // with changes; the piece holding ` d` and ` e` has none and is dropped, so the output no
    // longer reaches the input's last line.
    let out = common::run_ok(&["split", "1", "--at", "3"], diff);
    assert!(
        !out.ends_with(b" e"),
        "the tail was dropped, so the output cannot end on the input's last line"
    );
    assert_eq!(
        out.last(),
        Some(&b'\n'),
        "the output ends on a different line, so it keeps its newline: {}",
        String::from_utf8_lossy(&out)
    );
}

/// The other half of the same rule: when the cut keeps the input's last line, the missing final
/// newline is the input's own and has to be reproduced. Dropping the newline is byte-for-byte
/// fidelity here, not truncation.
#[test]
fn split_that_keeps_the_tail_reproduces_the_missing_newline() {
    // Two changes, one on either side of the cut, so both pieces survive and the last piece
    // carries ` e` — the line the input ends on.
    let diff = "\
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
 e";
    let out = common::run_ok(&["split", "1", "--at", "3"], diff);
    assert!(
        out.ends_with(b" e"),
        "both pieces survive, so the output reaches the input's last line: {}",
        String::from_utf8_lossy(&out)
    );
}

/// The mirror-image decision on the `select` side, recorded rather than changed: an `@L` cut of
/// the last sub-hunk is treated as not reaching the input's last line even when it keeps it, so
/// the missing final newline is not reproduced and the output gets one. The error leans the safe
/// way — an extra newline makes the result more applicable, not less — but it is a deliberate
/// approximation of `emit`'s byte-for-byte promise, and a test is what tells the next reader that.
#[test]
fn a_line_slice_of_the_last_sub_hunk_gets_a_final_newline_it_did_not_have() {
    // Two hunks, the input ending mid-line on ` r`. `2@L1` keeps that context line.
    let diff = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 p
-q
+Q
 r
@@ -10,3 +10,3 @@
 p
-q
+Q
 r";
    let out = common::run_ok(&["select", "2@L1"], diff);
    assert!(
        out.ends_with(b" r\n"),
        "an @L pick is not credited with reaching the last line: {}",
        String::from_utf8_lossy(&out)
    );

    // Taking the same sub-hunk whole is credited, and the input's own ending is reproduced.
    let whole = common::run_ok(&["select", "2"], diff);
    assert!(
        whole.ends_with(b" r"),
        "a whole pick of the last sub-hunk ends where the input did: {}",
        String::from_utf8_lossy(&whole)
    );
}

/// Re-emitting a diff whose every hunk is followed by a line must stay linear in the number of
/// hunks. Scanning the whole trailer per hunk made it quadratic, and the cost is invisible on a
/// small diff: it shows only at scale. Measured as a ratio between two sizes rather than against
/// a wall-clock budget, so the test says the same thing on a fast laptop and a loaded runner —
/// four times the input costs about four times as much when linear, sixteen when quadratic.
#[test]
fn emitting_trailing_lines_stays_linear_in_the_number_of_hunks() {
    /// A diff of `hunks` one-line changes, each followed by a blank separator line.
    fn diff_with_separators(hunks: usize) -> String {
        let mut d = String::from("diff --git a/f b/f\n--- a/f\n+++ b/f\n");
        for i in 0..hunks {
            let base = i * 4 + 1;
            d.push_str(&format!(
                "@@ -{base},3 +{base},3 @@\n ctx{i}\n-old{i}\n+new{i}\n\n"
            ));
        }
        d
    }

    fn split_duration(hunks: usize) -> std::time::Duration {
        let diff = diff_with_separators(hunks);
        let started = std::time::Instant::now();
        let out = common::run_ok_text(&["split", "1", "--at", "1"], &diff);
        let elapsed = started.elapsed();
        assert_eq!(
            out.matches("@@ -").count(),
            hunks,
            "every hunk must survive the split"
        );
        elapsed
    }

    /// The fastest of three runs. A shared runner can only make a run slower — a scheduler
    /// pause on the large size, or an unusually quick small one, moves the ratio without
    /// anything changing in the code — and the minimum is the measurement least exposed to
    /// that. One measurement each left only a factor of two between the linear expectation
    /// and the threshold.
    fn best_of_three(hunks: usize) -> std::time::Duration {
        (0..3)
            .map(|_| split_duration(hunks))
            .min()
            .expect("three runs")
    }

    const SMALL: usize = 8_000;
    let small = best_of_three(SMALL);
    let large = best_of_three(SMALL * 4);
    let ratio = large.as_secs_f64() / small.as_secs_f64().max(f64::MIN_POSITIVE);
    assert!(
        ratio < 8.0,
        "four times the hunks took {ratio:.1}x the time ({small:?} -> {large:?}); \
         linear emission costs about 4x, quadratic about 16x"
    );
}

/// The "input had no final newline" flag describes one specific line of the input — its last.
/// A selection that does not end on that line must not inherit it: the line it does end on had
/// a newline in the input, and dropping it truncates a line the caller never asked to change.
#[test]
fn a_selection_that_drops_the_last_line_still_ends_with_a_newline() {
    let diff = concat!(
        "diff --git a/f b/f\n",
        "--- a/f\n",
        "+++ b/f\n",
        "@@ -1,5 +1,5 @@\n",
        " a\n",
        "-b\n",
        "+B\n",
        " c\n",
        " d\n",
        "-e\n",
        "+E", // the input ends here, without a newline
    );

    let out = common::run_ok_text(&["select", "1"], diff);
    assert!(
        out.ends_with('\n'),
        "the selected sub-hunk ends on a line the input terminated: {out:?}"
    );

    // Selecting the sub-hunk that does end on that line keeps the input byte-for-byte.
    let out = common::run_ok_text(&["select", "2"], diff);
    assert!(
        !out.ends_with('\n'),
        "the last sub-hunk does end on the unterminated line: {out:?}"
    );
}

/// A hunk header hunkpick cannot represent must be refused, not quietly rewritten. Both forms
/// below parsed as if the extra token were not there, and the emitted diff dropped bytes the
/// input carried — with exit 0, against the byte-for-byte promise of `emit`.
#[test]
fn a_hunk_header_with_junk_is_a_parse_error() {
    for header in ["@@ -1,3,9 +1,3 @@", "@@ -1,3 +1,3 junk @@ sect"] {
        let diff = format!("diff --git a/f b/f\n--- a/f\n+++ b/f\n{header}\n a\n-b\n+B\n c\n",);
        let out = Command::cargo_bin("hunkpick")
            .unwrap()
            .arg("list")
            .write_stdin(diff)
            .assert()
            .code(2);
        out.stderr(predicate::str::contains("hunk header"));
    }
}

/// `list --json` is read line by line by shell pipelines (`| jq`, `| while read`), so its output
/// has to end with a newline like every other stream of text this tool writes.
#[test]
fn json_listing_ends_with_a_newline() {
    let diff = concat!(
        "diff --git a/f b/f\n",
        "--- a/f\n",
        "+++ b/f\n",
        "@@ -1 +1 @@\n",
        "-old\n",
        "+new\n",
    );

    let out = common::run_ok_text(&["list", "--json"], diff);
    assert!(
        out.ends_with("]\n"),
        "json listing tail: {:?}",
        &out[out.len().saturating_sub(8)..]
    );
}

/// `--verify-result-diff-git` needs `git` on PATH. When it is absent the check never ran, so the
/// result diff was not rejected — reporting exit 70 ("verification failed") blames hunkpick's
/// output for a missing tool. That is an environment failure: exit 74, like any other I/O.
#[test]
#[cfg(unix)]
fn a_missing_git_binary_is_not_a_verification_failure() {
    let diff = concat!(
        "diff --git a/f b/f\n",
        "--- a/f\n",
        "+++ b/f\n",
        "@@ -1 +1 @@\n",
        "-old\n",
        "+new\n",
    );

    let assert = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1", "--verify-result-diff-git"])
        .env("PATH", "/nonexistent")
        .write_stdin(diff)
        .assert()
        .code(74);
    assert.stderr(predicate::str::contains("git"));
}

/// A one-line change to the file `common::repo_with` seeds below, used by the git-check tests.
const CHANGE_B: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
";

/// `git apply --check` answers 1 when the patch does not apply. Anything else is git itself
/// failing — 128 for a fatal error, no code at all when a signal killed it — and says nothing
/// about the result diff. Reporting that as exit 70 tells the caller hunkpick produced a broken
/// diff (ADR 0013 reserves 70 for exactly that) and points them at filing a bug on the tool
/// instead of at their repository.
#[test]
fn a_fatal_git_failure_is_not_a_verification_failure() {
    let dir = common::repo_with(&[("f", "a\nb\nc\n")]);
    // An unterminated section header: git reads no config file past it and gives up with
    // `fatal: bad config line N in file .git/config`, exit 128.
    let config = dir.path().join(".git").join("config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("[core\n");
    std::fs::write(&config, text).unwrap();

    let assert = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1", "--verify-result-diff-git", "-C"])
        .arg(dir.path())
        .write_stdin(CHANGE_B)
        .assert()
        .code(74);
    assert.stderr(
        predicate::str::contains("rejected")
            .not()
            .and(predicate::str::contains("128")),
    );
}

/// A git killed outright — the OOM killer, a job-control timeout — leaves no exit code and no
/// stderr. Reported as a rejection that reads "rejected the result diff: " with nothing after
/// the colon: a verdict with no reason. The status has to be named instead.
#[test]
#[cfg(unix)]
fn a_git_killed_by_a_signal_is_not_a_verification_failure() {
    let bin = tempfile::tempdir().unwrap();
    let stub = bin.path().join("git");
    std::fs::write(&stub, "#!/bin/sh\nkill -TERM $$\n").unwrap();
    let mut perms = std::fs::metadata(&stub).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&stub, perms).unwrap();

    let assert = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1", "--verify-result-diff-git"])
        .env("PATH", bin.path())
        .write_stdin(CHANGE_B)
        .assert()
        .code(74);
    assert.stderr(
        predicate::str::contains("rejected")
            .not()
            .and(predicate::str::contains("signal")),
    );
}

/// A mistyped `-C DIR` reaches `Command::spawn` as the same `NotFound` a missing `git` binary
/// does, and used to print the same sentence about git. The argument is the caller's own, and
/// they were about to go looking for a broken git installation; the path has to be in the text,
/// and a bad argument value is a usage error (exit 2), not an environment failure.
#[test]
fn a_directory_that_does_not_exist_is_reported_against_the_argument() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-directory");

    let assert = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1", "--verify-result-diff-git", "-C"])
        .arg(&missing)
        .write_stdin(CHANGE_B)
        .assert()
        .code(2);
    assert.stderr(
        predicate::str::contains(missing.to_string_lossy().into_owned())
            .and(predicate::str::contains("-C")),
    );
}

/// The same argument pointing at a file rather than a directory: git would report
/// `Not a directory (os error 20)` against its own name, which is again not where the mistake is.
#[test]
fn a_path_that_is_not_a_directory_is_reported_against_the_argument() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("some.diff");
    std::fs::write(&file, "").unwrap();

    let assert = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "1", "--verify-result-diff-git", "-C"])
        .arg(&file)
        .write_stdin(CHANGE_B)
        .assert()
        .code(2);
    assert.stderr(predicate::str::contains(
        file.to_string_lossy().into_owned(),
    ));
}

/// A diff redirected to a file by Windows PowerShell 5.1 lands in UTF-16LE with a BOM. Every
/// other byte is then NUL, so the binary-input guard fires and sends the reader looking for a
/// binary file instead of at the encoding of their own patch.
#[test]
fn a_utf16_diff_is_diagnosed_as_an_encoding_problem() {
    let text = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n+new\n";
    let mut utf16 = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }

    let assert = Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(utf16)
        .assert()
        .code(2);
    let stderr = assert.get_output().stderr.clone();
    assert.stderr(predicate::str::contains("UTF-16"));
    // This message is addressed to a Windows user by construction, and a Windows console on
    // cp866 or cp1251 shows anything above ASCII as mojibake.
    // `no_source_string_literal_leaves_ascii` holds the rule for the sources; this holds it for
    // the sentence that actually gets printed.
    assert!(
        stderr.is_ascii(),
        "the encoding diagnostic must be ASCII: {}",
        String::from_utf8_lossy(&stderr)
    );
}

/// A diff arriving through a pipe is the normal case, and it must stay quiet: the terminal hint
/// below would otherwise land in the stderr of every scripted invocation.
#[test]
fn a_piped_diff_produces_no_terminal_hint() {
    let diff = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n";

    Command::cargo_bin("hunkpick")
        .unwrap()
        .arg("list")
        .write_stdin(diff)
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}

/// `word` as one argument for a POSIX shell. `script -c` hands its argument to a shell rather
/// than executing it, so the build path reaches that shell as text: a space in it — a checkout
/// under one, or a `CARGO_TARGET_DIR` pointing at one — splits the command, and a metacharacter
/// in it would be executed. This is the only place in the repository that builds a command line
/// as a string; everywhere else `git` and `hunkpick` are spawned with an argument array.
///
/// Single quotes suspend every expansion a shell does. The one byte they cannot carry is the
/// single quote itself, spelled here the usual way: close, escape one, reopen.
#[cfg(target_os = "linux")]
fn single_quoted(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// The escaping above, on the shapes that would otherwise reach the shell as syntax.
#[test]
#[cfg(target_os = "linux")]
fn a_path_is_one_word_however_it_is_spelled() {
    assert_eq!(single_quoted("/tmp/hp target/x"), "'/tmp/hp target/x'");
    assert_eq!(single_quoted("/a;rm -rf b/x"), "'/a;rm -rf b/x'");
    assert_eq!(single_quoted("/a$(id)/x"), "'/a$(id)/x'");
    assert_eq!(single_quoted("/it's/x"), r"'/it'\''s/x'");
}

/// With no pipe and no `-i`, hunkpick reads the terminal — the behaviour of any filter, and the
/// only way a hand-pasted diff can work. Silence makes that indistinguishable from a hang: the
/// user who typed `hunkpick list` instead of `git diff | hunkpick list` sees a dead terminal.
/// One line to stderr names what it is waiting for.
///
/// Linux-only: the test needs a pty, and `script -qec CMD FILE` is the util-linux spelling.
#[test]
#[cfg(target_os = "linux")]
fn a_terminal_on_stdin_gets_a_hint() {
    let exe = assert_cmd::cargo::cargo_bin("hunkpick");
    let command = format!("{} list", single_quoted(&exe.to_string_lossy()));
    let out = std::process::Command::new("script")
        .args(["-qec", &command, "/dev/null"])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("util-linux script provides the pty");

    let seen = String::from_utf8_lossy(&out.stdout);
    assert!(
        seen.contains("reading a diff from the terminal"),
        "the hint must reach the user before the read blocks: {seen:?}"
    );
}
