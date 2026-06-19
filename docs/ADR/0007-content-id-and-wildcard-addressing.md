# ADR 0007 — Content-id and wildcard sub-hunk addressing

Date: 2026-06-19

## Status

Accepted

Augments [ADR 0002](0002-per-file-hunk-addressing.md). The `path:index` and bare-index
forms from 0002 are unchanged; this record adds two further selector forms.

## Context

ADR 0002 addresses sub-hunks by a 1-based per-file index. Use by an automated coding
agent surfaced two limitations of index-only addressing:

1. **Indices are not stable across a re-diff.** In an iterative `diff → stage → re-diff`
   loop, staging or editing one change renumbers the sub-hunks after it (and shifts
   every hunk's `@@` line numbers). An index captured from one `list` run may point at a
   different change on the next run, so the agent must re-read `list` after every step.
2. **Selecting "all sub-hunks of a file" requires knowing the count.** With only explicit
   indices and ranges, an agent first has to read the sub-hunk count from `list` to build
   a `1-N` range, an extra round trip for a common operation.

## Decision

Add two selector forms alongside the existing ones.

### Content id (`@<id>`)

`list` reports a 16-hex **content id** for every sub-hunk (a new `id` field in `--json`
and a column in the human listing). `select` accepts `@<id>` and emits every sub-hunk
whose id equals `<id>` (matched case-insensitively).

The id is `FNV-1a-64` over the file's old and new paths and, for each **changed**
(added or deleted) line, its kind marker, raw bytes, and no-newline flag. It **excludes**
context lines, the `@@` line numbers, and the section header. FNV-1a was chosen for a
fixed, portable, dependency-free hash: the id must be identical between a `list` run and
a later `select`, which `std`'s `DefaultHasher` does not guarantee across versions.
Cryptographic strength is not needed.

Stability guarantee: because the id hashes only the changed lines, it is unchanged across
a re-diff in both cases that matter to an iterative `diff → stage → re-diff` loop — when
an unrelated edit only shifts the sub-hunk's line numbers, **and** when staging a
neighbouring sub-hunk rewrites this change's surrounding context (or causes the enclosing
hunk to be re-split). The id changes only when the sub-hunk's own `+`/`-` lines change.
It therefore identifies "this change", not "this change in this position" or "in this
context": a caller captures `@<id>` once and keeps addressing the change across staging
steps without re-reading `list`.

The cost of excluding context is that the same `+`/`-` lines in different surrounding
context share an id. This was previously rejected (an earlier draft hashed context to
keep physically distinct identical edits distinct), but the `id_count` field
(below) makes that sharing observable: a consumer sees `id_count > 1` and falls back to
`path:N` to pick one, so the ambiguity is surfaced rather than silent. The gain —
re-diff stability across neighbour staging, the common agent loop — outweighs it.

The path is part of the hashed input, so the same textual change in two different files
gets different ids. `@<id>` selecting *all* matches makes "stage this change everywhere
it occurs" well defined. Because the hash is not collision-proof, `select` verifies that
all sub-hunks sharing a matched id have identical changed lines; if two genuinely
different changes ever collide, it reports the id and exits 2 (a usage error) so the
caller falls back to `path:N`. This turns a vanishingly rare collision into a loud
failure rather than a silent wrong pick.

### Wildcard (`*`)

`path:*` selects every sub-hunk of the named file; bare `*` selects every sub-hunk of a
single-file diff (same single-file restriction as a bare index list).

### Precedence and scope

Selectors are parsed in the order: `path:set` (where `set` is an index list or `*`),
then `@id`, then a bare `set`. Checking `path:set` first keeps a file literally named
`@foo` addressable as `@foo:1`. `split` is unchanged — it addresses one *original* hunk
by `path:N` / `N` and accepts neither `*` nor `@id`.

## Consequences

- Agents get a re-diff-stable handle for any change: capture `@id` from `list` and keep
  using it across staging steps, whether line numbers shift or a neighbour's staging
  rewrites the context — the id depends only on the changed lines. The listing must be
  re-read only when the change's own `+`/`-` lines change.
- Changes with identical `+`/`-` lines but different context share an id. `id_count`
  makes this observable, and `path:N` addresses one of them; a hash collision between
  genuinely different changes is still detected and rejected.
- `path:*` / `*` removes the count-lookup round trip for whole-file selection.
- `list` now computes a hash and allocates an id string per sub-hunk. The cost is linear
  in the diff size and confined to `list` (and to `@id` resolution, which scans the whole
  patch); `path:N` / `*` selection keeps the lazy per-named-file auto-split from 0002.
- The `--json` schema gains `id` and `id_count` fields — an additive change. `id_count`
  (how many sub-hunks share the id) lets a consumer see, without counting the listing
  itself, whether `@<id>` resolves to one sub-hunk or several.
- Accidental hash collisions between distinct changes are detected and rejected rather
  than mis-selected; intentional duplicates are selected together.
