// Integration tests for the content-id (`@<id>`) and `*` (all) selector forms, driven through
// the CLI against real `git diff` output.

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

/// Sub-hunk ids across all files, in `list` order.
fn ids(diff: &str) -> Vec<String> {
    let out = run_ok(&["list", "--json"], diff);
    let v: Value = serde_json::from_slice(&out).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .flat_map(|f| {
            f["hunks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|h| h["id"].as_str().unwrap().to_string())
        })
        .collect()
}

fn revert(dir: &tempfile::TempDir) {
    common::sys(dir, &["checkout", "--", "."]);
}

/// The id of a change must not change when an unrelated edit above it shifts its line numbers.
#[test]
fn id_is_stable_across_line_number_shift() {
    let dir = common::repo_with(&[("f", "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n")]);

    // Variant A: change only the lower line `i`.
    let diff_a = common::diff_after(&dir, &[("f", "a\nb\nc\nd\ne\nf\ng\nh\nI\nj\n")]);
    let ids_a = ids(&diff_a);
    assert_eq!(ids_a.len(), 1, "variant A has one sub-hunk");
    let id_lower = ids_a[0].clone();
    revert(&dir);

    // Variant B: insert a new first line (shifting everything down) AND make the same lower
    // change. Two hunks now; the lower one has identical content but shifted line numbers.
    let diff_b = common::diff_after(&dir, &[("f", "X\na\nb\nc\nd\ne\nf\ng\nh\nI\nj\n")]);
    let ids_b = ids(&diff_b);
    assert_eq!(ids_b.len(), 2, "variant B has two sub-hunks");
    assert!(
        ids_b.contains(&id_lower),
        "the lower change keeps its id despite the line-number shift: {id_lower} not in {ids_b:?}"
    );
    revert(&dir);
}

/// The id of a change survives staging a neighbouring change, even though that alters this
/// change's surrounding context. This is the guarantee the context-free id buys over an
/// index-based handle: capture `@<id>` once, keep using it across `diff -> stage -> re-diff`.
#[test]
fn id_is_stable_across_neighbour_staging() {
    // Two changes close enough to share one hunk (b->B and d->D, the MULTI layout). Auto-split
    // gives two sub-hunks whose context windows overlap, so staging one rewrites the other's
    // context.
    let dir = common::repo_with(&[("f", "a\nb\nc\nd\ne\n")]);
    let diff = common::diff_after(&dir, &[("f", "a\nB\nc\nD\ne\n")]);
    let ids_before = ids(&diff);
    assert_eq!(
        ids_before.len(),
        2,
        "one hunk auto-splits into two sub-hunks"
    );
    let id_second = ids_before[1].clone(); // the d->D change

    // Stage only the FIRST change (b->B) into the index, leaving the working tree with both.
    let staged = run_ok(&["select", "1"], &diff);
    let mut child = std::process::Command::new("git")
        .args(["apply", "--cached"])
        .current_dir(dir.path())
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    child.stdin.take().unwrap().write_all(&staged).unwrap();
    assert!(child.wait().unwrap().success(), "git apply --cached failed");

    // Re-diff index vs working tree: only d->D remains, but its context line above is now the
    // staged "B" instead of "b" — a different context window than in the original diff.
    let rediff = common::diff_after(&dir, &[("f", "a\nB\nc\nD\ne\n")]);
    assert!(
        rediff.contains("+D"),
        "re-diff still carries the d->D change"
    );
    assert!(!rediff.contains("+B"), "b->B is staged, not in the re-diff");
    let ids_after = ids(&rediff);

    assert!(
        ids_after.contains(&id_second),
        "the surviving change keeps its id across neighbour staging: {id_second} not in {ids_after:?}"
    );
    common::sys(&dir, &["reset", "-q"]);
    revert(&dir);
}

/// `select @<id>` emits exactly the addressed change and the result applies via git.
#[test]
fn select_by_id_round_trips_through_git() {
    let dir = common::repo_with(&[("f", "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n")]);
    // Two distant changes -> two separate hunks.
    let diff = common::diff_after(&dir, &[("f", "A\nb\nc\nd\ne\nf\ng\nh\nI\nj\n")]);
    let id_list = ids(&diff);
    assert_eq!(id_list.len(), 2);
    let id_lower = id_list[1].clone(); // the i->I change
    revert(&dir);

    // Output contains the addressed change only.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", &format!("@{id_lower}")])
        .write_stdin(diff.clone())
        .assert()
        .success()
        .stdout(predicate::str::contains("+I"))
        .stdout(predicate::str::contains("+A").not());

    // And it applies cleanly against the reverted working tree.
    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            &format!("@{id_lower}"),
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(diff)
        .assert()
        .success();
}

/// Bare `*` selects every sub-hunk; the combined diff applies via git.
#[test]
fn star_selects_every_subhunk_and_applies() {
    let dir = common::repo_with(&[("f", "a\nb\nc\nd\ne\nf\ng\n")]);
    let diff = common::diff_after(&dir, &[("f", "a\nB\nc\nD\ne\nF\ng\n")]);
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
        .write_stdin(diff.clone())
        .assert()
        .success()
        .stdout(predicate::str::contains("+B"))
        .stdout(predicate::str::contains("+D"))
        .stdout(predicate::str::contains("+F"));
}

/// Two byte-identical changes (same context + edit) share an id; `@<id>` selects both.
#[test]
fn id_selects_all_identical_changes() {
    // Two identical changes separated by distinct filler so they form separate hunks. Their
    // context windows happen to match here, but the id is context-free, so they would share an
    // id even if the surrounding context differed.
    let base = "k\nl\nm\nx\nn\no\np\nF1\nF2\nF3\nF4\nF5\nF6\nF7\nk\nl\nm\nx\nn\no\np\n";
    let edited = "k\nl\nm\ny\nn\no\np\nF1\nF2\nF3\nF4\nF5\nF6\nF7\nk\nl\nm\ny\nn\no\np\n";
    let dir = common::repo_with(&[("f", base)]);
    let diff = common::diff_after(&dir, &[("f", edited)]);

    let id_list = ids(&diff);
    assert_eq!(id_list.len(), 2, "two changes");
    assert_eq!(
        id_list[0], id_list[1],
        "identical changes must share an id: {id_list:?}"
    );
    revert(&dir);

    // `@<id>` emits both occurrences (two `+y` lines), and the result applies.
    let out = run_ok(&["select", &format!("@{}", id_list[0])], &diff);
    let text = String::from_utf8(out).unwrap();
    assert_eq!(
        text.matches("+y").count(),
        2,
        "both identical changes selected:\n{text}"
    );

    Command::cargo_bin("hunkpick")
        .unwrap()
        .args([
            "select",
            &format!("@{}", id_list[0]),
            "--verify-result-diff-git",
            "-C",
            dir.path().to_str().unwrap(),
        ])
        .write_stdin(diff)
        .assert()
        .success();
}
