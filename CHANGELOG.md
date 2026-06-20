# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2026-06-20

### Changed

- `hunkpick --help` now shows usage examples and a content-id (`@<id>`) section
  inline, so the common recipes are visible without drilling into each
  subcommand's `--help`. The short `-h` stays a compact summary and points to
  `--help` for the examples.

## [0.2.0] - 2026-06-19

### Added

- Content ids for sub-hunks. `list` (human and `--json`) now reports a 16-hex `id`
  per sub-hunk, derived from the file paths and the sub-hunk's changed (`+`/`-`) lines
  only — independent of its context lines, the `@@` line numbers, and the section header.
  The id is stable across a re-diff in the common agent loop: it is unchanged both when an
  unrelated edit only shifts a change's line numbers and when staging a neighbour rewrites
  the change's surrounding context (or re-splits the enclosing hunk). It changes only when
  the change's own `+`/`-` lines change. `list --json` also reports `id_count` per
  sub-hunk — how many sub-hunks share that id (`1` = unique), so a consumer can tell
  whether `@<id>` addresses one sub-hunk or several.
- `@<id>` selector for `select`: emits every sub-hunk whose content id equals `<id>`
  (matched case-insensitively). Changes with identical `+`/`-` lines share an id and are
  selected together (use `path:N` to pick one, guided by `id_count`); an id shared by
  changes whose `+`/`-` lines genuinely differ (an accidental hash collision) is reported
  and exits with code 2.
- `*` selector: `path:*` selects every sub-hunk of a file; bare `*` selects every
  sub-hunk of a single-file diff. Removes the need to first read the sub-hunk count
  from `list`.

## [0.1.0] - 2026-06-19

### Added

- Unified-diff parser with full round-trip emitter: parses `git diff`, `diff -u`,
  rename/mode/new-file/deleted-file/binary headers, no-newline markers, CRLF line
  endings, and plain (non-git) diffs; emits a semantically equivalent patch.
- `list` subcommand: auto-splits each hunk into minimal sub-hunks and lists them per
  file with a 1-based per-file index. Human-readable output by default; `--json` emits
  a stable machine schema with `path`, `binary`, `index`, `old_start`, `old_lines`,
  `new_start`, `new_lines`, `added`, `deleted`, `header`, and `preview` fields.
  `--color auto|always|never` controls ANSI colour (respects `NO_COLOR`).
- `select` subcommand: emits only the chosen sub-hunks as a valid unified diff.
  Selector syntax: bare `1,3` or `2-4` for single-file diffs; `path:1,3` and
  `path:2-4` for multi-file diffs; multiple selectors may be combined. A binary file
  referenced by any selector is emitted whole.
- `split` subcommand: explicitly splits one original hunk (addressed `path:N` or `N`)
  at given new-file line numbers (must be context lines); emits the whole patch with
  that hunk replaced by the pieces.
- Auto-split semantics with non-overlapping old-file ranges: boundary context between
  adjacent change runs becomes trailing context of the earlier sub-hunk; later
  sub-hunks start at their change run. Selecting all sub-hunks is apply-equivalent to
  the original patch.
- Result-diff verification for `select` and `split`: internal consistency check (hunk
  header counts match body, hunks are ordered and non-overlapping) runs by default;
  disable with `--no-verify-result-diff-internal`. Optional `git apply --check` via
  `--verify-result-diff-git`; `-C <DIR>` sets the working tree directory and requires
  `--verify-result-diff-git`.
- Git-agnostic design: reads stdin, writes stdout; does not call `git diff` itself.
  Works with diffs from any source (git, Mercurial, SVN, plain `diff -u`).
- Encoding-agnostic byte-oriented core: diff content is parsed, processed, and emitted
  as raw bytes, so any encoding (including invalid UTF-8) round-trips byte-for-byte.
  Only `list` paths/previews are decoded lossily for display.
- Input validation before parsing: empty/whitespace-only input is a no-op (exit 0);
  binary input (NUL byte) and text with no diff markers are rejected with exit code 2.
- Input source selection: `-i, --input FILE` reads the diff from a file (`-` means
  stdin) on every subcommand; stdin remains the default.
- Input size limit: `--max-input-bytes N` caps the input (default 64 MiB; `0` disables);
  exceeding it is a usage error (exit code 2). The input buffer is freed after parsing so
  it does not coexist with the result diff on the heap.
- Edge-case support: rename diffs, mode-only changes, new-file and deleted-file
  patches, binary file entries, `\ No newline at end of file` markers, CRLF line
  endings, and plain (non-extended-header) unified diffs.
- Structured exit codes: 0 success, 2 usage/parse error, 70 verification failure,
  74 I/O error, 130 signal termination.
- Prebuilt binaries on GitHub Releases for `x86_64-unknown-linux-gnu`,
  `aarch64-apple-darwin`, `x86_64-apple-darwin`, and `x86_64-pc-windows-msvc`,
  installable with `cargo binstall hunkpick`.
