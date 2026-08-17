// Differential tests: hunkpick against real git on generated diffs.
//
// The hand-written tests elsewhere pin the cases someone thought of. These generate diffs
// instead — a base file, a few random edits, whatever `git diff` makes of them — and assert
// properties that hold for every input: a selection applies, staging one sub-hunk at a time
// converges on the target, and a sub-hunk's header says what git would say. Every defect this
// file guards against was found this way and not by reading the code.
//
// Deterministic on purpose: the cases come from a fixed range of seeds, so a failure is
// reproducible from the seed printed in the assertion message rather than being a flake.

mod common;

use assert_cmd::Command;
use common::{apply_cached, git_output, repo_with, revert, sys};
use tempfile::TempDir;

/// How many generated cases the whole-selection properties run over. Each case is a commit plus
/// a handful of hunkpick invocations, so this stays in the low seconds.
///
/// Windows gets a smaller count. A case is dominated by process spawns — `git` and `hunkpick`
/// through `assert_cmd` — and spawning there costs an order of magnitude more than on Unix, so
/// the full count exceeds the per-test timeout in `.config/nextest.toml`. The reduced run still
/// exercises what is specific to the platform (path handling, line endings, invoking git at
/// all); raising the timeout instead would weaken the guard against a genuinely hung test.
#[cfg(not(windows))]
const CASES: u64 = 200;
#[cfg(windows)]
const CASES: u64 = 30;

/// How many cases the staging loop runs over: each one re-diffs after every sub-hunk, so it
/// costs a multiple of the others.
#[cfg(not(windows))]
const STAGING_CASES: u64 = 40;
#[cfg(windows)]
const STAGING_CASES: u64 = 8;

/// SplitMix64. Deterministic and dependency-free: the same seed yields the same case on every
/// machine, which is what makes a failure reproducible.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A base file and the target it is edited into. The edits — replace, delete, insert, append —
/// are the shapes that produce the hunk forms worth exercising: a pure deletion at the end of a
/// file, an insertion with no old-side lines, replacements adjacent to context.
fn random_case(seed: u64) -> (String, String) {
    let mut rng = Rng::new(seed);
    let n = 8 + rng.below(13);
    let base: Vec<String> = (1..=n).map(|i| format!("line {i}")).collect();
    let mut target = base.clone();
    for _ in 0..1 + rng.below(4) {
        if target.is_empty() {
            target.push(format!("added {}", rng.below(1000)));
            continue;
        }
        let at = rng.below(target.len());
        match rng.below(4) {
            0 => target[at] = format!("changed {}", rng.below(1000)),
            1 => {
                target.remove(at);
            }
            2 => target.insert(at, format!("inserted {}", rng.below(1000))),
            _ => target.push(format!("appended {}", rng.below(1000))),
        }
    }
    (join(&base), join(&target))
}

fn join(lines: &[String]) -> String {
    lines
        .iter()
        .map(|l| format!("{l}\n"))
        .collect::<Vec<_>>()
        .concat()
}

/// Commit `base`, write `target`, and return the diff git makes of the pair, leaving the
/// working tree back at `base` so the diff can be applied to it.
fn case_diff(dir: &TempDir, seed: u64) -> String {
    let (base, target) = random_case(seed);
    std::fs::write(dir.path().join("f"), &base).unwrap();
    sys(dir, &["add", "f"]);
    // Two consecutive seeds can generate the same base; committing nothing is not an error here.
    sys(dir, &["commit", "-q", "-m", "case", "--allow-empty"]);
    std::fs::write(dir.path().join("f"), &target).unwrap();
    let diff = git_output(dir, &["diff", "--", "f"]);
    revert(dir);
    diff
}

/// The number of sub-hunks hunkpick reports for `diff`.
fn sub_hunk_count(diff: &str) -> usize {
    let out = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["list", "--json"])
        .write_stdin(diff.to_string())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    json.as_array().unwrap()[0]["hunks"]
        .as_array()
        .unwrap()
        .len()
}

/// Run `hunkpick select` with the git check enabled against `dir`'s working tree.
fn select_checked(dir: &TempDir, diff: &str, selectors: &[String]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::cargo_bin("hunkpick").unwrap();
    cmd.arg("select");
    cmd.args(selectors);
    cmd.args([
        "--verify-result-diff-git",
        "-C",
        dir.path().to_str().unwrap(),
    ]);
    cmd.write_stdin(diff.to_string()).assert()
}

/// Selecting everything must reproduce the target: hunkpick's own idea of "all sub-hunks" has
/// to be the input's whole change, and the result has to apply. Both the git check and the
/// applied content are asserted — a diff can pass `--check` and still be the wrong diff.
#[test]
fn selecting_everything_reproduces_the_target() {
    let dir = repo_with(&[]);
    for seed in 0..CASES {
        let (_, target) = random_case(seed);
        let diff = case_diff(&dir, seed);
        if diff.is_empty() {
            continue; // the edits cancelled out
        }
        let out = select_checked(&dir, &diff, &["*".to_string()])
            .try_success()
            .unwrap_or_else(|e| panic!("seed {seed}: select '*' failed: {e}"))
            .get_output()
            .stdout
            .clone();

        apply_to_worktree(&dir, &out, seed);
        let got = std::fs::read_to_string(dir.path().join("f")).unwrap();
        assert_eq!(got, target, "seed {seed}: applying every sub-hunk");
        revert(&dir);
    }
}

/// Any subset of the sub-hunks has to apply on its own. This is the property the anchor
/// arithmetic exists for: dropping a sub-hunk shifts every later one on the new side.
#[test]
fn every_random_subset_applies() {
    let dir = repo_with(&[]);
    for seed in 0..CASES {
        let diff = case_diff(&dir, seed);
        if diff.is_empty() {
            continue;
        }
        let count = sub_hunk_count(&diff);
        let mut rng = Rng::new(seed ^ 0xA5A5_A5A5);
        let subset: Vec<String> = (1..=count)
            .filter(|_| rng.next() % 2 == 0)
            .map(|i| i.to_string())
            .collect();
        if subset.is_empty() {
            continue;
        }
        select_checked(&dir, &diff, &subset)
            .try_success()
            .unwrap_or_else(|e| panic!("seed {seed}: subset {subset:?} failed: {e}"));
    }
}

/// The workflow the tool exists for: stage one sub-hunk, re-diff, repeat. Every round has to
/// make progress and the index has to end up holding exactly the target.
#[test]
fn staging_one_sub_hunk_at_a_time_converges_on_the_target() {
    let dir = repo_with(&[]);
    for seed in 0..STAGING_CASES {
        let (_, target) = random_case(seed);
        let diff = case_diff(&dir, seed);
        if diff.is_empty() {
            continue;
        }
        // Put the target back: staging reads the working tree against the index.
        std::fs::write(dir.path().join("f"), &target).unwrap();

        let mut rounds = 0;
        loop {
            let diff = git_output(&dir, &["diff", "--", "f"]);
            if diff.is_empty() {
                break;
            }
            rounds += 1;
            assert!(rounds <= 64, "seed {seed}: staging does not converge");
            let out = Command::cargo_bin("hunkpick")
                .unwrap()
                .args(["select", "1"])
                .write_stdin(diff)
                .assert()
                .try_success()
                .unwrap_or_else(|e| panic!("seed {seed}: round {rounds}: {e}"))
                .get_output()
                .stdout
                .clone();
            apply_cached(&dir, &out);
        }

        let staged = git_output(&dir, &["show", ":f"]);
        assert_eq!(
            staged, target,
            "seed {seed}: staged content after {rounds} rounds"
        );
        sys(&dir, &["commit", "-q", "-m", "staged"]);
    }
}

/// hunkpick has to accept its own output. The result of a selection is a diff in its own
/// right: `list` must read it without complaint (the input check and the result check agreeing
/// on what a valid diff is), and selecting everything from it must give it back unchanged.
/// A check that rejects a diff the tool itself just produced is how the zero-count convention
/// went unnoticed.
#[test]
fn a_selection_is_valid_input_and_a_fixed_point() {
    let dir = repo_with(&[]);
    for seed in 0..CASES {
        let diff = case_diff(&dir, seed);
        if diff.is_empty() {
            continue;
        }
        let once = hunkpick_select_all(&diff);

        Command::cargo_bin("hunkpick")
            .unwrap()
            .arg("list")
            .write_stdin(once.clone())
            .assert()
            .try_success()
            .unwrap_or_else(|e| panic!("seed {seed}: own output rejected as input: {e}"));

        let twice = hunkpick_select_all(&once);
        assert_eq!(twice, once, "seed {seed}: selecting everything twice");
    }
}

/// `select '*'` output as text.
fn hunkpick_select_all(diff: &str) -> String {
    let out = Command::cargo_bin("hunkpick")
        .unwrap()
        .args(["select", "*"])
        .write_stdin(diff.to_string())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).unwrap()
}

/// Apply `diff` to the working tree of `dir`, reporting the seed if git refuses.
fn apply_to_worktree(dir: &TempDir, diff: &[u8], seed: u64) {
    let mut child = std::process::Command::new("git")
        .args(["apply"])
        .current_dir(dir.path())
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child.stdin.take().unwrap().write_all(diff).unwrap();
    }
    assert!(
        child.wait().unwrap().success(),
        "seed {seed}: git apply rejected the result"
    );
}
