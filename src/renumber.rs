//! New-side (`+`) line numbers of a result diff.
//!
//! A hunk's new-side start follows from its old-side start plus the net size of everything the
//! same diff already changed above it, so dropping a sub-hunk invalidates every later anchor
//! carried over from the input. `git apply` searches from that position, so a stale anchor
//! misplaces the change rather than merely looking wrong. Why the result recomputes them
//! instead of trusting the input: `docs/ADR/0010-result-diff-owns-new-side-anchors.md`.
//!
//! [`renumber_new_side`] recomputes the anchors from the result alone;
//! [`expected_new_start`] is the same arithmetic exposed for the consistency check in
//! [`crate::validate`].

use crate::model::{FileContent, Hunk, Patch};

/// The 1-based position a hunk's side starts at, given the header's `start` and line `count`.
///
/// A zero count means the side contributes nothing there — a pure insertion has no old-side
/// lines, a pure deletion no new-side lines — and git writes the *preceding* line number in the
/// header (`@@ -2,0 +3 @@`, `@@ -3 +2,0 @@`). Normalising both cases to "the line this hunk
/// starts at" keeps the offset arithmetic below uniform.
pub(crate) fn anchor(start: u32, count: u32) -> i64 {
    i64::from(start) + i64::from(count == 0)
}

/// The new-side start a hunk must carry, given its old-side header and `delta` — the net
/// `added - deleted` of every hunk emitted before it in the same file.
///
/// The inverse of [`anchor`]: the new-side anchor is the old-side one shifted by `delta`, and a
/// hunk with no new-side lines reports the line before it.
pub fn expected_new_start(h: &Hunk, delta: i64) -> u32 {
    let a = anchor(h.old_start, h.old_lines) + delta;
    let start = if h.new_lines == 0 { a - 1 } else { a };
    // A well-formed result never drives this negative; clamp rather than wrap on a synthetic one.
    start.clamp(0, i64::from(u32::MAX)) as u32
}

/// Recompute every hunk's new-side start from the patch itself, per file.
///
/// Call this on a result diff assembled from a subset of another diff's hunks (`select`), where
/// the anchors inherited from the input describe a file the result no longer produces.
pub fn renumber_new_side(patch: &mut Patch) {
    for f in &mut patch.files {
        let FileContent::Text(hunks) = &mut f.content else {
            continue; // a binary patch carries no line numbers
        };
        let mut delta: i64 = 0;
        for h in hunks.iter_mut() {
            h.new_start = expected_new_start(h, delta);
            let (add, del) = h.change_counts();
            delta += i64::from(add) - i64::from(del);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::emit;
    use crate::parser::parse;

    /// Parse a diff, drop the hunks whose 0-based index is not in `keep`, renumber, re-emit.
    fn keep_hunks(src: &str, keep: &[usize]) -> String {
        let mut p = parse(src.as_bytes()).unwrap();
        for f in &mut p.files {
            if let FileContent::Text(hunks) = &mut f.content {
                let mut i = 0;
                hunks.retain(|_| {
                    let k = keep.contains(&i);
                    i += 1;
                    k
                });
            }
        }
        renumber_new_side(&mut p);
        String::from_utf8(emit(&p)).unwrap()
    }

    const INSERT_THEN_REPLACE: &str = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -2,3 +2,4 @@
 b
+new
 c
 d
@@ -17,3 +18,3 @@
 q
-r
+R
 s
";

    #[test]
    fn dropping_an_insertion_shifts_the_next_hunk_back() {
        let out = keep_hunks(INSERT_THEN_REPLACE, &[1]);
        assert!(out.contains("@@ -17,3 +17,3 @@"), "{out}");
    }

    #[test]
    fn keeping_everything_reproduces_the_input_anchors() {
        let out = keep_hunks(INSERT_THEN_REPLACE, &[0, 1]);
        assert!(out.contains("@@ -2,3 +2,4 @@"), "{out}");
        assert!(out.contains("@@ -17,3 +18,3 @@"), "{out}");
    }

    #[test]
    fn dropping_a_deletion_shifts_the_next_hunk_forward() {
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -2,3 +2,2 @@
 b
-c
 d
@@ -17,3 +16,3 @@
 q
-r
+R
 s
";
        let out = keep_hunks(src, &[1]);
        assert!(out.contains("@@ -17,3 +17,3 @@"), "{out}");
    }

    #[test]
    fn zero_count_sides_report_the_preceding_line() {
        // A pure deletion has no new-side lines: git writes the line before it (`+N-1,0`).
        // A pure insertion has no old-side lines: its own start is already the following line.
        let src = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,2 +1,0 @@
-a
-b
@@ -5,0 +4,2 @@
+x
+y
";
        let out = keep_hunks(src, &[0, 1]);
        assert!(out.contains("@@ -1,2 +0,0 @@"), "{out}");
        assert!(out.contains("@@ -5,0 +4,2 @@"), "{out}");
        // Dropping the deletion moves the insertion back to its old-side position.
        let only_insert = keep_hunks(src, &[1]);
        assert!(only_insert.contains("@@ -5,0 +6,2 @@"), "{only_insert}");
    }
}
