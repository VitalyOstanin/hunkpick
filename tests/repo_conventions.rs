//! Conventions the repository states about itself, checked rather than remembered.

use std::path::{Path, PathBuf};

/// The limit `.editorconfig` sets for the file types it still sets one for.
const MAX_LINE: usize = 100;

/// Directories walked for the check, with the extensions each contributes. `.github/` and
/// `scripts/` are excluded from the published crate (see `exclude` in Cargo.toml), so a run
/// from a crates.io tarball simply finds fewer of them.
const ROOTS: [(&str, &[&str]); 4] = [
    ("src", &["rs"]),
    ("tests", &["rs"]),
    ("fuzz", &["rs", "toml"]),
    (".github", &["yml", "yaml"]),
];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every file under `dir` whose extension is in `exts`, recursively.
fn files(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            // Build output, not source: `fuzz/target` holds compiled artefacts, and
            // `fuzz/corpus` and `fuzz/artifacts` hold generated inputs.
            if !matches!(
                path.file_name().and_then(|n| n.to_str()),
                Some("target" | "corpus" | "artifacts")
            ) {
                files(&path, exts, out);
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.push(path);
        }
    }
}

/// `.editorconfig` sets `max_line_length = 100`, and it applies to code, workflows and the fuzz
/// manifest — Markdown and `Cargo.toml` are exempted there, with the reasons written next to the
/// exemption. `cargo fmt` enforces the same width for code but leaves comments and string
/// literals alone, which is exactly where the long lines came from before this test existed.
#[test]
fn line_length_stays_within_the_editorconfig_limit() {
    let repo = repo();
    let mut offenders = Vec::new();
    for (dir, exts) in ROOTS {
        let mut paths = Vec::new();
        files(&repo.join(dir), exts, &mut paths);
        paths.sort();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue; // not UTF-8: a fuzz seed or a fixture, not a source file
            };
            for (no, line) in text.lines().enumerate() {
                if line.chars().count() > MAX_LINE {
                    let rel = path.strip_prefix(&repo).unwrap_or(&path).display();
                    offenders.push(format!(
                        "{rel}:{}: {} columns",
                        no + 1,
                        line.chars().count()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "{} line(s) over {MAX_LINE} columns; wrap them, or state the exemption \
         in .editorconfig:\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// `tests/edge_corpus.rs` introduces each of its tests with a banner naming it. The banners were
/// numbered once, and the numbers stopped matching the tests the first time one was added without
/// one — after which "test 14" pointed at the fifteenth test, and the last cycle managed to add
/// both an unnumbered test and a new number in the same range. Names cannot drift that way, and
/// this keeps the set of banners equal to the set of tests, which a comment could not.
#[test]
fn every_edge_corpus_test_is_introduced_by_a_banner_naming_it() {
    let path = repo().join("tests").join("edge_corpus.rs");
    let text = std::fs::read_to_string(&path).expect("the file is part of the published crate");
    let lines: Vec<&str> = text.lines().collect();

    let mut banners = Vec::new();
    let mut tests = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        // A banner is a comment line between two rules of dashes.
        let between_rules = i > 0
            && lines[i - 1].starts_with("// ---")
            && lines.get(i + 1).is_some_and(|l| l.starts_with("// ---"));
        if between_rules {
            if let Some(name) = line.strip_prefix("// ") {
                banners.push(name.trim().to_string());
            }
        }
        // No let-chain here: this crate builds on 1.85, where that syntax does not exist yet.
        let after_test_attribute = lines[..i]
            .iter()
            .rev()
            .take(3)
            .any(|l| l.trim() == "#[test]");
        if after_test_attribute {
            if let Some(name) = line.strip_prefix("fn ") {
                tests.push(name.split('(').next().unwrap_or_default().to_string());
            }
        }
    }

    banners.sort();
    tests.sort();
    assert_eq!(
        banners, tests,
        "every test needs a banner naming it, and every banner a test of that name"
    );
}

/// `.editorconfig` asks shell scripts to indent by four spaces, and until now that was a
/// request an editor might honour rather than a rule. shellcheck, added to CI in the same
/// cycle, does not close this: it reads a script for what it does, not for how it is laid out.
/// The scripts carry release-critical logic, so a diff of one should show the change and not
/// a re-indentation around it.
#[test]
fn every_shell_script_indents_by_four_spaces() {
    let dir = repo().join("scripts");
    let mut scripts = Vec::new();
    files(&dir, &["sh"], &mut scripts);
    if scripts.is_empty() {
        // scripts/ is excluded from the published crate; a run from a crates.io tarball
        // has nothing to check here.
        return;
    }
    scripts.sort();

    let mut offenders = Vec::new();
    for path in &scripts {
        let text = std::fs::read_to_string(path).expect("a readable script");
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
            let reason = if line[..indent].contains('\t') {
                "a tab"
            } else if indent % 4 != 0 {
                "an indent that is not a multiple of four"
            } else {
                continue;
            };
            offenders.push(format!(
                "{}:{}: {reason}",
                path.strip_prefix(repo()).unwrap_or(path).display(),
                i + 1
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "{} line(s) disagree with the four-space indent .editorconfig sets for *.sh:\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}
