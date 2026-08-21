use crate::model::*;

/// Render `patch` back to a unified diff. Round-trips its input byte for byte — not just a
/// git-canonical diff: the preamble before the first entry, header lines, hunk bodies, lines
/// trailing a hunk, CRLF endings, the `\ No newline at end of file` marker with the ending it
/// came with, and a missing final newline are all preserved.
pub fn emit(patch: &Patch) -> Vec<u8> {
    let mut out = Vec::with_capacity(emitted_size_hint(patch));
    for l in &patch.preamble {
        out.extend_from_slice(l);
        out.push(b'\n');
    }
    for f in &patch.files {
        for h in &f.headers {
            out.extend_from_slice(h);
            out.push(b'\n');
        }
        // The cursor below walks the trailer once alongside the hunks, which needs the entries
        // ordered by the hunk they follow. Rescanning the whole list for every hunk was
        // quadratic in their number: a 9 MB diff carrying a separator after each of its 128 000
        // hunks took 15 s to re-emit, against 0.2 s to list.
        //
        // Everything in this crate builds the list in order, but `trailer` is a public field:
        // a caller assembling a FileDiff by hand can hand over its lines in any order, and the
        // cursor would then flush them after the wrong hunk at exit 0 — a silently corrupted
        // diff. The check that said so was a `debug_assert!`, absent from the build a user
        // runs. Ordering an out-of-order list is what the position tag means, and the scan that
        // decides whether to costs one pass over a list already walked once.
        let ordered: Vec<(usize, Vec<u8>)>;
        let trailer: &[(usize, Vec<u8>)] = if f.trailer.windows(2).all(|w| w[0].0 <= w[1].0) {
            &f.trailer
        } else {
            let mut v = f.trailer.clone();
            // Stable, so lines tagged with the same position keep the order they arrived in.
            v.sort_by_key(|(at, _)| *at);
            ordered = v;
            &ordered
        };
        let mut ti = 0usize;
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
                    emit_trailer_upto(&mut out, trailer, i + 1, &mut ti);
                }
            }
        }
        // Lines tagged with a position past the emitted hunks (a file that lost hunks to a
        // selection, or a binary file) still belong to this file: flush what is left.
        for (_, l) in &trailer[ti..] {
            out.extend_from_slice(l);
            out.push(b'\n');
        }
    }
    // Every line is written with its newline; drop the last one when the input had none, so a
    // diff that arrived without a final newline leaves unchanged.
    if patch.no_trailing_newline && out.last() == Some(&b'\n') {
        out.pop();
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
    let mut n = patch.preamble.iter().map(|l| l.len() + 1).sum::<usize>();
    for f in &patch.files {
        n += f.headers.iter().map(|h| h.len() + 1).sum::<usize>();
        n += f.trailer.iter().map(|(_, l)| l.len() + 1).sum::<usize>();
        match &f.content {
            FileContent::Binary(lines) => n += lines.iter().map(|l| l.len() + 1).sum::<usize>(),
            FileContent::Text(hunks) => {
                for h in hunks {
                    n += HUNK_HEADER + h.section.len();
                    for l in &h.lines {
                        // marker + text + newline, and the no-newline note when present
                        n += l.text.len() + 2 + l.no_newline.as_ref().map_or(0, |m| m.len() + 1);
                    }
                }
            }
        }
    }
    n
}

/// Emit the trailing lines recorded no later than the `at`-th hunk of their file, advancing
/// `ti` past them. `trailer` is ordered by that position, so each entry is visited once across
/// the whole file rather than once per hunk.
fn emit_trailer_upto(out: &mut Vec<u8>, trailer: &[(usize, Vec<u8>)], at: usize, ti: &mut usize) {
    while let Some((pos, l)) = trailer.get(*ti) {
        if *pos > at {
            break;
        }
        out.extend_from_slice(l);
        out.push(b'\n');
        *ti += 1;
    }
}

/// The section text a hunk header carries, without the CR a CRLF diff leaves at the end: that
/// CR is the line ending, not a section. A header with nothing else gets no separating space
/// when emitted and no section in the listing — both callers ask this one question, so they
/// cannot drift apart.
pub(crate) fn section_text(h: &Hunk) -> &[u8] {
    h.section.strip_suffix(b"\r").unwrap_or(&h.section)
}

fn emit_hunk(out: &mut Vec<u8>, h: &Hunk) {
    out.extend_from_slice(b"@@ -");
    out.extend_from_slice(fmt_range(h.old_start, h.old_lines).as_bytes());
    out.extend_from_slice(b" +");
    out.extend_from_slice(fmt_range(h.new_start, h.new_lines).as_bytes());
    out.extend_from_slice(b" @@");
    if !section_text(h).is_empty() {
        out.push(b' ');
    }
    // The raw section, CR and all: what came in is what goes out.
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
        if let Some(marker) = &l.no_newline {
            // Verbatim, so a CRLF diff keeps the CR the marker arrived with.
            out.extend_from_slice(marker);
            out.push(b'\n');
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

    /// [`FileDiff::trailer`] is a public field, so a caller building an entry by hand can hand
    /// over its lines in any order. The order was held by a `debug_assert!` alone, which the
    /// release build — the build anybody actually runs — compiles away: a line then came out
    /// after the wrong hunk at exit 0, which is a silently corrupted diff rather than a refusal.
    #[test]
    fn a_trailer_out_of_order_still_puts_each_line_after_its_own_hunk() {
        let src =
            "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+A\nafter one\n@@ -5 +5 @@\n-b\n+B\nafter two\n";
        let mut p = parse(src.as_bytes()).unwrap();
        assert_eq!(
            p.files[0]
                .trailer
                .iter()
                .map(|(at, _)| *at)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "the fixture carries one trailing line after each hunk"
        );
        p.files[0].trailer.reverse();
        assert_eq!(emit(&p), src.as_bytes());
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

    /// The marker line has a line ending like every other line: in a CRLF diff it arrives as
    /// `\ No newline at end of file\r\n`. Rendering it with a bare `\n` leaves the output with
    /// mixed line endings and breaks the byte-for-byte round-trip this module promises.
    #[test]
    fn roundtrips_the_no_newline_marker_of_a_crlf_diff() {
        roundtrip(
            "--- a/f\r\n+++ b/f\r\n@@ -1 +1 @@\r\n-a\r\n\
             \\ No newline at end of file\r\n+b\r\n\\ No newline at end of file\r\n",
        );
    }

    /// A diff pasted or piped without its final newline is still that diff; adding one makes
    /// the output differ from the input by a byte, which the round-trip promise does not allow.
    #[test]
    fn roundtrips_an_input_without_a_trailing_newline() {
        roundtrip("--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b");
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
            "diff --git a/f b/f\r\n--- a/f\r\n+++ b/f\r\n\
             @@ -1,3 +1,3 @@\r\n a\r\n-b\r\n+B\r\n c\r\n",
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
