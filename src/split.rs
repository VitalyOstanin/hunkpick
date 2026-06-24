use crate::model::*;
use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum SplitError {
    NotAContextLine(u32),
    OutOfRange(u32),
    /// An `INDEX@lo-hi` range references added lines outside `1..=added` of the sub-hunk.
    AddedLineOutOfRange {
        lo: usize,
        hi: usize,
        added: usize,
    },
    /// An `INDEX@lo-hi` cut would fall on a context or deletion line rather than between two
    /// additions. Carries the 1-based added-line number at the offending boundary.
    NotAnAdditionBoundary(usize),
}

impl fmt::Display for SplitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SplitError::NotAContextLine(n) => {
                write!(f, "new-file line {n} is a change line, not a context line")
            }
            SplitError::OutOfRange(n) => write!(f, "new-file line {n} is out of range"),
            SplitError::AddedLineOutOfRange { lo, hi, added } => write!(
                f,
                "added-line range {lo}-{hi} is out of range (sub-hunk has {added} added line(s))"
            ),
            SplitError::NotAnAdditionBoundary(n) => write!(
                f,
                "cannot cut at added line {n}: the cut would fall on a context or deletion line, \
                 not between two additions"
            ),
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
    use std::collections::BTreeSet;
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

/// Cut one sub-hunk to the inclusive added-line range `[lo, hi]` (1-based over the sub-hunk's
/// added lines). The cut is allowed only on an addition|addition boundary; deletions and
/// surrounding context attach to the first piece (when `lo == 1`) and the trailing piece
/// (when `hi == A`), so concatenating the pieces of one sub-hunk reproduces it. A pure-addition
/// piece becomes `@@ -L,0 +M,k @@` at the shared old anchor `L`.
pub fn slice_added_range(h: &Hunk, lo: usize, hi: usize) -> Result<Hunk, SplitError> {
    // Absolute indices (in h.lines) of every added line.
    let add_pos: Vec<usize> = h
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| matches!(l.kind, LineKind::Add))
        .map(|(i, _)| i)
        .collect();
    let a = add_pos.len();
    if lo < 1 || lo > hi || hi > a {
        return Err(SplitError::AddedLineOutOfRange { lo, hi, added: a });
    }
    let p_lo = add_pos[lo - 1];
    let p_hi = add_pos[hi - 1];
    // Left cut: when not starting at the first added line, the line immediately before the
    // first selected addition must itself be an addition (an addition|addition boundary).
    if lo > 1 && !matches!(h.lines[p_lo - 1].kind, LineKind::Add) {
        return Err(SplitError::NotAnAdditionBoundary(lo));
    }
    // Right cut: when not ending at the last added line, the line immediately after the last
    // selected addition must be an addition.
    if hi < a && !matches!(h.lines[p_hi + 1].kind, LineKind::Add) {
        return Err(SplitError::NotAnAdditionBoundary(hi + 1));
    }
    let start = if lo == 1 { 0 } else { p_lo };
    let end = if hi == a { h.lines.len() } else { p_hi + 1 };
    Ok(rebuild_subhunk(h, &h.lines[start..end], start))
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

    const PURE_ADD: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -0,0 +1,4 @@
+l1
+l2
+l3
+l4
";

    const MIXED: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,4 @@
 ctx
-a
+b
+c
+d
";

    #[test]
    fn slice_pure_addition_first_part() {
        // Added lines: l1=1 l2=2 l3=3 l4=4. Take 1..2.
        let h = hunk(PURE_ADD);
        let part = slice_added_range(&h, 1, 2).unwrap();
        assert_eq!(part.old_start, 0);
        assert_eq!(part.old_lines, 0);
        assert_eq!(part.new_start, 1);
        assert_eq!(part.new_lines, 2);
        let added: Vec<_> = part
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Add)
            .map(|l| l.text.clone())
            .collect();
        assert_eq!(added, vec![b"l1".to_vec(), b"l2".to_vec()]);
    }

    #[test]
    fn slice_pure_addition_second_part_anchor() {
        // Take 3..4: a pure insertion at the same old anchor (old_lines == 0).
        let h = hunk(PURE_ADD);
        let part = slice_added_range(&h, 3, 4).unwrap();
        assert_eq!(part.old_lines, 0);
        assert_eq!(part.new_lines, 2);
        let added: Vec<_> = part
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Add)
            .map(|l| l.text.clone())
            .collect();
        assert_eq!(added, vec![b"l3".to_vec(), b"l4".to_vec()]);
    }

    #[test]
    fn slice_full_range_ok() {
        let h = hunk(PURE_ADD);
        assert!(slice_added_range(&h, 1, 4).is_ok());
    }

    #[test]
    fn slice_out_of_range_errors() {
        let h = hunk(PURE_ADD); // 4 added lines
        assert!(matches!(
            slice_added_range(&h, 1, 5),
            Err(SplitError::AddedLineOutOfRange {
                lo: 1,
                hi: 5,
                added: 4
            })
        ));
        assert!(matches!(
            slice_added_range(&h, 0, 1),
            Err(SplitError::AddedLineOutOfRange { .. })
        ));
    }

    #[test]
    fn slice_mixed_first_part_keeps_deletion_and_context() {
        // Added lines in MIXED: b=1 c=2 d=3. Take 1..2 -> keeps leading ctx + deletion.
        let h = hunk(MIXED);
        let part = slice_added_range(&h, 1, 2).unwrap();
        // old side: ctx + del = 2; new side: ctx + 2 adds = 3.
        assert_eq!(part.old_lines, 2);
        assert_eq!(part.new_lines, 3);
        assert!(part
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Del && l.text == b"a"));
    }

    #[test]
    fn slice_mixed_tail_is_pure_insertion() {
        // Take 3..3 (just +d): pure insertion, no deletion/context attached.
        let h = hunk(MIXED);
        let part = slice_added_range(&h, 3, 3).unwrap();
        assert_eq!(part.old_lines, 0);
        assert_eq!(part.new_lines, 1);
        assert!(part.lines.iter().all(|l| l.kind == LineKind::Add));
    }

    #[test]
    fn slice_rejects_non_addition_boundary() {
        // A run where additions are split by a deletion: +x -y +z.
        let h = hunk(
            "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,1 +1,2 @@
+x
-y
+z
",
        );
        // Added lines: x=1, z=2. Taking just 2 (+z) would cut between +x and -y|+z:
        // the line before +z is the deletion -y, not an addition.
        assert!(matches!(
            slice_added_range(&h, 2, 2),
            Err(SplitError::NotAnAdditionBoundary(_))
        ));
    }

    #[test]
    fn slice_roundtrip_concatenates_to_original() {
        // Cutting PURE_ADD into [1-2] + [3-4] and concatenating reproduces the original body.
        let h = hunk(PURE_ADD);
        let p1 = slice_added_range(&h, 1, 2).unwrap();
        let p2 = slice_added_range(&h, 3, 4).unwrap();
        let mut combined = p1.lines.clone();
        combined.extend(p2.lines.clone());
        assert_eq!(combined, h.lines);
    }

    #[test]
    fn slice_pieces_apply_independently_via_git() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let h = hunk(PURE_ADD);
        // Two pieces covering the file-creation block; each must apply on its own.
        for (lo, hi) in [(1usize, 2usize), (3, 4)] {
            let piece = slice_added_range(&h, lo, hi).unwrap();
            let diff = assemble(&[piece]);
            let dir = tempfile::tempdir().unwrap();
            // PURE_ADD is `@@ -0,0 +1,4 @@`: applies to an empty file `f`.
            std::fs::write(dir.path().join("f"), "").unwrap();
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
                "piece {lo}-{hi} failed to apply:\n{}",
                String::from_utf8_lossy(&diff)
            );
        }
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
}
