use crate::model::*;
use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum SplitError {
    NotAContextLine(u32),
    OutOfRange(u32),
    /// An `INDEX@L<set>` selection references a changed line outside `1..=changed` of the
    /// sub-hunk. Carries the offending 1-based index and the sub-hunk's changed-line count.
    ChangedLineOutOfRange {
        index: usize,
        changed: usize,
    },
    /// An `INDEX@L<set>` selection resolved to no changed lines (an empty set). A defensive
    /// invariant: from the CLI an empty `@L` set is already rejected earlier by the selector
    /// parser (`empty index set`), so this is only reachable by a direct library caller.
    NoChangedLinesSelected,
}

impl fmt::Display for SplitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SplitError::NotAContextLine(n) => {
                write!(f, "new-file line {n} is a change line, not a context line")
            }
            SplitError::OutOfRange(n) => write!(f, "new-file line {n} is out of range"),
            SplitError::ChangedLineOutOfRange { index, changed } => write!(
                f,
                "changed-line index {index} is out of range (sub-hunk has {changed} changed line(s))"
            ),
            SplitError::NoChangedLinesSelected => {
                write!(f, "the selection references no changed lines")
            }
        }
    }
}

/// Auto-split a hunk into minimal sub-hunks at context gaps between change runs.
/// Returns the hunk unchanged (as a single element) if it has zero or one change run.
///
/// Each sub-hunk carries its surrounding context: the first sub-hunk gets all context
/// up to (and including) the inter-run boundary; subsequent sub-hunks start directly at
/// their change run (no leading shared boundary context). This produces non-overlapping
/// old-file ranges so the sub-hunks can be emitted together and applied with `git apply`.
pub fn auto_split_hunk(h: &Hunk) -> Vec<Hunk> {
    let runs = change_runs(h);
    if runs.len() <= 1 {
        return vec![h.clone()];
    }
    let n = h.lines.len();
    let mut result = Vec::with_capacity(runs.len());
    for ri in 0..runs.len() {
        // First sub-hunk starts at the beginning of the whole hunk.
        // Subsequent sub-hunks start directly at their change run (no shared boundary
        // context with the preceding sub-hunk) so that old-file ranges do not overlap.
        let lead_from = if ri == 0 { 0 } else { runs[ri].0 };
        // Trailing context: everything up to (but not including) the next change run,
        // or the end of the hunk for the last sub-hunk.
        let trail_to = if ri + 1 == runs.len() {
            n
        } else {
            runs[ri + 1].0
        };
        let slice = &h.lines[lead_from..trail_to];
        result.push(rebuild_subhunk(h, slice, lead_from));
    }
    result
}

/// Explicitly split a hunk at the given new-file line numbers (context lines only).
pub fn split_hunk_at(h: &Hunk, new_line_cuts: &[u32]) -> Result<Vec<Hunk>, SplitError> {
    let mut new_no = h.new_start;
    let mut cut_indices: Vec<usize> = Vec::new();
    let mut wanted: BTreeSet<u32> = new_line_cuts.iter().copied().collect();
    for (i, l) in h.lines.iter().enumerate() {
        let here = match l.kind {
            LineKind::Context | LineKind::Add => {
                let n = new_no;
                new_no += 1;
                n
            }
            LineKind::Del => continue,
        };
        if wanted.remove(&here) {
            if !matches!(l.kind, LineKind::Context) {
                return Err(SplitError::NotAContextLine(here));
            }
            cut_indices.push(i);
        }
    }
    if let Some(&missing) = wanted.iter().next() {
        return Err(SplitError::OutOfRange(missing));
    }
    if cut_indices.is_empty() {
        return Ok(vec![h.clone()]);
    }
    // Build slice boundaries: each piece ends just after its cut line (cut line becomes
    // trailing context of the piece), the next piece starts just after the cut line.
    // This produces non-overlapping old-file ranges while keeping trailing context in
    // each piece so that `git apply` can locate the patch position.
    let n = h.lines.len();
    let mut starts = vec![0usize];
    let mut ends: Vec<usize> = cut_indices.iter().map(|&ci| ci + 1).collect();
    ends.push(n);
    starts.extend(cut_indices.iter().map(|&ci| ci + 1));
    Ok(rebuild_pieces(h, &starts, &ends))
}

/// Emit a piece of `h` that realises only the selected changed lines. `selected` holds 1-based
/// indices over `h`'s changed (Add/Del) lines in body order (`1..=changed`, where
/// `changed == added + deleted`). Each body line is rewritten:
///   - a context line is kept as context;
///   - a selected deletion stays a deletion; an unselected deletion becomes a context line (the
///     line is retained in this partial application, and it anchors the resulting hunk);
///   - a selected addition stays an addition; an unselected addition is omitted (not yet added).
///
/// Because unselected deletions are kept as context, every subset of changed lines is realisable
/// as one applicable hunk — there is no boundary restriction, and a deletion split by additions
/// (`+x -y +z`) can be addressed, keeping both leading and trailing context so the piece
/// anchors under `git apply`. The
/// old-side footprint (`old_start`, `old_lines`) is invariant: every original context and
/// deletion line is still present on the old side (a deletion either stays a deletion or becomes
/// context, both counting toward `old_lines`). Errors if `selected` is empty or references a
/// changed line outside `1..=changed`.
pub fn slice_changed_lines(h: &Hunk, selected: &BTreeSet<usize>) -> Result<Hunk, SplitError> {
    let changed = h.changed_lines().count();
    if selected.is_empty() {
        return Err(SplitError::NoChangedLinesSelected);
    }
    // `selected` is sorted (BTreeSet); parsing guarantees every index is >= 1, so only the upper
    // bound can be out of range.
    if let Some(&max) = selected.iter().next_back() {
        if max > changed {
            return Err(SplitError::ChangedLineOutOfRange {
                index: max,
                changed,
            });
        }
    }
    let mut lines: Vec<Line> = Vec::with_capacity(h.lines.len());
    let mut ci = 0usize; // 1-based changed-line counter
    for l in &h.lines {
        match l.kind {
            LineKind::Context => lines.push(l.clone()),
            LineKind::Del => {
                ci += 1;
                if selected.contains(&ci) {
                    lines.push(l.clone());
                } else {
                    // Retained line: emit as context so the hunk stays anchored.
                    lines.push(Line {
                        kind: LineKind::Context,
                        text: l.text.clone(),
                        no_newline: l.no_newline,
                    });
                }
            }
            LineKind::Add => {
                ci += 1;
                if selected.contains(&ci) {
                    lines.push(l.clone());
                }
                // Unselected addition: omit entirely.
            }
        }
    }
    // A retained (unselected) deletion that sat at EOF became a context line still carrying the
    // `\ No newline at end of file` flag. If it is no longer the last line — selected additions
    // follow it — a plain context line is both malformed (a mid-hunk no-newline marker) and wrong:
    // appending after a no-newline line requires that line to gain a trailing newline. Represent
    // that the way git does — delete the no-newline line and re-add it with a newline — so the
    // piece applies. A well-formed diff never carries a no-newline flag on a non-last context
    // line, so this only ever touches lines this transform just converted from a deletion;
    // deletions and additions keep their own flags (a valid `-a\No newline +b` at EOF round-trips).
    let n = lines.len();
    if lines
        .iter()
        .enumerate()
        .any(|(i, l)| i + 1 < n && matches!(l.kind, LineKind::Context) && l.no_newline)
    {
        let mut fixed: Vec<Line> = Vec::with_capacity(lines.len() + 1);
        for (i, l) in lines.into_iter().enumerate() {
            if i + 1 < n && matches!(l.kind, LineKind::Context) && l.no_newline {
                fixed.push(Line {
                    kind: LineKind::Del,
                    text: l.text.clone(),
                    no_newline: true,
                });
                fixed.push(Line {
                    kind: LineKind::Add,
                    text: l.text,
                    no_newline: false,
                });
            } else {
                fixed.push(l);
            }
        }
        lines = fixed;
    }
    let (ctx, add, del) = count_kinds(&lines);
    Ok(Hunk {
        old_start: h.old_start,
        old_lines: ctx + del,
        new_start: h.new_start,
        new_lines: ctx + add,
        section: h.section.clone(),
        lines,
    })
}

/// Indices of maximal Add/Del runs as (start, end_exclusive).
fn change_runs(h: &Hunk) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < h.lines.len() {
        if matches!(h.lines[i].kind, LineKind::Add | LineKind::Del) {
            let start = i;
            while i < h.lines.len() && matches!(h.lines[i].kind, LineKind::Add | LineKind::Del) {
                i += 1;
            }
            runs.push((start, i));
        } else {
            i += 1;
        }
    }
    runs
}

/// Build non-overlapping sub-hunks from separate start/end index arrays.
///
/// Each piece `i` covers `h.lines[starts[i]..ends[i]]`. The cut (context) line becomes
/// the last line (trailing context) of the preceding piece; the next piece starts just
/// after it. This guarantees old-file ranges do not overlap while preserving at least one
/// context line in each piece so that `git apply` can locate the hunk without `--unidiff-zero`.
fn rebuild_pieces(h: &Hunk, starts: &[usize], ends: &[usize]) -> Vec<Hunk> {
    assert_eq!(starts.len(), ends.len());
    let mut result = Vec::new();
    for (start, end) in starts.iter().zip(ends.iter()) {
        // Skip empty pieces. A cut on the hunk's last line yields a trailing `start == end`
        // slice that would emit a degenerate `@@ -X,0 +Y,0 @@` stanza git rejects.
        if start >= end {
            continue;
        }
        let slice = &h.lines[*start..*end];
        // Skip context-only pieces. A cut on the hunk's first context line yields a
        // leading slice with no Add/Del lines, which produces a hunk with zero net
        // change that `git apply` rejects. Such a piece carries no change to stage.
        let (_, add, del) = count_kinds(slice);
        if add + del == 0 {
            continue;
        }
        result.push(rebuild_subhunk(h, slice, *start));
    }
    result
}

/// Build a Hunk from a slice of `h.lines` starting at absolute index `abs_start`,
/// recomputing old/new start offsets and line counts.
fn rebuild_subhunk(h: &Hunk, slice: &[Line], abs_start: usize) -> Hunk {
    // Old/new offsets of the slice are the line counts of everything before it; the slice's
    // own old/new lengths are its counts. Context lines advance both sides.
    let (pre_ctx, pre_add, pre_del) = count_kinds(&h.lines[..abs_start]);
    let old_off = pre_ctx + pre_del;
    let new_off = pre_ctx + pre_add;
    let (ctx, add, del) = count_kinds(slice);
    let old_lines = ctx + del;
    let new_lines = ctx + add;
    Hunk {
        old_start: h.old_start + old_off,
        old_lines,
        new_start: h.new_start + new_off,
        new_lines,
        section: if abs_start == 0 {
            h.section.clone()
        } else {
            Vec::new()
        },
        lines: slice.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FileContent;
    use crate::parser::parse;

    fn hunk(src: &str) -> Hunk {
        let p = parse(src.as_bytes()).unwrap();
        let FileContent::Text(h) = &p.files[0].content else {
            panic!()
        };
        h[0].clone()
    }

    /// Reassemble a one-file patch from sub-hunks, as raw bytes, for `git apply --check`.
    fn assemble(subs: &[Hunk]) -> Vec<u8> {
        let mut diff = b"--- a/f\n+++ b/f\n".to_vec();
        for s in subs {
            diff.extend_from_slice(
                format!(
                    "@@ -{},{} +{},{} @@\n",
                    s.old_start, s.old_lines, s.new_start, s.new_lines
                )
                .as_bytes(),
            );
            for l in &s.lines {
                diff.push(match l.kind {
                    LineKind::Context => b' ',
                    LineKind::Add => b'+',
                    LineKind::Del => b'-',
                });
                diff.extend_from_slice(&l.text);
                diff.push(b'\n');
                if l.no_newline {
                    diff.extend_from_slice(b"\\ No newline at end of file\n");
                }
            }
        }
        diff
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

    #[test]
    fn splits_two_changes_separated_by_context() {
        let h = hunk(TWO_CHANGES);
        let subs = auto_split_hunk(&h);
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].old_start, 1);
        assert_eq!(
            subs[0]
                .lines
                .iter()
                .filter(|l| l.kind == LineKind::Add)
                .count(),
            1
        );
        // Each sub-hunk's header counts must match its body.
        for s in &subs {
            let ctx = s
                .lines
                .iter()
                .filter(|l| l.kind == LineKind::Context)
                .count() as u32;
            let del = s.lines.iter().filter(|l| l.kind == LineKind::Del).count() as u32;
            let add = s.lines.iter().filter(|l| l.kind == LineKind::Add).count() as u32;
            assert_eq!(s.old_lines, ctx + del);
            assert_eq!(s.new_lines, ctx + add);
        }
    }

    #[test]
    fn single_change_returns_one() {
        let h = hunk(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
",
        );
        assert_eq!(auto_split_hunk(&h).len(), 1);
    }

    #[test]
    fn explicit_split_on_context_line() {
        let h = hunk(TWO_CHANGES);
        // New-file line numbers: a=1 B=2 c=3 D=4 e=5. Cut at context line 3 (c).
        let subs = split_hunk_at(&h, &[3]).unwrap();
        assert_eq!(subs.len(), 2);
        // The cut line c becomes trailing context of piece 0; piece 1 starts after it at
        // new-file line 4 (the -d/+D change). This keeps old-file ranges non-overlapping.
        assert_eq!(subs[1].new_start, 4);
        for s in &subs {
            let ctx = s
                .lines
                .iter()
                .filter(|l| l.kind == LineKind::Context)
                .count() as u32;
            let del = s.lines.iter().filter(|l| l.kind == LineKind::Del).count() as u32;
            let add = s.lines.iter().filter(|l| l.kind == LineKind::Add).count() as u32;
            assert_eq!(s.old_lines, ctx + del);
            assert_eq!(s.new_lines, ctx + add);
        }
    }

    #[test]
    fn explicit_split_on_first_context_line_drops_context_only_piece() {
        let h = hunk(TWO_CHANGES);
        // New-file line numbers: a=1 B=2 c=3 D=4 e=5. Cut at the first context line a=1.
        // The leading piece would be context-only (just `a`); it carries no change and
        // must be dropped rather than emitted as a degenerate zero-change hunk.
        let subs = split_hunk_at(&h, &[1]).unwrap();
        for s in &subs {
            let add = s.lines.iter().filter(|l| l.kind == LineKind::Add).count();
            let del = s.lines.iter().filter(|l| l.kind == LineKind::Del).count();
            assert!(add + del > 0, "no context-only sub-hunk emitted");
            assert!(s.old_lines > 0 && s.new_lines > 0);
        }
    }

    #[test]
    fn explicit_split_rejects_change_line() {
        let h = hunk(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
",
        );
        // new-file line 2 is "B" (an Add) -> not a context line.
        assert_eq!(split_hunk_at(&h, &[2]), Err(SplitError::NotAContextLine(2)));
    }

    #[test]
    fn explicit_split_out_of_range() {
        let h = hunk(TWO_CHANGES);
        assert_eq!(split_hunk_at(&h, &[99]), Err(SplitError::OutOfRange(99)));
    }

    #[test]
    fn explicit_split_on_last_context_line_drops_empty_piece() {
        // Cutting at the hunk's final context line leaves nothing after the cut, so the
        // trailing piece would be an empty `@@ -X,0 +Y,0 @@` stanza that `git apply`
        // rejects as a corrupt patch. The split must drop such degenerate pieces.
        let h = hunk(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,3 +1,3 @@
 a
-b
+B
 c
",
        );
        // New-file line numbers: a=1 B=2 c=3. Cut at context line 3 (c), the last line.
        let subs = split_hunk_at(&h, &[3]).unwrap();
        for s in &subs {
            assert!(!s.lines.is_empty(), "empty sub-hunk piece produced: {s:?}");
            assert!(
                s.old_lines > 0 || s.new_lines > 0,
                "degenerate zero-count sub-hunk produced: {s:?}"
            );
        }
        // The only meaningful piece is the change itself; the empty trailing piece is gone.
        assert_eq!(subs.len(), 1);
    }

    #[test]
    fn explicit_split_combined_applies_via_git() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let h = hunk(TWO_CHANGES);
        let pieces = split_hunk_at(&h, &[3]).unwrap();
        let diff = assemble(&pieces);

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
            "git apply --check failed for combined explicit-split patch:\n{}",
            String::from_utf8_lossy(&diff)
        );
    }

    #[test]
    fn auto_split_subhunks_apply_via_git() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let h = hunk(TWO_CHANGES);
        let subs = auto_split_hunk(&h);
        // Reassemble a patch with all sub-hunks for file "f".
        let diff = assemble(&subs);

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
            "git apply --check failed for split patch:\n{}",
            String::from_utf8_lossy(&diff)
        );
    }

    /// True if the assembled patch of `subs` applies cleanly to a file `f` seeded with
    /// `file_content` in a fresh git repo (`git apply --check`).
    fn git_apply_ok(subs: &[Hunk], file_content: &str) -> bool {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let diff = assemble(subs);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), file_content).unwrap();
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
        child.wait().unwrap().success()
    }

    /// Build a BTreeSet of the given 1-based changed-line indices.
    fn sel(indices: &[usize]) -> BTreeSet<usize> {
        indices.iter().copied().collect()
    }

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
    fn slice_changed_separates_deletions() {
        // Select both deletions (changed lines 1,2): a pure deletion piece.
        let h = hunk(REPLACEMENT);
        let p = slice_changed_lines(&h, &sel(&[1, 2])).unwrap();
        assert_eq!(p.old_lines, 2);
        assert_eq!(p.new_lines, 0);
        assert!(p.lines.iter().all(|l| l.kind == LineKind::Del));
    }

    #[test]
    fn slice_changed_separates_additions() {
        // Select both additions (changed lines 3,4): the deletions become context so the piece
        // keeps an anchor; it is not a zero-context hunk.
        let h = hunk(REPLACEMENT);
        let p = slice_changed_lines(&h, &sel(&[3, 4])).unwrap();
        assert_eq!(p.old_lines, 2); // two context lines (the retained deletions)
        assert_eq!(p.new_lines, 4); // two context + two additions
        assert_eq!(
            p.lines.iter().filter(|l| l.kind == LineKind::Del).count(),
            0
        );
        assert_eq!(
            p.lines
                .iter()
                .filter(|l| l.kind == LineKind::Context)
                .count(),
            2
        );
        assert_eq!(
            p.lines.iter().filter(|l| l.kind == LineKind::Add).count(),
            2
        );
    }

    #[test]
    fn slice_changed_del_and_add_pieces_apply_independently_via_git() {
        // The key agent operation: stage a replacement's removals separately from its insertions.
        // Both pieces must apply to the original file on their own.
        let h = hunk(REPLACEMENT);
        let dels = slice_changed_lines(&h, &sel(&[1, 2])).unwrap();
        let adds = slice_changed_lines(&h, &sel(&[3, 4])).unwrap();
        assert!(git_apply_ok(&[dels], "a\nb\n"), "deletion piece must apply");
        assert!(git_apply_ok(&[adds], "a\nb\n"), "addition piece must apply");
    }

    const ADD_SPLIT_BY_DEL: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,1 +1,2 @@
+x
-y
+z
";

    #[test]
    fn slice_changed_addresses_deletion_split_by_additions() {
        // `+x -y +z`: the deletion (changed line 2) is surrounded by additions.
        // `slice_changed_lines` isolates it: select just the deletion.
        let h = hunk(ADD_SPLIT_BY_DEL);
        let p = slice_changed_lines(&h, &sel(&[2])).unwrap();
        assert_eq!(p.old_lines, 1);
        assert_eq!(p.new_lines, 0);
        assert_eq!(p.lines.len(), 1);
        assert_eq!(p.lines[0].kind, LineKind::Del);
        assert_eq!(p.lines[0].text, b"y");
        assert!(git_apply_ok(&[p], "y\n"), "isolated deletion must apply");
    }

    #[test]
    fn slice_changed_selecting_additions_around_deletion_keeps_it_as_context() {
        // Select the two additions of `+x -y +z`; the deletion becomes context and anchors it.
        let h = hunk(ADD_SPLIT_BY_DEL);
        let p = slice_changed_lines(&h, &sel(&[1, 3])).unwrap();
        assert_eq!(p.old_lines, 1); // the retained deletion, now context
        assert_eq!(p.new_lines, 3); // context + two additions
        assert!(git_apply_ok(&[p], "y\n"), "addition piece must apply");
    }

    #[test]
    fn slice_changed_roundtrip_full_selection_reproduces_body() {
        // Selecting every changed line reproduces the original sub-hunk body.
        let h = hunk(REPLACEMENT);
        let all = slice_changed_lines(&h, &sel(&[1, 2, 3, 4])).unwrap();
        assert_eq!(all.lines, h.lines);
    }

    #[test]
    fn slice_changed_readds_no_newline_line_when_additions_follow() {
        // Old file `a` has no trailing newline; it is replaced by `b\nc\n`. Selecting only the
        // additions retains `-a` (no_newline). Because content now follows it, `a` must gain a
        // trailing newline, so the piece deletes the no-newline `a` and re-adds it with a newline
        // (git's representation) rather than emitting a malformed mid-hunk no-newline context.
        let h = hunk(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1 +1,2 @@
-a
\\ No newline at end of file
+b
+c
",
        );
        // Changed lines: 1=`-a`(no_newline), 2=`+b`, 3=`+c`. Select the two additions.
        let p = slice_changed_lines(&h, &sel(&[2, 3])).unwrap();
        assert_eq!(p.lines[0].kind, LineKind::Del);
        assert_eq!(p.lines[0].text, b"a");
        assert!(
            p.lines[0].no_newline,
            "the deleted `a` keeps the no-newline flag"
        );
        assert_eq!(p.lines[1].kind, LineKind::Add);
        assert_eq!(p.lines[1].text, b"a");
        assert!(
            !p.lines[1].no_newline,
            "the re-added `a` gains a trailing newline"
        );
        // old side: one deletion; new side: re-added a + b + c.
        assert_eq!(p.old_lines, 1);
        assert_eq!(p.new_lines, 3);
        assert!(
            git_apply_ok(&[p], "a"),
            "addition piece must apply to `a` (no trailing newline)"
        );
    }

    #[test]
    fn slice_changed_out_of_range_and_empty_error() {
        let h = hunk(REPLACEMENT); // 4 changed lines
        assert!(matches!(
            slice_changed_lines(&h, &sel(&[5])),
            Err(SplitError::ChangedLineOutOfRange {
                index: 5,
                changed: 4
            })
        ));
        assert_eq!(
            slice_changed_lines(&h, &sel(&[])),
            Err(SplitError::NoChangedLinesSelected)
        );
    }
}
