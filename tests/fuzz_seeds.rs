//! The committed fuzz seeds have to still be what they claim to be.
//!
//! A seed that stops reaching the code it was written for does not fail anything: the target
//! returns early and the run goes on, one shape poorer. That is how `simple-content-id` sat in
//! the corpus carrying a made-up `@0123456789abcdef` — sixteen hex digits a mutation will never
//! arrive at, so the `@id` resolution path had no generative reach at all.
//!
//! `Cargo.toml` keeps `fuzz/` out of the published crate, so an absent directory means "not this
//! checkout" rather than a failure.

use hunkpick::{emit, parser, select, validate};
use std::fs;
use std::path::{Path, PathBuf};

fn seed_dir(name: &str) -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/seeds")
        .join(name);
    dir.is_dir().then_some(dir)
}

/// Every regular file in the directory, by name, in a stable order.
fn seeds(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = fs::read_dir(dir)
        .expect("reading the seed directory")
        .map(|e| e.expect("a directory entry").path())
        .filter(|p| p.is_file())
        .map(|p| {
            let name = p
                .file_name()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            (name, fs::read(&p).expect("reading a seed"))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `fuzz/seeds/diff/` is what the `parse` and `roundtrip` targets start from, and `roundtrip`
/// asserts that `emit . parse` is the identity. A seed that does not hold that property is not
/// a starting point; it is a crash waiting to be reported as one.
#[test]
fn every_shared_seed_parses_and_round_trips() {
    let Some(dir) = seed_dir("diff") else { return };
    let files = seeds(&dir);
    assert!(!files.is_empty(), "fuzz/seeds/diff is empty");
    for (name, bytes) in files {
        let patch =
            parser::parse(&bytes).unwrap_or_else(|e| panic!("seed {name} does not parse: {e}"));
        assert_eq!(
            emit::emit(&patch),
            bytes,
            "seed {name} does not come back out of emit unchanged"
        );
    }
}

/// `fuzz/seeds/selectors/` carries a diff, a NUL byte and selector lines. Each seed exists to
/// reach a selector form, so each has to get past `parse_selectors` and `select` — a stale id
/// or a index past the end of its own diff makes the target return before the code the seed
/// was written for.
#[test]
fn every_selector_seed_resolves_against_its_own_diff() {
    let Some(dir) = seed_dir("selectors") else {
        return;
    };
    let files = seeds(&dir);
    assert!(!files.is_empty(), "fuzz/seeds/selectors is empty");
    for (name, bytes) in files {
        let nul = bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or_else(|| panic!("seed {name} carries no NUL separating diff from selectors"));
        let (diff, rest) = (&bytes[..nul], &bytes[nul + 1..]);

        let patch =
            parser::parse(diff).unwrap_or_else(|e| panic!("seed {name} does not parse: {e}"));
        validate::validate_input(&patch)
            .unwrap_or_else(|e| panic!("seed {name} is not a diff the CLI would accept: {e}"));

        let args: Vec<String> = rest
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| {
                String::from_utf8(l.to_vec())
                    .unwrap_or_else(|_| panic!("seed {name}: selectors are ASCII"))
            })
            .collect();
        assert!(!args.is_empty(), "seed {name} names no selector");

        let selectors = select::parse_selectors(&args)
            .unwrap_or_else(|e| panic!("seed {name}: selectors {args:?} do not parse: {e}"));
        let result = select::select(&patch, &selectors)
            .unwrap_or_else(|e| panic!("seed {name}: selectors {args:?} do not resolve: {e}"));
        validate::validate_internal(&result)
            .unwrap_or_else(|e| panic!("seed {name}: the selection is not self-consistent: {e}"));
    }
}

/// A content id is sixteen hex digits: no mutation arrives at a real one, so the seeds are the
/// only reach the `@id` path has. Both of its branches need one — a unique id, and an id two
/// byte-identical sub-hunks share, which is where the collision check lives.
#[test]
fn the_seeds_reach_both_branches_of_id_resolution() {
    let Some(dir) = seed_dir("selectors") else {
        return;
    };
    let mut unique = 0usize;
    let mut shared = 0usize;
    for (_, bytes) in seeds(&dir) {
        let Some(nul) = bytes.iter().position(|&b| b == 0) else {
            continue;
        };
        let (diff, rest) = (&bytes[..nul], &bytes[nul + 1..]);
        if !rest.starts_with(b"@") {
            continue;
        }
        let patch = parser::parse(diff).expect("a seed parses");
        let args: Vec<String> = rest
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| String::from_utf8(l.to_vec()).expect("ASCII selectors"))
            .collect();
        let selectors = select::parse_selectors(&args).expect("a seed's selectors parse");
        let result = select::select(&patch, &selectors).expect("a seed's selectors resolve");
        let picked: usize = result
            .files
            .iter()
            .map(|f| match &f.content {
                hunkpick::model::FileContent::Text(hunks) => hunks.len(),
                hunkpick::model::FileContent::Binary(_) => 1,
            })
            .sum();
        if picked > 1 { shared += 1 } else { unique += 1 }
    }
    assert!(unique > 0, "no seed selects by an id naming one sub-hunk");
    assert!(
        shared > 0,
        "no seed selects by an id shared between sub-hunks"
    );
}
