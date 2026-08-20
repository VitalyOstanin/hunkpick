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
use common::{apply_cached, git_output, repo_with, revert, select_checked, sys};
use tempfile::TempDir;

/// How many generated cases the whole-selection properties run over. Each case is a commit plus
/// a handful of hunkpick invocations.
///
/// Measured on Linux (debug build, warm cache, one test at a time) for every test that draws
/// its count from here: 4.5 s `selecting_everything…`, 4.1 s `a_selection_is_valid_input…`,
/// 3.9 s `every_random_subset…`, 1.5 s the staging loop, 1.2 s the multi-file case, 1.0 s the
/// line slices — against the 60 s the profile in `.config/nextest.toml` allows (`slow-timeout`
/// 30 s, `terminate-after = 2`). Better than a tenfold margin on the slowest, so a timeout
/// there means something hung, not that the machine was busy.
///
/// Windows gets a smaller count. A case is dominated by process spawns — `git` and `hunkpick`
/// through `assert_cmd` — and spawning there costs an order of magnitude more than on Unix, so
/// the full count exceeds that budget. The reduced run still exercises what is specific to the
/// platform (path handling, line endings, invoking git at all); raising the timeout instead
/// would weaken the guard against a genuinely hung test.
///
/// `HUNKPICK_DIFF_CASES` overrides both counts (the staging one scaled in the same proportion),
/// so a longer soak can be run without touching the source. Raise the timeout with it: at 2000
/// cases the slowest test above takes 44.5 s on the machine where it takes 4.5 s here, and the
/// profile would kill it at 60 s — a soak that ends in a kill looks like a hang rather than a
/// finished soak. The pair to use is
/// `HUNKPICK_DIFF_CASES=2000 cargo nextest run --all-features --slow-timeout period=300s,terminate-after=2`.
#[cfg(not(windows))]
const DEFAULT_CASES: u64 = 200;
#[cfg(windows)]
const DEFAULT_CASES: u64 = 30;

/// How many cases the staging loop runs over: each one re-diffs after every sub-hunk, so it
/// costs a multiple of the others.
#[cfg(not(windows))]
const DEFAULT_STAGING_CASES: u64 = 40;
#[cfg(windows)]
const DEFAULT_STAGING_CASES: u64 = 8;

/// The case count for this run: `HUNKPICK_DIFF_CASES` if it parses as a positive number, the
/// platform default otherwise. An unusable value is ignored rather than failing the test — the
/// counts are a knob, not part of what is under test.
fn cases() -> u64 {
    std::env::var("HUNKPICK_DIFF_CASES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_CASES)
}

/// The staging-loop count, kept in the same proportion to [`cases`] as the defaults are.
fn staging_cases() -> u64 {
    (cases() * DEFAULT_STAGING_CASES / DEFAULT_CASES).max(1)
}

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
    lines.iter().map(|l| format!("{l}\n")).collect()
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
    common::list_json(diff).as_array().unwrap()[0]["hunks"]
        .as_array()
        .unwrap()
        .len()
}

/// Selecting everything must reproduce the target: hunkpick's own idea of "all sub-hunks" has
/// to be the input's whole change, and the result has to apply. Both the git check and the
/// applied content are asserted — a diff can pass `--check` and still be the wrong diff.
#[test]
fn selecting_everything_reproduces_the_target() {
    let dir = repo_with(&[]);
    for seed in 0..cases() {
        let (_, target) = random_case(seed);
        let diff = case_diff(&dir, seed);
        if diff.is_empty() {
            continue; // the edits cancelled out
        }
        let out = select_checked(&dir, &diff, &["*"])
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
    for seed in 0..cases() {
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
        let args: Vec<&str> = subset.iter().map(String::as_str).collect();
        select_checked(&dir, &diff, &args)
            .try_success()
            .unwrap_or_else(|e| panic!("seed {seed}: subset {subset:?} failed: {e}"));
    }
}

/// The workflow the tool exists for: stage one sub-hunk, re-diff, repeat. Every round has to
/// make progress and the index has to end up holding exactly the target.
#[test]
fn staging_one_sub_hunk_at_a_time_converges_on_the_target() {
    let dir = repo_with(&[]);
    for seed in 0..staging_cases() {
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
    for seed in 0..cases() {
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
///
/// Goes through `common::apply_diff` rather than spawning `git` directly: that helper drops the
/// repository-locating variables and points the configuration files at absent paths, so a
/// developer's own `apply.whitespace` or `GIT_DIR` cannot decide what this test sees — or send
/// the patch into their working tree instead of the temporary one.
fn apply_to_worktree(dir: &TempDir, diff: &[u8], seed: u64) {
    common::apply_diff(dir, &["apply"], diff, &format!("seed {seed}: git apply"));
}

/// A multi-file diff, with a non-ASCII name and a binary file among the entries.
///
/// The generators above edit one ASCII-named text file, so `path:` resolution, quoted paths
/// (git escapes non-ASCII names under the default `core.quotePath`) and a hunkless binary entry
/// were held only by hand-written examples. Here they are part of every generated case.
fn multi_file_case(dir: &TempDir, seed: u64) -> String {
    let mut rng = Rng::new(seed ^ 0x5EED_1234);
    let names = ["a.rs", "dir/naïve.txt", "bin.dat"];
    let base: Vec<String> = names
        .iter()
        .map(|_| {
            let n = 6 + rng.below(6);
            (1..=n).map(|i| format!("line {i}\n")).collect::<String>()
        })
        .collect();

    for (name, content) in names.iter().zip(&base) {
        let full = dir.path().join(name);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, content).unwrap();
    }
    // The third file is binary: a NUL makes git treat it as such, so its entry is a `GIT binary
    // patch` with no hunks.
    std::fs::write(dir.path().join("bin.dat"), [0u8, 1, 2, 3, 4]).unwrap();
    sys(dir, &["add", "."]);
    sys(dir, &["commit", "-q", "-m", "case", "--allow-empty"]);

    for (name, content) in names.iter().take(2).zip(&base) {
        let edited: String = content
            .lines()
            .map(|l| {
                if rng.next() % 3 == 0 {
                    format!("changed {}\n", rng.below(1000))
                } else {
                    format!("{l}\n")
                }
            })
            .collect();
        std::fs::write(dir.path().join(name), edited).unwrap();
    }
    std::fs::write(dir.path().join("bin.dat"), [0u8, 9, 9, 9, 9]).unwrap();

    let diff = git_output(dir, &["diff", "--binary"]);
    revert(dir);
    diff
}

/// Every sub-hunk of a multi-file diff — text, non-ASCII path, binary entry — must come back out
/// of `select '*'` and apply.
#[test]
fn a_multi_file_diff_with_a_binary_and_a_non_ascii_path_round_trips() {
    let dir = repo_with(&[]);
    for seed in 0..staging_cases() {
        let diff = multi_file_case(&dir, seed);
        if diff.is_empty() {
            continue;
        }
        // A bare `*` only resolves in a single-file diff, so each entry is named — with the path
        // as `list --json` reports it, which closes the documented list-then-select loop over a
        // quoted non-ASCII name and a binary entry.
        let selectors: Vec<String> = listed_paths(&diff)
            .into_iter()
            .map(|p| format!("{p}:*"))
            .collect();
        if selectors.is_empty() {
            continue;
        }
        let out = Command::cargo_bin("hunkpick")
            .unwrap()
            .arg("select")
            .args(&selectors)
            .write_stdin(diff.clone())
            .assert()
            .try_success()
            .unwrap_or_else(|e| panic!("seed {seed}: selecting every entry failed: {e}"))
            .get_output()
            .stdout
            .clone();
        apply_to_worktree(&dir, &out, seed);
        revert(&dir);
        sys(&dir, &["commit", "-q", "-m", "round", "--allow-empty"]);
    }
}

/// The paths `list --json` reports for `diff`, in order.
fn listed_paths(diff: &str) -> Vec<String> {
    common::list_json(diff)
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect()
}

/// New-side line numbers of the context lines inside `diff`'s hunks, excluding the first and last
/// body line of each hunk: `split --at` needs a context line with body on both sides of it.
fn interior_context_lines(diff: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut new_line = 0u32;
    let mut pending: Option<u32> = None;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("@@ ") {
            // `@@ -a,b +c,d @@` — take the new-side start.
            let plus = rest.split('+').nth(1).unwrap_or("");
            let num = plus.split([',', ' ']).next().unwrap_or("0");
            new_line = num.parse().unwrap_or(0);
            pending = None;
            continue;
        }
        match line.as_bytes().first() {
            Some(b' ') => {
                // Held back one line: a context line only qualifies once another body line
                // follows it, which the next iteration decides.
                if let Some(candidate) = pending.take() {
                    out.push(candidate);
                }
                if new_line > 0 {
                    pending = Some(new_line);
                }
                new_line += 1;
            }
            Some(b'+') => {
                if let Some(candidate) = pending.take() {
                    out.push(candidate);
                }
                new_line += 1;
            }
            Some(b'-') => {
                if let Some(candidate) = pending.take() {
                    out.push(candidate);
                }
            }
            _ => pending = None,
        }
    }
    out
}

/// The changed-line indices of the first sub-hunk that has more than one, via `list --json`.
fn first_multi_line_sub_hunk(diff: &str) -> Option<(usize, Vec<usize>)> {
    let json = common::list_json(diff);
    for hunk in json.as_array()?[0]["hunks"].as_array()? {
        let lines: Vec<usize> = hunk["changed_lines"]
            .as_array()?
            .iter()
            .map(|l| l["i"].as_u64().unwrap() as usize)
            .collect();
        if lines.len() > 1 {
            return Some((hunk["index"].as_u64().unwrap() as usize, lines));
        }
    }
    None
}

/// `@L` slices and `split` results have to apply, not merely pass hunkpick's own checks. Both
/// rewrite hunk bodies and anchors — `@L` turns unselected deletions into context, `split` cuts
/// one hunk into several — so git is the authority on whether the result is a diff at all.
#[test]
fn line_slices_and_splits_apply_via_git() {
    let dir = repo_with(&[]);
    // Both branches below are conditional on the generated diff having the right shape; without
    // counting them a change to the generator could quietly reduce this test to a no-op.
    let (mut sliced, mut split) = (0u32, 0u32);
    for seed in 0..staging_cases() {
        let diff = case_diff(&dir, seed);
        if diff.is_empty() {
            continue;
        }

        if let Some((index, lines)) = first_multi_line_sub_hunk(&diff) {
            // A pseudo-random non-empty subset of that sub-hunk's changed lines.
            let mut rng = Rng::new(seed ^ 0x11AA_22BB);
            let mut subset: Vec<String> = lines
                .iter()
                .filter(|_| rng.next() % 2 == 0)
                .map(|i| i.to_string())
                .collect();
            if subset.is_empty() {
                subset.push(lines[0].to_string());
            }
            let selector = format!("{index}@L{}", subset.join(","));
            let out = Command::cargo_bin("hunkpick")
                .unwrap()
                .args(["select", &selector])
                .write_stdin(diff.clone())
                .assert()
                .try_success()
                .unwrap_or_else(|e| panic!("seed {seed}: select {selector} failed: {e}"))
                .get_output()
                .stdout
                .clone();
            common::apply_diff(
                &dir,
                &["apply", "--check"],
                &out,
                &format!("seed {seed}: git apply --check of {selector}"),
            );
            sliced += 1;
        }

        if let Some(&at) = interior_context_lines(&diff).first() {
            let out = Command::cargo_bin("hunkpick")
                .unwrap()
                .args(["split", "1", "--at", &at.to_string()])
                .write_stdin(diff.clone())
                .assert()
                .try_success()
                .unwrap_or_else(|e| panic!("seed {seed}: split at {at} failed: {e}"))
                .get_output()
                .stdout
                .clone();
            common::apply_diff(
                &dir,
                &["apply", "--check"],
                &out,
                &format!("seed {seed}: git apply --check of split at {at}"),
            );
            split += 1;
        }
    }
    assert!(
        sliced > 0 && split > 0,
        "the generator stopped producing sliceable/splittable diffs: {sliced} slices, {split} splits"
    );
}
