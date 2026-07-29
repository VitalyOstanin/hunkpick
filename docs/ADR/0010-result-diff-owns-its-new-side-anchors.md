# ADR 0010 — The result diff owns its new-side anchors

Date: 2026-07-29

## Status

Accepted

## Context

A hunk header carries two positions: `-old_start,old_lines` and `+new_start,new_lines`.
Only the old-side one is an independent fact about the file being patched. The new-side
start follows from it: it is the old-side start shifted by the net `added - deleted` of
everything the *same diff* changes above the hunk.

`select` emits a subset of the input's sub-hunks. Until this decision it cloned each kept
sub-hunk with the `new_start` it carried in the input diff, where the dropped sub-hunks
were still present. The result therefore described a file it does not produce: every hunk
after a dropped one was off by the net size of what was dropped.

This is not cosmetic. `git apply` locates a hunk by searching from the *new-side*
position (`apply_one_fragment` starts at `frag->newpos - 1`), widening outwards until the
preimage matches. A drifted anchor has two outcomes:

1. The search finds the intended position anyway (the surrounding context is unique
   nearby) — the defect stays invisible;
2. The search reaches a different occurrence of the same context first — the patch
   applies cleanly **to the wrong place**, or, when that position conflicts, the whole
   patch is rejected with `patch does not apply` naming the *old* line number, which is
   the one that was correct.

Outcome 2 was observed in practice: selecting three sub-hunks of a 37-sub-hunk file in
one invocation was rejected, and rewriting only the three `@@` headers made the same
bodies apply. The workaround was to select one sub-hunk per invocation with a `git diff`
between rounds — one process and one re-diff per change, which defeats naming several
selectors at once.

The internal consistency check (ADR 0004) did not catch it: header counts, ordering and
old-side non-overlap all hold for a diff with stale anchors.

An adjacent restriction had the same root cause. An `@L` slice that keeps its unselected
deletions as context grows that sub-hunk's new-side span; combined with a later sub-hunk
in one emit, the inherited anchors made the two new-side ranges overlap, and the result
was rejected as `OverlappingHunks` rather than emitted.

## Decision

- **The result diff is self-contained.** New-side starts are recomputed from the emitted
  hunks alone, never carried over from the input. `select` runs the pass
  (`renumber::renumber_new_side`) over its result before verification and emission; per
  file, walking the kept hunks in order:

  ```
  new_start[i] = old_start[i] + Σ (added - deleted) over the kept hunks 0..i-1
  ```

  A side whose line count is zero reports the preceding line, matching how git writes it
  (`@@ -2,0 +3 @@`, `@@ -3 +2,0 @@`); the arithmetic is done on the normalised anchor and
  converted back on the way out.

- **The same relation is an invariant of every emitted diff**, checked by
  `validate_internal` as `StaleNewStart` alongside the count, ordering and overlap checks.
  A stale anchor is a hard error at emit time (exit 70) instead of a `git apply` surprise
  at the caller.

- **`split` does not renumber.** It replaces one hunk with pieces of itself and leaves the
  other hunks of the input in place, so the input's anchors remain the truth for them.
  Rewriting numbers the user did not ask to change would hide a malformed input rather
  than surface it; the invariant check reports one instead.

## Consequences

- Several sub-hunks can be named in one `select` invocation and the result applies at the
  intended positions. The `diff → stage → re-diff` loop remains useful for splitting one
  sub-hunk across rounds, but is no longer a workaround for multi-selector staging.
- `@L` slices combine with other sub-hunks in one invocation; the growth of a slice's
  new-side span is accounted for, so the previously rejected combination emits and
  applies. Several `@L` pieces of the *same* sub-hunk remain a usage error — their
  old-side ranges coincide, which no renumbering can fix.
- A result diff produced by an earlier version (or any diff whose anchors are stale) is
  now rejected by the default check instead of being passed on. `select` repairs such a
  diff when re-run over it; `split` reports it.
- The pass is O(hunks) per file and needs no git repository, so it does not weaken the
  git-agnostic property of ADR 0001.
