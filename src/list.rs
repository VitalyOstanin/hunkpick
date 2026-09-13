use crate::emit::{fmt_range, section_text};
use crate::model::*;
use crate::select::{SubhunkFilter, build_view};
use crate::subhunk_id::{format_id, subhunk_hash, subhunk_id};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Write as _;

#[derive(Serialize)]
struct JsonChangedLine {
    /// 1-based index over the sub-hunk's changed (`+`/`-`) lines in body order. Pass a set of
    /// these to `select INDEX@L<set>` to stage an arbitrary subset of the sub-hunk's changes.
    i: usize,
    /// `"add"` for an inserted line, `"del"` for a removed one.
    kind: &'static str,
    /// The line text (without the leading marker), decoded lossily for display.
    text: String,
}

#[derive(Serialize)]
struct JsonHunk {
    index: usize,
    /// Stable content id; pass as `@<id>` to `select`. See [`crate::subhunk_id`].
    id: String,
    /// How many sub-hunks in the whole patch share this id. `1` means the id is unique
    /// (so `@<id>` addresses exactly this sub-hunk); `> 1` means `@<id>` would select all
    /// of them — use `path:N` to pick one.
    id_count: usize,
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
    added: u32,
    deleted: u32,
    /// True when the sub-hunk is all additions (a file-creation or pure-append block).
    addition_only: bool,
    /// The sub-hunk's changed (`+`/`-`) lines, in body order, each with its 1-based index for
    /// `select INDEX@L<set>`. Additions and deletions share one numbering. Empty for a binary
    /// or all-context sub-hunk.
    changed_lines: Vec<JsonChangedLine>,
    header: String,
    preview: String,
}

/// The sub-hunk's changed (`+`/`-`) lines as JSON entries, 1-based in body order.
fn changed_lines(h: &Hunk) -> Vec<JsonChangedLine> {
    h.changed_lines()
        .map(|(i, l)| JsonChangedLine {
            i,
            kind: match l.kind {
                LineKind::Add => "add",
                LineKind::Del => "del",
                LineKind::Context => unreachable!("context lines are filtered out"),
            },
            text: String::from_utf8_lossy(&l.text).into_owned(),
        })
        .collect()
}

#[derive(Serialize)]
struct JsonFile {
    path: String,
    binary: bool,
    hunks: Vec<JsonHunk>,
}

fn header_string(h: &Hunk) -> String {
    let old = fmt_range(h.old_start, h.old_lines);
    let new = fmt_range(h.new_start, h.new_lines);
    let mut s = format!("@@ -{old} +{new} @@");
    let section = section_text(h);
    if !section.is_empty() {
        s.push(' ');
        s.push_str(&String::from_utf8_lossy(section));
    }
    s
}

/// True when the sub-hunk consists solely of additions (no context, no deletions): a
/// file-creation or pure-append block. An empty body is not addition-only.
fn addition_only(h: &Hunk) -> bool {
    !h.lines.is_empty() && h.lines.iter().all(|l| matches!(l.kind, LineKind::Add))
}

fn preview(h: &Hunk) -> String {
    for l in &h.lines {
        match l.kind {
            LineKind::Add => return format!("+{}", String::from_utf8_lossy(&l.text)),
            LineKind::Del => return format!("-{}", String::from_utf8_lossy(&l.text)),
            LineKind::Context => {}
        }
    }
    String::new()
}

/// The addressable sub-hunks of `patch` as pretty-printed JSON: an array of files, each with
/// its sub-hunks (1-based `index`, content `id`, `id_count`, line ranges, changed lines).
///
/// [`ListOptions::filter`] narrows the listing to the sub-hunks it holds (`list --only`), leaving
/// every field as it is in the full listing: `index` keeps numbering the file's sub-hunks and
/// `id_count` keeps counting the whole patch, so a selector read off a narrowed listing addresses
/// the same sub-hunk as one read off the full one. The other options do not apply here: colour is
/// never written into JSON, and the changed lines [`ListOptions::lines`] prints are in the schema
/// unconditionally, under `changed_lines`.
///
/// Text fields (`path`, `header`, `preview`, `changed_lines[].text`) report the diff's own
/// content and are deliberately not display-sanitised the way [`list_human`] sanitises its
/// output: JSON escaping hides control characters but not bidirectional overrides, which come
/// back out of a parser unchanged. A consumer that prints these fields to a terminal must
/// escape them itself. They are also lossy for bytes that are not valid UTF-8 (JSON must be
/// UTF-8), so a path read from here does not necessarily round-trip into a `path:N` selector —
/// the content `id` is the exact handle for such a file.
pub fn list_json(patch: &Patch, opts: &ListOptions<'_>) -> String {
    let view = build_view(patch);
    // Hash each sub-hunk once, keeping the per-file hashes alongside the view so the second
    // pass reuses them instead of recomputing. `hashes[fi][si]` is the hash of the `si`-th
    // sub-hunk of file `fi`.
    let hashes: Vec<Vec<u64>> = view
        .iter()
        .zip(&patch.files)
        .map(|(subs, f)| subs.iter().map(|h| subhunk_hash(f, h)).collect())
        .collect();
    // Histogram of content hashes across the whole patch, so each sub-hunk can report how
    // many sub-hunks share its id (`id_count`).
    let mut counts: HashMap<u64, usize> = HashMap::new();
    for file_hashes in &hashes {
        for hash in file_hashes {
            *counts.entry(*hash).or_insert(0) += 1;
        }
    }
    let files: Vec<JsonFile> = view
        .iter()
        .enumerate()
        .filter(|(fi, _)| shows_file(opts.filter, *fi))
        .map(|(fi, subs)| {
            let f = &patch.files[fi];
            JsonFile {
                path: f.display_path(),
                binary: matches!(f.content, FileContent::Binary(_)),
                hunks: subs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| shows_subhunk(opts.filter, fi, i + 1))
                    .map(|(i, h)| json_hunk(h, i, hashes[fi][i], &counts))
                    .collect(),
            }
        })
        .collect();
    // Infallible in practice: `JsonFile` serializes owned strings, integers and bools with
    // no map keys or custom `Serialize` that could error. `expect` documents that invariant.
    serde_json::to_string_pretty(&files).expect("serializing the JSON hunk list cannot fail")
}

/// One sub-hunk as it appears in `list --json`: `index` is 1-based (the number selectors use),
/// `hash` is its precomputed content hash and `counts` the patch-wide histogram behind
/// `id_count`.
fn json_hunk(h: &Hunk, index: usize, hash: u64, counts: &HashMap<u64, usize>) -> JsonHunk {
    let (added, deleted) = h.change_counts();
    JsonHunk {
        index: index + 1,
        id: format_id(hash),
        id_count: counts[&hash],
        old_start: h.old_start,
        old_lines: h.old_lines,
        new_start: h.new_start,
        new_lines: h.new_lines,
        added,
        deleted,
        addition_only: addition_only(h),
        changed_lines: changed_lines(h),
        header: header_string(h),
        preview: preview(h),
    }
}

/// Whether `filter` lets file `fi` into the listing. No filter lists everything.
fn shows_file(filter: Option<&SubhunkFilter>, fi: usize) -> bool {
    filter.is_none_or(|f| f.contains_key(&fi))
}

/// Whether `filter` lets the `index`-th (1-based) sub-hunk of file `fi` into the listing.
fn shows_subhunk(filter: Option<&SubhunkFilter>, fi: usize, index: usize) -> bool {
    filter.is_none_or(|f| f.get(&fi).is_some_and(|picked| picked.contains(&index)))
}

// SGR (Select Graphic Rendition) parameter codes used for the human-readable listing.
const SGR_BOLD: &str = "1";
const SGR_RED: &str = "31";
const SGR_GREEN: &str = "32";

/// Escape what a terminal would act on rather than show. The listing is what an operator reads
/// before picking a sub-hunk, and its text comes from the diff being filtered: an escape
/// sequence in that content could repaint or rewrite the listing, and a bidirectional
/// override could reorder it. Escaped as `\xNN` / `\u{NNNN}`, so the text stays readable.
/// The JSON listing needs no such pass — serde escapes control characters itself.
fn sanitize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // C0 controls (including ESC and the tab a terminal would expand), and DEL.
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            // Bidirectional formatting: can visually reorder the rest of the line.
            '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {
                let _ = write!(out, "\\u{{{:04x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn paint(s: &str, code: &str, color: bool) -> String {
    if color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// What a listing shows and how. Both [`list_human`] and [`list_json`] take it, so a later
/// listing option changes this type rather than either signature; the JSON listing reads only
/// [`ListOptions::filter`], since colour has no meaning there and the per-line detail is part of
/// its schema already.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListOptions<'a> {
    /// Colour the listing with SGR sequences.
    pub color: bool,
    /// Print each sub-hunk's changed lines under its header line, numbered the way
    /// `select INDEX@L<set>` numbers them.
    pub lines: bool,
    /// List only the files and sub-hunks this filter holds; `None` lists everything. Built by
    /// [`crate::select::resolve_subhunk_filter`] from the same selectors `select` takes.
    pub filter: Option<&'a SubhunkFilter>,
}

/// The addressable sub-hunks of `patch` for a terminal: one line per sub-hunk with its index,
/// content id, `@@` header, change counts, the `[+add]` marker and a preview of the first
/// changed line, and — with [`ListOptions::lines`] — that sub-hunk's changed lines underneath.
/// Control bytes from the diff are escaped; [`ListOptions::color`] adds SGR sequences.
///
/// A filtered listing keeps the numbering of the full one: the sub-hunks left out are skipped,
/// not renumbered, so an index read here addresses the same sub-hunk in `select`.
pub fn list_human(patch: &Patch, opts: &ListOptions<'_>) -> String {
    let color = opts.color;
    let view = build_view(patch);
    let mut out = String::new();
    for (fi, subs) in view.iter().enumerate() {
        if !shows_file(opts.filter, fi) {
            continue;
        }
        let f = &patch.files[fi];
        out.push_str(&sanitize(&f.display_path()));
        if matches!(f.content, FileContent::Binary(_)) {
            out.push_str(" (binary)\n");
            continue;
        }
        out.push('\n');
        for (i, h) in subs.iter().enumerate() {
            if !shows_subhunk(opts.filter, fi, i + 1) {
                continue;
            }
            let (added, deleted) = h.change_counts();
            let idx = paint(&format!("[{}]", i + 1), SGR_BOLD, color);
            let id = subhunk_id(f, h);
            let pv = sanitize(&preview(h));
            let pv = if pv.starts_with('+') {
                paint(&pv, SGR_GREEN, color)
            } else if pv.starts_with('-') {
                paint(&pv, SGR_RED, color)
            } else {
                pv
            };
            // Write directly into the output buffer rather than building a temporary
            // String per line (this runs once per sub-hunk).
            let marker = if addition_only(h) { " [+add]" } else { "" };
            let _ = writeln!(
                out,
                "  {idx} {id} {}  +{added} -{deleted}{marker}  {pv}",
                sanitize(&header_string(h))
            );
            if opts.lines {
                write_changed_lines(&mut out, h);
            }
        }
    }
    out
}

/// The sub-hunk's changed (`+`/`-`) lines under its header line, one per line: the 1-based index
/// `select INDEX@L<set>` addresses it by, the kind, and the text. The numbering is
/// [`Hunk::changed_lines`], the same source `list --json` and the `@L` cut read, so what is
/// printed is what a selector takes — with no translation step in between.
///
/// Printed whole: a long sub-hunk is exactly the one an `@L` cut is needed for, and an elision
/// would hide the numbers the caller came for. Not coloured either: the kind stands in a column
/// of its own, and the line carries no other mark to distinguish.
fn write_changed_lines(out: &mut String, h: &Hunk) {
    // Right-align the indices on the widest of them, so the kind column stays a column. The
    // width comes from the numbering itself, not from the `+N -M` counts: the two agree today,
    // and taking the width from a second source would let a later change to either of them
    // shift the column without anything saying so.
    let width = h.changed_lines().count().to_string().len();
    for (i, l) in h.changed_lines() {
        let kind = match l.kind {
            LineKind::Add => '+',
            LineKind::Del => '-',
            LineKind::Context => unreachable!("context lines are filtered out"),
        };
        let text = sanitize(&String::from_utf8_lossy(&l.text));
        let _ = writeln!(out, "      {i:>width$} {kind} {text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    /// In a CRLF diff the header's trailing CR is the line ending, not section text. `emit`
    /// already knows that; the listing must agree, or it prints a separating space and a raw
    /// control byte — inside a JSON string field that consumers parse.
    #[test]
    fn crlf_header_carries_no_section_in_the_listing() {
        let src = "--- a/f\r\n+++ b/f\r\n@@ -1,3 +1,3 @@\r\n a\r\n-b\r\n+B\r\n c\r\n";
        let p = parse(src.as_bytes()).unwrap();
        let view = build_view(&p);
        assert_eq!(header_string(&view[0][0]), "@@ -1,3 +1,3 @@");
    }

    #[test]
    fn human_escapes_terminal_control_sequences() {
        // Content of the diff being filtered must not be able to repaint or reorder the
        // listing the operator reads.
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1 +1 @@
-\x1b[31mred\u{202e}reversed
+plain
";
        let p = parse(src.as_bytes()).unwrap();
        let out = list_human(&p, &ListOptions::default());
        assert!(
            !out.contains('\u{1b}'),
            "no raw ESC in the listing: {out:?}"
        );
        assert!(!out.contains('\u{202e}'), "no bidi override: {out:?}");
        assert!(out.contains("\\x1b"), "escaped ESC expected: {out:?}");
        assert!(
            out.contains("\\u{202e}"),
            "escaped override expected: {out:?}"
        );
    }

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

    // Two byte-identical changes (same context and edit) -> same content id.
    const DUP: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-x
+Y
 b
@@ -10,3 +10,3 @@
 a
-x
+Y
 b
";

    #[test]
    fn json_id_count_is_one_for_unique_ids() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&list_json(&p, &ListOptions::default())).unwrap();
        assert_eq!(v[0]["hunks"][0]["id_count"], 1);
        assert_eq!(v[0]["hunks"][1]["id_count"], 1);
    }

    #[test]
    fn json_id_count_marks_duplicates() {
        let p = parse(DUP.as_bytes()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&list_json(&p, &ListOptions::default())).unwrap();
        let hunks = &v[0]["hunks"];
        assert_eq!(
            hunks[0]["id"], hunks[1]["id"],
            "identical changes share an id"
        );
        assert_eq!(hunks[0]["id_count"], 2);
        assert_eq!(hunks[1]["id_count"], 2);
    }

    #[test]
    fn json_has_two_subhunks() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let j = list_json(&p, &ListOptions::default());
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v[0]["path"], "f");
        assert_eq!(v[0]["hunks"].as_array().unwrap().len(), 2);
        assert_eq!(v[0]["hunks"][0]["index"], 1);
    }

    #[test]
    fn json_includes_subhunk_id() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let j = list_json(&p, &ListOptions::default());
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let id = v[0]["hunks"][0]["id"].as_str().expect("id field present");
        assert_eq!(id.len(), 16, "id must be 16 hex chars");
        let view = build_view(&p);
        let expected = subhunk_id(&p.files[0], &view[0][0]);
        assert_eq!(id, expected, "json id must match the canonical sub-hunk id");
    }

    #[test]
    fn human_shows_subhunk_id() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let out = list_human(&p, &ListOptions::default());
        let view = build_view(&p);
        let id = subhunk_id(&p.files[0], &view[0][0]);
        assert!(
            out.contains(&id),
            "human output must contain id {id}:\n{out}"
        );
    }

    #[test]
    fn human_lists_indices() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let out = list_human(&p, &ListOptions::default());
        assert!(out.contains("f"));
        assert!(out.contains("[1]"));
        assert!(out.contains("[2]"));
    }

    const NEW_FILE: &str = "\
diff --git a/f b/f
new file mode 100644
--- /dev/null
+++ b/f
@@ -0,0 +1,2 @@
+x
+y
";

    #[test]
    fn json_marks_addition_only() {
        let p = parse(NEW_FILE.as_bytes()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&list_json(&p, &ListOptions::default())).unwrap();
        assert_eq!(v[0]["hunks"][0]["addition_only"], true);
    }

    #[test]
    fn json_addition_only_false_for_mixed() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&list_json(&p, &ListOptions::default())).unwrap();
        assert_eq!(v[0]["hunks"][0]["addition_only"], false);
    }

    #[test]
    fn human_marks_addition_only() {
        let p = parse(NEW_FILE.as_bytes()).unwrap();
        let out = list_human(&p, &ListOptions::default());
        assert!(
            out.contains("[+add]"),
            "addition-only marker missing:\n{out}"
        );
    }

    #[test]
    fn human_no_marker_for_mixed() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let out = list_human(&p, &ListOptions::default());
        assert!(
            !out.contains("[+add]"),
            "addition-only marker must not appear for a mixed sub-hunk:\n{out}"
        );
    }

    #[test]
    fn json_lists_changed_lines_with_indices() {
        // The first sub-hunk of TWO_CHANGES is the b->B change: one deletion, one addition,
        // numbered 1 and 2 in body order (deletion first).
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&list_json(&p, &ListOptions::default())).unwrap();
        let cl = &v[0]["hunks"][0]["changed_lines"];
        assert_eq!(cl.as_array().unwrap().len(), 2);
        assert_eq!(cl[0]["i"], 1);
        assert_eq!(cl[0]["kind"], "del");
        assert_eq!(cl[0]["text"], "b");
        assert_eq!(cl[1]["i"], 2);
        assert_eq!(cl[1]["kind"], "add");
        assert_eq!(cl[1]["text"], "B");
    }

    #[test]
    fn json_changed_lines_number_across_dels_and_adds() {
        // A replacement `-a -b +A +B`: four changed lines numbered 1..4 (both deletions, then
        // both additions), matching the `select INDEX@L<set>` numbering.
        let p = parse(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
-a
-b
+A
+B
"
            .as_bytes(),
        )
        .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&list_json(&p, &ListOptions::default())).unwrap();
        let cl = v[0]["hunks"][0]["changed_lines"]
            .as_array()
            .unwrap()
            .clone();
        let kinds: Vec<&str> = cl.iter().map(|e| e["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["del", "del", "add", "add"]);
        let idx: Vec<u64> = cl.iter().map(|e| e["i"].as_u64().unwrap()).collect();
        assert_eq!(idx, vec![1, 2, 3, 4]);
    }

    #[test]
    fn deletion_only_is_not_flagged() {
        // A pure-deletion sub-hunk is not addition-only: deletions are not `LineKind::Add`.
        let p = parse(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,1 @@
 keep
-gone
"
            .as_bytes(),
        )
        .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&list_json(&p, &ListOptions::default())).unwrap();
        assert_eq!(v[0]["hunks"][0]["addition_only"], false);
        assert!(!list_human(&p, &ListOptions::default()).contains("[+add]"));
    }

    const TWO_FILES: &str = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -1,3 +1,3 @@
 a
-x
+X
 b
diff --git a/b.rs b/b.rs
--- a/b.rs
+++ b/b.rs
@@ -1,3 +1,3 @@
 a
-y
+Y
 b
";

    /// The filter `list --only` builds: file `fi` of the patch, showing the 1-based sub-hunk
    /// `indices` named for it (empty for a binary entry, which has none).
    fn only(fi: usize, indices: &[usize]) -> SubhunkFilter {
        let mut f = SubhunkFilter::new();
        f.insert(fi, indices.iter().copied().collect());
        f
    }

    #[test]
    fn human_lines_carry_the_numbering_the_json_listing_reports() {
        // The whole point of the detail: a caller must be able to read an `@L` index off the
        // human listing instead of pulling `changed_lines` out of the JSON one.
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let out = list_human(
            &p,
            &ListOptions {
                lines: true,
                ..Default::default()
            },
        );
        let v: serde_json::Value =
            serde_json::from_str(&list_json(&p, &ListOptions::default())).unwrap();
        for hi in 0..2 {
            for entry in v[0]["hunks"][hi]["changed_lines"].as_array().unwrap() {
                let i = entry["i"].as_u64().unwrap();
                let kind = if entry["kind"] == "add" { '+' } else { '-' };
                let text = entry["text"].as_str().unwrap();
                let expected = format!("{i} {kind} {text}");
                assert!(
                    out.lines().any(|l| l.trim_start() == expected),
                    "detail line {expected:?} missing from:\n{out}"
                );
            }
        }
    }

    #[test]
    fn human_lines_are_off_by_default() {
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let out = list_human(&p, &ListOptions::default());
        assert_eq!(
            out.lines().count(),
            3,
            "one path line and two sub-hunk lines expected:\n{out}"
        );
    }

    #[test]
    fn human_lines_right_align_the_indices() {
        // Ten changed lines: the kind column must stay a column, so `1` is padded to the width
        // of `10` rather than shifting the rest of the line.
        let mut src = String::from("--- a/f\n+++ b/f\n@@ -1,10 +1,10 @@\n");
        for i in 0..10 {
            src.push_str(&format!("-l{i}\n"));
        }
        for i in 0..10 {
            src.push_str(&format!("+L{i}\n"));
        }
        let p = parse(src.as_bytes()).unwrap();
        let out = list_human(
            &p,
            &ListOptions {
                lines: true,
                ..Default::default()
            },
        );
        // Twenty changed lines in body order: the ten deletions are 1..10, the ten additions
        // 11..20, so the widest index is two digits and `1` is padded to it.
        assert!(
            out.contains("       1 - l0\n"),
            "padded first index:\n{out}"
        );
        assert!(
            out.contains("      10 - l9\n"),
            "widest deletion index:\n{out}"
        );
        assert!(out.contains("      20 + L9\n"), "widest index:\n{out}");
    }

    #[test]
    fn human_lines_escape_terminal_control_sequences() {
        // The detail prints diff content, so it is escaped exactly as the rest of the listing.
        let src = "\
--- a/f
+++ b/f
@@ -1 +1 @@
-\x1b[31mred\u{202e}reversed
+plain
";
        let p = parse(src.as_bytes()).unwrap();
        let out = list_human(
            &p,
            &ListOptions {
                lines: true,
                ..Default::default()
            },
        );
        assert!(!out.contains('\u{1b}'), "no raw ESC: {out:?}");
        assert!(!out.contains('\u{202e}'), "no bidi override: {out:?}");
        assert!(out.contains("\\x1b"), "escaped ESC expected: {out:?}");
        assert!(out.contains("\\u{202e}"), "escaped override: {out:?}");
    }

    #[test]
    fn a_filtered_human_listing_keeps_the_full_numbering() {
        // Skipping a sub-hunk must not renumber the ones that remain: the index printed here is
        // the index `select` takes.
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let out = list_human(
            &p,
            &ListOptions {
                filter: Some(&only(0, &[2])),
                ..Default::default()
            },
        );
        assert!(out.contains("[2]"), "the addressed index stays 2:\n{out}");
        assert!(
            !out.contains("[1]"),
            "the other sub-hunk is left out:\n{out}"
        );
        assert!(
            out.contains("-d"),
            "sub-hunk 2 is the d -> D change, previewed by its deletion:\n{out}"
        );
    }

    #[test]
    fn a_filter_leaves_out_a_file_it_does_not_name() {
        let p = parse(TWO_FILES.as_bytes()).unwrap();
        let out = list_human(
            &p,
            &ListOptions {
                filter: Some(&only(1, &[1])),
                ..Default::default()
            },
        );
        assert!(out.contains("b.rs"), "the named file is listed:\n{out}");
        assert!(!out.contains("a.rs"), "the other file is not:\n{out}");
    }

    #[test]
    fn a_filtered_json_listing_keeps_index_and_id_count() {
        // `id_count` counts the whole patch, filtered or not: it answers "would @id address more
        // than this one", which the filter does not change.
        let p = parse(DUP.as_bytes()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&list_json(
            &p,
            &ListOptions {
                filter: Some(&only(0, &[2])),
                ..Default::default()
            },
        ))
        .unwrap();
        let hunks = v[0]["hunks"].as_array().unwrap();
        assert_eq!(hunks.len(), 1, "one sub-hunk listed");
        assert_eq!(hunks[0]["index"], 2, "the index is not renumbered");
        assert_eq!(
            hunks[0]["id_count"], 2,
            "the id is still shared with the other"
        );
    }

    #[test]
    fn the_changed_line_detail_is_never_coloured() {
        // ADR 0014: the kind stands in a column of its own, and colour marks the index and the
        // preview. With colour on, the header line carries SGR and the detail lines do not.
        let p = parse(TWO_CHANGES.as_bytes()).unwrap();
        let out = list_human(
            &p,
            &ListOptions {
                color: true,
                lines: true,
                ..Default::default()
            },
        );
        let mut header_lines = 0;
        let mut detail_lines = 0;
        for line in out.lines() {
            if line.trim_start().starts_with('\u{1b}') {
                header_lines += 1;
            } else if line.starts_with("      ") {
                detail_lines += 1;
                assert!(!line.contains('\u{1b}'), "detail line coloured: {line:?}");
            }
        }
        assert_eq!(header_lines, 2, "both sub-hunk lines are coloured:\n{out}");
        assert_eq!(detail_lines, 4, "four changed lines printed:\n{out}");
    }

    #[test]
    fn a_changed_line_that_is_not_utf8_is_shown_lossily() {
        // The core is byte-oriented; only what the listing displays is decoded, and it is decoded
        // lossily rather than refused. Such a file is addressed by its content id.
        let mut src = b"--- a/f\n+++ b/f\n@@ -1 +1 @@\n-".to_vec();
        src.extend_from_slice(&[0xff, 0xfe]);
        src.extend_from_slice(b"\n+ok\n");
        let p = parse(&src).unwrap();
        let out = list_human(
            &p,
            &ListOptions {
                lines: true,
                ..Default::default()
            },
        );
        assert!(
            out.contains('\u{fffd}'),
            "undecodable bytes become U+FFFD:\n{out}"
        );
        assert!(out.contains("+ ok"), "the other line is intact:\n{out}");
    }
}
