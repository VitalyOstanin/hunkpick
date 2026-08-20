use crate::model::*;
use crate::renumber::renumber_new_side;
use crate::split::auto_split_hunk;
use crate::split::slice_changed_lines;
use crate::subhunk_id::subhunk_hash;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;

/// One parsed selector argument.
#[derive(Debug, PartialEq, Eq)]
pub enum Selector {
    /// Address sub-hunks within one file (the `path:set` and bare-`set` forms).
    File {
        /// Path as the user wrote it, or `None` for the bare form (resolved against a
        /// single-file diff later). Raw bytes, because a diff can name a file whose path is
        /// not valid UTF-8 and the selector has to be able to spell it.
        path: Option<Vec<u8>>,
        /// Which sub-hunks of that file are addressed.
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
    /// `*`: every sub-hunk of the addressed file (and the entry itself when it has none).
    All,
    /// An explicit list of 1-based sub-hunk indices, ranges already expanded.
    List(Vec<usize>),
    /// `INDEX@L<set>`: one sub-hunk cut to an arbitrary subset of its changed (`+`/`-`) lines,
    /// numbered `1..=changed` in body order. Any subset is realisable (a deletion split by
    /// additions, a replacement's removals separated from its insertions).
    LineSet {
        /// 1-based index of the sub-hunk being cut.
        index: usize,
        /// 1-based indices over that sub-hunk's changed lines, ranges already expanded.
        lines: Vec<usize>,
    },
}

/// Why a selection could not be made. Every variant is a usage error (exit code 2).
#[derive(Debug, PartialEq, Eq)]
pub enum SelectError {
    /// A selector argument does not parse. Carries the argument and the specific reason.
    BadSelector(String),
    /// A `path:` selector names a file the diff does not contain.
    UnknownPath(String),
    /// A `path:` selector matches more than one file of the diff.
    AmbiguousPath(String),
    /// An index addresses a sub-hunk the file does not have. Carries `path:index`.
    NoIndex(String),
    /// An `@<id>` selector matched no sub-hunk in the patch.
    UnknownId(String),
    /// An `@<id>` selector matched sub-hunks with differing content (an accidental hash
    /// collision between distinct changes). Carries the colliding id.
    IdCollision(String),
    /// No selector was given, so nothing would be emitted.
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

/// Lets callers treat it as a boxed [`std::error::Error`], as the Rust API guidelines ask
/// of a public error type.
impl std::error::Error for SelectError {}

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

/// Per-file auto-split view: entry `i` holds the ordered sub-hunks of `patch.files[i]`
/// (empty for a binary file), so a position in the result is the file index.
pub fn build_view(patch: &Patch) -> Vec<Vec<Hunk>> {
    patch.files.iter().map(build_file_subs).collect()
}

/// Parse selector args. Forms, in order of precedence:
///
/// 1. `path:set` — `set` is `*` or a comma-separated list of indices/ranges (`1,3`, `2-4`,
///    `src/f:1,3-5`, `src/f:*`). Recognised when a ':' is present and the text after the LAST
///    ':' parses as a valid set; the path may itself contain ':' or a leading '@'.
/// 2. `@id` — address sub-hunks by content id (a non-empty hex string; the leading `@` is
///    only the id form when the rest is all hex digits).
/// 3. bare `set` — `*` or an index list, for single-file diffs (the path is resolved later).
pub fn parse_selectors<S: AsRef<OsStr>>(args: &[S]) -> Result<Vec<Selector>, SelectError> {
    let mut out = Vec::new();
    // One allowance for the whole invocation, spent by every selector that materialises indices.
    let mut budget = IndexBudget::new();
    for arg in args {
        let bytes = os_bytes(arg.as_ref());
        // A path that is not valid UTF-8 can only be the `path:set` form: everything else in
        // the grammar is ASCII. Handle it on bytes and skip the textual forms below.
        let Ok(a) = std::str::from_utf8(bytes) else {
            out.push(parse_binary_path_form(bytes, &mut budget)?);
            continue;
        };
        // 1. path:set form. Checked first so a file named "@foo" (addressed "@foo:1") and a
        //    path containing ':' are not misread as an id or a bare set.
        if let Some(sel) = parse_path_form(a, &mut budget)? {
            out.push(sel);
            continue;
        }
        // 2. @id form. The id must be a non-empty hex string; any other character (including
        //    a second '@') is not a valid id character and is rejected as a bad selector.
        if let Some(id) = a.strip_prefix('@') {
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(SelectError::BadSelector(a.to_string()));
            }
            out.push(Selector::Id(id.to_string()));
            continue;
        }
        // 3. bare set. A parse failure here is terminal (unlike the path form above), so the
        //    specific reason is surfaced to the user rather than discarded.
        let indices = parse_index_set(a, &mut budget).map_err(|e| match e {
            SetParseError::RemovedRange => SelectError::RemovedRangeForm(a.to_string()),
            e => SelectError::BadSelector(format!("{a} ({e})")),
        })?;
        out.push(Selector::File {
            path: None,
            indices,
        });
    }
    Ok(out)
}

/// Read `arg` as the `path:set` form. `Ok(None)` means it is not that form and the caller is to
/// try the remaining ones; an `Err` means it is that form and the set inside it is broken.
fn parse_path_form(arg: &str, budget: &mut IndexBudget) -> Result<Option<Selector>, SelectError> {
    let Some((path, set)) = arg.rsplit_once(':') else {
        return Ok(None);
    };
    if path.is_empty() {
        return Ok(None);
    }
    match parse_index_set(set, budget) {
        Ok(indices) => Ok(Some(Selector::File {
            path: Some(path.as_bytes().to_vec()),
            indices,
        })),
        // The removed `@lo-hi` form is unambiguous — surface it here rather than letting the
        // arg fall through to be reported as a generic bad selector.
        Err(SetParseError::RemovedRange) => Err(SelectError::RemovedRangeForm(arg.to_string())),
        // A failure here is ambiguous: the text after the last ':' may be part of the path
        // rather than a set. Only when it is built exclusively from set characters is it
        // certainly meant as a set, and then its reason is the real one — otherwise report no
        // match and let the arg be re-read as a whole.
        Err(e) if looks_like_index_set(set) => {
            Err(SelectError::BadSelector(format!("{arg} ({e})")))
        }
        Err(_) => Ok(None),
    }
}

/// The bytes of a command-line argument, as the platform stores them: the raw bytes on Unix,
/// where an argument (and a path in a diff) is an arbitrary byte string, and WTF-8 elsewhere.
/// Going through `to_string_lossy` instead would replace an unpaired surrogate with U+FFFD and
/// silently address a different file than the one asked for.
fn os_bytes(arg: &OsStr) -> &[u8] {
    arg.as_encoded_bytes()
}

/// Read a selector whose bytes are not valid UTF-8. Only `path:set` can look like that — the
/// rest of the grammar is ASCII — so the path is taken verbatim and only the set after the
/// last ':' is parsed as text.
fn parse_binary_path_form(bytes: &[u8], budget: &mut IndexBudget) -> Result<Selector, SelectError> {
    let shown = String::from_utf8_lossy(bytes).into_owned();
    let Some(colon) = bytes.iter().rposition(|&b| b == b':') else {
        return Err(SelectError::BadSelector(format!(
            "{shown} (not valid UTF-8; only a path:set selector may hold such bytes)"
        )));
    };
    let (path, set) = (&bytes[..colon], &bytes[colon + 1..]);
    let Ok(set) = std::str::from_utf8(set) else {
        return Err(SelectError::BadSelector(format!(
            "{shown} (the set after ':' must be ASCII)"
        )));
    };
    let indices = parse_index_set(set, budget).map_err(|e| match e {
        SetParseError::RemovedRange => SelectError::RemovedRangeForm(shown.clone()),
        e => SelectError::BadSelector(format!("{shown} ({e})")),
    })?;
    Ok(Selector::File {
        path: Some(path.to_vec()),
        indices,
    })
}

/// Whether `s` is spelled entirely from the characters an index set is made of, so a parse
/// failure is a fault of the set rather than a sign that the text belongs to the path.
fn looks_like_index_set(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ',' | '-' | '*' | '@' | 'L' | 'l'))
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
            SetParseError::TooLarge => write!(
                f,
                "range too large: at most {MAX_SELECTOR_INDICES} indices across all selectors"
            ),
            SetParseError::RemovedRange => write!(f, "the @lo-hi range form was removed; use @L"),
        }
    }
}

/// Parse the index part of a `File` selector: `*` (all), `INDEX@L<set>` (one sub-hunk cut to a
/// subset of its changed lines), or a comma-separated index list.
fn parse_index_set(s: &str, budget: &mut IndexBudget) -> Result<IndexSet, SetParseError> {
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
        let lines = parse_index_list(set, budget)?;
        return Ok(IndexSet::LineSet { index, lines });
    }
    parse_index_list(s, budget).map(IndexSet::List)
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

/// Upper bound on the number of indices one invocation may materialise. The real sub-hunk
/// count is only known later in `select`, so a range like `1-9999999999` from the command
/// line would otherwise expand into a multi-gigabyte `Vec` before any bound check runs.
/// This cap is far above any real diff's sub-hunk count; exceeding it is treated as a bad
/// selector rather than an allocation.
const MAX_SELECTOR_INDICES: usize = 1 << 20;

/// What is left of [`MAX_SELECTOR_INDICES`] for the rest of the invocation.
///
/// The cap used to be per selector, while the number of selectors is bounded only by the length
/// of the command line — 200 copies of `1-1048576` (2.6 KB of argv) reached 1.6 GB of RSS before
/// a single index was compared against the diff. One budget for the whole call is what the
/// constant was introduced to provide.
struct IndexBudget {
    left: usize,
}

impl IndexBudget {
    fn new() -> Self {
        Self {
            left: MAX_SELECTOR_INDICES,
        }
    }

    /// Claim `n` indices, or refuse if the invocation has spent its allowance.
    fn take(&mut self, n: usize) -> Result<(), SetParseError> {
        self.left = self.left.checked_sub(n).ok_or(SetParseError::TooLarge)?;
        Ok(())
    }
}

fn parse_index_list(s: &str, budget: &mut IndexBudget) -> Result<Vec<usize>, SetParseError> {
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
            // Claimed before the range is materialised: `1-9999999999` must be refused, not
            // allocated and then refused.
            budget.take(hi - lo + 1)?;
            v.extend(lo..=hi);
        } else {
            // Bare indices are charged too, so a list of many of them (`1,1,1,...`) has the
            // same ceiling as a range.
            budget.take(1)?;
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
    // Compare the changed (added/deleted) lines of two sub-hunks, in order; context lines are
    // excluded so the same change in different surrounding context compares equal. Uses the shared
    // `Hunk::changed_lines` numbering, dropping the index for a content-only comparison.
    fn same_changed(a: &Hunk, b: &Hunk) -> bool {
        a.changed_lines()
            .map(|(_, l)| l)
            .eq(b.changed_lines().map(|(_, l)| l))
    }
    let Some(((first_file, first_sub), rest)) = items.split_first() else {
        return true;
    };
    rest.iter().all(|(f, s)| {
        f.new_path == first_file.new_path
            && f.old_path == first_file.old_path
            && same_changed(s, first_sub)
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

/// What the selectors picked out of one file.
///
/// Whole sub-hunks live in a set, not a list: naming the same one twice — a repeated `@id`, a
/// repeated `*`, an index listed twice — asks for the same sub-hunk, and appending a pick per
/// naming made the accumulator grow as (selectors x matches) instead of as the file. The
/// duplicates were dropped, but only at emission, after the whole array had been built. Reading
/// every id out of `list --json` and passing them back is that shape whenever sub-hunks share an
/// id, and it turned a 150 KB diff into 10 s and 3 GB of RSS.
#[derive(Default)]
struct FilePicks {
    /// 1-based sub-hunk indices taken whole, in sub-hunk order.
    whole: BTreeSet<usize>,
    /// `@L` slices, in the order they were named. Held apart from `whole`: a slice is not
    /// idempotent — two line sets of one sub-hunk are a conflict, not a repeat — so each one
    /// has to reach [`reject_conflicting_line_set_picks`].
    lines: Vec<(usize, Vec<usize>)>,
}

impl FilePicks {
    /// How many sub-hunks this file contributes to the result. Used by the guard tests that
    /// assert the accumulator stays the size of the file rather than of the selector list.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.whole.len() + self.lines.len()
    }

    /// The picks as one list in sub-hunk order, which is old-file order. Equal indices cannot
    /// occur across the two halves — [`reject_conflicting_line_set_picks`] runs first and
    /// refuses them — so the order is total.
    fn into_ordered(self) -> Vec<Chosen> {
        let mut picks: Vec<Chosen> = self
            .whole
            .into_iter()
            .map(Chosen::Whole)
            .chain(
                self.lines
                    .into_iter()
                    .map(|(index, lines)| Chosen::Lines { index, lines }),
            )
            .collect();
        picks.sort_by_key(|c| c.index());
        picks
    }
}

/// The name to show for a file in an error message: the path as the user wrote it, or the
/// diff's own display path when the selector carried no explicit path (a single-file diff).
fn display_name(patch: &Patch, fi: usize, path: Option<&[u8]>) -> String {
    path.map(|p| String::from_utf8_lossy(p).into_owned())
        .unwrap_or_else(|| patch.files[fi].display_path())
}

/// Build the result patch: resolve `selectors` against `patch`, cut or clone the addressed
/// sub-hunks, and recompute the new-side anchors so the result stands on its own.
pub fn select(patch: &Patch, selectors: &[Selector]) -> Result<Patch, SelectError> {
    // Auto-split lazily, only for files a selector actually names, and cache by file index so
    // each referenced file is split once (selectors may target the same file repeatedly). The
    // cache is shared between the resolution and emission phases below.
    let mut subs_cache: BTreeMap<usize, Vec<Hunk>> = BTreeMap::new();
    let chosen = resolve_selectors(patch, selectors, &mut subs_cache)?;
    if chosen.is_empty() {
        return Err(SelectError::EmptySelection);
    }
    let mut out = emit_selection(patch, chosen, &subs_cache)?;
    // Every emitted hunk still carries the new-side start it had in the input diff, where the
    // hunks left out were still present. Those anchors describe a file this result does not
    // produce, and `git apply` searches from the new-side position — recompute them from the
    // result alone (see [`crate::renumber`]).
    renumber_new_side(&mut out);
    Ok(out)
}

/// Resolution phase: turn each selector into a per-file map of chosen sub-hunks, auto-splitting
/// (and caching, via `subs_cache`) each referenced file on demand. The cache is returned to the
/// caller because the emission phase reuses the same splits.
fn resolve_selectors(
    patch: &Patch,
    selectors: &[Selector],
    subs_cache: &mut BTreeMap<usize, Vec<Hunk>>,
) -> Result<BTreeMap<usize, FilePicks>, SelectError> {
    let mut chosen: BTreeMap<usize, FilePicks> = BTreeMap::new();
    // Built once for the whole invocation: every path selector looks its file up here.
    let paths = PathIndex::new(patch);
    // The id index costs a pass over the whole patch, so it is built on first use: an
    // invocation without `@id` selectors never touches a file no selector names.
    let mut ids: Option<IdIndex> = None;
    // Ids already resolved in this invocation. Resolving one is idempotent — the same matches,
    // the same collision verdict, the same picks — but not free: the collision check compares
    // the changed lines of every match against the first, so an id shared by M sub-hunks costs
    // M comparisons per occurrence. A script that reads the ids out of `list --json` names such
    // an id once per sub-hunk, which made the check quadratic on its own: 8 000 identical
    // changes took over six minutes even with the picks deduplicated.
    let mut resolved_ids: HashSet<u64> = HashSet::new();
    for sel in selectors {
        match sel {
            Selector::Id(id) => {
                let index = ids.get_or_insert_with(|| IdIndex::new(patch, subs_cache));
                resolve_id(patch, id, subs_cache, index, &mut chosen, &mut resolved_ids)?
            }
            Selector::File { path, indices } => resolve_file_selector(
                patch,
                &paths,
                path.as_deref(),
                indices,
                subs_cache,
                &mut chosen,
            )?,
        }
    }
    Ok(chosen)
}

/// Resolve one `path:set` selector (including the bare-set form, where `path` is `None`) and
/// record its picks in `chosen`.
fn resolve_file_selector(
    patch: &Patch,
    paths: &PathIndex<'_>,
    path: Option<&[u8]>,
    indices: &IndexSet,
    subs_cache: &mut BTreeMap<usize, Vec<Hunk>>,
    chosen: &mut BTreeMap<usize, FilePicks>,
) -> Result<(), SelectError> {
    let fi = paths.resolve(path)?;
    // A binary file has no sub-hunks; a non-line-set selector picks the whole binary change.
    // A line-set selector makes no sense for a binary file.
    if matches!(patch.files[fi].content, FileContent::Binary(_)) {
        if let IndexSet::LineSet { .. } = indices {
            return Err(SelectError::LineSelect(format!(
                "{} is a binary file",
                patch.files[fi].display_path()
            )));
        }
        chosen.entry(fi).or_default();
        return Ok(());
    }
    let subs = subs_cache
        .entry(fi)
        .or_insert_with(|| build_file_subs(&patch.files[fi]));
    match indices {
        IndexSet::All => {
            // A text entry may legitimately have no hunks — a pure rename or a mode change.
            // `*` then means "take this entry", the same as for a binary file, so record it
            // even though there is nothing to pick.
            let picks = chosen.entry(fi).or_default();
            picks.whole.extend(1..=subs.len());
        }
        IndexSet::List(v) => {
            // Every index of this selector lands in the same file, so the entry is looked up
            // once rather than per index. An error below aborts the whole selection anyway,
            // so an entry created for a selector that then fails is never observed.
            let picks = chosen.entry(fi).or_default();
            for &idx in v {
                if idx > subs.len() {
                    return Err(SelectError::NoIndex(format!(
                        "{}:{idx}",
                        display_name(patch, fi, path)
                    )));
                }
                picks.whole.insert(idx);
            }
        }
        IndexSet::LineSet { index, lines } => {
            if *index > subs.len() {
                return Err(SelectError::NoIndex(format!(
                    "{}:{index}",
                    display_name(patch, fi, path)
                )));
            }
            // The concrete range check (against the sub-hunk's changed-line count) and
            // normalisation (sort + dedup, via a `BTreeSet`) happen at emission in
            // `slice_changed_lines`; no need to canonicalise the raw indices here.
            chosen
                .entry(fi)
                .or_default()
                .lines
                .push((*index, lines.clone()));
        }
    }
    Ok(())
}

/// A sub-hunk addressed by `@L` (a changed-line subset) must be its ONLY selection. Partial `@L`
/// pieces of one sub-hunk emitted together would carry mutually inconsistent new-side line
/// numbers, and combining `@L` with a whole pick of the same sub-hunk double-counts its lines.
/// Reject as a usage error before emission; the diff -> stage -> re-diff loop is the way to
/// combine such pieces.
fn reject_conflicting_line_set_picks(picks: &FilePicks) -> Result<(), SelectError> {
    // Count once per sub-hunk index instead of rescanning the slices for each one: the
    // per-pick scan is quadratic, and a scripted caller can pass a long list of selectors.
    // Only slices are counted here — a whole pick is idempotent and the set already holds it
    // at most once, so a repeat cannot make a conflict out of nothing.
    let mut sliced: BTreeMap<usize, usize> = BTreeMap::new();
    for (index, _) in &picks.lines {
        *sliced.entry(*index).or_insert(0) += 1;
    }
    for (index, times) in sliced {
        if times > 1 || picks.whole.contains(&index) {
            return Err(SelectError::LineSelect(format!(
                "sub-hunk {index} is addressed by @L together with another \
                 selection of the same sub-hunk; address it once, or stage the \
                 pieces in separate rounds"
            )));
        }
    }
    Ok(())
}

/// A whole-file deletion has nothing to select from: every line of it is a deletion, so an `@L`
/// slice keeps the unselected ones as context and the result declares a file removed while its
/// body still lists lines. `git apply` rejects that with `deleted file <path> still has
/// contents`, and the internal check cannot see it — it compares counts, order and anchors, not
/// the headers against the body. Selecting every changed line is fine: no context survives, and
/// the result is the deletion itself.
fn reject_partial_selection_of_a_deleted_file(
    f: &FileDiff,
    hunks: &[Hunk],
) -> Result<(), SelectError> {
    let declares_deletion = f.new_path.as_deref() == Some(b"/dev/null".as_slice())
        || f.headers
            .iter()
            .any(|h| h.starts_with(b"deleted file mode"));
    if !declares_deletion {
        return Ok(());
    }
    if hunks
        .iter()
        .any(|h| h.lines.iter().any(|l| l.kind == LineKind::Context))
    {
        return Err(SelectError::LineSelect(format!(
            "{}: the entry deletes the file as a whole, so a partial @L selection cannot be \
             expressed as one diff; select the sub-hunk whole, or stage the removal on its own",
            String::from_utf8_lossy(f.old_path.as_deref().unwrap_or(b"?"))
        )));
    }
    Ok(())
}

/// Turn ordered, deduplicated picks into the hunks of one file: a whole pick clones its
/// sub-hunk, an `@L` pick cuts it down to the selected changed lines.
fn materialise_picks(subs: &[Hunk], picks: &[Chosen]) -> Result<Vec<Hunk>, SelectError> {
    let mut hunks = Vec::with_capacity(picks.len());
    for pick in picks {
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
    Ok(hunks)
}

/// Emission phase: materialise the resolved selections into a result patch. Per file, order the
/// picks, drop duplicates, and cut/clone each sub-hunk. `subs_cache` must already hold the
/// splits for every referenced file (populated by `resolve_selectors`).
fn emit_selection(
    patch: &Patch,
    chosen: BTreeMap<usize, FilePicks>,
    subs_cache: &BTreeMap<usize, Vec<Hunk>>,
) -> Result<Patch, SelectError> {
    let mut files = Vec::new();
    let mut last_fi = None;
    // Whether the file emitted last ends on the same line its source did — see the flag at the
    // end of this function. Set per file; only the value left by the last one matters.
    let mut ends_on_the_files_last_line = false;
    for (fi, picks) in chosen {
        last_fi = Some(fi);
        let src = &patch.files[fi];
        let content = match &src.content {
            // A binary file has no sub-hunks; its picks vec is always empty. It is taken whole,
            // so its last line is emitted.
            FileContent::Binary(b) => {
                ends_on_the_files_last_line = true;
                FileContent::Binary(b.clone())
            }
            FileContent::Text(_) => {
                reject_conflicting_line_set_picks(&picks)?;
                // Ordered by sub-hunk index so emitted hunks follow old-file order. Distinct
                // sub-hunks are disjoint, so no overlap check is needed, and a duplicate whole
                // pick can no longer arrive: the accumulator holds those in a set.
                let picks = picks.into_ordered();
                let subs = &subs_cache[&fi];
                // The file's last line is the last line of its last sub-hunk, and it survives
                // only when that sub-hunk is taken whole — an `@L` cut may drop it.
                ends_on_the_files_last_line =
                    matches!(picks.last(), Some(Chosen::Whole(i)) if *i == subs.len());
                let hunks = materialise_picks(subs, &picks)?;
                reject_partial_selection_of_a_deleted_file(src, &hunks)?;
                FileContent::Text(hunks)
            }
        };
        // Only the file's tail (lines after its last hunk, e.g. the `-- \n<version>` signature
        // of a format-patch) carries over: it keeps its meaning whichever sub-hunks were
        // picked. Lines recorded between hunks have no defined place once hunks are dropped
        // or split, so they are not emitted.
        let src_hunks = src.hunk_count();
        let out_hunks = content.hunk_count();
        let trailer: Vec<_> = src
            .trailer
            .iter()
            .filter(|(at, _)| *at == src_hunks)
            .map(|(_, l)| (out_hunks, l.clone()))
            .collect();
        // A tail is emitted after the hunks and always carries over, so with one present the
        // file ends on the line it ended on before, whichever sub-hunks were picked.
        ends_on_the_files_last_line |= !trailer.is_empty();
        files.push(FileDiff {
            headers: src.headers.clone(),
            trailer,
            old_path: src.old_path.clone(),
            new_path: src.new_path.clone(),
            content,
        });
    }
    // The preamble carries over with the files: a format-patch input stays a mail, head and
    // footer both. Its diffstat then describes the original commit rather than the selection —
    // the caller chose to filter a mail, and hunkpick does not rewrite prose.
    // The flag is about one line of the input — its last. It carries over only when the result
    // ends on that same line: the input's last file is also the result's last, and that file is
    // emitted down to its final line. Otherwise the result ends somewhere else, and dropping the
    // newline there would truncate a line nobody asked to change.
    let ends_on_the_inputs_last_line = patch.no_trailing_newline
        && last_fi == patch.files.len().checked_sub(1)
        && ends_on_the_files_last_line;
    Ok(Patch {
        preamble: patch.preamble.clone(),
        files,
        no_trailing_newline: ends_on_the_inputs_last_line,
    })
}

/// Resolve an `@<id>` selector: match every sub-hunk in the patch whose content hash equals
/// `id`, confirm via [`all_same_content`] that the matches carry identical changed (`+`/`-`)
/// lines under the same path (otherwise an accidental hash collision between distinct changes),
/// and record their indices in `chosen`. Surrounding context is deliberately not compared —
/// the id is context-free, so the same change in a different context shares it. Binary files
/// have no sub-hunks and are skipped. This necessarily scans (and auto-splits) the whole patch,
/// unlike path selectors.
fn resolve_id(
    patch: &Patch,
    id: &str,
    subs_cache: &mut BTreeMap<usize, Vec<Hunk>>,
    ids: &IdIndex,
    chosen: &mut BTreeMap<usize, FilePicks>,
    resolved: &mut HashSet<u64>,
) -> Result<(), SelectError> {
    // Compare 64-bit hashes rather than rendered hex strings to avoid an allocation per
    // sub-hunk across the full scan. `from_str_radix` accepts upper- or lowercase hex.
    let target = u64::from_str_radix(id, 16).map_err(|_| SelectError::UnknownId(id.to_string()))?;
    // A repeat asks for what the first occurrence already recorded, and its verdict was the
    // same: an id that resolved cannot come back as unknown or as a collision. Only its cost
    // would repeat.
    if resolved.contains(&target) {
        return Ok(());
    }

    let matched: &[(usize, usize)] = ids.lookup(target);
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
    for &(fi, si) in matched {
        chosen.entry(fi).or_default().whole.insert(si);
    }
    resolved.insert(target);
    Ok(())
}

/// Content hash to the sub-hunks carrying it: `(file index, 1-based sub-hunk index)`.
///
/// An `@id` has to look at the whole patch — the id says nothing about which file holds it — so
/// the auto-split and the hashing are unavoidable once. Repeating the scan per id is not:
/// selecting by id is a batch operation (read the ids from `list --json`, stage them together),
/// and a linear scan per id makes that quadratic in the size of the diff. Built on the first
/// `@id` of an invocation and reused by the rest; a patch with no id selector never pays for it.
/// Measured on a 20 000 sub-hunk diff, release build, selecting every id: 0.22 s scanning per
/// id against 0.06 s through this index.
struct IdIndex {
    by_id: HashMap<u64, Vec<(usize, usize)>>,
}

impl IdIndex {
    /// Hash every sub-hunk of every text file, filling `subs_cache` with the splits so the
    /// emission phase reuses them. Binary entries have no sub-hunks and are skipped.
    fn new(patch: &Patch, subs_cache: &mut BTreeMap<usize, Vec<Hunk>>) -> Self {
        let mut by_id: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
        for (fi, f) in patch.files.iter().enumerate() {
            if matches!(f.content, FileContent::Binary(_)) {
                continue;
            }
            let subs = subs_cache.entry(fi).or_insert_with(|| build_file_subs(f));
            for (si, sub) in subs.iter().enumerate() {
                by_id
                    .entry(subhunk_hash(f, sub))
                    .or_default()
                    .push((fi, si + 1));
            }
        }
        Self { by_id }
    }

    /// Every sub-hunk carrying `target`, in patch order. Byte-identical changes share an id, so
    /// more than one match is normal — the caller checks they really are identical.
    fn lookup(&self, target: u64) -> &[(usize, usize)] {
        self.by_id.get(&target).map_or(&[], Vec::as_slice)
    }
}

/// Which entries of a patch carry a given path. `Many` is kept as a distinct state rather than
/// a list: the only thing the caller does with it is refuse the selector as ambiguous.
#[derive(Clone, Copy)]
enum PathOwner {
    One(usize),
    Many,
}

/// Path (either side) to the entry carrying it, built once per invocation.
///
/// Scanning `patch.files` per selector is O(selectors x files), and naming one selector per
/// file is the documented scripted use: 16 000 of each took 1.5 s that way against a fraction
/// of that here. Borrows the patch, so no path bytes are copied.
pub(crate) struct PathIndex<'a> {
    by_path: HashMap<&'a [u8], PathOwner>,
    file_count: usize,
}

impl<'a> PathIndex<'a> {
    pub(crate) fn new(patch: &'a Patch) -> Self {
        let mut by_path: HashMap<&'a [u8], PathOwner> = HashMap::with_capacity(patch.files.len());
        for (fi, f) in patch.files.iter().enumerate() {
            for path in [f.new_path.as_deref(), f.old_path.as_deref()]
                .into_iter()
                .flatten()
            {
                by_path
                    .entry(path)
                    .and_modify(|owner| {
                        // The two sides of one entry name the same path in the common case;
                        // that is not an ambiguity, a second *entry* is.
                        if !matches!(owner, PathOwner::One(seen) if *seen == fi) {
                            *owner = PathOwner::Many;
                        }
                    })
                    .or_insert(PathOwner::One(fi));
            }
        }
        PathIndex {
            by_path,
            file_count: patch.files.len(),
        }
    }

    /// Resolve an optional path to a file index. With no path, succeeds only for single-file
    /// diffs.
    pub(crate) fn resolve(&self, path: Option<&[u8]>) -> Result<usize, SelectError> {
        let Some(p) = path else {
            return if self.file_count == 1 {
                Ok(0)
            } else {
                Err(SelectError::AmbiguousPath(
                    "<no path on multi-file diff>".into(),
                ))
            };
        };
        match self.by_path.get(p) {
            Some(PathOwner::One(fi)) => Ok(*fi),
            Some(PathOwner::Many) => Err(SelectError::AmbiguousPath(
                String::from_utf8_lossy(p).into_owned(),
            )),
            None => Err(SelectError::UnknownPath(
                String::from_utf8_lossy(p).into_owned(),
            )),
        }
    }
}

/// Resolve `path:N` / `N` to (file_index, original_hunk_index_0based) for the `split` command.
///
/// Takes the address as an [`OsStr`] rather than a `&str` for the same reason
/// [`parse_selectors`] does: a diff can name a file whose path is not valid UTF-8, and an
/// address that cannot spell those bytes leaves such a file addressable by `select` and
/// unreachable by `split`. Only the index after the last ':' is text; the path is bytes.
pub fn resolve_hunk(patch: &Patch, addr: &OsStr) -> Result<(usize, usize), SelectError> {
    let bytes = os_bytes(addr);
    let shown = String::from_utf8_lossy(bytes).into_owned();
    let index_of = |b: &[u8]| std::str::from_utf8(b).ok().and_then(|s| s.parse().ok());
    // A path may itself contain ':', so the split point is the last one whose tail is an index.
    let (path, n): (Option<&[u8]>, Option<usize>) = match bytes.iter().rposition(|&b| b == b':') {
        Some(i) if i > 0 && index_of(&bytes[i + 1..]).is_some() => {
            (Some(&bytes[..i]), index_of(&bytes[i + 1..]))
        }
        _ => (None, index_of(bytes)),
    };
    let n = n.filter(|&n| n > 0).ok_or_else(|| {
        // A bare address that is not an index is reported the way an unparsable selector is.
        SelectError::BadSelector(shown.clone())
    })?;
    let fi = PathIndex::new(patch).resolve(path)?;
    match &patch.files[fi].content {
        FileContent::Text(h) if n <= h.len() => Ok((fi, n - 1)),
        FileContent::Text(_) => Err(SelectError::NoIndex(shown)),
        FileContent::Binary(_) => Err(SelectError::BadSelector(format!("{shown} (binary file)"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::emit;
    use crate::gittest::applies_to_file;
    use crate::parser::parse;
    use crate::subhunk_id::subhunk_id;

    const TWO_CHANGES: &str = "\
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
            trailer: Vec::new(),
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
                    no_newline: None,
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
        assert!(parse_index_list("1-100000000", &mut IndexBudget::new()).is_err());
        assert!(parse_selectors(&["1-100000000".to_string()]).is_err());
    }

    /// The allowance belongs to the invocation, not to the selector. It used to be checked per
    /// selector while their number is bounded only by the length of the command line, so 200
    /// copies of `1-1048576` — 2.6 KB of argv against a four-sub-hunk diff — reached 1.6 GB of
    /// RSS before a single index was compared against the diff.
    #[test]
    fn the_index_allowance_is_shared_by_every_selector() {
        let one = format!("1-{MAX_SELECTOR_INDICES}");
        // One selector may spend the whole allowance.
        assert!(parse_selectors(std::slice::from_ref(&one)).is_ok());
        // Two may not, however they are spelled.
        assert!(parse_selectors(&[one.clone(), one.clone()]).is_err());
        assert!(parse_selectors(&[format!("f:{one}"), format!("g:{one}")]).is_err());
        // And the allowance is spent by `@L` line sets too, not only by whole-sub-hunk lists.
        assert!(parse_selectors(&[format!("1@L1-{MAX_SELECTOR_INDICES}"), one]).is_err());
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
                path: Some(b"src/f".to_vec()),
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
                path: Some(b"@foo".to_vec()),
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
                path: Some(b"src/f".to_vec()),
                indices: IndexSet::List(vec![2, 3, 4]),
            }
        );
    }

    #[test]
    fn select_first_subhunk_only() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
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
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
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
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let view = build_view(&p);
        let subs = &view[0];
        // Second sub-hunk: the d->D change.
        let id = subhunk_id(&p.files[0], &subs[1]);
        let sels = parse_selectors(&[format!("@{id}")]).unwrap();
        let out = select(&p, &sels).unwrap();
        let text = String::from_utf8(emit(&out)).unwrap();
        assert!(text.contains("+D"), "addressed change present: {text}");
        assert!(!text.contains("+B"), "other change excluded: {text}");
    }

    /// Resolving `@id` selectors must stay linear in the size of the diff. Selecting by id is a
    /// batch operation — read the ids from `list --json`, stage them in one invocation — and a
    /// scan over every sub-hunk's hash per id makes that quadratic. Exercised here rather than
    /// through the CLI because Windows caps a command line at 32 KiB, well below eight thousand
    /// selectors.
    #[test]
    fn many_ids_resolve_without_quadratic_blowup() {
        const RUNS: usize = 8_000;
        let mut diff = format!(
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,{n} +1,{n} @@\n",
            n = RUNS * 2
        );
        for i in 0..RUNS {
            diff.push_str(&format!(" ctx{i}\n-old{i}\n+new{i}\n"));
        }
        let p = parse(diff.as_bytes()).unwrap();
        let ids: Vec<String> = build_view(&p)[0]
            .iter()
            .map(|sub| format!("@{}", subhunk_id(&p.files[0], sub)))
            .collect();
        assert_eq!(ids.len(), RUNS, "one sub-hunk per change run");
        let sels = parse_selectors(&ids).unwrap();

        let started = std::time::Instant::now();
        let out = select(&p, &sels).unwrap();
        let elapsed = started.elapsed();

        assert_eq!(out.files[0].hunk_count(), RUNS, "every id is selected");
        // The nextest profile bounds a test's runtime, but the documented fallback
        // (`cargo test -- --test-threads=4`) does not: a quadratic regression would hang there
        // instead of failing. Linear resolution of this input is a fraction of a second even in
        // a debug build, so a minute is unreachable without a change in complexity.
        assert!(
            elapsed < std::time::Duration::from_secs(60),
            "resolving {RUNS} ids took {elapsed:?}"
        );
    }

    /// The other half of the same guarantee, and the shape a script actually produces: every
    /// change in the file is identical, so they all share one id, and reading the ids out of
    /// `list --json` yields that id once per sub-hunk. Recording a pick per (occurrence x match)
    /// made the accumulator grow as the square of the number of changes — 8 000 of them cost
    /// A repeated `*` is the same accumulator defect reached without ids: each one used to add a
    /// pick per sub-hunk of the file. Nobody writes it by hand, but the cost belongs to the
    /// The accumulator must hold one pick per selected sub-hunk, not one per (occurrence x
    /// match). Every change here is identical, so all of them share one id, and reading the ids
    /// out of `list --json` yields that id once per sub-hunk — the shape a script produces. A
    /// pick appended per pair made the accumulator grow as the square of the number of changes
    /// (8 000 of them cost 10 s and 3 GB of RSS), and the duplicates were only dropped at
    /// emission, after the whole array had been built. Asserted on the count rather than on
    /// time or memory: the defect is the count, and measuring it needs no large input.
    #[test]
    fn a_shared_id_records_one_pick_per_sub_hunk() {
        const RUNS: usize = 200;
        let mut diff = format!(
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,{n} +1,{n} @@\n",
            n = RUNS * 2
        );
        // The context differs so the sub-hunks stay separate; the changed lines are identical,
        // so every sub-hunk hashes to the same id (context is not part of the hash).
        for i in 0..RUNS {
            diff.push_str(&format!(" ctx{i}\n-old\n+new\n"));
        }
        let p = parse(diff.as_bytes()).unwrap();
        let ids: Vec<String> = build_view(&p)[0]
            .iter()
            .map(|sub| format!("@{}", subhunk_id(&p.files[0], sub)))
            .collect();
        assert_eq!(
            ids.iter().collect::<BTreeSet<_>>().len(),
            1,
            "identical changes must share one id"
        );
        let sels = parse_selectors(&ids).unwrap();

        let mut subs_cache = BTreeMap::new();
        let chosen = resolve_selectors(&p, &sels, &mut subs_cache).unwrap();
        let picks: usize = chosen.values().map(|f| f.len()).sum();
        assert_eq!(
            picks, RUNS,
            "{RUNS} copies of one id matching {RUNS} sub-hunks must record {RUNS} picks"
        );
    }

    /// The same accumulator defect reached without ids: each `*` used to add a pick per
    /// sub-hunk of the file. Nobody repeats `*` by hand, but the cost belongs to the
    /// accumulator, not to the spelling of the selector.
    #[test]
    fn a_repeated_star_records_one_pick_per_sub_hunk() {
        const RUNS: usize = 200;
        let mut diff = format!(
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,{n} +1,{n} @@\n",
            n = RUNS * 2
        );
        for i in 0..RUNS {
            diff.push_str(&format!(" ctx{i}\n-old{i}\n+new{i}\n"));
        }
        let p = parse(diff.as_bytes()).unwrap();
        let sels = parse_selectors(&vec!["*".to_string(); RUNS]).unwrap();

        let mut subs_cache = BTreeMap::new();
        let chosen = resolve_selectors(&p, &sels, &mut subs_cache).unwrap();
        let picks: usize = chosen.values().map(|f| f.len()).sum();
        assert_eq!(
            picks, RUNS,
            "{RUNS} repeats of `*` over {RUNS} sub-hunks must record {RUNS} picks"
        );

        // And the selection itself is unchanged by the repetition.
        let out = select(&p, &sels).unwrap();
        assert_eq!(out.files[0].hunk_count(), RUNS);
    }

    /// The other half of the same cost, which deduplicating the picks does not touch: the
    /// collision check compares the changed lines of every match against the first, so an id
    /// shared by M sub-hunks costs M comparisons per occurrence. Naming it once per sub-hunk —
    /// what a script reading `list --json` produces — made that quadratic on its own: 8 000
    /// identical changes took 389 s with the picks already deduplicated. Kept end-to-end rather
    /// than structural because the work is inside the check, not in a countable result.
    #[test]
    fn a_shared_id_named_once_per_match_stays_linear() {
        const RUNS: usize = 8_000;
        let mut diff = format!(
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,{n} +1,{n} @@\n",
            n = RUNS * 2
        );
        for i in 0..RUNS {
            diff.push_str(&format!(" ctx{i}\n-old\n+new\n"));
        }
        let p = parse(diff.as_bytes()).unwrap();
        let ids: Vec<String> = build_view(&p)[0]
            .iter()
            .map(|sub| format!("@{}", subhunk_id(&p.files[0], sub)))
            .collect();
        assert_eq!(
            ids.iter().collect::<BTreeSet<_>>().len(),
            1,
            "identical changes must share one id"
        );
        let sels = parse_selectors(&ids).unwrap();

        let started = std::time::Instant::now();
        let out = select(&p, &sels).unwrap();
        let elapsed = started.elapsed();

        assert_eq!(
            out.files[0].hunk_count(),
            RUNS,
            "every sub-hunk is selected"
        );
        // The nextest profile bounds a test's runtime, but the documented fallback
        // (`cargo test -- --test-threads=4`) does not: a quadratic regression would run for
        // minutes there instead of failing. Linear resolution of this input is a fraction of a
        // second even in a debug build.
        assert!(
            elapsed < std::time::Duration::from_secs(60),
            "resolving {RUNS} copies of one shared id took {elapsed:?}"
        );
    }

    #[test]
    fn select_id_is_case_insensitive() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let view = build_view(&p);
        let id = subhunk_id(&p.files[0], &view[0][0]).to_uppercase();
        let sels = parse_selectors(&[format!("@{id}")]).unwrap();
        assert!(select(&p, &sels).is_ok(), "uppercase id must still match");
    }

    #[test]
    fn select_id_selects_all_identical_subhunks() {
        let p = parse(SAME_TWICE.as_bytes()).unwrap();
        let view = build_view(&p);
        let subs = &view[0];
        assert_eq!(subs.len(), 2);
        let id0 = subhunk_id(&p.files[0], &subs[0]);
        let id1 = subhunk_id(&p.files[0], &subs[1]);
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
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let sels = parse_selectors(&["@0000000000000000".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::UnknownId(_))));
    }

    #[test]
    fn collision_check_distinguishes_distinct_content() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let view = build_view(&p);
        let subs = &view[0];
        let f = &p.files[0];
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
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let sels = parse_selectors(&["9".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::NoIndex(_))));
    }

    #[test]
    fn select_empty_is_error() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        assert_eq!(select(&p, &[]), Err(SelectError::EmptySelection));
    }

    #[test]
    fn resolve_hunk_addresses_original_hunk() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        assert_eq!(resolve_hunk(&p, OsStr::new("1")).unwrap(), (0, 0));
    }

    #[test]
    #[cfg(unix)]
    fn selector_path_may_hold_invalid_utf8() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        // A diff can name a file whose path is not valid UTF-8, so the selector must be able
        // to spell it: the path is kept as raw bytes, only the set after ':' is text.
        let sels = parse_selectors(&[OsString::from_vec(b"bad\xffname.txt:1,3".to_vec())]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: Some(b"bad\xffname.txt".to_vec()),
                indices: IndexSet::List(vec![1, 3]),
            }
        );
        // Without a set there is nothing such an argument could be: report why, not "bad
        // selector".
        match parse_selectors(&[OsString::from_vec(b"bad\xffname.txt".to_vec())]) {
            Err(SelectError::BadSelector(msg)) => {
                assert!(msg.contains("not valid UTF-8"), "message was {msg:?}")
            }
            other => panic!("expected BadSelector, got {other:?}"),
        }
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
    fn path_form_reports_the_set_reason_not_the_whole_arg() {
        // In `path:set` a broken set used to be re-read as a bare set together with the path,
        // so `f:2-1` was reported as "not a number: f:2" instead of naming the real fault.
        let cases = [
            ("f:2-1", "reversed range"),
            ("f:0", "1-based"),
            ("f:1-99999999", "range too large"),
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
        // A colon inside a path is still a path: the text after it is not set-shaped, so the
        // arg falls through to the path form rather than being reported as a broken set.
        let parsed = parse_selectors(&["x:y:1".to_string()]).unwrap();
        assert!(
            matches!(&parsed[0], Selector::File { path: Some(p), .. } if p == b"x:y"),
            "expected path form for x:y:1, got {parsed:?}"
        );
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
    fn select_line_set_first_two_changed_lines() {
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
    fn select_second_subhunk_applies_via_git() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        // Select only the SECOND sub-hunk (the d->D change).
        let sels = parse_selectors(&["2".to_string()]).unwrap();
        let out = select(&p, &sels).unwrap();
        let diff = emit(&out);
        assert!(
            applies_to_file(&diff, "a\nb\nc\nd\ne\n"),
            "second-only sub-hunk failed to apply:\n{}",
            String::from_utf8_lossy(&diff)
        );
    }

    /// A single-file replacement diff: file `a,b` -> `A,B` as one contiguous run.
    /// Changed lines: 1=`-a`, 2=`-b`, 3=`+A`, 4=`+B`.
    const REPLACEMENT: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
-a
-b
+A
+B
";

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
                path: Some(b"src/f".to_vec()),
                indices: IndexSet::LineSet {
                    index: 2,
                    lines: vec![1],
                },
            }
        );
        // A range inside the set expands the same way behind a path.
        let sels = parse_selectors(&["src/f:2@L1-2,4".to_string()]).unwrap();
        assert_eq!(
            sels[0],
            Selector::File {
                path: Some(b"src/f".to_vec()),
                indices: IndexSet::LineSet {
                    index: 2,
                    lines: vec![1, 2, 4],
                },
            }
        );
    }

    #[test]
    fn parse_line_set_rejects_malformed() {
        // Empty set, zero index bound, zero sub-hunk index, reversed range inside the set.
        assert!(parse_selectors(&["1@L".to_string()]).is_err());
        assert!(parse_selectors(&["1@L0".to_string()]).is_err());
        assert!(parse_selectors(&["0@L1".to_string()]).is_err());
        assert!(parse_selectors(&["1@L3-1".to_string()]).is_err());
        // '@id@Lset' is NOT supported: only a numeric index may precede '@'.
        assert!(parse_selectors(&["@deadbeef@L1-2".to_string()]).is_err());
    }

    #[test]
    fn select_line_set_separates_deletions_from_additions() {
        // The key agent operation: two invocations, each applying to the original file, stage the
        // removals and the insertions of a replacement independently.
        let p = parse(REPLACEMENT.as_bytes()).unwrap();

        let dels = select(&p, &parse_selectors(&["1@L1,2".to_string()]).unwrap()).unwrap();
        let dels_text = String::from_utf8(emit(&dels)).unwrap();
        assert!(dels_text.contains("-a") && dels_text.contains("-b"));
        assert!(!dels_text.contains("+A") && !dels_text.contains("+B"));
        assert!(
            applies_to_file(&emit(&dels), "a\nb\n"),
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
            applies_to_file(&emit(&adds), "a\nb\n"),
            "addition piece must apply"
        );
    }

    #[test]
    fn select_line_set_out_of_range_errors() {
        let p = parse(REPLACEMENT.as_bytes()).unwrap(); // 4 changed lines
        let sels = parse_selectors(&["1@L5".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::LineSelect(_))));
    }

    #[test]
    fn select_line_set_unknown_index_errors() {
        let p = parse(REPLACEMENT.as_bytes()).unwrap();
        let sels = parse_selectors(&["9@L1".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::NoIndex(_))));
    }

    #[test]
    fn select_line_set_combined_with_whole_same_subhunk_rejected() {
        // `@L` of a sub-hunk plus the whole sub-hunk double-counts its lines: usage error.
        let p = parse(REPLACEMENT.as_bytes()).unwrap();
        let sels = parse_selectors(&["1".to_string(), "1@L1".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::LineSelect(_))));
    }

    #[test]
    fn select_two_line_sets_of_same_subhunk_rejected() {
        // Two `@L` selections of the same sub-hunk in one invocation would emit mutually
        // inconsistent pieces: usage error, use the re-diff loop instead.
        let p = parse(REPLACEMENT.as_bytes()).unwrap();
        let sels = parse_selectors(&["1@L1,2".to_string(), "1@L3,4".to_string()]).unwrap();
        assert!(matches!(select(&p, &sels), Err(SelectError::LineSelect(_))));
    }
}
