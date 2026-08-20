//! Checks on the shell examples the documentation shows.
//!
//! `select --help` already has its examples run as written (tests/roundtrip.rs). The README's
//! did not, and that is why two of them kept recommending `--verify-result-diff-git` on a
//! `git diff |` pipeline after the README itself gained a paragraph explaining that such a
//! command reports a correct result as a failure.
//!
//! `Cargo.toml` keeps the contributor guides and `scripts/` out of the published crate, so
//! every lookup here treats an absent file as "not this checkout" rather than as a failure.
//! `README.md` is published and always present.

use std::fs;
use std::path::Path;

const DOCS: [&str; 3] = ["README.md", "CONTRIBUTING.md", "RELEASING.md"];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The file's text, or `None` when this checkout does not carry it.
fn doc(name: &str) -> Option<String> {
    let path = root().join(name);
    if !path.is_file() {
        return None;
    }
    Some(fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display())))
}

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

/// `--verify-result-diff-git` runs `git apply --check`, which reads the working tree. In the
/// staging pipeline the tree already holds the edits the diff describes, so git answers "patch
/// does not apply" and hunkpick exits 70 for a correct result. An example that pipes `git diff`
/// into a command carrying the flag therefore fails as written — it is the one shape the flag
/// must never be shown in.
#[test]
fn no_documented_example_runs_the_git_check_on_a_staging_pipeline() {
    let mut checked = 0usize;
    for name in DOCS {
        let Some(text) = doc(name) else { continue };
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

/// The documentation points at scripts rather than repeating a long command — the fuzzing run
/// needs a toolchain override, an explicit triple, a corpus directory and a hang timeout, and a
/// command missing any of them fails in a way that does not name the cause. A renamed or
/// removed script turns those instructions into a dead end.
#[test]
fn every_script_the_docs_name_exists_and_is_executable() {
    if !root().join("scripts").is_dir() {
        return; // the published crate excludes scripts/ along with the guides that name them
    }
    let mut checked = 0usize;
    for name in DOCS {
        let Some(text) = doc(name) else { continue };
        for token in text.split(|c: char| !(c.is_ascii_alphanumeric() || "/._-".contains(c))) {
            if !token.starts_with("scripts/") || !token.ends_with(".sh") {
                continue;
            }
            let path = root().join(token);
            checked += 1;
            assert!(path.is_file(), "{name} names {token}, which does not exist");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(&path).expect("stat").permissions().mode();
                assert!(
                    mode & 0o111 != 0,
                    "{name} names {token}, which is not executable (mode {mode:o})"
                );
            }
        }
    }
    assert!(
        checked > 0,
        "no documentation names a script; this test now covers nothing"
    );
}
