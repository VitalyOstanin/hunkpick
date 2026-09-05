//! Conventions the repository states about itself, checked rather than remembered.

use std::path::{Path, PathBuf};

/// The limit `.editorconfig` sets for the file types it still sets one for.
const MAX_LINE: usize = 100;

/// Directories walked for the check, with the extensions each contributes. Which of them a run
/// from a crates.io tarball finds is answered by [`excluded_from_the_published_crate`], off the
/// manifest — named a second time here, the two lists would be free to disagree.
const ROOTS: [(&str, &[&str]); 4] = [
    ("src", &["rs"]),
    ("tests", &["rs"]),
    ("fuzz", &["rs", "toml"]),
    (".github", &["yml", "yaml"]),
];

/// The three options every script turns on, spelled once so the checks and the message they
/// print cannot drift apart.
const STRICT_MODE: &str = "set -euo pipefail";

/// The file that marks a directory under `tests/` as holding helpers rather than a test of its
/// own: cargo builds `tests/<dir>/mod.rs` into no test binary. Every `.rs` file beside one is
/// read as a shared helper, so a companion module pulled in by `mod cmd;` is one too.
const SHARED_MODULE: &str = "mod.rs";

/// What marks the declaration below it as a test, which [`tests_in`] reads `tests/edge_corpus.rs`
/// for.
const THE_TEST_ATTRIBUTE: &str = "#[test]";

/// What a comment opens with — the two slashes and nothing after them, so `///` and `//!` are
/// comments too: [`tests_in`] passes over one standing between an attribute and the declaration
/// it marks, and nothing else reads by it. A banner is narrower, and [`A_BANNER`] is what
/// [`banners_in`] reads one by. Neither becomes the other without loss, and the losses differ:
/// [`A_BANNER`] widened to this takes a doc comment between two rules for a banner, while this
/// narrowed to [`A_BANNER`] drops the mark on `//no space` and loses the test below it, whose
/// banner goes on naming it.
const A_COMMENT: &str = "//";

/// What a banner opens with, which is the spelling the corpus writes: [`banners_in`] takes the
/// name from what follows it, so a doc comment between two rules names nothing and a comment run
/// together with its text is not one either.
const A_BANNER: &str = "// ";

/// What an attribute opens with, which [`tests_in`] passes over as it does a comment.
const AN_ATTRIBUTE: &str = "#[";

/// What a declaration opens with, which is what [`tests_in`] reads a name from.
const A_DECLARATION: &str = "fn ";

/// What a rule of dashes opens with: [`banners_in`] takes the comment standing between two of
/// them for a banner.
const A_RULE: &str = "// ---";

/// What both sides of the failure of [`the_shared_helpers_start_the_binary_in_one_place`] open
/// with, written once so the two cannot drift apart. The space before what follows belongs to the
/// place the two are joined, not to the words themselves.
const THE_OPENING_OF_A_FAILURE: &str = "the shared helpers start the binary";

/// The ways a test can start the binary: the builder `assert_cmd` offers, and the path cargo
/// exports to an integration test. Both are ways in, so a check that counts one counts the
/// other.
const WAYS_TO_START_THE_BINARY: [&str; 2] = ["cargo_bin(", "CARGO_BIN_EXE"];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The text of `path`, which a check has named outright: a file a check names and the tree does
/// not hold is the check being wrong about the repository, not a file to pass over. The checks
/// that read what they walked say something else when a file will not open — a fuzz seed is not
/// UTF-8, and `scripts/` is kept out of the published crate — so they read it themselves.
fn text_of(path: &Path) -> String {
    std::fs::read_to_string(path).expect("the file is part of the published crate")
}

/// `path` as it is written in this repository, for a message that names a file: an absolute path
/// says where the checkout happens to be, which is not what the reader of a failure needs.
fn under_the_repo(path: &Path) -> String {
    path.strip_prefix(repo())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Every file under `<repo>/<dir>` with one of `exts`, sorted. Empty for a run from a crates.io
/// tarball over a directory the manifest keeps out of the published crate, and the walk says so
/// by asking [`excluded_from_the_published_crate`] rather than by naming those directories here.
///
/// The way in, and the only one: each check reads a list like this and passes when it is empty,
/// so a mistyped directory or extension would turn every one of them into a test that reads no
/// file and reports nothing. Nothing to look at is an answer only where there is no directory to
/// look in, and that is what the two assertions below hold.
fn walk(dir: &str, exts: &[&str]) -> Vec<PathBuf> {
    /// Every file under `dir` whose extension is in `exts`, recursively. Kept inside its caller
    /// so a collector that makes neither promise cannot be reached around the walk that does.
    fn collect(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
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
                    collect(&path, exts, out);
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

    let path = repo().join(dir);
    let mut found = Vec::new();
    collect(&path, exts, &mut found);
    found.sort();
    assert!(
        path.is_dir() || excluded_from_the_published_crate(dir),
        "`{dir}` is walked for the check and is not on disk, and the manifest does not keep it \
         out of the published crate: the name is wrong"
    );
    assert!(
        !found.is_empty() || !path.is_dir(),
        "`{}` is there but the walk came back with no {} file in it",
        path.display(),
        exts.iter()
            .map(|e| format!("`.{e}`"))
            .collect::<Vec<_>>()
            .join(" or ")
    );
    found
}

/// Whether the manifest keeps `dir` out of the published crate. A run from a crates.io tarball
/// finds no `.github/`, `scripts/` or `fuzz/`, so a walk that comes back with nothing is an
/// answer for those; for a directory that ships, not being on disk means the name is wrong.
/// Read off `Cargo.toml` rather than named a second time here.
fn excluded_from_the_published_crate(dir: &str) -> bool {
    let manifest = std::fs::read_to_string(repo().join("Cargo.toml")).expect("the manifest reads");
    excluded_in(&manifest, dir)
}

/// What stands between the brackets of the package's `exclude`, gathered from the line that sets
/// the key to the line that closes the list.
///
/// The key is the one the package section sets, not the first `exclude` in the file: the word
/// opens the comment above the list, opens the name of any other key spelled from it, and is a
/// key of `[workspace]` as well — where it names members the workspace leaves out, not files the
/// crate is published without. Each of the three answers about a directory this repository does
/// not exclude at all. Comments are dropped before any of it, because a comment is free to hold
/// a bracket, a comma or a quoted word, and each is what this is looking for.
fn the_exclude_list(manifest: &str) -> Option<String> {
    let mut section = "";
    let mut reading = false;
    let mut list = String::new();
    for line in manifest.lines().map(without_a_comment) {
        let mut line = line.trim();
        if !reading {
            if line.starts_with('[') {
                section = line;
            }
            if section != "[package]" || !sets_the_exclude_key(line) {
                continue;
            }
            reading = true;
            line = line.split_once('[')?.1;
        }
        if let Some(closed_at) = first_outside_a_string(line, ']') {
            list.push_str(&line[..closed_at]);
            return Some(list);
        }
        list.push_str(line);
        // A line break between two entries is a way of writing the list, and the space that
        // replaces it keeps the entries apart for the split on commas that reads them.
        list.push(' ');
    }
    None
}

/// `line` up to the comment it ends with, if it has one. A `#` opens a comment only outside a
/// string, so a path written with one in it keeps it.
fn without_a_comment(line: &str) -> &str {
    match first_outside_a_string(line, '#') {
        Some(at) => &line[..at],
        None => line,
    }
}

/// Every place `looked_for` stands in `line` outside a string.
///
/// All three characters this is asked about are ones a path may hold and TOML gives a meaning
/// to only outside a string: the `#` that opens a comment, the `]` that closes the list and the
/// `,` that separates one entry from the next. They are answered by one pass rather than a
/// reading each, so that a quoted entry cannot be honoured in one and overlooked in another.
/// TOML writes a string basic (`"x"`, where a backslash escapes what follows it) or literal
/// (`'x'`, where nothing does), and the pass reads both.
///
/// Every place rather than the first: a list holds as many separators as it has entries, and a
/// reading that stops at the first would part the list once and take the rest whole. Reported as
/// they are read rather than gathered, so that a caller after the first — the `#` that opens a
/// comment, the `]` that closes the list — stops the pass at it and asks for no place beyond.
fn outside_a_string(line: &str, looked_for: char) -> impl Iterator<Item = usize> + '_ {
    let mut quote = None;
    let mut escaped = false;
    line.char_indices().filter_map(move |(at, c)| {
        if escaped {
            escaped = false;
            return None;
        }
        match (quote, c) {
            (Some('"'), '\\') => escaped = true,
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), c) if c == open => quote = None,
            (None, c) if c == looked_for => return Some(at),
            _ => {}
        }
        None
    })
}

/// The first place `looked_for` stands in `line` outside a string, if it stands there at all.
fn first_outside_a_string(line: &str, looked_for: char) -> Option<usize> {
    outside_a_string(line, looked_for).next()
}

/// Whether `line` sets `exclude`, rather than merely opening with the word. A key is what stands
/// before the `=`, so `excluded-by-anything = [...]` is a different key and not this one.
fn sets_the_exclude_key(line: &str) -> bool {
    line.strip_prefix("exclude")
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// The entries `list` is written with, parted at the separator rather than at line breaks: one
/// entry per line is a way of writing the list, not part of it. The separator is the comma that
/// stands outside a string, since inside one it is a character of the name.
///
/// What stands between two separators and holds nothing is not an entry: a list written with a
/// comma before its close — the form this repository writes — would otherwise carry the name
/// nobody wrote, and an empty list would carry it alone.
fn entries_of(list: &str) -> Vec<&str> {
    let mut entries = Vec::new();
    let mut from = 0;
    for at in outside_a_string(list, ',') {
        entries.push(&list[from..at]);
        from = at + ','.len_utf8();
    }
    entries.push(&list[from..]);
    entries.retain(|entry| !entry.trim().is_empty());
    entries
}

/// Whether `manifest` keeps `dir` out of the published crate. Stated as a function over the text
/// so the rule can be checked against a manifest written in a form this repository does not use
/// today: cargo takes the list on one line or on many, and an entry may anchor itself to the
/// package root with a leading `/`.
fn excluded_in(manifest: &str, dir: &str) -> bool {
    let list = the_exclude_list(manifest).expect("the manifest sets `exclude`");
    entries_of(&list)
        .into_iter()
        .map(the_name_in)
        // Trimmed at both ends: a trailing `/` says the entry is a directory, a leading one
        // anchors it to the package root, and neither is part of the name.
        .any(|excluded| excluded.trim_matches('/') == dir)
}

/// The name `entry` writes: what stands between the quotes that open and close it.
///
/// Both quotes: TOML writes a string basic (`"x"`, where a backslash escapes what follows it)
/// or literal (`'x'`, where nothing does), and a path is the kind of value written literally to
/// keep a backslash out of an escape. One pair is taken off rather than every quote at either
/// end, since a quote of the other kind inside them is a character of the name.
///
/// Of the escapes a basic string writes, the two a path may hold are resolved: the quote that
/// would otherwise end the entry, and the backslash that writes itself. Anything else a
/// backslash opens (`\n`, `\uXXXX`) is left as the two characters written — an exclude list
/// naming such a path is not a form this repository reads.
fn the_name_in(entry: &str) -> String {
    let entry = entry.trim();
    if let Some(literal) = entry.strip_prefix('\'').and_then(|e| e.strip_suffix('\'')) {
        return literal.to_owned();
    }
    let Some(basic) = entry.strip_prefix('"').and_then(|e| e.strip_suffix('"')) else {
        return entry.to_owned();
    };
    let mut name = String::new();
    let mut reading = basic.chars();
    while let Some(c) = reading.next() {
        match (c, reading.clone().next()) {
            ('\\', Some(escaped @ ('"' | '\\'))) => {
                name.push(escaped);
                reading.next();
            }
            _ => name.push(c),
        }
    }
    name
}

/// A comma separates one entry from the next only outside a string: inside one it is a
/// character of the name, and a path is free to hold it on every system this runs on. Read as a
/// separator wherever it stands, `exclude = ["a,b"]` becomes two entries that were never
/// written, and the directory that was excluded is read as shipping.
#[test]
fn a_comma_inside_an_entry_is_a_character_of_the_name() {
    let manifest = "[package]\nexclude = [\"a,b\", \".github/\"]\n";

    assert!(excluded_in(manifest, "a,b"), "the entry written is `a,b`");
    assert!(!excluded_in(manifest, "a"), "`a` was never excluded");
    assert!(!excluded_in(manifest, "b"), "`b` was never excluded");
}

/// A comma before the close is a way of writing the list, not a separator with an entry behind
/// it — cargo reads the list written either way, and this repository writes it with one. Taken
/// as an entry, what stands there is the name nobody wrote: the empty one, which answers that a
/// directory with no name is kept out of the crate.
#[test]
fn a_separator_before_the_close_leaves_no_entry_behind_it() {
    let trailing = "[package]\nexclude = [\".github/\",]\n";
    assert!(
        excluded_in(trailing, ".github"),
        "the entry written is `.github/`"
    );
    assert!(
        !excluded_in(trailing, ""),
        "no entry names the empty string"
    );

    let empty = "[package]\nexclude = []\n";
    assert!(!excluded_in(empty, ""), "an empty list excludes nothing");
}

/// The quotes around an entry say where it starts and ends; the name is what stands between
/// them, with the escapes a basic string gives a meaning to resolved. Read by trimming quotes
/// off both ends, `"a\"b#c"` keeps the backslash the escape was written with and loses a quote
/// at each end of `"''"`, so a directory cargo excludes is read as shipping.
#[test]
fn the_name_of_an_entry_is_what_stands_inside_its_quotes() {
    let escaped_quote = "[package]\nexclude = [\"a\\\"b#c\", \".github/\"]\n";
    assert!(excluded_in(escaped_quote, "a\"b#c"), "the name is `a\"b#c`");

    let quotes_of_the_other_kind = "[package]\nexclude = [\"''\", \".github/\"]\n";
    assert!(
        excluded_in(quotes_of_the_other_kind, "''"),
        "the name is two single quotes"
    );
}

/// The manifest is read for an answer about a directory that is not on disk, and that answer is
/// wrong in the only run where it is asked — from a crates.io tarball — if the reading is tied
/// to how the list happens to be written today. Cargo accepts the list on one line or on many,
/// accepts an entry anchored to the package root with a leading `/` (the form `bstr` and
/// `aho-corasick` are published in), accepts an entry written as a literal string, lets a
/// comment stand above the key, inside the list or after an entry — and holds a key of the same
/// name in `[workspace]`, where it means something else entirely.
#[test]
fn the_exclude_list_is_read_in_every_form_cargo_accepts() {
    let forms = [
        (
            "on its own lines",
            "[package]\nexclude = [\n    \".github/\",\n    \"scripts/\",\n]\n",
        ),
        (
            "on one line",
            "[package]\nexclude = [\".github/\", \"scripts/\"]\n",
        ),
        (
            "anchored to the root",
            "[package]\nexclude = [\"/.github\", \"/scripts\"]\n",
        ),
        (
            "under a comment holding a bracket",
            "[package]\n# what exclude covers: [.github] and the scripts\n\
             exclude = [\"/.github\"]\n",
        ),
        (
            "after a key whose name opens with the same word",
            "[package]\nexcluded-by-mistake = [\"src\"]\nexclude = [\"/.github\"]\n",
        ),
        (
            "beside a workspace that excludes a member",
            "[workspace]\nexclude = [\"src\"]\n\n[package]\nexclude = [\"/.github\"]\n",
        ),
        (
            "as literal strings",
            "[package]\nexclude = ['.github/', 'scripts/']\n",
        ),
        (
            "under a comment inside the list holding a bracket",
            "[package]\nexclude = [\n    # kept out of the crate [and out of the docs]\n\
                 \".github/\",\n    \"scripts/\",\n]\n",
        ),
        (
            "with a comment after an entry",
            "[package]\nexclude = [\n    \".github/\", # CI config\n    \"scripts/\",\n]\n",
        ),
        (
            "with an entry holding a closing bracket",
            "[package]\nexclude = [\"a]b\", \".github/\", \"scripts/\"]\n",
        ),
        (
            "with an entry holding an escaped quote",
            "[package]\nexclude = [\"a\\\"b#c\", \".github/\", \"scripts/\"]\n",
        ),
    ];

    for (form, manifest) in forms {
        assert!(
            excluded_in(manifest, ".github"),
            "`.github` is excluded by a manifest that writes the list {form}, and was read as \
             kept in the published crate"
        );
        assert!(
            !excluded_in(manifest, "src"),
            "`src` ships, and a manifest writing the list {form} was read as excluding it"
        );
    }
}

/// Every shell script the repository ships, sorted.
fn shell_scripts() -> Vec<PathBuf> {
    walk("scripts", &["sh"])
}

/// Whether `text` turns the options on before it runs anything: the shebang, blank lines and
/// comments may come first, a command may not. Stated as a function over the text so the rule
/// can be checked against a script that is not in the repository — walking `scripts/` only ever
/// shows what the scripts happen to do today.
fn opens_with_strict_mode(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        == Some(STRICT_MODE)
}

/// `.editorconfig` sets `max_line_length = 100`, and it applies to code, workflows and the fuzz
/// manifest — Markdown and `Cargo.toml` are exempted there, with the reasons written next to the
/// exemption. `cargo fmt` enforces the same width for code but leaves comments and string
/// literals alone, which is exactly where the long lines came from before this test existed.
#[test]
fn line_length_stays_within_the_editorconfig_limit() {
    let mut offenders = Vec::new();
    for (dir, exts) in ROOTS {
        for path in walk(dir, exts) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue; // not UTF-8: a fuzz seed or a fixture, not a source file
            };
            for (no, line) in text.lines().enumerate() {
                if line.chars().count() > MAX_LINE {
                    let rel = under_the_repo(&path);
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

/// The names the banners of `lines` give, in the order the file writes them: a banner is the
/// comment line between two rules of dashes, and a rule is not one — three of them in a row would
/// otherwise name a test whose name is dashes. Only the margin is read, here and in [`tests_in`]:
/// the corpus writes diffs as string literals, and the lines of a hunk carry their marker — a
/// space, a plus or a minus — so a rule copied into one stands off the margin. Headers of a diff
/// do reach the margin, and so would a fixture writing a file rather than a diff; what holds is
/// that neither is a comment, and that both walks read the margin the same way. The name is what
/// stands in the banner less the spaces around it, which is how [`tests_in`] reads the name of a
/// declaration too, so either written wider than the corpus writes it still names the same test.
/// A banner with nothing left in it after the spaces names no test and is no banner.
fn banners_in(lines: &[&str]) -> Vec<String> {
    let is_rule = |line: &str| line.starts_with(A_RULE);
    let mut banners = Vec::new();
    for triple in lines.windows(3) {
        let [above, line, below] = triple else {
            continue;
        };
        if is_rule(above) && is_rule(below) && !is_rule(line) {
            if let Some(name) = line.strip_prefix(A_BANNER) {
                let named = name.trim();
                if !named.is_empty() {
                    banners.push(named.to_string());
                }
            }
        }
    }
    banners
}

/// The names of the tests `lines` declares, in the order the file writes them: the first
/// declaration below [`THE_TEST_ATTRIBUTE`] is the one it stands over, however many attributes,
/// comments or blank lines come between. A window of lines instead would take in whatever a short
/// test is followed by, and the mark is dropped where anything else stands, so a declaration the
/// walk does not know does not hand it to whatever comes after. The name is what stands between
/// the opening and the brackets less the spaces around it, as [`banners_in`] reads the name of a
/// banner, and a declaration with nothing left in it names no test.
fn tests_in(lines: &[&str]) -> Vec<String> {
    let mut tests = Vec::new();
    let mut below_the_attribute = false;
    for &line in lines {
        if line == THE_TEST_ATTRIBUTE {
            below_the_attribute = true;
            continue;
        }
        if !below_the_attribute {
            continue;
        }
        if let Some(name) = line.strip_prefix(A_DECLARATION) {
            let named = name.split_once('(').map_or(name, |(named, _)| named).trim();
            if !named.is_empty() {
                tests.push(named.to_string());
            }
            below_the_attribute = false;
        } else if !line.starts_with(AN_ATTRIBUTE)
            && !line.starts_with(A_COMMENT)
            && !line.trim().is_empty()
        {
            below_the_attribute = false;
        }
    }
    tests
}

/// `tests/edge_corpus.rs` writes diffs as string literals, and the lines of a hunk carry their
/// marker — a context line is the line it copies with a space in front. Read past that marker, the
/// corpus would introduce tests nobody wrote.
#[test]
fn a_line_inside_a_fixture_is_neither_a_banner_nor_a_test() {
    let fixture = [
        " // ---",
        " // from_a_fixture",
        " // ---",
        " #[test]",
        " fn from_a_fixture() {}",
    ];

    assert_eq!(
        banners_in(&fixture),
        Vec::<String>::new(),
        "a rule copied into a diff is the diff, not a banner"
    );
    assert_eq!(
        tests_in(&fixture),
        Vec::<String>::new(),
        "an attribute copied into a diff introduces nothing"
    );
}

/// The corpus writes its tests at the margin, and both walks read them there: a test written
/// inside a module is out of the convention rather than half in it, and neither walk takes it,
/// so the two lists stay equal.
#[test]
fn a_test_inside_a_module_is_outside_what_the_corpus_states() {
    let lines = [
        "mod inner {",
        "    // ---",
        "    // nested",
        "    // ---",
        "    #[test]",
        "    fn nested() {}",
        "}",
    ];

    assert_eq!(
        banners_in(&lines),
        tests_in(&lines),
        "neither walk reads it"
    );
    assert_eq!(
        tests_in(&lines),
        Vec::<String>::new(),
        "what stands inside a module is not what the corpus states"
    );
}

/// A declaration the walk does not know keeps nothing waiting: `async fn` is not read as a
/// declaration here, and the mark it was left with would otherwise land on the next declaration
/// the walk does know.
#[test]
fn a_declaration_the_walk_does_not_know_drops_the_mark() {
    let lines = ["#[test]", "async fn nested() {}", "fn helper() {}"];

    assert_eq!(
        tests_in(&lines),
        Vec::<String>::new(),
        "a mark that found no declaration it knows is dropped where that declaration stood"
    );
}

/// A comment and a blank line below the attribute are neither it nor the declaration, and both
/// belong to the declaration: dropped on either, the test below would be lost and its banner
/// would be reported as naming nothing. The corpus writes neither of them there today — the only
/// lines between an attribute and its declaration are other attributes — so what the walk does
/// with them is stated here rather than found out, and what the pair holds together is that the
/// mark carries over more than one line.
#[test]
fn a_comment_and_a_blank_line_below_the_attribute_keep_the_mark() {
    let lines = ["#[test]", "// what this one is about", "", "fn a() {}"];

    assert_eq!(
        tests_in(&lines),
        vec!["a".to_string()],
        "the attribute still stands over the declaration below the comment"
    );
}

/// What [`banners_in`] says of a rule, held where it can be broken: three rules in a row, whose
/// middle one the corpus check would report as a banner nobody wrote.
#[test]
fn a_rule_is_not_a_banner_of_its_own() {
    let lines = ["// ---", "// ---", "// ---"];

    assert_eq!(
        banners_in(&lines),
        Vec::<String>::new(),
        "a third rule names no test, so it introduces none"
    );
}

/// The attribute marks the declaration it stands over, and a helper written below a one-line test
/// is not that declaration: counted as a test, it would want a banner naming it and the corpus
/// check would fail over a convention nobody broke.
#[test]
fn a_function_written_below_a_test_is_not_a_test_itself() {
    let lines = ["#[test]", "fn a() {}", "", "fn helper() {}"];

    assert_eq!(
        tests_in(&lines),
        vec!["a".to_string()],
        "the attribute belongs to the declaration it stands over, not to the one after that"
    );
}

/// A comment below the attribute is a comment however it is written: read for a space after the
/// two slashes, `///`, `//!` and a comment run together with its text would drop the mark, and the
/// test below them would be lost while its banner still names it.
#[test]
fn every_spelling_of_a_comment_below_the_attribute_keeps_the_mark() {
    for comment in [
        "/// what this one is about",
        "//! about the file",
        "//no space",
    ] {
        let lines = ["#[test]", comment, "fn a() {}"];

        assert_eq!(
            tests_in(&lines),
            vec!["a".to_string()],
            "`{comment}` is a comment, and the declaration below it is still the one marked"
        );
    }
}

/// A banner is a comment written the way the corpus writes it, with a space after the slashes: a
/// doc comment standing between two rules belongs to what is below it, and a comment run together
/// with its text is not the spelling the corpus states. Read loosely, the first would give the
/// name `/ name` and the second would pass unremarked.
#[test]
fn a_banner_is_a_comment_written_the_way_the_corpus_writes_one() {
    for comment in ["/// name", "//name", "//!name"] {
        let lines = ["// ---", comment, "// ---"];

        assert_eq!(
            banners_in(&lines),
            Vec::<String>::new(),
            "`{comment}` is not how the corpus writes a banner"
        );
    }
}

/// The name a banner gives is what stands in it, less the spaces around it: the corpus writes one
/// space after the slashes and none at the end, and a banner spelled wider would otherwise be
/// compared with the declaration below it and found to name another test.
#[test]
fn a_banner_written_with_spare_spaces_names_what_it_holds() {
    let lines = ["// ---", "//   spaced   name  ", "// ---"];

    assert_eq!(
        banners_in(&lines),
        vec!["spaced   name".to_string()],
        "the spaces around a name are the spelling of the banner, not part of the name"
    );
}

/// A banner with nothing left in it after the spaces names no test, and naming one is what a
/// banner is for: read as a banner, it would introduce a test called nothing. Dropped instead, it
/// leaves the corpus check one banner short of its tests, and the check falls on the count as it
/// does on a name.
#[test]
fn a_banner_with_no_name_left_in_it_is_not_a_banner() {
    let lines = ["// ---", "//   ", "// ---"];

    assert_eq!(
        banners_in(&lines),
        Vec::<String>::new(),
        "a banner names a test, and there is no name here for it to give"
    );
}

/// The spacing of a declaration is no more part of its name than the spacing of a banner is of
/// that one: read as written, a declaration spaced wider than the corpus writes it would be
/// compared with the banner naming it and the two would be found to name different tests.
#[test]
fn a_declaration_written_with_spare_spaces_names_what_it_holds() {
    let lines = ["// ---", "//  a ", "// ---", "#[test]", "fn  a () {}"];

    assert_eq!(
        tests_in(&lines),
        vec!["a".to_string()],
        "the spaces on either side of a name are the spelling of the declaration, not the name"
    );
    assert_eq!(
        banners_in(&lines),
        tests_in(&lines),
        "the two walks read a name the same way, so spacing is what neither of them reads"
    );
}

/// A declaration with nothing left in it after the spaces names no test, as a banner without a
/// name introduces none: read as a test, it would want a banner naming nothing, and the corpus
/// check would fail over a convention nobody broke. It is still a declaration, though, so the
/// mark it stood under is spent on it rather than handed to the next one — valid Rust does not
/// write such a line, and the walk is told what to do with it here rather than by the corpus.
#[test]
fn a_declaration_with_no_name_left_in_it_names_nothing() {
    let lines = ["#[test]", "fn  () {}", "fn later() {}"];

    assert_eq!(
        tests_in(&lines),
        Vec::<String>::new(),
        "a name is what the walk reads here, there is none in this line to read, \
         and the declaration below is not the one the mark stood over"
    );
}

/// A line of spaces is a blank line: dropping the mark on one would lose the test below it while
/// its banner stays, and no line of the corpus is written that way for the walk to find out.
#[test]
fn a_line_of_spaces_below_the_attribute_keeps_the_mark() {
    let lines = ["#[test]", "   ", "fn a() {}"];

    assert_eq!(
        tests_in(&lines),
        vec!["a".to_string()],
        "a line holding nothing but spaces holds nothing"
    );
}

/// Attributes and comments are passed over where the corpus writes them, which is the margin: read
/// past an indent, the walk would carry the mark through a body and take the declaration after it.
#[test]
fn an_indented_line_below_the_attribute_drops_the_mark() {
    for indented in ["    #[cfg(unix)]", "    // a note"] {
        let lines = ["#[test]", indented, "fn a() {}"];

        assert_eq!(
            tests_in(&lines),
            Vec::<String>::new(),
            "`{indented}` stands off the margin, where the walk does not read"
        );
    }
}

/// The attribute the corpus writes below `#[test]` is passed over: `#[cfg(unix)]` stands between
/// the mark and the declaration of one of its tests, and `#[cfg(target_os = "linux")]` of two.
#[test]
fn an_attribute_below_the_attribute_keeps_the_mark() {
    let lines = ["#[test]", "#[cfg(unix)]", "fn a() {}"];

    assert_eq!(
        tests_in(&lines),
        vec!["a".to_string()],
        "an attribute between the mark and the declaration is not what drops it"
    );
}

/// Each half of reading at the margin is held on its own: a rule indented is no rule, and a
/// comment indented between two rules has no name to give.
#[test]
fn an_indented_line_is_read_by_neither_half_of_a_banner() {
    let indented_rule = ["    // ---", "// name", "    // ---"];
    let indented_name = ["// ---", "    // name", "// ---"];

    assert_eq!(
        banners_in(&indented_rule),
        Vec::<String>::new(),
        "a rule away from the margin is not one"
    );
    assert_eq!(
        banners_in(&indented_name),
        Vec::<String>::new(),
        "a comment away from the margin gives no name"
    );
}

/// A comment that only looks like a banner from one side is none: both rules are what makes one,
/// and a check holding one of them would take the prose above a test for its name.
#[test]
fn a_comment_needs_a_rule_on_both_sides_to_be_a_banner() {
    let rule_below_only = ["// prose", "// name", "// ---"];
    let rule_above_only = ["// ---", "// name", "// prose"];

    assert_eq!(
        banners_in(&rule_below_only),
        Vec::<String>::new(),
        "a rule below and none above introduces nothing"
    );
    assert_eq!(
        banners_in(&rule_above_only),
        Vec::<String>::new(),
        "a rule above and none below introduces nothing"
    );
}

/// Both walks read in the order the file writes, which is what the corpus check compares: sorted
/// lists would hold two banners swapped over their tests to be the same file.
#[test]
fn a_banner_and_a_test_are_read_in_the_order_they_stand() {
    let lines = [
        "// ---",
        "// b",
        "// ---",
        "#[test]",
        "fn b() {}",
        "// ---",
        "// a",
        "// ---",
        "#[test]",
        "fn a() {}",
    ];

    assert_eq!(
        banners_in(&lines),
        vec!["b".to_string(), "a".to_string()],
        "the order the file writes is what tells a banner from the one below it"
    );
    assert_eq!(
        tests_in(&lines),
        vec!["b".to_string(), "a".to_string()],
        "the tests are read the same way, so the two lists line up where nothing is out of place"
    );
}

/// `tests/edge_corpus.rs` introduces each of its tests with a banner naming it. The banners were
/// numbered once, and the numbers stopped matching the tests the first time one was added without
/// one — after which "test 14" pointed at the fifteenth test, and the last cycle managed to add
/// both an unnumbered test and a new number in the same range. Names cannot drift that way, and
/// this keeps the banners equal to the tests, which a comment could not.
///
/// The two are compared in the order the file writes them rather than as sets: two banners
/// swapped over their tests leave both sets equal, and every test would still be introduced by
/// a banner naming another one.
#[test]
fn every_edge_corpus_test_is_introduced_by_a_banner_naming_it() {
    let text = text_of(&repo().join("tests").join("edge_corpus.rs"));
    let lines: Vec<&str> = text.lines().collect();

    assert_eq!(
        banners_in(&lines),
        tests_in(&lines),
        "every test needs a banner naming it, and every banner a test of that name, \
         which is the order they stand in; a banner opens `{A_BANNER}` at the margin between \
         two rules and holds a name, so a comment opened otherwise, or holding nothing but \
         spaces, introduces no test, however the name itself is spaced"
    );
}

/// Every script opens with the same three options, so a command that fails stops the run
/// instead of letting the next one work on what the failed one did not produce. Five scripts
/// had them and two did not, which is the kind of difference nobody notices until a release
/// script carries on past a failed step.
#[test]
fn every_shell_script_stops_at_the_first_failure() {
    let mut offenders = Vec::new();
    for path in shell_scripts() {
        let text = std::fs::read_to_string(&path).expect("a readable script");
        if !opens_with_strict_mode(&text) {
            offenders.push(under_the_repo(&path));
        }
    }

    assert!(
        offenders.is_empty(),
        "these scripts do not open with `{STRICT_MODE}`: {}",
        offenders.join(", ")
    );
}

/// What "opens with" means, checked against text rather than against the scripts themselves:
/// the walk above can only report what the repository happens to hold today, and every script
/// in it passes either reading of the rule. A script that runs a command first has already run
/// it by the time a failure would have stopped the run, which is the whole point of the option.
#[test]
fn strict_mode_reached_after_a_command_is_not_opening_with_it() {
    let opens = format!("#!/usr/bin/env bash\n# what the script does\n\n{STRICT_MODE}\nrm -rf x\n");
    assert!(
        opens_with_strict_mode(&opens),
        "a shebang, a comment and a blank line may come before the options"
    );

    let too_late = format!("#!/usr/bin/env bash\nrm -rf x\n{STRICT_MODE}\n");
    assert!(
        !opens_with_strict_mode(&too_late),
        "the options are turned on after a command has already run"
    );
}

/// `.editorconfig` asks shell scripts to indent by four spaces, and until now that was a
/// request an editor might honour rather than a rule. shellcheck, added to CI in the same
/// cycle, does not close this: it reads a script for what it does, not for how it is laid out.
/// The scripts carry release-critical logic, so a diff of one should show the change and not
/// a re-indentation around it.
#[test]
fn every_shell_script_indents_by_four_spaces() {
    let mut offenders = Vec::new();
    for path in shell_scripts() {
        let text = std::fs::read_to_string(&path).expect("a readable script");
        for (no, line) in text.lines().enumerate() {
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
            offenders.push(format!("{}:{}: {reason}", under_the_repo(&path), no + 1));
        }
    }

    assert!(
        offenders.is_empty(),
        "{} line(s) disagree with the four-space indent .editorconfig sets for *.sh:\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// What a failure of [`the_shared_helpers_start_the_binary_in_one_place`] says. The check falls
/// both ways, and the empty side has nothing to list: a message that promises addresses and
/// prints none says neither what was read nor what was looked for in it. Reading nothing at all
/// is the same promise made emptier, so that side names no list rather than an empty one.
fn what_the_starts_say(starts: &[String], read: &[String]) -> String {
    let found = if !starts.is_empty() {
        format!(
            "where these stand, and one start is what they hold:\n{}",
            starts.join("\n")
        )
    } else if read.is_empty() {
        "nowhere, and no helper was read to look in".to_string()
    } else {
        format!("nowhere among the helpers read {}", a_list_of(read))
    };
    format!("{THE_OPENING_OF_A_FAILURE} {found}")
}

/// How [`what_the_starts_say`] writes the addresses it read, written once: a test holds that the
/// empty list is never printed, and it can only hold it against the same brackets the message
/// puts around one.
fn a_list_of(read: &[String]) -> String {
    format!("({})", read.join(", "))
}

/// Whether what was said opens with the words every side shares and `what` after them: the
/// opening is written once in [`THE_OPENING_OF_A_FAILURE`], and each test of the message holds
/// the word its own side goes on with.
fn opens_the_sentence_with(said: &str, what: &str) -> bool {
    said.starts_with(&format!("{THE_OPENING_OF_A_FAILURE} {what}"))
}

/// Whether what was said is a whole sentence rather than one broken off: a message ending in a
/// newline, a colon or a space stops where a list was to follow. Every side of
/// [`what_the_starts_say`] is held against this, and each against the same one of it — the tests
/// wrote the break out separately once, and the lists of what counts as a break drifted.
fn is_a_whole_sentence(said: &str) -> bool {
    !said.ends_with('\n') && !said.ends_with(':') && !said.ends_with(' ')
}

/// What [`opens_the_sentence_with`] holds, held where it can be broken: the three tests that call
/// it all pass it a whole message and the word its side goes on with, so between them the helper
/// could read `what` not at all, or look for the opening anywhere in the message rather than at
/// its start. The first of those would leave the space between the opening and what follows it
/// held by nothing, which is what all three of them say they hold.
#[test]
fn a_sentence_opens_with_the_words_every_side_shares() {
    let opening = THE_OPENING_OF_A_FAILURE;

    assert!(
        opens_the_sentence_with(&format!("{opening} where these stand"), "where"),
        "the opening and the word after it are how a side of the message begins"
    );
    assert!(
        !opens_the_sentence_with(&format!("{opening}where these stand"), "where"),
        "the opening and what follows it are two words, not one"
    );
    assert!(
        !opens_the_sentence_with(&format!("{opening} nowhere, and no helper"), "where"),
        "another side of the message is not this one"
    );
    assert!(
        !opens_the_sentence_with(&format!("read: {opening} where"), "where"),
        "a message opens where it starts, not wherever the words turn up"
    );
}

/// What [`a_list_of`] writes, held where it can be changed: the message and the test of the empty
/// side both go through this helper, so between them the brackets and what stands between two
/// addresses could be anything at all and neither would notice.
#[test]
fn a_list_is_written_in_brackets_with_its_addresses_told_apart() {
    assert_eq!(
        a_list_of(&["a.rs".to_string(), "b.rs".to_string()]),
        "(a.rs, b.rs)",
        "a list stands in brackets, and what is in it is told apart"
    );

    assert_eq!(
        a_list_of(&[]),
        "()",
        "the empty list is the brackets alone, which is what the message must not print"
    );
}

/// What [`is_a_whole_sentence`] holds, held where it can be broken: read only from the tests that
/// call it, the predicate could answer `true` to everything and every one of them would still
/// pass, since none of them asks it about a message that stops short.
#[test]
fn a_sentence_is_whole_unless_it_stops_where_a_list_would_follow() {
    for broken in ["what was said:", "what was said ", "what was said\n"] {
        assert!(
            !is_a_whole_sentence(broken),
            "`{broken}` stops where something was to follow"
        );
    }

    assert!(
        is_a_whole_sentence("what was said"),
        "a message that named what it promised ends where it ends"
    );
}

/// The side of [`what_the_starts_say`] with nothing read at all: it is still a sentence, not one
/// broken off before what it promised to name, and an empty pair of brackets is such a break —
/// the message would offer a list and hold none. The caller cannot reach this side, since it
/// stops on helpers it did not find; what is held here is the message, not a run.
#[test]
fn a_failure_with_nothing_read_is_still_a_sentence() {
    let said = what_the_starts_say(&[], &[]);

    assert!(
        !said.contains(&a_list_of(&[])),
        "a list is promised only where there is one: {said}"
    );
    assert!(
        is_a_whole_sentence(&said),
        "the sentence stops where it is done, not where a list was to follow: {said}"
    );
    assert!(
        opens_the_sentence_with(&said, "nowhere"),
        "the opening and what follows it are two words, not one: {said}"
    );
}

/// The empty side of [`what_the_starts_say`], which is the one with nothing to list: what it read
/// is what tells the reader where to look.
#[test]
fn a_failure_with_no_start_names_the_files_it_read() {
    let read = [
        "tests/common/mod.rs".to_string(),
        "tests/common/cmd.rs".to_string(),
    ];

    let said = what_the_starts_say(&[], &read);

    assert!(
        said.contains(&a_list_of(&read)),
        "nothing found is what was read, written as the list it is: {said}"
    );
    assert!(
        is_a_whole_sentence(&said),
        "the sentence stops where it is done, not where a list was to follow: {said}"
    );
    assert!(
        opens_the_sentence_with(&said, "nowhere"),
        "the opening and what follows it are two words, not one: {said}"
    );
}

/// Two starts are what a second answer to "what a run is" looks like, so both are named. The count
/// is left to `assert_eq!`, which prints it of its own: said twice, it reads as two numbers to
/// reconcile. This side is held whole as the empty ones are, and it is asked once more with
/// nothing read: what was found is what it says, and what was read does not enter it — so a
/// failure here can be the sentence breaking off or the reading leaking in, not only a start
/// gone missing.
#[test]
fn a_failure_with_two_starts_holds_both_of_them() {
    let starts = ["a.rs:7".to_string(), "b.rs:9".to_string()];

    let said = what_the_starts_say(&starts, &["a.rs".to_string(), "b.rs".to_string()]);

    assert!(
        said.contains("a.rs:7") && said.contains("b.rs:9"),
        "both starts are what a second answer looks like: {said}"
    );
    assert!(
        !said.contains('2'),
        "the count is what `assert_eq!` prints of its own, and these two carry no other 2: {said}"
    );
    assert!(
        opens_the_sentence_with(&said, "where"),
        "the opening and what follows it are two words, not one: {said}"
    );

    assert!(
        is_a_whole_sentence(&said),
        "the sentence stops where it is done, not where a list was to follow: {said}"
    );

    assert_eq!(
        what_the_starts_say(&starts, &[]),
        said,
        "what was found is what this side is about, and what was read does not enter it"
    );
}

/// The shared helpers start the binary in one place. Two of them built the command themselves,
/// so what a run is — the stream it takes, the arguments it is given — was written twice and was
/// free to differ; the stream did differ, and a file whose input is not text had to keep a helper
/// of its own because of it.
///
/// Every `.rs` file beside a [`SHARED_MODULE`] under `tests/` is read rather than the one
/// directory that holds the helpers today, and both of [`WAYS_TO_START_THE_BINARY`] are counted
/// where they stand rather than by the line: a second shared module, a companion file beside
/// this one, the path cargo exports used in place of the builder, or two starts written on one
/// line would each be a second answer to what a run is. A line holding two of them is reported
/// twice at the one address, which is what counting where they stand means.
///
/// What the rule is about is that there is one start, not what it is called: a start these
/// helpers agree on is the way in whatever name it is given, and asking for a name meant reading
/// Rust — visibility, `async`, comments, lifetimes inside a signature — in a test, which is a
/// second parser to be wrong. Nothing is stripped for the same reason: a mention counts wherever
/// it stands, so these helpers say "the binary" in prose rather than spelling the way in.
#[test]
fn the_shared_helpers_start_the_binary_in_one_place() {
    let shared: Vec<PathBuf> = walk("tests", &["rs"])
        .into_iter()
        .filter(|file| file.with_file_name(SHARED_MODULE).is_file())
        .collect();
    assert!(
        !shared.is_empty(),
        "no `{SHARED_MODULE}` under `tests/`: the helpers every test binary shares are read here"
    );

    let mut starts = Vec::new();
    let mut read = Vec::new();
    for file in shared {
        let text = text_of(&file);
        let named = under_the_repo(&file);
        read.push(named.clone());
        for (no, line) in text.lines().enumerate() {
            let found: usize = WAYS_TO_START_THE_BINARY
                .iter()
                .map(|way| line.matches(way).count())
                .sum();
            for _ in 0..found {
                starts.push(format!("{named}:{}", no + 1));
            }
        }
    }

    assert_eq!(starts.len(), 1, "{}", what_the_starts_say(&starts, &read));
}
