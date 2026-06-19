use crate::model::*;

pub fn emit(patch: &Patch) -> Vec<u8> {
    let mut out = Vec::new();
    for f in &patch.files {
        for h in &f.headers {
            out.extend_from_slice(h);
            out.push(b'\n');
        }
        match &f.content {
            FileContent::Binary(lines) => {
                for l in lines {
                    out.extend_from_slice(l);
                    out.push(b'\n');
                }
            }
            FileContent::Text(hunks) => {
                for h in hunks {
                    emit_hunk(&mut out, h);
                }
            }
        }
    }
    out
}

fn emit_hunk(out: &mut Vec<u8>, h: &Hunk) {
    out.extend_from_slice(b"@@ -");
    out.extend_from_slice(fmt_range(h.old_start, h.old_lines).as_bytes());
    out.extend_from_slice(b" +");
    out.extend_from_slice(fmt_range(h.new_start, h.new_lines).as_bytes());
    out.extend_from_slice(b" @@");
    if !h.section.is_empty() {
        out.push(b' ');
        out.extend_from_slice(&h.section);
    }
    out.push(b'\n');
    for l in &h.lines {
        out.push(match l.kind {
            LineKind::Context => b' ',
            LineKind::Add => b'+',
            LineKind::Del => b'-',
        });
        out.extend_from_slice(&l.text);
        out.push(b'\n');
        if l.no_newline {
            out.extend_from_slice(b"\\ No newline at end of file\n");
        }
    }
}

/// Git omits the ",1" suffix for single-line ranges; we match that so round-trip
/// of git-canonical diffs is byte-identical.
pub(crate) fn fmt_range(start: u32, count: u32) -> String {
    if count == 1 {
        start.to_string()
    } else {
        format!("{start},{count}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    fn roundtrip(src: &str) {
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(emit(&p), src.as_bytes(), "round-trip mismatch");
    }

    #[test]
    fn roundtrips_git_diff() {
        roundtrip(
            "\
diff --git a/f.txt b/f.txt
index 111..222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,3 +1,3 @@ ctx
 a
-b
+B
 c
",
        );
    }

    #[test]
    fn roundtrips_no_newline_and_binary() {
        roundtrip(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1 +1 @@
-old
\\ No newline at end of file
+new
\\ No newline at end of file
",
        );
        roundtrip(
            "\
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
",
        );
    }

    #[test]
    fn roundtrips_multi_file_and_multi_hunk() {
        roundtrip(
            "\
diff --git a/x b/x
--- a/x
+++ b/x
@@ -1,2 +1,2 @@
 a
-b
+B
@@ -10,2 +10,3 @@
 p
+q
 r
diff --git a/y b/y
--- a/y
+++ b/y
@@ -1 +1 @@
-3
+4
",
        );
    }
}
