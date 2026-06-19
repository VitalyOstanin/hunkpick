# ADR 0003 — Non-overlapping auto-split: boundary context goes to the earlier sub-hunk

Date: 2026-06-19

## Status

Accepted

## Context

`hunkpick` automatically decomposes each hunk into sub-hunks at boundaries between
adjacent change runs (maximal contiguous sequences of `+`/`-` lines). A "boundary"
is the context line or lines that separate two change runs within one hunk.

The central question is: where does that boundary context go?

**Option A — shared context (git add -p style)**: the boundary context is duplicated
into the trailing context of sub-hunk N and the leading context of sub-hunk N+1. Both
sub-hunks cover the same old-file lines for the shared context rows — their old-file
ranges overlap.

**Option B — non-overlapping (hunkpick approach)**: the boundary context becomes the
trailing context of sub-hunk N only. Sub-hunk N+1 starts directly at its change run,
with no leading copy of the shared context.

`git add -p` uses option A. It can do so because it applies each hunk individually in
separate `git apply` invocations, and `git apply` accepts overlap across separate
invocations.

`hunkpick select` emits **all selected sub-hunks as a single combined patch** and
applies them in one `git apply` call. If two sub-hunks share old-file line coverage
for the boundary context, their old-file ranges overlap and `git apply` rejects the
combined patch (verified with git 2.53.0 on a patch whose two hunks both cover old
line 3):

```
error: patch failed: f:3
error: f: patch does not apply
```

## Decision

Auto-split produces **strictly non-overlapping old-file ranges**. Boundary context
between adjacent change runs is assigned to the earlier sub-hunk as trailing context.
Later sub-hunks start at their change run.

Formally, for change runs at positions `[r0_start, r0_end)` and `[r1_start, r1_end)`:
- Sub-hunk 0 covers `h.lines[0..r1_start]` (includes all context up to and including
  the boundary).
- Sub-hunk 1 covers `h.lines[r1_start..end_of_hunk]` (starts at the second change
  run, no leading shared context).

## Consequences

- All selected sub-hunks can be emitted as one patch and applied with a single
  `git apply --cached` call without overlap errors.
- **Round-trip property**: selecting all sub-hunks for a file produces a diff that
  applies equivalently to the original. The output is not byte-identical to the
  original (one hunk header becomes several), but the resulting file content is
  identical.
- Later sub-hunks lack leading context for the lines between change runs. `git apply`
  locates them by exact line match rather than context proximity. This is reliable for
  the typical case of applying to the same file that produced the diff.
- This behaviour differs from `git add -p`. Users accustomed to `git add -p`'s
  shared-context split should be aware that hunkpick sub-hunks are not byte-identical
  to what `git add -p` would produce.
- The internal consistency check (`validate_internal`) enforces non-overlap and
  detects any regression in the split logic.
