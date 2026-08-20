//! Checks on the shell examples the documentation shows.
//!
//! `select --help` already has its examples run as written (tests/roundtrip.rs). The README's
//! did not, and that is why two of them kept recommending `--verify-result-diff-git` on a
//! `git diff |` pipeline after the README itself gained a paragraph explaining that such a
//! command reports a correct result as a failure.

use std::fs;
use std::path::Path;

/// The `sh` code blocks of a Markdown file, as lines, with the fences removed.
fn shell_lines(markdown: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut inside = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            inside = trimmed == "```sh" || trimmed == "```bash";
            continue;
        }
        if inside {
            lines.push(line);
        }
    }
    lines
}

fn doc(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// `--verify-result-diff-git` runs `git apply --check`, which reads the working tree. In the
/// staging pipeline the tree already holds the edits the diff describes, so git answers "patch
/// does not apply" and hunkpick exits 70 for a correct result. An example that pipes `git diff`
/// into a command carrying the flag therefore fails as written — it is the one shape the flag
/// must never be shown in.
#[test]
fn no_documented_example_runs_the_git_check_on_a_staging_pipeline() {
    let mut checked = 0usize;
    for name in ["README.md", "CONTRIBUTING.md"] {
        let text = doc(name);
        for line in shell_lines(&text) {
            if !line.contains("--verify-result-diff-git") {
                continue;
            }
            checked += 1;
            assert!(
                !line.contains("git diff"),
                "{name} shows the git check on a `git diff |` pipeline, where it rejects a \
                 correct result: {line}"
            );
        }
    }
    assert!(
        checked > 0,
        "no documented example uses --verify-result-diff-git; this test now covers nothing"
    );
}
