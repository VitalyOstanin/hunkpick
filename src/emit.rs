use crate::model::*;

/// Render `patch` back to a unified diff. Round-trips a git-canonical diff byte for byte:
/// header lines, hunk bodies, trailing lines after a hunk and CRLF endings are all preserved.
pub fn emit(patch: &Patch) -> Vec<u8> {
    let mut out = Vec::with_capacity(emitted_size_hint(patch));
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
                for (i, h) in hunks.iter().enumerate() {
                    emit_hunk(&mut out, h);
                    emit_trailer(&mut out, f, i + 1);
                }
            }
        }
        // Lines tagged with a position past the emitted hunks (a file that lost hunks to a
        // selection, or a binary file) still belong to this file: flush what is left.
        let emitted = match &f.content {
            FileContent::Text(h) => h.len(),
            FileContent::Binary(_) => 0,
        };
        for (at, l) in &f.trailer {
            if *at > emitted {
                out.extend_from_slice(l);
                out.push(b'\n');
            }
        }
    }
    out
}

/// Roughly how many bytes [`emit`] will produce: every line it writes, plus its newline and
/// its one-byte marker where there is one. The `@@` headers are the only part not measured
/// exactly (their line numbers are counted as a fixed allowance), so the result is a hint —
/// close enough to spare the output buffer a series of reallocations on a large diff.
fn emitted_size_hint(patch: &Patch) -> usize {
    /// Allowance per `@@ -X,Y +Z,W @@` header: the punctuation plus room for four numbers.
    const HUNK_HEADER: usize = 40;
    /// `\ No newline at end of file` plus its newline.
    const NO_NEWLINE: usize = 28;

    let mut n = 0;
    for f in &patch.files {
        n += f.headers.iter().map(|h| h.len() + 1).sum::<usize>();
        n += f.trailer.iter().map(|(_, l)| l.len() + 1).sum::<usize>();
        match &f.content {
            FileContent::Binary(lines) => n += lines.iter().map(|l| l.len() + 1).sum::<usize>(),
            FileContent::Text(hunks) => {
                for h in hunks {
                    n += HUNK_HEADER + h.section.len();
                    for l in &h.lines {
                        // marker + text + newline, and the no-newline note when flagged
                        n += l.text.len() + 2 + if l.no_newline { NO_NEWLINE } else { 0 };
                    }
                }
            }
        }
    }
    n
}

/// Emit the trailing lines recorded right after the `at`-th hunk of `f`.
fn emit_trailer(out: &mut Vec<u8>, f: &FileDiff, at: usize) {
    for (pos, l) in &f.trailer {
        if *pos == at {
            out.extend_from_slice(l);
            out.push(b'\n');
        }
    }
}

fn emit_hunk(out: &mut Vec<u8>, h: &Hunk) {
    out.extend_from_slice(b"@@ -");
    out.extend_from_slice(fmt_range(h.old_start, h.old_lines).as_bytes());
    out.extend_from_slice(b" +");
    out.extend_from_slice(fmt_range(h.new_start, h.new_lines).as_bytes());
    out.extend_from_slice(b" @@");
    // The section text is separated by a space, but a CRLF diff leaves a bare CR here: that
    // is the line ending, not a section, and prefixing it with a space would alter the header.
    if !h
        .section
        .strip_suffix(b"\r")
        .unwrap_or(&h.section)
        .is_empty()
    {
        out.push(b' ');
    }
    out.extend_from_slice(&h.section);
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
    fn roundtrips_crlf_diff_byte_for_byte() {
        // The CR belongs to the line ending of the hunk header, not to its section text.
        roundtrip(
            "diff --git a/f b/f\r\n--- a/f\r\n+++ b/f\r\n@@ -1,3 +1,3 @@\r\n a\r\n-b\r\n+B\r\n c\r\n",
        );
        // A header that does carry section text keeps the separating space.
        roundtrip("--- a/f\r\n+++ b/f\r\n@@ -1,2 +1,2 @@ fn one()\r\n x\r\n-y\r\n+Y\r\n");
    }

    #[test]
    fn lines_after_the_last_hunk_stay_after_it() {
        // A blank separator between hunks (common in pasted or mail-transported diffs) is
        // not part of any hunk body. It must keep its place: emitting it with the leading
        // headers would move it above the first `@@`, and git rejects the result as garbage.
        roundtrip(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,2 @@
 a
-b
+B

@@ -10,2 +10,2 @@
 p
-q
+Q
",
        );
    }

    #[test]
    fn empty_context_line_is_emitted_with_its_marker() {
        // Input whose context line lost its trailing space in transport: the emitted diff
        // restores the marker, so the result is what `git apply` expects.
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B

";
        // Spelled with concat! so the single-space context line stays visible in the source.
        let expected = concat!(
            "diff --git a/f b/f\n",
            "--- a/f\n",
            "+++ b/f\n",
            "@@ -1,3 +1,3 @@\n",
            " a\n",
            "-b\n",
            "+B\n",
            " \n",
        );
        let p = parse(src.as_bytes()).unwrap();
        assert_eq!(String::from_utf8(emit(&p)).unwrap(), expected);
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
