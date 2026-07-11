# ADR 0008 — Added-line range addressing (`INDEX@RANGE`) in `select`

Date: 2026-06-24

## Status

Superseded by [ADR 0009](0009-changed-line-addressing-supersedes-range.md)

Complements ADR 0007 (content-id and wildcard addressing).

## Context

Auto-split (ADR 0003) divides a hunk into sub-hunks at context gaps between change runs. A
sub-hunk that is entirely additions — a block of new functions, or a file-creation diff
`@@ -0,0 +1,N @@` — is one atomic sub-hunk with no internal context line, so `split --at`
(which cuts only on a context line) cannot divide it. Staging part of such a block (e.g.
splitting a new-function block across commits next to the matching code change) was therefore
impossible: the whole block was forced into one commit.

## Decision

Add a per-line range selector to `select`:

```
[path:]INDEX@RANGE      RANGE = lo-hi | lo- | -hi | N
```

- `RANGE` numbers the sub-hunk's **added (`+`) lines**, 1-based; context and deletions have no
  number. For a pure-addition sub-hunk this is every body line.
- The cut is allowed only on an **addition|addition boundary**: when a piece does not start at
  the first added line, the line immediately before its first added line must also be an
  addition; symmetrically at the end. Cutting where the boundary is a context or deletion line
  is rejected.
- **Distribution.** Leading context and all deletions attach to the piece that starts at added
  line 1; trailing context attaches to the piece ending at the last added line; interior pieces
  are pure insertions `@@ -L,0 +k @@` at the shared old anchor `L`. Concatenating the pieces of
  one sub-hunk reproduces it (round-trip).
- **Only a numeric index may precede `@`.** Content-id (`@id`) and wildcard (`*`) are not
  accepted as the address of a range.

The cut itself lives in `split::slice_added_range`, reusing the existing `rebuild_subhunk`
offset/count recomputation. `select` resolves open range ends against the sub-hunk's added-line
count before slicing.

### Why only a numeric index precedes `@`

- A content id hashes only the `+/-` lines and the path, so byte-identical blocks share an id
  and `@id` deliberately selects them all (ADR 0007). Applying one `RANGE` to several blocks of
  different added-line counts is ambiguous.
- The range cut is destructive: after the first piece is staged and committed, the original
  block changes and the id of the original sub-hunk no longer exists. The stability that
  motivates content ids gives no benefit to a cut.
- `*` addresses all sub-hunks at once, which is incompatible with a single line range.

### Incidental tightening of `@id` parsing

Adding the `INDEX@RANGE` form required distinguishing it from the `@id` form at parse time. As
part of this, the `@id` parser was tightened to accept only a non-empty hex id (`subhunk_id`
always emits 16 lowercase hex digits). Previously any non-empty token after `@` parsed as an id
and only failed later at resolve time with `UnknownId`; now a non-hex `@token` is rejected at
parse time as a bad selector. This is a fail-fast improvement consistent with ADR 0007's id
format; no valid id is affected.

## Consequences

- `select`'s selector grammar gains one form; `IndexSet` gains a `Ranged` variant; `SplitError`
  gains `AddedLineOutOfRange` and `NotAnAdditionBoundary`; `SelectError` gains `Range`.
- `list` reports `addition_only` (`--json`) and a `[+range]` marker (human) so the
  freely-splittable sub-hunks are visible.
- The result diff is verified by the existing two-tier check (ADR 0004): two pure-addition
  pieces at the shared anchor `-L,0` pass the internal overlap check (equal ends are not an
  overlap).
- `split --at` is unchanged; it still cuts only on context lines. The two mechanisms do not
  overlap: `split` rewrites one original hunk into all its pieces; `select INDEX@RANGE` emits
  one chosen range.
- Overlapping selections of one sub-hunk (a whole sub-hunk plus a range of it, or two
  intersecting ranges) are currently caught downstream by the internal consistency check rather
  than as a usage error; refining that diagnostic is tracked in TODO.md.
