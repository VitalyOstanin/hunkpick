# TODO

## Contents

- [Fixed](#fixed)
- [Per-line cutting: remaining edges](#per-line-cutting-remaining-edges)

## Fixed

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

- **`@L` across several sub-hunks in one invocation when deletions become
  context.** Selecting additions (which turns that sub-hunk's deletions into
  context) grows its new-side span; combined with another sub-hunk in the same
  emit, the new-side ranges can overlap and `validate_internal` rejects it
  (`OverlappingHunks`, exit 70) rather than as a clean usage error. Safe (no bad
  patch emitted), but the diagnostic could be clearer, or the new-side anchors
  recomputed so the combination is valid. The re-diff loop avoids it.

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
