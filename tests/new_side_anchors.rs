// Integration tests for the new-side (`+`) line numbers of a result diff.
//
// Dropping a sub-hunk changes how many lines the kept part of the result adds or removes, so
// every later hunk's new-side anchor has to be recomputed from the result alone. Carrying the
// input diff's value over is silently wrong: `git apply` starts its search at the new-side
// position, so a stale anchor either drifts onto a different match of the same context (the
// change lands in the wrong place) or is rejected outright.

mod common;

use common::run_ok_text as run_ok;

/// The `@@` header lines of a diff, in order.
fn headers(diff: &str) -> Vec<String> {
    diff.lines()
        .filter(|l| l.starts_with("@@"))
        .map(str::to_string)
        .collect()
}

/// A file of 30 lines with one insertion near the top and one replacement further down.
/// Skipping the insertion must leave the replacement's new-side start equal to its old-side
/// start — nothing before it changes the file's length any more.
#[test]
fn skipping_an_insertion_resets_the_next_hunks_new_start() {
    let old: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    let new: String = (1..=30)
        .map(|i| match i {
            3 => "line 3\ninserted A\n".to_string(),
            20 => "LINE 20\n".to_string(),
            _ => format!("line {i}\n"),
        })
        .collect();
    let dir = common::repo_with(&[("f.txt", &old)]);
    let diff = common::diff_after(&dir, &[("f.txt", &new)]);
    assert_eq!(
        headers(&diff),
        vec!["@@ -1,6 +1,7 @@", "@@ -17,7 +18,7 @@ line 16"],
        "input diff shape changed:\n{diff}"
    );

    let out = run_ok(&["select", "2"], &diff);
    assert_eq!(
        headers(&out),
        vec!["@@ -17,7 +17,7 @@ line 16"],
        "sub-hunk 1 was dropped, so the new-side start must fall back to the old-side one:\n{out}"
    );
}

/// Selecting both sub-hunks keeps the input's anchors: nothing was dropped, so the second
/// hunk still starts one line later on the new side.
#[test]
fn selecting_everything_keeps_the_input_anchors() {
    let old: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    let new: String = (1..=30)
        .map(|i| match i {
            3 => "line 3\ninserted A\n".to_string(),
            20 => "LINE 20\n".to_string(),
            _ => format!("line {i}\n"),
        })
        .collect();
    let dir = common::repo_with(&[("f.txt", &old)]);
    let diff = common::diff_after(&dir, &[("f.txt", &new)]);

    let out = run_ok(&["select", "*"], &diff);
    assert_eq!(
        headers(&out),
        vec!["@@ -1,6 +1,7 @@", "@@ -17,7 +18,7 @@ line 16"]
    );
}

/// An `@L` slice inherits its anchor the same way a whole sub-hunk does, and its own
/// `added - deleted` differs from the full sub-hunk's (an unselected deletion becomes context).
/// The emitted anchor must come from the result, not from the input.
#[test]
fn line_set_slice_gets_a_recomputed_anchor() {
    let old: String = (1..=30).map(|i| format!("line {i}\n")).collect();
    let new: String = (1..=30)
        .map(|i| match i {
            3 => "line 3\ninserted A\n".to_string(),
            20 => "LINE 20\n".to_string(),
            _ => format!("line {i}\n"),
        })
        .collect();
    let dir = common::repo_with(&[("f.txt", &old)]);
    let diff = common::diff_after(&dir, &[("f.txt", &new)]);

    // Sub-hunk 2 is `-line 20` / `+LINE 20`; take only the addition, so the deletion stays as
    // context and the slice adds one line to a file nothing before it has changed.
    let out = run_ok(&["select", "2@L2"], &diff);
    assert_eq!(headers(&out), vec!["@@ -17,7 +17,8 @@ line 16"], "{out}");
}

/// An `@L` slice that keeps its unselected deletions as context grows the sub-hunk's new-side
/// span. Combined with a later sub-hunk in the same emit, the anchors have to account for that
/// growth: with the inherited values the new-side ranges overlapped and the result was rejected
/// (`OverlappingHunks`, exit 70) instead of applying.
#[test]
fn line_set_combined_with_a_later_subhunk_applies() {
    let old: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    let new: String = (1..=20)
        .map(|i| match i {
            5 | 6 | 10 => format!("NEW {i}\n"),
            _ => format!("line {i}\n"),
        })
        .collect();
    let dir = common::repo_with(&[("f.txt", &old)]);
    let diff = common::diff_after(&dir, &[("f.txt", &new)]);
    common::sys(&dir, &["checkout", "--", "."]);

    // Sub-hunk 1 is `-line 5 -line 6 +NEW 5 +NEW 6`; take only the two additions (changed
    // lines 3 and 4), which keeps both deletions as context and adds two lines.
    let out = run_ok(&["select", "1@L3,4", "2"], &diff);
    assert_eq!(
        headers(&out),
        vec!["@@ -2,8 +2,10 @@ line 1", "@@ -10,4 +12,4 @@"],
        "the later sub-hunk must be shifted by the two lines the slice adds:\n{out}"
    );

    std::fs::write(dir.path().join("sel.diff"), &out).unwrap();
    common::sys(&dir, &["apply", "--cached", "sel.diff"]);
    let staged = common::diff_staged(&dir);
    assert!(staged.contains("+NEW 5"), "{staged}");
    assert!(staged.contains("+NEW 10"), "{staged}");
}

/// A file where the same six-line block occurs twice. Sub-hunk 1 deletes ten lines before
/// both copies; sub-hunk 2 changes a line inside the *second* copy. Selecting only sub-hunk 2
/// with a stale (ten lines too early) new-side anchor makes `git apply` search from a position
/// closer to the first copy and edit that one instead — it succeeds and corrupts the file.
#[test]
fn stale_anchor_would_apply_to_the_wrong_copy_of_duplicated_context() {
    let mut old = String::from("head\n");
    for i in 1..=10 {
        old.push_str(&format!("d{i}\n"));
    }
    let block = "b\nc\nBLK1\nBLK2\nBLK3\nBLK4\n";
    old.push_str(block);
    old.push_str(block);
    let new = format!("head\n{block}b\nc\nBLK1\nCHANGED\nBLK3\nBLK4\n");

    let dir = common::repo_with(&[("f.txt", &old)]);
    let diff = common::diff_after(&dir, &[("f.txt", &new)]);
    // Revert the working tree so the selection can be applied to the original content.
    common::sys(&dir, &["checkout", "--", "."]);

    let out = run_ok(&["select", "2"], &diff);
    assert_eq!(
        headers(&out),
        vec!["@@ -18,6 +18,6 @@ BLK4"],
        "sub-hunk 1 (-10 lines) was dropped, so the anchor must not drift:\n{out}"
    );

    // Apply for real and check *where* the change landed: a stale anchor applies cleanly
    // but edits the first copy of the block.
    std::fs::write(dir.path().join("sel.diff"), &out).unwrap();
    common::sys(&dir, &["apply", "--cached", "sel.diff"]);
    let staged = common::diff_staged(&dir);
    assert!(
        staged.contains("@@ -18,6 +18,6 @@"),
        "the change must land in the second copy of the block, got:\n{staged}"
    );
}
