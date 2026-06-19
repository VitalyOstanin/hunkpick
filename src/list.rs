use crate::model::*;
use crate::select::build_view;
use serde::Serialize;

#[derive(Serialize)]
struct JsonHunk {
    index: usize,
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
    added: u32,
    deleted: u32,
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
    let view = build_view(patch);
    let mut files = Vec::new();
    for (fi, subs) in &view {
        let f = &patch.files[*fi];
        let binary = matches!(f.content, FileContent::Binary(_));
        let hunks = subs
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let (added, deleted) = h.change_counts();
                JsonHunk {
                    index: i + 1,
                    old_start: h.old_start,
                    old_lines: h.old_lines,
                    new_start: h.new_start,
                    new_lines: h.new_lines,
                    added,
                    deleted,
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
            let idx = paint(&format!("[{}]", i + 1), "1", color);
            let pv = preview(h);
            let pv = if pv.starts_with('+') {
                paint(&pv, "32", color)
            } else if pv.starts_with('-') {
                paint(&pv, "31", color)
            } else {
                pv
            };
            out.push_str(&format!(
                "  {idx} {}  +{added} -{deleted}  {pv}\n",
                header_string(h)
            ));
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
    fn human_lists_indices() {
        let p = parse(MULTI.as_bytes()).unwrap();
        let out = list_human(&p, false);
        assert!(out.contains("f"));
        assert!(out.contains("[1]"));
        assert!(out.contains("[2]"));
    }
}
