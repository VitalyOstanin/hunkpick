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
