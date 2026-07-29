# TODO

## Contents

- [Fixed](#fixed)
- [Per-line cutting: remaining edges](#per-line-cutting-remaining-edges)

## Fixed

### `select` carried over the input diff's new-side line numbers (fixed after 0.6.0)

Every kept hunk was emitted with the `new_start` it had in the input diff, where
the sub-hunks left out were still present. Dropping a sub-hunk changes how many
lines the result adds or removes above each later hunk, so those anchors
described a file the selection does not produce. `git apply` starts its search at
the new-side position, so the consequence was either a rejection
(`patch does not apply`, reported against the *old* line number, which is the one
that was correct) or — where the surrounding context occurs more than once — a
clean application to the wrong occurrence. Selecting several sub-hunks in one
invocation therefore needed the `diff → stage → re-diff` loop as a workaround.

Fixed by recomputing the anchors from the result alone (`src/renumber.rs`), per
file, before the result is emitted:

```
new_start[i] = old_start[i] + Σ (added - deleted) over the kept hunks 0..i-1
```

with a side whose line count is zero reporting the preceding line, as git writes
it (`@@ -2,0 +3 @@`, `@@ -3 +2,0 @@`). `@L` slices are covered by the same pass:
an unselected deletion becomes context and an unselected addition disappears,
which changes the hunk's own `added - deleted`. The default internal consistency
check now verifies the same relation (`StaleNewStart`), so a stale anchor is a
hard error at emit time instead of a `git apply` surprise at the caller.

Regression tests in `tests/new_side_anchors.rs`, including the case a header-only
assertion misses: a file whose six-line block occurs twice, where the stale
anchor applied the change cleanly to the wrong copy.

The same pass also settles the earlier "`@L` across several sub-hunks in one
invocation" item: selecting a sub-hunk's additions keeps its deletions as context
and grows its new-side span, which with the inherited anchors made the new-side
ranges of that piece and a later sub-hunk overlap (`OverlappingHunks`, exit 70).
The recomputed anchors account for the growth, so the combination emits and
applies; the remaining restriction is only on several `@L` pieces of the *same*
sub-hunk, whose old-side ranges do coincide.

### Legacy `@lo-hi` leading slice dropped trailing context (removed in 0.5.0)

Fixed by **removing** the `INDEX@lo-hi` added-line range selector entirely
(0.5.0). Its contiguous-slice implementation dropped the trailing context on a
leading/interior slice of a mid-file addition block, so `git apply` rejected the
piece (`patch does not apply`). A contiguous slice cannot both omit the trailing
additions and keep the context after them, so there was no in-place fix; the
per-line `@L<set>` cutter already keeps both leading and trailing context and
strictly subsumes `@lo-hi` (it can express any added-line range plus deletions,
interior slices, and replacements). See ADR 0009. A selector still using the
`@lo-hi` form is now a usage error (exit 2) that points the caller at `@L`.

## Per-line cutting: remaining edges

### Done

The generalized per-line selector `INDEX@L<set>` (`slice_changed_lines`) is
implemented. It selects an arbitrary subset of a sub-hunk's changed (`+`/`-`)
lines, numbered `1..N` in body order (deletions and additions share one
numbering, reported as `changed_lines` in `list --json`). Each unselected
deletion is kept as a context line — anchoring the piece so it applies without
`--unidiff-zero` — and each unselected addition is omitted, so every subset is
realisable as one applicable hunk with no boundary restriction. This subsumes
what a deletion-side `@d` cut would have offered: a deletion surrounded by
additions (`+x -y +z`) can be isolated, and a replacement's removals can be
separated from its insertions across `diff → stage → re-diff` rounds. Verified
end to end via `git apply`.

The primary consumer is a coding agent, which reads `changed_lines` from
`list --json` and constructs `@L` selectors programmatically.

The earlier "classify overlapping-selection errors as usage errors" item is also
resolved: `select` rejects overlapping selections of one sub-hunk (a whole plus a
range, or two overlapping ranges, or `@L` combined with another selection of the
same sub-hunk) as a usage error (exit 2) before the result reaches
`validate_internal`.

### Remaining

- **Multiple `@L` pieces of the same sub-hunk in one invocation.** Currently a
  usage error: separate pieces would carry mutually inconsistent new-side line
  numbers. Combining them in one emitted diff would need each piece's new-side
  anchor recomputed against the intermediate file the earlier pieces produce. The
  supported path today is the `diff → stage → re-diff` loop (one piece per round).
  Lift this only if a single-invocation multi-piece cut proves worth the anchor
  bookkeeping.

- **Genuinely zero-context edges.** The convert-unselected-deletions-to-context
  rule removes most zero-context cases, but a context-less run (a whole-file
  replacement, a file creation/deletion) can still yield a piece git needs
  `--unidiff-zero` for. If such cases matter, add an explicit `--unidiff-zero`
  opt-in (git does not content-verify those hunks, so keep it off by default).

### Fundamental limits (out of scope)

- a single changed line is the atom — half a line cannot be staged;
- a unified diff does not record which deletion pairs with which addition, so a
  "semantically correct" split of a replacement is inherently ambiguous;
- some intermediate states are unbuildable — a property of any line-wise staging.
