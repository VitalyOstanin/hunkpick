use crate::model::*;
use crate::select::build_view;
use serde::Serialize;
use std::fmt::Write as _;

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
    /// True when the sub-hunk is all additions and can be cut at any added line via
    /// `select INDEX@lo-hi`.
    addition_only: bool,
    header: String,
    preview: String,
}

#[derive(Serialize)]
struct JsonFile {
    path: String,
    binary: bool,
    hunks: Vec<JsonHunk>,
}

fn header_string(h: &Hunk) -> String {
    let old = crate::emit::fmt_range(h.old_start, h.old_lines);
    let new = crate::emit::fmt_range(h.new_start, h.new_lines);
    let mut s = format!("@@ -{old} +{new} @@");
    if !h.section.is_empty() {
        s.push(' ');
        s.push_str(&String::from_utf8_lossy(&h.section));
    }
    s
}

/// True when the sub-hunk consists solely of additions (no context, no deletions): it can be
/// cut at any added line with `select INDEX@lo-hi`. An empty body is not addition-only.
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

pub fn list_json(patch: &Patch) -> String {
    use crate::subhunk_id::subhunk_hash;
    let view = build_view(patch);
    // Histogram of content hashes across the whole patch, so each sub-hunk can report how
    // many sub-hunks share its id (`id_count`).
    let mut counts: std::collections::HashMap<u64, usize> = std::collections::HashMap::new();
    for (fi, subs) in &view {
        let f = &patch.files[*fi];
        for h in subs {
            *counts.entry(subhunk_hash(f, h)).or_insert(0) += 1;
        }
    }
    let mut files = Vec::new();
    for (fi, subs) in &view {
        let f = &patch.files[*fi];
        let binary = matches!(f.content, FileContent::Binary(_));
        let hunks = subs
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let (added, deleted) = h.change_counts();
                let hash = subhunk_hash(f, h);
                JsonHunk {
                    index: i + 1,
                    id: format!("{hash:016x}"),
                    id_count: counts[&hash],
                    old_start: h.old_start,
                    old_lines: h.old_lines,
                    new_start: h.new_start,
                    new_lines: h.new_lines,
                    added,
                    deleted,
                    addition_only: addition_only(h),
                    header: header_string(h),
                    preview: preview(h),
                }
            })
            .collect();
        files.push(JsonFile {
            path: f.display_path(),
            binary,
            hunks,
        });
    }
    serde_json::to_string_pretty(&files).unwrap()
}

// SGR (Select Graphic Rendition) parameter codes used for the human-readable listing.
const SGR_BOLD: &str = "1";
const SGR_RED: &str = "31";
const SGR_GREEN: &str = "32";

fn paint(s: &str, code: &str, color: bool) -> String {
    if color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn list_human(patch: &Patch, color: bool) -> String {
    let view = build_view(patch);
    let mut out = String::new();
    for (fi, subs) in &view {
        let f = &patch.files[*fi];
        out.push_str(&f.display_path());
        if matches!(f.content, FileContent::Binary(_)) {
            out.push_str(" (binary)\n");
            continue;
        }
        out.push('\n');
        for (i, h) in subs.iter().enumerate() {
            let (added, deleted) = h.change_counts();
            let idx = paint(&format!("[{}]", i + 1), SGR_BOLD, color);
            let id = crate::subhunk_id::subhunk_id(f, h);
            let pv = preview(h);
            let pv = if pv.starts_with('+') {
                paint(&pv, SGR_GREEN, color)
            } else if pv.starts_with('-') {
                paint(&pv, SGR_RED, color)
            } else {
                pv
            };
            // Write directly into the output buffer rather than building a temporary
            // String per line (this runs once per sub-hunk).
            let marker = if addition_only(h) { " [+range]" } else { "" };
            let _ = writeln!(
                out,
                "  {idx} {id} {}  +{added} -{deleted}{marker}  {pv}",
                header_string(h)
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let p = parse(MULTI.as_bytes()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&list_json(&p)).unwrap();
        assert_eq!(v[0]["hunks"][0]["id_count"], 1);
        assert_eq!(v[0]["hunks"][1]["id_count"], 1);
    }

    #[test]
    fn json_id_count_marks_duplicates() {
        let p = parse(DUP.as_bytes()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&list_json(&p)).unwrap();
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
        let p = parse(MULTI.as_bytes()).unwrap();
        let j = list_json(&p);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v[0]["path"], "f");
        assert_eq!(v[0]["hunks"].as_array().unwrap().len(), 2);
        assert_eq!(v[0]["hunks"][0]["index"], 1);
    }

    #[test]
    fn json_includes_subhunk_id() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let j = list_json(&p);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        let id = v[0]["hunks"][0]["id"].as_str().expect("id field present");
        assert_eq!(id.len(), 16, "id must be 16 hex chars");
        let view = crate::select::build_view(&p);
        let expected = crate::subhunk_id::subhunk_id(&p.files[0], &view[0].1[0]);
        assert_eq!(id, expected, "json id must match the canonical sub-hunk id");
    }

    #[test]
    fn human_shows_subhunk_id() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let out = list_human(&p, false);
        let view = crate::select::build_view(&p);
        let id = crate::subhunk_id::subhunk_id(&p.files[0], &view[0].1[0]);
        assert!(
            out.contains(&id),
            "human output must contain id {id}:\n{out}"
        );
    }

    #[test]
    fn human_lists_indices() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let out = list_human(&p, false);
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
        let v: serde_json::Value = serde_json::from_str(&list_json(&p)).unwrap();
        assert_eq!(v[0]["hunks"][0]["addition_only"], true);
    }

    #[test]
    fn json_addition_only_false_for_mixed() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&list_json(&p)).unwrap();
        assert_eq!(v[0]["hunks"][0]["addition_only"], false);
    }

    #[test]
    fn human_marks_addition_only() {
        let p = parse(NEW_FILE.as_bytes()).unwrap();
        let out = list_human(&p, false);
        assert!(
            out.contains("[+range]"),
            "addition-only marker missing:\n{out}"
        );
    }

    #[test]
    fn human_no_marker_for_mixed() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let out = list_human(&p, false);
        assert!(
            !out.contains("[+range]"),
            "addition-only marker must not appear for a mixed sub-hunk:\n{out}"
        );
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
        let v: serde_json::Value = serde_json::from_str(&list_json(&p)).unwrap();
        assert_eq!(v[0]["hunks"][0]["addition_only"], false);
        assert!(!list_human(&p, false).contains("[+range]"));
    }
}
