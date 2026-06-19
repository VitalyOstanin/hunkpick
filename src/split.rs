use crate::model::*;

#[derive(Debug, PartialEq, Eq)]
pub enum SplitError {
    NotAContextLine(u32),
    OutOfRange(u32),
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
        let slice = &h.lines[*start..*end];
        result.push(rebuild_subhunk(h, slice, *start));
    }
    result
}

/// Build a Hunk from a slice of `h.lines` starting at absolute index `abs_start`,
/// recomputing old/new start offsets and line counts.
fn rebuild_subhunk(h: &Hunk, slice: &[Line], abs_start: usize) -> Hunk {
    let mut old_off = 0u32;
    let mut new_off = 0u32;
    for l in &h.lines[..abs_start] {
        match l.kind {
            LineKind::Context => {
                old_off += 1;
                new_off += 1;
            }
            LineKind::Del => old_off += 1,
            LineKind::Add => new_off += 1,
        }
    }
    let mut old_lines = 0u32;
    let mut new_lines = 0u32;
    for l in slice {
        match l.kind {
            LineKind::Context => {
                old_lines += 1;
                new_lines += 1;
            }
            LineKind::Del => old_lines += 1,
            LineKind::Add => new_lines += 1,
        }
    }
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
}
