use crate::model::*;
use crate::split::auto_split_hunk;
use crate::split::slice_changed_lines;
use crate::subhunk_id::subhunk_hash;
use std::collections::BTreeSet;
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
    /// `INDEX@L<set>`: one sub-hunk cut to an arbitrary subset of its changed (`+`/`-`) lines,
    /// numbered `1..=changed` in body order. Any subset is realisable (a deletion split by
    /// additions, a replacement's removals separated from its insertions).
    LineSet {
        index: usize,
        lines: Vec<usize>,
    },
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
    /// A selector used the removed `INDEX@lo-hi` added-line range form. Carries the offending
    /// selector so the message can point the caller at the `@L` replacement.
    RemovedRangeForm(String),
    /// An `INDEX@L<set>` line-set selector could not be applied (an out-of-range changed line,
    /// or the sub-hunk combined with another selection).
    LineSelect(String),
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
            SelectError::RemovedRangeForm(s) => write!(
                f,
                "{s}: the @lo-hi added-line range form was removed; use @L<lines> instead \
                 (changed-line indices, e.g. @L1-3; see 'hunkpick list --json' changed_lines)"
            ),
            SelectError::LineSelect(m) => write!(f, "line selector: {m}"),
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
/// 2. `@id` — address sub-hunks by content id (a non-empty hex string; the leading `@` is
///    only the id form when the rest is all hex digits).
/// 3. bare `set` — `*` or an index list, for single-file diffs (the path is resolved later).
pub fn parse_selectors(args: &[String]) -> Result<Vec<Selector>, SelectError> {
    let mut out = Vec::new();
    for a in args {
        // 1. path:set form. Checked first so a file named "@foo" (addressed "@foo:1") and a
        //    path containing ':' are not misread as an id or a bare set.
        if let Some((p, l)) = a.rsplit_once(':') {
            if !p.is_empty() {
                match parse_index_set(l) {
                    Ok(indices) => {
                        out.push(Selector::File {
                            path: Some(p.to_string()),
                            indices,
                        });
                        continue;
                    }
                    // The removed `@lo-hi` form is unambiguous — surface it here rather than
                    // letting the arg fall through to be reported as a generic bad selector.
                    Err(SetParseError::RemovedRange) => {
                        return Err(SelectError::RemovedRangeForm(a.clone()));
                    }
                    // Any other parse failure only means "not the path:set form"; fall through.
                    Err(_) => {}
                }
            }
        }
        // 2. @id form. The id must be a non-empty hex string; any other character (including
        //    a second '@') is not a valid id character and is rejected as a bad selector.
        if let Some(id) = a.strip_prefix('@') {
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(SelectError::BadSelector(a.clone()));
            }
            out.push(Selector::Id(id.to_string()));
            continue;
        }
        // 3. bare set. A parse failure here is terminal (unlike the path form above), so the
        //    specific reason is surfaced to the user rather than discarded.
        let indices = parse_index_set(a).map_err(|e| match e {
            SetParseError::RemovedRange => SelectError::RemovedRangeForm(a.clone()),
            e => SelectError::BadSelector(format!("{a} ({e})")),
        })?;
        out.push(Selector::File {
            path: None,
            indices,
        });
    }
    Ok(out)
}

/// Why an index-set selector failed to parse. Carried up so the CLI can report the specific
/// fault — a reversed range, a zero bound, a non-numeric bound — instead of a bare
/// "bad selector". In the `path:set` form a parse failure only signals "not this form" and the
/// reason is discarded; it is surfaced only for a bare set (see `parse_selectors`).
#[derive(Debug, PartialEq, Eq)]
enum SetParseError {
    Empty,
    NotANumber(String),
    ZeroBound,
    ReversedRange {
        lo: usize,
        hi: usize,
    },
    TooLarge,
    /// The `INDEX@lo-hi` added-line range form (anything after `@` that is not an `L<set>`).
    /// The form was removed; the CLI turns this into a `RemovedRangeForm` selector error that
    /// steers the caller to `@L`.
    RemovedRange,
}

impl fmt::Display for SetParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetParseError::Empty => write!(f, "empty index set"),
            SetParseError::NotANumber(s) => write!(f, "not a number: {s}"),
            SetParseError::ZeroBound => write!(f, "indices are 1-based, 0 is not valid"),
            SetParseError::ReversedRange { lo, hi } => write!(f, "reversed range: {lo}-{hi}"),
            SetParseError::TooLarge => write!(f, "range too large"),
            SetParseError::RemovedRange => write!(f, "the @lo-hi range form was removed; use @L"),
        }
    }
}

/// Parse the index part of a `File` selector: `*` (all), `INDEX@L<set>` (one sub-hunk cut to a
/// subset of its changed lines), or a comma-separated index list.
fn parse_index_set(s: &str) -> Result<IndexSet, SetParseError> {
    if s == "*" {
        return Ok(IndexSet::All);
    }
    // `INDEX@L<set>`: a numeric index, then `@L`, then the changed-line set. Checked before the
    // index-list form so the '@' is not mistaken for a malformed list entry. Only a numeric
    // index may precede '@' — `@id` (content id) is a different form handled in parse_selectors
    // (it starts with '@', so it never reaches here as `INDEX@...`). Anything after '@' that is
    // not an `L<set>` is the removed `@lo-hi` added-line range form, reported distinctly so the
    // CLI can steer the caller to `@L`.
    if let Some((idx, rest)) = s.split_once('@') {
        let index = parse_pos(idx)?;
        let Some(set) = rest.strip_prefix('L') else {
            return Err(SetParseError::RemovedRange);
        };
        let lines = parse_index_list(set)?;
        return Ok(IndexSet::LineSet { index, lines });
    }
    parse_index_list(s).map(IndexSet::List)
}

/// Parse a 1-based, non-zero position.
fn parse_pos(s: &str) -> Result<usize, SetParseError> {
    let n: usize = s
        .parse()
        .map_err(|_| SetParseError::NotANumber(s.to_string()))?;
    if n == 0 {
        Err(SetParseError::ZeroBound)
    } else {
        Ok(n)
    }
}

/// Upper bound on the number of indices a selector may materialise. The real sub-hunk
/// count is only known later in `select`, so a range like `1-9999999999` from the command
/// line would otherwise expand into a multi-gigabyte `Vec` before any bound check runs.
/// This cap is far above any real diff's sub-hunk count; exceeding it is treated as a bad
/// selector rather than an allocation.
const MAX_SELECTOR_INDICES: usize = 1 << 20;

fn parse_index_list(s: &str) -> Result<Vec<usize>, SetParseError> {
    if s.is_empty() {
        return Err(SetParseError::Empty);
    }
    let mut v = Vec::new();
    for part in s.split(',') {
        if let Some((lo_s, hi_s)) = part.split_once('-') {
            let lo = parse_pos(lo_s)?;
            let hi = parse_pos(hi_s)?;
            if hi < lo {
                return Err(SetParseError::ReversedRange { lo, hi });
            }
            let span = hi - lo + 1;
            if span > MAX_SELECTOR_INDICES || v.len() + span > MAX_SELECTOR_INDICES {
                return Err(SetParseError::TooLarge);
            }
            v.extend(lo..=hi);
        } else {
            // Cap single indices too, so a list of many bare indices (`1,1,1,...`) has the same
            // allocation ceiling as a range.
            if v.len() + 1 > MAX_SELECTOR_INDICES {
                return Err(SetParseError::TooLarge);
            }
            v.push(parse_pos(part)?);
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

/// One resolved selection within a file: a whole sub-hunk, or a sub-hunk cut to an arbitrary
/// set of its changed lines.
#[derive(Clone)]
enum Chosen {
    Whole(usize),
    /// A subset of one sub-hunk's changed (`+`/`-`) lines, 1-based over `1..=changed` in body
    /// order. Sorted and deduplicated. Addressed by `INDEX@L<set>`.
    Lines {
        index: usize,
        lines: Vec<usize>,
    },
}

impl Chosen {
    fn index(&self) -> usize {
        match self {
            Chosen::Whole(i) => *i,
            Chosen::Lines { index, .. } => *index,
        }
    }
}

/// The name to show for a file in an error message: the path as the user wrote it, or the
/// diff's own display path when the selector carried no explicit path (a single-file diff).
fn display_name(patch: &Patch, fi: usize, path: &Option<String>) -> String {
    path.clone()
        .unwrap_or_else(|| patch.files[fi].display_path())
}

pub fn select(patch: &Patch, selectors: &[Selector]) -> Result<Patch, SelectError> {
    // Auto-split lazily, only for files a selector actually names, and cache by file index so
    // each referenced file is split once (selectors may target the same file repeatedly). The
    // cache is shared between the resolution and emission phases below.
    let mut subs_cache: std::collections::BTreeMap<usize, Vec<Hunk>> =
        std::collections::BTreeMap::new();
    let chosen = resolve_selectors(patch, selectors, &mut subs_cache)?;
    if chosen.is_empty() {
        return Err(SelectError::EmptySelection);
    }
    emit_selection(patch, chosen, &subs_cache)
}

/// Resolution phase: turn each selector into a per-file map of chosen sub-hunks, auto-splitting
/// (and caching, via `subs_cache`) each referenced file on demand. The cache is returned to the
/// caller because the emission phase reuses the same splits.
fn resolve_selectors(
    patch: &Patch,
    selectors: &[Selector],
    subs_cache: &mut std::collections::BTreeMap<usize, Vec<Hunk>>,
) -> Result<std::collections::BTreeMap<usize, Vec<Chosen>>, SelectError> {
    let mut chosen: std::collections::BTreeMap<usize, Vec<Chosen>> =
        std::collections::BTreeMap::new();
    for sel in selectors {
        match sel {
            Selector::Id(id) => resolve_id(patch, id, subs_cache, &mut chosen)?,
            Selector::File { path, indices } => {
                let fi = resolve_file(patch, path.as_deref())?;
                // A binary file has no sub-hunks; a non-line-set selector picks the whole binary
                // change. A line-set selector makes no sense for a binary file.
                if matches!(patch.files[fi].content, FileContent::Binary(_)) {
                    if let IndexSet::LineSet { .. } = indices {
                        return Err(SelectError::LineSelect(format!(
                            "{} is a binary file",
                            patch.files[fi].display_path()
                        )));
                    }
                    chosen.entry(fi).or_default();
                    continue;
                }
                let subs = subs_cache
                    .entry(fi)
                    .or_insert_with(|| build_file_subs(&patch.files[fi]));
                match indices {
                    IndexSet::All => {
                        for i in 1..=subs.len() {
                            chosen.entry(fi).or_default().push(Chosen::Whole(i));
                        }
                    }
                    IndexSet::List(v) => {
                        for &idx in v {
                            if idx > subs.len() {
                                return Err(SelectError::NoIndex(format!(
                                    "{}:{idx}",
                                    display_name(patch, fi, path)
                                )));
                            }
                            chosen.entry(fi).or_default().push(Chosen::Whole(idx));
                        }
                    }
                    IndexSet::LineSet { index, lines } => {
                        if *index > subs.len() {
                            return Err(SelectError::NoIndex(format!(
                                "{}:{index}",
                                display_name(patch, fi, path)
                            )));
                        }
                        // The concrete range check (against the sub-hunk's changed-line count)
                        // and normalisation (sort + dedup, via a `BTreeSet`) happen at emission in
                        // `slice_changed_lines`; no need to canonicalise the raw indices here.
                        chosen.entry(fi).or_default().push(Chosen::Lines {
                            index: *index,
                            lines: lines.clone(),
                        });
                    }
                }
            }
        }
    }
    Ok(chosen)
}

/// Emission phase: materialise the resolved selections into a result patch. Per file, order the
/// picks, drop duplicates, and cut/clone each sub-hunk. `subs_cache` must already hold the
/// splits for every referenced file (populated by `resolve_selectors`).
fn emit_selection(
    patch: &Patch,
    chosen: std::collections::BTreeMap<usize, Vec<Chosen>>,
    subs_cache: &std::collections::BTreeMap<usize, Vec<Hunk>>,
) -> Result<Patch, SelectError> {
    let mut files = Vec::new();
    for (fi, mut picks) in chosen {
        let src = &patch.files[fi];
        let content = match &src.content {
            // A binary file has no sub-hunks; its picks vec is always empty.
            FileContent::Binary(b) => FileContent::Binary(b.clone()),
            FileContent::Text(_) => {
                let subs = &subs_cache[&fi];
                // A sub-hunk addressed by `@L` (a changed-line subset) must be its ONLY selection.
                // Partial `@L` pieces of one sub-hunk emitted together would carry mutually
                // inconsistent new-side line numbers, and combining `@L` with a whole pick of the
                // same sub-hunk double-counts its lines. Reject as a usage error before emission;
                // the diff -> stage -> re-diff loop is the way to combine such pieces.
                for p in &picks {
                    if let Chosen::Lines { index, .. } = p {
                        if picks.iter().filter(|q| q.index() == *index).count() > 1 {
                            return Err(SelectError::LineSelect(format!(
                                "sub-hunk {index} is addressed by @L together with another \
                                 selection of the same sub-hunk; address it once, or stage the \
                                 pieces in separate rounds"
                            )));
                        }
                    }
                }
                // Order by sub-hunk index so emitted hunks follow old-file order and equal-index
                // whole picks are adjacent for the dedup below. Distinct sub-hunks are disjoint,
                // so no overlap check is needed: the only same-index multiplicity is a duplicate
                // whole (dropped here) or a whole+`@L` collision (rejected above).
                picks.sort_by_key(|c| c.index());
                // Drop exact duplicate whole selections (a sub-hunk named twice).
                picks.dedup_by(
                    |a, b| matches!((a, b), (Chosen::Whole(x), Chosen::Whole(y)) if x == y),
                );
                let mut hunks = Vec::with_capacity(picks.len());
                for pick in &picks {
                    match pick {
                        Chosen::Whole(i) => hunks.push(subs[i - 1].clone()),
                        Chosen::Lines { index, lines } => {
                            let set: BTreeSet<usize> = lines.iter().copied().collect();
                            let cut = slice_changed_lines(&subs[index - 1], &set)
                                .map_err(|e| SelectError::LineSelect(e.to_string()))?;
                            hunks.push(cut);
                        }
                    }
                }
                FileContent::Text(hunks)
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
    chosen: &mut std::collections::BTreeMap<usize, Vec<Chosen>>,
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
        chosen.entry(fi).or_default().push(Chosen::Whole(si));
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
    fn parse_lineset_selector_basic() {
        let sels = parse_selectors(&["1@L1,3".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: None,
                indices: IndexSet::LineSet {
                    index: 1,
                    lines: vec![1, 3],
                },
            }
        );
    }

    #[test]
    fn parse_lineset_with_path_and_range() {
        let sels = parse_selectors(&["src/f:2@L1-2,4".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: Some("src/f".to_string()),
                indices: IndexSet::LineSet {
                    index: 2,
                    lines: vec![1, 2, 4],
                },
            }
        );
    }

    #[test]
    fn removed_range_form_reports_friendly_error() {
        // The old `@lo-hi` added-line range form was removed. Any `INDEX@<not L>` selector,
        // bare or path-qualified, must fail with a message that names the `@L` replacement so a
        // caller can self-correct — not a bare "bad selector".
        for sel in ["1@1-3", "1@91-", "1@-90", "2@5", "src/f:1@1-90"] {
            match parse_selectors(&[sel.to_string()]) {
                Err(SelectError::RemovedRangeForm(s)) => {
                    assert_eq!(s, sel);
                    assert!(
                        format!("{}", SelectError::RemovedRangeForm(s)).contains("@L"),
                        "message for {sel} must steer to @L"
                    );
                }
                other => panic!("selector {sel}: expected RemovedRangeForm, got {other:?}"),
            }
        }
    }

    #[test]
    fn bad_selector_reports_specific_reason() {
        // A bare selector that fails to parse must carry *why* it failed, not a bare
        // "bad selector": reversed range, zero bound, non-numeric bound.
        let cases = [
            ("2-1", "reversed range"),
            ("0", "1-based"),
            ("a", "not a number"),
        ];
        for (sel, needle) in cases {
            match parse_selectors(&[sel.to_string()]) {
                Err(SelectError::BadSelector(msg)) => assert!(
                    msg.contains(needle),
                    "selector {sel}: message {msg:?} lacks {needle:?}"
                ),
                other => panic!("selector {sel}: expected BadSelector, got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_lineset_rejects_malformed() {
        // empty set, zero index, zero line, id-form before '@'
        assert!(parse_selectors(&["1@L".to_string()]).is_err());
        assert!(parse_selectors(&["0@L1".to_string()]).is_err());
        assert!(parse_selectors(&["1@L0".to_string()]).is_err());
        assert!(parse_selectors(&["1@L2-1".to_string()]).is_err());
        // '@id@Lset' is NOT supported: only a numeric index may precede '@'.
        assert!(parse_selectors(&["@deadbeef@L1-2".to_string()]).is_err());
    }

    const PURE_ADD_FILE: &str = "\
diff --git a/f b/f
new file mode 100644
--- /dev/null
+++ b/f
@@ -0,0 +1,4 @@
+l1
+l2
+l3
+l4
";

    #[test]
    fn select_lineset_first_two_changed_lines() {
        let p = parse(PURE_ADD_FILE.as_bytes()).unwrap();
        let sels = parse_selectors(&["1@L1,2".to_string()]).unwrap();
        let out = select(&p, &sels).unwrap();
        let text = String::from_utf8(emit(&out)).unwrap();
        assert!(text.contains("+l1"));
        assert!(text.contains("+l2"));
        assert!(!text.contains("+l3"));
        assert!(!text.contains("+l4"));
    }

    #[test]
    fn select_lineset_out_of_range_errors() {
        let p = parse(PURE_ADD_FILE.as_bytes()).unwrap();
        let sels = parse_selectors(&["1@L1-99".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::LineSelect(_))));
    }

    #[test]
    fn select_lineset_unknown_index_errors() {
        let p = parse(PURE_ADD_FILE.as_bytes()).unwrap();
        let sels = parse_selectors(&["9@L1,2".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::NoIndex(_))));
    }

    #[test]
    fn select_whole_and_lineset_of_same_subhunk_rejected() {
        // A whole sub-hunk and an `@L` subset of the same sub-hunk double-count its lines.
        // Selecting both is contradictory and rejected as a selector error.
        let p = parse(PURE_ADD_FILE.as_bytes()).unwrap();
        let sels = parse_selectors(&["1".to_string(), "1@L1,2".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::LineSelect(_))));
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

    /// A single-file replacement diff: file `a,b` -> `A,B` as one contiguous run.
    /// Changed lines: 1=`-a`, 2=`-b`, 3=`+A`, 4=`+B`.
    const REPL_FILE: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
-a
-b
+A
+B
";

    /// True if `diff_bytes` applies to a file `f` seeded with `content` in a fresh git repo.
    fn apply_ok(diff_bytes: &[u8], content: &str) -> bool {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), content).unwrap();
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
        child.stdin.take().unwrap().write_all(diff_bytes).unwrap();
        child.wait().unwrap().success()
    }

    #[test]
    fn parse_line_set_selector() {
        let sels = parse_selectors(&["1@L1,3".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: None,
                indices: IndexSet::LineSet {
                    index: 1,
                    lines: vec![1, 3],
                },
            }
        );
        // Ranges inside the set expand like an index list.
        let sels = parse_selectors(&["2@L1-2,4".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: None,
                indices: IndexSet::LineSet {
                    index: 2,
                    lines: vec![1, 2, 4],
                },
            }
        );
    }

    #[test]
    fn parse_line_set_with_path() {
        let sels = parse_selectors(&["src/f:2@L1".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: Some("src/f".to_string()),
                indices: IndexSet::LineSet {
                    index: 2,
                    lines: vec![1],
                },
            }
        );
    }

    #[test]
    fn parse_line_set_rejects_malformed() {
        // Empty set, zero index bound, reversed range inside the set.
        assert!(parse_selectors(&["1@L".to_string()]).is_err());
        assert!(parse_selectors(&["1@L0".to_string()]).is_err());
        assert!(parse_selectors(&["1@L3-1".to_string()]).is_err());
    }

    #[test]
    fn select_line_set_separates_deletions_from_additions() {
        // The key agent operation: two invocations, each applying to the original file, stage the
        // removals and the insertions of a replacement independently.
        let p = parse(REPL_FILE.as_bytes()).unwrap();

        let dels = select(&p, &parse_selectors(&["1@L1,2".to_string()]).unwrap()).unwrap();
        let dels_text = String::from_utf8(emit(&dels)).unwrap();
        assert!(dels_text.contains("-a") && dels_text.contains("-b"));
        assert!(!dels_text.contains("+A") && !dels_text.contains("+B"));
        assert!(
            apply_ok(&emit(&dels), "a\nb\n"),
            "deletion piece must apply"
        );

        let adds = select(&p, &parse_selectors(&["1@L3,4".to_string()]).unwrap()).unwrap();
        let adds_text = String::from_utf8(emit(&adds)).unwrap();
        assert!(adds_text.contains("+A") && adds_text.contains("+B"));
        assert!(
            !adds_text.contains("-a"),
            "deletions must be context, not `-`"
        );
        assert!(
            apply_ok(&emit(&adds), "a\nb\n"),
            "addition piece must apply"
        );
    }

    #[test]
    fn select_line_set_out_of_range_errors() {
        let p = parse(REPL_FILE.as_bytes()).unwrap(); // 4 changed lines
        let sels = parse_selectors(&["1@L5".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::LineSelect(_))));
    }

    #[test]
    fn select_line_set_unknown_index_errors() {
        let p = parse(REPL_FILE.as_bytes()).unwrap();
        let sels = parse_selectors(&["9@L1".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::NoIndex(_))));
    }

    #[test]
    fn select_line_set_combined_with_whole_same_subhunk_rejected() {
        // `@L` of a sub-hunk plus the whole sub-hunk double-counts its lines: usage error.
        let p = parse(REPL_FILE.as_bytes()).unwrap();
        let sels = parse_selectors(&["1".to_string(), "1@L1".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::LineSelect(_))));
    }

    #[test]
    fn select_two_line_sets_of_same_subhunk_rejected() {
        // Two `@L` selections of the same sub-hunk in one invocation would emit mutually
        // inconsistent pieces: usage error, use the re-diff loop instead.
        let p = parse(REPL_FILE.as_bytes()).unwrap();
        let sels = parse_selectors(&["1@L1,2".to_string(), "1@L3,4".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::LineSelect(_))));
    }
}
