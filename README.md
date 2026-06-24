# hunkpick

[![crates.io](https://img.shields.io/crates/v/hunkpick.svg)](https://crates.io/crates/hunkpick)
[![docs.rs](https://docs.rs/hunkpick/badge.svg)](https://docs.rs/hunkpick)
[![CI](https://github.com/VitalyOstanin/hunkpick/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/VitalyOstanin/hunkpick/actions/workflows/ci.yml?query=branch%3Amaster)
[![license](https://img.shields.io/crates/l/hunkpick.svg)](https://github.com/VitalyOstanin/hunkpick/blob/master/LICENSE)

Non-interactive unified-diff hunk picker and splitter — a pure stdin→stdout filter for
staging subsets of changes without interactive prompts.

## Table of Contents

- [Why / Motivation](#why--motivation)
- [Installation](#installation)
- [Usage](#usage)
  - [list](#list)
  - [select](#select)
  - [split](#split)
  - [Staging recipe](#staging-recipe)
- [Selectors](#selectors)
  - [Content ids](#content-ids)
  - [Splitting an addition-only block: `INDEX@RANGE`](#splitting-an-addition-only-block-indexrange)
- [Verification](#verification)
- [Input handling](#input-handling)
- [Auto-split and non-overlap](#auto-split-and-non-overlap)
- [Exit codes](#exit-codes)
- [Comparison to filterdiff](#comparison-to-filterdiff)
- [Development](#development)
- [License](#license)

## Why / Motivation

The standard non-interactive approach for staging a subset of hunks uses
[`filterdiff`](https://linux.die.net/man/1/filterdiff) from the
[patchutils](https://cyberelk.net/tim/software/patchutils/) suite:

```sh
git diff path | filterdiff --hunks=1,3 | git apply --cached
```

`filterdiff` works at the granularity of whole hunks as they appear in the diff.
If a single hunk contains multiple independent change runs separated by context
lines, `filterdiff` cannot address them individually — the entire hunk is either
included or excluded.

`hunkpick` fills this gap:

- **Auto-split**: each hunk is automatically decomposed into minimal sub-hunks,
  one per contiguous change run. The resulting sub-hunks are addressable individually
  by a stable 1-based per-file index.
- **Per-file addressing**: selectors use `path:1,3` syntax, which is unambiguous in
  multi-file diffs and composable in scripts. A `*` selects every sub-hunk of a file.
- **Content ids**: each sub-hunk also carries a content-derived `@<id>`. It hashes only
  the file paths and the sub-hunk's changed (`+`/`-`) lines — not its context or the `@@`
  line numbers — so the id stays the same across a re-diff even when an edit elsewhere
  shifts its line numbers or staging a neighbour rewrites its surrounding context. An
  agent can capture `@<id>` once and keep using it across a staging loop. (Byte-identical
  changes share an id; `list --json` reports `id_count`. See [Content ids](#content-ids).)
- **Built-in verification**: the result diff is checked for internal consistency by
  default; an optional `git apply --check` run is available on demand.
- **Git-agnostic**: `hunkpick` reads a diff from stdin and writes to stdout. It does
  not call `git diff` itself and works with any diff source (git, Mercurial, SVN, or
  plain `diff -u` output). Application to the index is left to the caller via
  `git apply --cached`.
- **Encoding-agnostic**: the diff is processed as raw bytes end to end. Content in any
  encoding — including invalid UTF-8 — round-trips byte-for-byte; only the path and
  preview shown by `list` are decoded lossily for display.
- **Cross-platform, including Windows**: `filterdiff`/`patchutils` is a Unix toolchain
  that is awkward to obtain and run on Windows. `hunkpick` is a single self-contained
  binary built for Linux, macOS, and Windows (`x86_64-pc-windows-msvc`), with no runtime
  dependencies.
- **AI-agent integration**: the first consumer is an automated coding agent. Staging a
  precise subset of a diff programmatically needs non-interactive operation (no
  `git add -p` prompts), a stable machine-readable `--json` listing, deterministic
  per-file sub-hunk addressing, and structured exit codes — none of which the
  interactive `git add -p` or the whole-hunk-only `filterdiff` provides.

## Installation

**From crates.io:**

```sh
cargo install hunkpick
```

**Prebuilt binary via [cargo-binstall](https://github.com/cargo-bins/cargo-binstall)** (downloads the release artifact from GitHub instead of compiling):

```sh
cargo binstall hunkpick
```

Prebuilt binaries are published for `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`, and `x86_64-pc-windows-msvc`. On other targets `cargo binstall` falls back to a source build.

**From source:**

```sh
git clone https://github.com/VitalyOstanin/hunkpick.git
cd hunkpick
cargo build --release
# binary is at target/release/hunkpick
```

Minimum supported Rust version: **1.85**.

## Usage

All subcommands read a unified diff from **stdin** by default and write to **stdout**.
Use `-i, --input FILE` to read from a file instead (`-` means stdin). See
[Input handling](#input-handling) for the size limit.

### list

Parse the diff, auto-split each hunk into minimal sub-hunks, and list them per file
with their 1-based per-file index.

```sh
# Human-readable output (default)
git diff src/main.rs | hunkpick list

# Machine-readable JSON
git diff src/main.rs | hunkpick list --json

# Control colorisation
git diff src/main.rs | hunkpick list --color always
```

**Example human output:**

```
src/main.rs
  [1] 114ccaaa7ce6c0f1 @@ -10,4 +10,4 @@  +1 -1  +let x = 1;
  [2] 8002dd73f0dfd2f4 @@ -20,6 +20,6 @@  +1 -1  +fn bar() {
```

The 16-hex token after the index is the sub-hunk's **content id** (see
[Selectors](#selectors)).

**JSON schema** (`--json`): array of file objects, each with `path`, `binary`, and
`hunks` (array of sub-hunk objects with `index`, `id`, `id_count`, `old_start`,
`old_lines`, `new_start`, `new_lines`, `added`, `deleted`, `header`, `preview`).
`id_count` is how many sub-hunks across the whole patch share that `id` (`1` = unique).

Binary files are listed with `"binary": true` and an empty `hunks` array.

### select

Emit only the chosen sub-hunks as a valid unified diff.

```sh
# Select sub-hunks 1 and 3 from a single-file diff
git diff src/main.rs | hunkpick select 1,3 | git apply --cached

# Select sub-hunks from specific files in a multi-file diff
git diff | hunkpick select src/main.rs:1,3 src/lib.rs:2 | git apply --cached

# Same when the diff is taken over an explicit file list (git diff file1 file2 fileN).
# With more than one file, every selector must carry a path: prefix (a bare index is
# only allowed for a single-file diff).
git diff src/a.rs src/b.rs src/c.rs | hunkpick select src/a.rs:1,3 src/c.rs:2-4 | git apply --cached

# Select a range
git diff path | hunkpick select path:2-4 | git apply --cached

# Select every sub-hunk of a file (or the whole single-file diff)
git diff | hunkpick select src/main.rs:* | git apply --cached
git diff src/main.rs | hunkpick select '*' | git apply --cached

# Select by content id (from `list --json`), stable across re-diffs
git diff | hunkpick select @8002dd73f0dfd2f4 | git apply --cached

# Content ids work across a multi-file diff too: the file path is part of the id, so
# an id addresses the change in its own file (the same edit elsewhere gets another id).
git diff src/a.rs src/b.rs src/c.rs | hunkpick select @8002dd73f0dfd2f4 | git apply --cached

# Several ids at once, mixed with path: selectors. Read the ids from `list --json` first
# (the machine-readable form, intended for tooling):
git diff | hunkpick list --json
git diff | hunkpick select @8002dd73f0dfd2f4 @bf7bdaaf30c1e2d4 src/lib.rs:2 | git apply --cached
```

A binary file referenced by any selector index is emitted whole.

### split

Split one original hunk (addressed by its 1-based index over the file's original
hunks, before auto-splitting) at specified new-file line numbers. The line numbers
must fall on context lines. The result is the complete patch with that hunk replaced
by the pieces.

```sh
# Split original hunk 1 in a single-file diff at new-file line 5
git diff src/lib.rs | hunkpick split 1 --at 5

# Same for a named file in a multi-file diff
git diff | hunkpick split src/lib.rs:1 --at 5,12

# With git verification
git diff src/lib.rs | hunkpick split 1 --at 5 --verify-result-diff-git -C /path/to/repo
```

### Staging recipe

```sh
# 1. Inspect what sub-hunks are available
git diff path/to/file.rs | hunkpick list --json

# 2. Stage only sub-hunks 1 and 3
git diff path/to/file.rs | hunkpick select 1,3 | git apply --cached
```

Splitting one file's mixed changes into several semantic commits, addressing
sub-hunks by content id. Bare indices renumber after each staging, but a `@<id>`
stays valid across the re-diff (see [Content ids](#content-ids)), so the listing
is captured once and never re-read:

```sh
# 1. Capture the ids once. `id_count` flags any id that selects more than one.
git diff src/indicator.js | hunkpick list --json

# 2. Stage and commit each group by @id (one or more ids each), re-running git
#    diff each round. The ids from step 1 remain valid even though staging
#    renumbers the bare indices.
git diff src/indicator.js | hunkpick select @bf7bdaaf30c1e2d4 | git apply --cached
git commit -m "fix: ..."

git diff src/indicator.js | hunkpick select @058b36528575a870 @399e1cd421e268cc | git apply --cached
git commit -m "feat: ..."

# 3. Whatever is left is the last group; `*` takes the remaining sub-hunks.
git diff src/indicator.js | hunkpick select '*' | git apply --cached
git commit -m "chore: ..."
```

## Selectors

Selectors are passed as positional arguments to `select`. Each selector addresses
sub-hunks within one file by their 1-based per-file index as reported by `list`.

| Form               | Meaning                                               |
|--------------------|-------------------------------------------------------|
| `1,3`              | Sub-hunks 1 and 3 (bare list, only for single-file diffs) |
| `2-4`              | Sub-hunks 2, 3, and 4 (bare range, single-file only) |
| `*`                | Every sub-hunk (bare `*`, single-file only)          |
| `src/foo.rs:1,3`   | Sub-hunks 1 and 3 within `src/foo.rs`                |
| `src/foo.rs:2-4`   | Sub-hunks 2 through 4 within `src/foo.rs`            |
| `src/foo.rs:*`     | Every sub-hunk of `src/foo.rs`                        |
| `@<id>`            | Every sub-hunk whose content id equals `<id>`         |

Multiple selectors can be combined: `hunkpick select src/a.rs:1 src/b.rs:2,3`.

Path matching checks both the old and new path of a file diff entry. A bare index
list or `*` (no `path:` prefix) is accepted only when the diff contains exactly one
file; otherwise `hunkpick` exits with code 2.

Selectors are matched in order of precedence: a `path:set` form is recognised first
(so a file literally named `@foo` is still addressable as `@foo:1`), then `@id`, then
a bare set.

### Content ids

`list` reports a 16-hex **content id** for every sub-hunk, also accepted by `select`
as `@<id>`. The id is a hash of the file paths and the sub-hunk's **changed (`+`/`-`)
lines only** — **not** its context lines, the `@@` line numbers, or the section header.
Ids are matched case-insensitively. Because the file path is part of the hash, ids
work across a multi-file diff: an `@<id>` addresses the change in its own file, and the
same edit applied to a different file gets a different id.

Because only the changed lines feed the id, it is stable across a re-diff in every
common case of an iterative `diff → stage → re-diff` loop:

- An unrelated edit elsewhere that only shifts this change's line numbers leaves its id
  unchanged.
- Staging a neighbouring sub-hunk — which rewrites this change's surrounding context, or
  causes the enclosing hunk to be re-split — also leaves its id unchanged, because the
  context is not part of the id.

So positional indices renumber as you stage changes, but a change's `@<id>` does not:
capture it once from `list` and keep using it across the loop without re-reading the
listing. The id changes only when the change's own `+`/`-` lines change.

Because context is excluded, two changes with **identical `+`/`-` lines** share an id
even if their surrounding context differs; `@<id>` then selects all of them. `list
--json` reports `id_count` (how many sub-hunks share the id), so a consumer can tell up
front whether `@<id>` is unique (`id_count == 1`) or would select several; to address
just one of several identical changes, use `path:N`. If an id is ever shared by
sub-hunks whose changed lines actually differ (an accidental hash collision), `select`
reports it and exits with code 2 — address those by `path:N`.

For the `split` subcommand the hunk address uses the same `path:N` / `N` form, but
`N` refers to the 1-based index over the file's **original** hunks (not auto-split
sub-hunks). `split` does not accept `*` or `@id`.

### Splitting an addition-only block: `INDEX@RANGE`

A sub-hunk that is all additions — a block of new functions appended to a file, or a
file-creation diff (`@@ -0,0 +1,N @@`) — is one atomic sub-hunk: auto-split has no context
line inside it to cut at. To stage part of such a block, address it with a per-line range:

```
[path:]INDEX@RANGE
```

`INDEX` is the 1-based sub-hunk index from `list`. **Only a numeric index may precede `@`** —
content ids (`@id`) and `*` are not accepted here. `RANGE` numbers the sub-hunk's **added (`+`)
lines**, 1-based:

| Form    | Meaning                          |
|---------|----------------------------------|
| `lo-hi` | added lines `lo` through `hi`    |
| `lo-`   | from `lo` to the last added line |
| `-hi`   | from the first added line to `hi` |
| `N`     | a single added line (`N-N`)      |

The cut is allowed only between two added lines; cutting where the boundary is a context or
deletion line is an error. `list` marks freely-splittable sub-hunks (`addition_only` in
`--json`, `[+range]` in the human listing).

Example — split a new file across two commits:

```sh
git diff src/lib.rs | hunkpick list                       # the block shows +N and the [+range] marker
git diff src/lib.rs | hunkpick select 1@1-90 | git apply --cached && git commit -m 'feat: part one'
git diff src/lib.rs | hunkpick select 1@91-  | git apply --cached && git commit -m 'feat: part two'
```

## Verification

### Internal consistency check (default)

After `select` or `split`, `hunkpick` verifies the result diff for internal
consistency: `@@` header counts match the body line counts, hunks within each file
are ordered, and their old-file ranges do not overlap. This check runs by default and
requires no git repository.

To disable it:

```sh
git diff path | hunkpick select 1 --no-verify-result-diff-internal
```

### Git apply check (optional)

Pass `--verify-result-diff-git` to additionally run `git apply --check` on the result
diff before emitting it. This confirms the diff applies cleanly to the working tree.

```sh
git diff path | hunkpick select 1 --verify-result-diff-git
```

Use `-C <DIR>` to specify the working tree directory (default: current directory).
`-C` requires `--verify-result-diff-git`; passing `-C` alone is a usage error.

```sh
git diff path | hunkpick select 1 --verify-result-diff-git -C /path/to/repo
```

### Verification failure

On any verification failure, `hunkpick` writes a diagnostic to stderr, writes
nothing to stdout, and exits with code **70**.

## Input handling

### Source

By default the diff is read from stdin. `-i, --input FILE` reads from a file instead;
`-i -` is an explicit stdin. The flag is available on every subcommand and may appear
after it:

```sh
hunkpick list --input changes.diff
hunkpick select 1,3 -i changes.diff | git apply --cached
git diff | hunkpick select 1,3            # stdin (default)
```

### Size limit

Input (from stdin or a file) is capped at **64 MiB** by default to guard against an
accidentally unbounded stream. Exceeding the limit is a usage error (exit code 2).
Override with `--max-input-bytes N`; `0` disables the limit.

```sh
hunkpick list --max-input-bytes 268435456 -i huge.diff   # raise to 256 MiB
hunkpick list --max-input-bytes 0 -i huge.diff           # no limit
```

Note: the working-set memory is several times the input size (the input buffer, the
parsed model, and the emitted diff coexist), so a 64 MiB input corresponds to a few
hundred MiB of peak RAM. Lower the limit if you run in a memory-constrained environment.

### Validation

`hunkpick` reads the input as raw bytes and validates it before parsing:

- **Empty or whitespace-only input** is a no-op: nothing is written and the exit code
  is 0, for every subcommand.
- **Binary input** (any NUL byte) is rejected with a diagnostic and exit code 2.
- **Text with no diff marker** (no line starting with `diff --git `, `--- `, `+++ `,
  `@@ `, or `Binary files `) is rejected with exit code 2.

Valid diff content is never decoded as UTF-8 internally, so lines in any byte encoding
(or with invalid UTF-8) pass through unchanged.

## Auto-split and non-overlap

`hunkpick` decomposes each hunk into sub-hunks automatically at boundaries between
adjacent change runs. A "change run" is a maximal contiguous sequence of `+`/`-`
lines. Context lines between change runs become the split boundary.

**Non-overlap guarantee**: sub-hunk old-file ranges are strictly non-overlapping.
The boundary context (lines between two change runs) becomes the *trailing* context
of the earlier sub-hunk. The later sub-hunk starts directly at its change run, with
no leading copy of the boundary context.

This differs from `git add -p`, which can share context between adjacent hunks
because it applies each hunk individually. `hunkpick select` emits all selected
sub-hunks as a single combined patch applied in one `git apply` call; overlapping
old-file ranges would cause `git apply` to reject the patch.

**Round-trip property**: selecting all sub-hunks for a file produces a diff that
applies equivalently to the original hunk. The output is not byte-identical to the
original (one hunk becomes several), but the applied result is the same.

## Exit codes

| Code | Meaning                                                         |
|------|-----------------------------------------------------------------|
|    0 | Success                                                         |
|    2 | Usage error: bad flag, bad selector, parse error, binary/non-diff input, input over size limit |
|   70 | Verification failure (internal consistency or `git apply --check`) |
|   74 | I/O error (reading stdin or writing stdout)                    |
|  130 | Interrupted (SIGINT or SIGTERM, default signal disposition)    |

## Comparison to filterdiff

| Capability                                      | filterdiff | hunkpick |
|-------------------------------------------------|:----------:|:--------:|
| Binary file pass-through                        |     ✅     |    ✅    |
| Select whole hunks from a diff                  |     ✅     |    ✅    |
| Works with any diff source (not git-specific)   |     ✅     |    ✅    |
| Address sub-hunks by per-file index             |     ❌     |    ✅    |
| Auto-split hunks at change-run boundaries       |     ❌     |    ✅    |
| Built-in result verification                    |     ❌     |    ✅    |
| Explicit hunk split at a named line             |     ❌     |    ✅    |
| Machine-readable listing (JSON)                 |     ❌     |    ✅    |
| Split an addition-only block by line range      |     ❌     |    ✅    |

## Development

Contributions are welcome. The crate has no build-time code generation and no external
runtime dependencies, so the standard cargo workflow applies.

```sh
# Run the full test suite (unit + integration + doc tests).
cargo test --all-features

# Lint with all warnings denied (the CI gate).
cargo clippy --all-targets --all-features -- -D warnings

# Check formatting (CI verifies this; use `cargo fmt --all` to apply).
cargo fmt --all --check

# Verify the code still builds on the minimum supported Rust version (1.85).
cargo +1.85 build --all-features
```

The CI workflow ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs the same
checks, using [`cargo-nextest`](https://nexte.st/) for the unit/integration tests and
`cargo test --doc` for doc tests. Test runner limits (per-test timeout and thread count)
live in [`.config/nextest.toml`](.config/nextest.toml); please keep tests fast and
hermetic — several tests shell out to `git apply --check` and require `git` on `PATH`.

## License

MIT. See [LICENSE](LICENSE).
