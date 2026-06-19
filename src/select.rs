use crate::model::*;
use crate::split::auto_split_hunk;
use crate::subhunk_id::subhunk_hash;
use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum Selector {
    File {
        path: Option<String>,
        indices: IndexSet,
    },
    /// Address sub-hunks by their content id (the `@<id>` form). Matches every sub-hunk in the
    /// patch whose [`subhunk_id`](crate::subhunk_id::subhunk_id) equals `id`.
    Id(String),
}

/// The index part of a `File` selector: either an explicit list of 1-based indices or `*`,
/// meaning every sub-hunk of the addressed file.
#[derive(Debug, PartialEq, Eq)]
pub enum IndexSet {
    All,
    List(Vec<usize>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum SelectError {
    BadSelector(String),
    UnknownPath(String),
    AmbiguousPath(String),
    NoIndex(String),
    /// An `@<id>` selector matched no sub-hunk in the patch.
    UnknownId(String),
    /// An `@<id>` selector matched sub-hunks with differing content (an accidental hash
    /// collision between distinct changes). Carries the colliding id.
    IdCollision(String),
    EmptySelection,
}

impl fmt::Display for SelectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectError::BadSelector(s) => write!(f, "bad selector: {s}"),
            SelectError::UnknownPath(p) => write!(f, "no file in the diff matches path: {p}"),
            SelectError::AmbiguousPath(p) => write!(f, "path matches more than one file: {p}"),
            SelectError::NoIndex(s) => write!(f, "no such sub-hunk: {s}"),
            SelectError::UnknownId(id) => write!(f, "no sub-hunk has id: {id}"),
            SelectError::IdCollision(id) => write!(
                f,
                "id {id} collides between distinct sub-hunks; address them by path:N instead"
            ),
            SelectError::EmptySelection => write!(f, "selection is empty"),
        }
    }
}

/// Auto-split one file's hunks into its ordered sub-hunks (empty for a binary file).
pub fn build_file_subs(f: &FileDiff) -> Vec<Hunk> {
    match &f.content {
        FileContent::Text(hunks) => {
            // Each source hunk yields at least one sub-hunk, so the hunk count is a lower
            // bound on the result length; reserve it to avoid repeated reallocation.
            let mut subs = Vec::with_capacity(hunks.len());
            for h in hunks {
                subs.extend(auto_split_hunk(h));
            }
            subs
        }
        FileContent::Binary(_) => Vec::new(),
    }
}

/// Per-file auto-split view: each file maps to its ordered sub-hunks (empty for binary).
pub fn build_view(patch: &Patch) -> Vec<(usize, Vec<Hunk>)> {
    patch
        .files
        .iter()
        .enumerate()
        .map(|(fi, f)| (fi, build_file_subs(f)))
        .collect()
}

/// Parse selector args. Forms, in order of precedence:
///
/// 1. `path:set` — `set` is `*` or a comma-separated list of indices/ranges (`1,3`, `2-4`,
///    `src/f:1,3-5`, `src/f:*`). Recognised when a ':' is present and the text after the LAST
///    ':' parses as a valid set; the path may itself contain ':' or a leading '@'.
/// 2. `@id` — address sub-hunks by content id.
/// 3. bare `set` — `*` or an index list, for single-file diffs (the path is resolved later).
pub fn parse_selectors(args: &[String]) -> Result<Vec<Selector>, SelectError> {
    let mut out = Vec::new();
    for a in args {
        // 1. path:set form. Checked first so a file named "@foo" (addressed "@foo:1") and a
        //    path containing ':' are not misread as an id or a bare set.
        if let Some((p, l)) = a.rsplit_once(':') {
            if !p.is_empty() {
                if let Ok(indices) = parse_index_set(l) {
                    out.push(Selector::File {
                        path: Some(p.to_string()),
                        indices,
                    });
                    continue;
                }
            }
        }
        // 2. @id form.
        if let Some(id) = a.strip_prefix('@') {
            if id.is_empty() {
                return Err(SelectError::BadSelector(a.clone()));
            }
            out.push(Selector::Id(id.to_string()));
            continue;
        }
        // 3. bare set.
        let indices = parse_index_set(a).map_err(|_| SelectError::BadSelector(a.clone()))?;
        out.push(Selector::File {
            path: None,
            indices,
        });
    }
    Ok(out)
}

/// Parse the index part of a `File` selector: `*` (all) or a comma-separated index list.
fn parse_index_set(s: &str) -> Result<IndexSet, ()> {
    if s == "*" {
        return Ok(IndexSet::All);
    }
    parse_index_list(s).map(IndexSet::List)
}

/// Upper bound on the number of indices a selector may materialise. The real sub-hunk
/// count is only known later in `select`, so a range like `1-9999999999` from the command
/// line would otherwise expand into a multi-gigabyte `Vec` before any bound check runs.
/// This cap is far above any real diff's sub-hunk count; exceeding it is treated as a bad
/// selector rather than an allocation.
const MAX_SELECTOR_INDICES: usize = 1 << 20;

fn parse_index_list(s: &str) -> Result<Vec<usize>, ()> {
    if s.is_empty() {
        return Err(());
    }
    let mut v = Vec::new();
    for part in s.split(',') {
        if let Some((lo, hi)) = part.split_once('-') {
            let lo: usize = lo.parse().map_err(|_| ())?;
            let hi: usize = hi.parse().map_err(|_| ())?;
            if lo == 0 || hi < lo {
                return Err(());
            }
            let span = hi - lo + 1;
            if span > MAX_SELECTOR_INDICES || v.len() + span > MAX_SELECTOR_INDICES {
                return Err(());
            }
            v.extend(lo..=hi);
        } else {
            let n: usize = part.parse().map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            v.push(n);
        }
    }
    Ok(v)
}

/// True if every `(file, sub-hunk)` pair has identical id-defining content: the file paths and
/// the sub-hunk's *changed* (added/deleted) lines, ignoring context lines — the same inputs the
/// content id hashes. Sub-hunks that share an id are normally the same change made in one or more
/// places (intentional duplicates, selected together); this distinguishes that case from an
/// accidental hash collision between genuinely different changes, which must be rejected.
pub(crate) fn all_same_content(items: &[(&FileDiff, &Hunk)]) -> bool {
    /// The changed (added/deleted) lines of a sub-hunk, in order; context lines are excluded so
    /// the same change in different surrounding context compares equal.
    fn changed_lines(sub: &Hunk) -> impl Iterator<Item = &Line> {
        sub.lines
            .iter()
            .filter(|l| !matches!(l.kind, LineKind::Context))
    }
    let Some(((first_file, first_sub), rest)) = items.split_first() else {
        return true;
    };
    rest.iter().all(|(f, s)| {
        f.new_path == first_file.new_path
            && f.old_path == first_file.old_path
            && changed_lines(s).eq(changed_lines(first_sub))
    })
}

pub fn select(patch: &Patch, selectors: &[Selector]) -> Result<Patch, SelectError> {
    use std::collections::BTreeMap;
    // Auto-split lazily, only for files a selector actually names, and cache by file index so
    // each referenced file is split once (selectors may target the same file repeatedly).
    let mut subs_cache: BTreeMap<usize, Vec<Hunk>> = BTreeMap::new();
    let mut chosen: BTreeMap<usize, Vec<usize>> = BTreeMap::new();

    for sel in selectors {
        match sel {
            Selector::Id(id) => resolve_id(patch, id, &mut subs_cache, &mut chosen)?,
            Selector::File { path, indices } => {
                let fi = resolve_file(patch, path.as_deref())?;
                // A binary file has no sub-hunks; any selector (`N`, `*`) picks the whole
                // binary change. Recording the file index is enough — emission clones the
                // binary body regardless of indices.
                if matches!(patch.files[fi].content, FileContent::Binary(_)) {
                    chosen.entry(fi).or_default();
                    continue;
                }
                let subs = subs_cache
                    .entry(fi)
                    .or_insert_with(|| build_file_subs(&patch.files[fi]));
                let idxs: Vec<usize> = match indices {
                    IndexSet::All => (1..=subs.len()).collect(),
                    IndexSet::List(v) => v.clone(),
                };
                for idx in idxs {
                    if idx > subs.len() {
                        let pname = path
                            .clone()
                            .unwrap_or_else(|| patch.files[fi].display_path());
                        return Err(SelectError::NoIndex(format!("{pname}:{idx}")));
                    }
                    chosen.entry(fi).or_default().push(idx);
                }
            }
        }
    }
    if chosen.is_empty() {
        return Err(SelectError::EmptySelection);
    }

    let mut files = Vec::new();
    for (fi, mut idxs) in chosen {
        idxs.sort_unstable();
        idxs.dedup();
        let src = &patch.files[fi];
        let content = match &src.content {
            FileContent::Binary(b) => FileContent::Binary(b.clone()),
            FileContent::Text(_) => {
                let subs = &subs_cache[&fi];
                FileContent::Text(idxs.iter().map(|&i| subs[i - 1].clone()).collect())
            }
        };
        files.push(FileDiff {
            headers: src.headers.clone(),
            old_path: src.old_path.clone(),
            new_path: src.new_path.clone(),
            content,
        });
    }
    Ok(Patch { files })
}

/// Resolve an `@<id>` selector: match every sub-hunk in the patch whose content hash equals
/// `id`, confirm the matches are byte-identical (otherwise an accidental hash collision between
/// distinct changes), and record their indices in `chosen`. Binary files have no sub-hunks and
/// are skipped. This necessarily scans (and auto-splits) the whole patch, unlike path selectors.
fn resolve_id(
    patch: &Patch,
    id: &str,
    subs_cache: &mut std::collections::BTreeMap<usize, Vec<Hunk>>,
    chosen: &mut std::collections::BTreeMap<usize, Vec<usize>>,
) -> Result<(), SelectError> {
    // Compare 64-bit hashes rather than rendered hex strings to avoid an allocation per
    // sub-hunk across the full scan. `from_str_radix` accepts upper- or lowercase hex.
    let target = u64::from_str_radix(id, 16).map_err(|_| SelectError::UnknownId(id.to_string()))?;

    let mut matched: Vec<(usize, usize)> = Vec::new();
    for (fi, f) in patch.files.iter().enumerate() {
        if matches!(f.content, FileContent::Binary(_)) {
            continue;
        }
        let subs = subs_cache.entry(fi).or_insert_with(|| build_file_subs(f));
        for (si, sub) in subs.iter().enumerate() {
            if subhunk_hash(f, sub) == target {
                matched.push((fi, si + 1));
            }
        }
    }
    if matched.is_empty() {
        return Err(SelectError::UnknownId(id.to_string()));
    }
    let refs: Vec<(&FileDiff, &Hunk)> = matched
        .iter()
        .map(|&(fi, si)| (&patch.files[fi], &subs_cache[&fi][si - 1]))
        .collect();
    if !all_same_content(&refs) {
        return Err(SelectError::IdCollision(id.to_string()));
    }
    for (fi, si) in matched {
        chosen.entry(fi).or_default().push(si);
    }
    Ok(())
}

/// Resolve an optional path to a file index. With no path, succeeds only for single-file diffs.
pub(crate) fn resolve_file(patch: &Patch, path: Option<&str>) -> Result<usize, SelectError> {
    match path {
        None => {
            if patch.files.len() == 1 {
                Ok(0)
            } else {
                Err(SelectError::AmbiguousPath(
                    "<no path on multi-file diff>".into(),
                ))
            }
        }
        Some(p) => {
            let matches: Vec<usize> = patch
                .files
                .iter()
                .enumerate()
                .filter(|(_, f)| {
                    f.new_path.as_deref() == Some(p.as_bytes())
                        || f.old_path.as_deref() == Some(p.as_bytes())
                })
                .map(|(i, _)| i)
                .collect();
            match matches.as_slice() {
                [one] => Ok(*one),
                [] => Err(SelectError::UnknownPath(p.to_string())),
                _ => Err(SelectError::AmbiguousPath(p.to_string())),
            }
        }
    }
}

/// Resolve `path:N` / `N` to (file_index, original_hunk_index_0based) for the `split` command.
pub fn resolve_hunk(patch: &Patch, addr: &str) -> Result<(usize, usize), SelectError> {
    let (path, nstr) = match addr.rsplit_once(':') {
        Some((p, n)) if !p.is_empty() && n.parse::<usize>().is_ok() => (Some(p.to_string()), n),
        _ => (None, addr),
    };
    let n: usize = nstr
        .parse()
        .map_err(|_| SelectError::BadSelector(addr.to_string()))?;
    if n == 0 {
        return Err(SelectError::BadSelector(addr.to_string()));
    }
    let fi = resolve_file(patch, path.as_deref())?;
    match &patch.files[fi].content {
        FileContent::Text(h) if n <= h.len() => Ok((fi, n - 1)),
        FileContent::Text(_) => Err(SelectError::NoIndex(addr.to_string())),
        FileContent::Binary(_) => Err(SelectError::BadSelector(format!("{addr} (binary file)"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::emit;
    use crate::parser::parse;

    const MULTI: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,5 +1,5 @@
 a
-b
+B
 c
-d
+D
 e
";

    /// A minimal text `FileDiff` with both paths set, for unit tests that build hunks directly.
    fn mk_file(path: &str) -> FileDiff {
        FileDiff {
            headers: Vec::new(),
            old_path: Some(path.as_bytes().to_vec()),
            new_path: Some(path.as_bytes().to_vec()),
            content: FileContent::Text(Vec::new()),
        }
    }

    /// A hunk with the given (kind, text) lines and zeroed line-number metadata.
    fn mk_hunk(lines: &[(LineKind, &str)]) -> Hunk {
        Hunk {
            old_start: 0,
            old_lines: 0,
            new_start: 0,
            new_lines: 0,
            section: Vec::new(),
            lines: lines
                .iter()
                .map(|&(kind, text)| Line {
                    kind,
                    text: text.as_bytes().to_vec(),
                    no_newline: false,
                })
                .collect(),
        }
    }

    #[test]
    fn parse_selector_bare_index_list() {
        let sels = parse_selectors(&["1,2".to_string()]).unwrap();
        assert_eq!(sels.len(), 1);
        assert_eq!(
            sels[0],
            Selector::File {
                path: None,
                indices: IndexSet::List(vec![1, 2]),
            }
        );
    }

    #[test]
    fn huge_range_is_rejected_without_allocating() {
        // The whole range is materialised into a Vec before the real sub-hunk count is
        // checked in `select`. An unbounded `hi` from the command line must be rejected
        // up front rather than allocating gigabytes.
        assert!(parse_index_list("1-100000000").is_err());
        assert!(parse_selectors(&["1-100000000".to_string()]).is_err());
    }

    #[test]
    fn parse_bare_star_is_all() {
        let sels = parse_selectors(&["*".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: None,
                indices: IndexSet::All,
            }
        );
    }

    #[test]
    fn parse_path_star_is_all() {
        let sels = parse_selectors(&["src/f:*".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: Some("src/f".to_string()),
                indices: IndexSet::All,
            }
        );
    }

    #[test]
    fn parse_id_selector() {
        let sels = parse_selectors(&["@a1b2c3d4e5f60718".to_string()]).unwrap();
        assert_eq!(sels[0], Selector::Id("a1b2c3d4e5f60718".to_string()));
    }

    #[test]
    fn parse_at_prefixed_path_is_path_form_not_id() {
        // A file literally named "@foo" addressed as "@foo:1": the ':' + valid index list
        // makes this the path form, not an id.
        let sels = parse_selectors(&["@foo:1".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: Some("@foo".to_string()),
                indices: IndexSet::List(vec![1]),
            }
        );
    }

    #[test]
    fn parse_bare_at_is_error() {
        // "@" with no id is not a valid selector.
        assert!(parse_selectors(&["@".to_string()]).is_err());
    }

    #[test]
    fn parse_selector_path_with_range() {
        let sels = parse_selectors(&["src/f:2-4".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: Some("src/f".to_string()),
                indices: IndexSet::List(vec![2, 3, 4]),
            }
        );
    }

    #[test]
    fn select_first_subhunk_only() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let sels = parse_selectors(&["1".to_string()]).unwrap();
        let out = select(&p, &sels).unwrap();
        let text = String::from_utf8(emit(&out)).unwrap();
        assert!(text.contains("+B"));
        assert!(!text.contains("+D")); // second change excluded
    }

    const TWO_FILES: &str = "\
diff --git a/x b/x
--- a/x
+++ b/x
@@ -1,3 +1,3 @@
 a
-b
+B
 c
diff --git a/y b/y
--- a/y
+++ b/y
@@ -1,3 +1,3 @@
 p
-q
+Q
 r
";

    #[test]
    fn select_across_two_files() {
        let p = parse(TWO_FILES.as_bytes()).unwrap();
        let sels = parse_selectors(&["x:1".to_string(), "y:1".to_string()]).unwrap();
        let out = select(&p, &sels).unwrap();
        assert_eq!(out.files.len(), 2);
        let text = String::from_utf8(emit(&out)).unwrap();
        assert!(text.contains("+B"));
        assert!(text.contains("+Q"));
    }

    const SAME_TWICE: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-x
+y
 b
@@ -10,3 +10,3 @@
 a
-x
+y
 b
";

    #[test]
    fn select_bare_star_selects_every_subhunk() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let sels = parse_selectors(&["*".to_string()]).unwrap();
        let out = select(&p, &sels).unwrap();
        let text = String::from_utf8(emit(&out)).unwrap();
        assert!(text.contains("+B"), "first change present: {text}");
        assert!(text.contains("+D"), "second change present: {text}");
    }

    #[test]
    fn select_path_star_selects_named_file_only() {
        let p = parse(TWO_FILES.as_bytes()).unwrap();
        let sels = parse_selectors(&["x:*".to_string()]).unwrap();
        let out = select(&p, &sels).unwrap();
        assert_eq!(out.files.len(), 1);
        let text = String::from_utf8(emit(&out)).unwrap();
        assert!(text.contains("+B"));
        assert!(!text.contains("+Q"), "file y must be excluded: {text}");
    }

    #[test]
    fn select_by_id_picks_matching_subhunk() {
        use crate::subhunk_id::subhunk_id;
        let p = parse(MULTI.as_bytes()).unwrap();
        let view = build_view(&p);
        let (fi, subs) = &view[0];
        // Second sub-hunk: the d->D change.
        let id = subhunk_id(&p.files[*fi], &subs[1]);
        let sels = parse_selectors(&[format!("@{id}")]).unwrap();
        let out = select(&p, &sels).unwrap();
        let text = String::from_utf8(emit(&out)).unwrap();
        assert!(text.contains("+D"), "addressed change present: {text}");
        assert!(!text.contains("+B"), "other change excluded: {text}");
    }

    #[test]
    fn select_id_is_case_insensitive() {
        use crate::subhunk_id::subhunk_id;
        let p = parse(MULTI.as_bytes()).unwrap();
        let view = build_view(&p);
        let id = subhunk_id(&p.files[0], &view[0].1[0]).to_uppercase();
        let sels = parse_selectors(&[format!("@{id}")]).unwrap();
        assert!(select(&p, &sels).is_ok(), "uppercase id must still match");
    }

    #[test]
    fn select_id_selects_all_identical_subhunks() {
        use crate::subhunk_id::subhunk_id;
        let p = parse(SAME_TWICE.as_bytes()).unwrap();
        let view = build_view(&p);
        let (fi, subs) = &view[0];
        assert_eq!(subs.len(), 2);
        let id0 = subhunk_id(&p.files[*fi], &subs[0]);
        let id1 = subhunk_id(&p.files[*fi], &subs[1]);
        assert_eq!(id0, id1, "identical changes must share an id");

        let sels = parse_selectors(&[format!("@{id0}")]).unwrap();
        let out = select(&p, &sels).unwrap();
        match &out.files[0].content {
            FileContent::Text(hunks) => {
                assert_eq!(hunks.len(), 2, "both identical sub-hunks must be selected")
            }
            _ => panic!("expected text content"),
        }
    }

    #[test]
    fn select_unknown_id_errors() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let sels = parse_selectors(&["@0000000000000000".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::UnknownId(_))));
    }

    #[test]
    fn collision_check_distinguishes_distinct_content() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let view = build_view(&p);
        let (fi, subs) = &view[0];
        let f = &p.files[*fi];
        assert!(
            all_same_content(&[(f, &subs[0]), (f, &subs[0])]),
            "identical sub-hunks are not a collision"
        );
        assert!(
            !all_same_content(&[(f, &subs[0]), (f, &subs[1])]),
            "distinct sub-hunks sharing an id is a collision"
        );
    }

    #[test]
    fn collision_check_ignores_context_differences() {
        // Same changed (+/-) lines but different surrounding context is the same change made
        // in two places — a legitimate duplicate selected together, not a hash collision. The
        // content check must compare only the changed lines, matching the context-free id.
        let f = mk_file("src/a.rs");
        let a = mk_hunk(&[
            (LineKind::Context, "before-a"),
            (LineKind::Del, "x"),
            (LineKind::Add, "y"),
            (LineKind::Context, "after-a"),
        ]);
        let b = mk_hunk(&[
            (LineKind::Context, "totally-different"),
            (LineKind::Del, "x"),
            (LineKind::Add, "y"),
        ]);
        assert!(
            all_same_content(&[(&f, &a), (&f, &b)]),
            "identical changes in different context must not count as a collision"
        );
    }

    #[test]
    fn collision_check_flags_different_changed_lines() {
        // Same context but different changed lines is a genuine collision and must be rejected.
        let f = mk_file("src/a.rs");
        let a = mk_hunk(&[(LineKind::Context, "ctx"), (LineKind::Add, "y")]);
        let b = mk_hunk(&[(LineKind::Context, "ctx"), (LineKind::Add, "z")]);
        assert!(
            !all_same_content(&[(&f, &a), (&f, &b)]),
            "distinct changed lines must count as a collision"
        );
    }

    #[test]
    fn select_unknown_index_errors() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let sels = parse_selectors(&["9".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::NoIndex(_))));
    }

    #[test]
    fn select_empty_is_error() {
        let p = parse(MULTI.as_bytes()).unwrap();
        assert_eq!(select(&p, &[]), Err(SelectError::EmptySelection));
    }

    #[test]
    fn resolve_hunk_addresses_original_hunk() {
        let p = parse(MULTI.as_bytes()).unwrap();
        assert_eq!(resolve_hunk(&p, "1").unwrap(), (0, 0));
    }

    #[test]
    fn select_second_subhunk_applies_via_git() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let p = parse(MULTI.as_bytes()).unwrap();
        // Select only the SECOND sub-hunk (the d->D change).
        let sels = parse_selectors(&["2".to_string()]).unwrap();
        let out = select(&p, &sels).unwrap();
        let diff = emit(&out);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), "a\nb\nc\nd\ne\n").unwrap();
        Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&dir)
            .status()
            .unwrap();
        let mut child = Command::new("git")
            .arg("apply")
            .arg("--check")
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&diff).unwrap();
        assert!(
            child.wait().unwrap().success(),
            "second-only sub-hunk failed to apply:\n{}",
            String::from_utf8_lossy(&diff)
        );
    }
}
