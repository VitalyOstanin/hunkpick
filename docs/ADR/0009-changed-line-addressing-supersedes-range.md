# ADR 0009 — Changed-line addressing (`INDEX@L<set>`) supersedes and removes `INDEX@RANGE`

Date: 2026-07-11

## Status

Accepted

Supersedes [ADR 0008](0008-added-line-range-addressing.md) (added-line range addressing).

## Context

ADR 0008 added `[path:]INDEX@lo-hi`, a per-line selector that cuts an addition-only sub-hunk
to an inclusive range of its **added** lines. Its implementation (`slice_added_range`) emits a
single **contiguous** slice of the sub-hunk body: leading context attaches to the piece that
starts at added line 1, trailing context to the piece that ends at the last added line.

A later selector, `[path:]INDEX@L<set>` (`slice_changed_lines`), addresses an arbitrary subset
of a sub-hunk's **changed** (`+`/`-`) lines: it walks the whole body, keeps every context line,
keeps selected changed lines, turns each unselected deletion into a context line, and omits each
unselected addition. Because it retains all context on both sides and never needs a contiguous
run, every subset is realisable as one applicable hunk.

Two problems with keeping both:

1. **`@lo-hi` is buggy for a mid-file addition block.** A leading or interior slice
   (`@lo-hi` with `hi < A`) keeps the leading context but drops the **trailing** context, because
   a contiguous slice cannot both omit the trailing additions and retain the context that sits
   after them. `git apply` then cannot anchor the piece and rejects it (`patch does not apply`).
   The default internal result-diff check does not catch this; only `--verify-result-diff-git`
   would. The trailing slice (`@hi-`) happens to work because it keeps its trailing context.

2. **`@L` strictly subsumes `@lo-hi`.** Any added-line range is expressible as an `@L` set (for a
   pure-addition sub-hunk the two numberings coincide), and `@L` additionally handles what
   `@lo-hi` cannot: interior slices that keep both-side context, isolating a deletion surrounded
   by additions, and separating a replacement's removals from its insertions. Maintaining a second,
   buggy, less-capable cutter is redundant.

## Decision

Remove `[path:]INDEX@lo-hi` entirely; `[path:]INDEX@L<set>` is the sole per-line cutter.

- `slice_added_range` and its `SplitError` variants (`AddedLineOutOfRange`, `NotAnAdditionBoundary`)
  are deleted. The `Ranged`/`LineRange` selector types and the `SelectError::Range` variant are
  deleted.
- A selector still using the removed form (`INDEX@<anything not L>`) is a **usage error** (exit 2)
  with a message that names the `@L` replacement, so a caller (typically a coding agent) can
  self-correct rather than see a bare "bad selector".
- The `list` human marker for an all-additions sub-hunk is renamed `[+range]` → `[+add]`; the
  `addition_only` field in `list --json` is unchanged (it is a structural fact, no longer tied to a
  range-cut feature).
- **Only a numeric index may precede `@`** — unchanged from ADR 0008; content ids (`@id`) and `*`
  are still rejected as the address of a cut.

This is a breaking change to the `select` selector grammar; released as 0.5.0.

## Consequences

- The leading/interior mid-file slice bug is gone: `@L` keeps both leading and trailing context, so
  every emitted piece anchors under `git apply` without `--unidiff-zero`.
- One cutter to maintain and document instead of two overlapping ones.
- Callers pinned to the `@lo-hi` form must migrate to `@L`. The numbering differs — `@lo-hi` counted
  only added (`+`) lines, `@L` counts all changed (`+`/`-`) lines — so a mechanical `lo-hi` → `L lo-hi`
  rewrite is exact only for a pure-addition sub-hunk. `list --json`'s `changed_lines` is the source
  of `@L` indices. The removed-form error steers callers to `@L`.
- `addition_only` in `list --json` remains for informational parity; it no longer advertises a
  range-cut capability (with `@L`, any sub-hunk is freely cuttable).
