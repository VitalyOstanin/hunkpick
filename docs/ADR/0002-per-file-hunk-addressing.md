# ADR 0002 — Per-file sub-hunk addressing with path:index selector syntax

Date: 2026-06-19

## Status

Accepted

## Context

`hunkpick` needs a selector syntax for addressing individual sub-hunks in the output
of `list`. Alternative addressing schemes considered:

1. **Global sequential index**: number sub-hunks across all files in document order
   (`1`, `2`, `3`, …). Simple but fragile — adding or removing a file shifts all
   subsequent indices.
2. **Per-file index only, no path prefix**: e.g. `1,3` always. Ambiguous when the
   diff contains multiple files; the user cannot distinguish "sub-hunk 1 of file A"
   from "sub-hunk 1 of file B".
3. **Per-file index with optional path prefix**: `path:1,3` for multi-file diffs;
   bare `1,3` for single-file diffs. Unambiguous in all cases; the path prefix is
   optional only where there is no ambiguity.

## Decision

Sub-hunks are addressed by a **1-based index within each file**. The selector syntax
is:

- `path:indices` — always unambiguous; `path` matches the new path or old path of the
  file diff entry.
- Bare `indices` — accepted only when the input diff contains exactly one file;
  otherwise `hunkpick` exits with code 2 (usage error).

`indices` is a comma-separated list of integers and ranges: `1,3`, `2-4`, `1,3-5`.
Ranges are inclusive on both ends.

For the `split` subcommand, the hunk address `path:N` / `N` follows the same rules,
with `N` indexing the file's original hunks (before auto-splitting).

## Consequences

- Selectors are unambiguous in multi-file diffs: each `path:index` pair identifies
  exactly one sub-hunk.
- Scripts and agents can construct selectors from the `list --json` output directly
  (`path` + `index` fields).
- Bare index lists remain usable for the common single-file case
  (`git diff path | hunkpick select 1,3`), reducing verbosity.
- Path matching checks both old and new paths, so selectors work correctly for renamed
  files when either side's path is used.
- Indices are stable within a session: the `list` output and the `select` / `split`
  addressing use the same numbering.
