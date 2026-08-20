# hunkpick

[![crates.io](https://img.shields.io/crates/v/hunkpick.svg)](https://crates.io/crates/hunkpick)
[![docs.rs](https://docs.rs/hunkpick/badge.svg)](https://docs.rs/hunkpick)
[![CI](https://github.com/VitalyOstanin/hunkpick/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/VitalyOstanin/hunkpick/actions/workflows/ci.yml?query=branch%3Amaster)
[![codecov](https://codecov.io/gh/VitalyOstanin/hunkpick/graph/badge.svg?branch=master)](https://codecov.io/gh/VitalyOstanin/hunkpick)
[![license](https://img.shields.io/crates/l/hunkpick.svg)](https://github.com/VitalyOstanin/hunkpick/blob/master/LICENSE)

Non-interactive unified-diff hunk picker and splitter — a pure stdin→stdout filter for
staging subsets of changes without interactive prompts. It is the scriptable,
non-interactive alternative to `git add -p`: pick or split hunks by index, range, or
content id inside a pipeline, with no prompts and machine-readable output.

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
  - [Splitting by individual changed lines: `INDEX@L<set>`](#splitting-by-individual-changed-lines-indexlset)
- [Verification](#verification)
- [Input handling](#input-handling)
- [Auto-split and non-overlap](#auto-split-and-non-overlap)
- [Exit codes](#exit-codes)
- [Comparison to filterdiff](#comparison-to-filterdiff)
- [Development](#development)
- [License](#license)

## Why / Motivation

Git's built-in way to stage a subset of hunks is the interactive `git add -p`. It
prompts for each hunk, so it cannot be driven from a script, a Makefile, or an
automated coding agent. The standard **non-interactive** substitute for `git add -p`
has been [`filterdiff`](https://linux.die.net/man/1/filterdiff) from the
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
- **Correct anchors for a partial selection**: leaving a sub-hunk out changes how many
  lines the result adds or removes, so every later hunk's new-side (`+`) start is
  recomputed from the emitted hunks alone. `git apply` searches from that position; a
  value carried over from the input diff would drift and, where the surrounding context
  repeats, land the change in the wrong place.
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

Colour in the default `--color auto` mode follows stdout: on when it is a
terminal, off when piped. The `NO_COLOR` environment variable (any non-empty
value) forces it off; `CLICOLOR_FORCE` (any non-empty value) forces it on even
when piped. `NO_COLOR` takes precedence when both are set. An explicit
`--color always|never` overrides all of these.

**Example human output:**

```
src/main.rs
  [1] 114ccaaa7ce6c0f1 @@ -10,4 +10,4 @@  +1 -1  +let x = 1;
  [2] 8002dd73f0dfd2f4 @@ -20,6 +20,6 @@  +1 -1  +fn bar() {
```

The 16-hex token after the index is the sub-hunk's **content id** (see
[Selectors](#selectors)). Each line then shows the hunk header, the `+N -M` change
counts, and a preview of the first changed line. A sub-hunk that only adds lines (a
file creation or a pure append, with no context and no deletions) is flagged `[+add]`
between the counts and the preview — the same property the JSON listing reports as
`addition_only`:

```
src/new_file.rs
  [1] 3f1c0a52d7b94e68 @@ -0,0 +1,12 @@  +12 -0 [+add]  +fn main() {
```

Text a terminal would act on rather than show — escape sequences, control bytes,
bidirectional overrides — is escaped in this listing (`\x1b`, `\u{202e}`), so a diff
being filtered cannot repaint or reorder what you are reading.

**The JSON listing does not do this.** Its text fields (`path`, `header`, `preview`,
`changed_lines[].text`) reproduce the diff's own bytes: JSON escaping covers control
characters, but a bidirectional override survives it and comes back out of any parser as the
character it was. That is deliberate — the machine-readable mode reports what the diff says,
not a display-safe rendering of it — so **a consumer that prints these fields to a terminal
must escape them itself**. Prefer the human listing when a person is reading the output.

All four are also lossy for a path or a line that is not valid UTF-8 (legal on
Unix): JSON must be UTF-8, so undecodable bytes become `U+FFFD`. A path taken from `list
--json` therefore does not necessarily round-trip back into a `path:N` selector. For such a
file, address the sub-hunk by its content id (`@id`), which is computed over the raw bytes and
is exact.

**JSON schema** (`--json`): array of file objects, each with `path`, `binary`, and
`hunks` (array of sub-hunk objects with `index`, `id`, `id_count`, `old_start`,
`old_lines`, `new_start`, `new_lines`, `added`, `deleted`, `addition_only`,
`changed_lines`, `header`, `preview`). `id_count` is how many sub-hunks across the whole
patch share that `id` (`1` = unique). `addition_only` is `true` when the sub-hunk is all
additions (a file-creation or pure-append block). `changed_lines` is the sub-hunk's
changed (`+`/`-`) lines in body order, each `{ i, kind, text }`: `i` is the 1-based index
for `select INDEX@L<set>`, `kind` is `"add"` or `"del"`. The `i` indices are positional —
they renumber after each staged round, unlike the sub-hunk `id`, so re-run `list --json`
each round (there is no stable per-line id).

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

# With git verification. The check reads the working tree, so it belongs on a patch file
# against a tree at the pre-patch state — not on a `git diff |` pipeline, where it would
# reject a correct result. See "Git apply check (optional)".
hunkpick split 1 --at 5 -i patch.diff --verify-result-diff-git -C /path/to/clean/checkout
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
| `1@L<set>`         | Cut sub-hunk 1 to a set of changed lines (see [Splitting by individual changed lines](#splitting-by-individual-changed-lines-indexlset)) |

Multiple selectors can be combined: `hunkpick select src/a.rs:1 src/b.rs:2,3`.

Path matching checks both the old and new path of a file diff entry. A bare index
list or `*` (no `path:` prefix) is accepted only when the diff contains exactly one
file; otherwise `hunkpick` exits with code 2.

The path part of a selector is compared as raw bytes, so a file whose name is not valid
UTF-8 (legal on Unix) stays addressable — pass the name exactly as the shell holds it. Only
the set after the `:` has to be ASCII. Git's `core.quotePath` (on by default) writes such
names quoted and C-escaped in the diff; hunkpick decodes them, so the selector always spells
the real name, not the escaped form.

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

### Splitting by individual changed lines: `INDEX@L<set>`

To stage part of one sub-hunk — including an atomic addition-only block (a block of new
functions appended to a file, or a file-creation diff `@@ -0,0 +1,N @@`) that auto-split
has no internal context line to cut at — address a subset of its **changed (`+`/`-`)
lines**:

```
[path:]INDEX@L<set>
```

`INDEX` is the 1-based sub-hunk index from `list`. **Only a numeric index may precede `@`** —
content ids (`@id`) and `*` are not accepted here. `<set>` starts with `L` and then numbers
the sub-hunk's changed lines `1..N` in body order — deletions and additions share one
numbering, exactly as `list --json` reports them under `changed_lines`. The set is a
comma-separated list of indices and ranges, e.g. `L1,3` or `L1-2,4`.

Each unselected deletion is kept as a context line and each unselected addition is omitted,
and both leading and trailing context of the sub-hunk are retained. A subset is therefore
realisable as a single hunk with no boundary restriction: a deletion surrounded by additions
(`+x -y +z`) can be isolated, and a replacement's removals can be separated from its
insertions.

Two cases are outside that:

1. **An entry that deletes the file entirely** (`+++ /dev/null`, or a `deleted file mode`
   header). Turning an unselected deletion into a context line would say the file still has
   that line after the patch, which contradicts the entry. A partial `@L` on such an entry is
   a usage error (exit 2); selecting all of its changed lines is fine.
2. **A piece that ends up with no context at all** — a whole-file replacement, a file creation
   or deletion. There is nothing to anchor it to, so `git apply` needs `--unidiff-zero` for
   it.

Example — split an addition-only block across two commits, one piece per round:

```sh
git diff src/lib.rs | hunkpick list                      # the block shows +N and the [+add] marker
git diff src/lib.rs | hunkpick select 1@L1-90 | git apply --cached && git commit -m 'feat: part one'
git diff src/lib.rs | hunkpick select 1@L1-30 | git apply --cached && git commit -m 'feat: part two'
```

The second round asks for `1-30`, not `91-120`: it runs a fresh `git diff`, which shows only
the 30 lines the first round left unstaged, numbered from 1 again. Reusing the first round's
numbers is a usage error (exit 2, "changed-line index 120 is out of range"). Selecting the
whole remainder — `1` or `*` — works as well.

A sub-hunk addressed by `@L` must be addressed **once per invocation**: combining it with
another `@L`, or with a whole selection of the same sub-hunk, is a usage error
(exit 2) — the pieces would carry inconsistent line numbers. Stage further pieces in
later `diff → stage → re-diff` rounds. A partial `@L` on an entry that deletes the file is a
usage error for the reason given above.

Example — separate a replacement's removals from its insertions. `list --json` shows the
changed lines and their indices:

```json
"changed_lines": [
  { "i": 1, "kind": "del", "text": "a" },
  { "i": 2, "kind": "del", "text": "b" },
  { "i": 3, "kind": "add", "text": "A" },
  { "i": 4, "kind": "add", "text": "B" }
]
```

```sh
git diff src/lib.rs | hunkpick select 1@L1,2 | git apply --cached && git commit -m 'remove a, b'
git diff src/lib.rs | hunkpick select 1@L1,2 | git apply --cached && git commit -m 'add A, B'
```

The second round re-runs `git diff`: after the deletions are committed the sub-hunk holds
only the two additions, now numbered 1 and 2.

## Verification

### Internal consistency check (default)

After `select` or `split`, `hunkpick` verifies the result diff for internal
consistency: `@@` header counts match the body line counts, hunks within each file
are ordered, their old-file ranges do not overlap, and each hunk's new-side (`+`) start
follows from its old-side start plus the net size of the hunks emitted before it. This
check runs by default and requires no git repository.

To disable it:

```sh
git diff path | hunkpick select 1 --no-verify-result-diff-internal
```

### Git apply check (optional)

Pass `--verify-result-diff-git` to additionally run `git apply --check` on the result
diff before emitting it. This confirms the diff applies cleanly to the working tree.

**Read that literally: to the working tree.** `git apply --check` without `--index` compares
against the files on disk, not against the index. In the staging pipeline this README is built
around — `git diff | hunkpick select ... | git apply --cached` — the working tree already
contains the edits the diff describes, so git reports `patch does not apply` and hunkpick exits
70 for a result that is perfectly correct. The flag is for the case where the tree *is* at the
state the diff expects: checking a patch file against a clean checkout, or pointing `-C` at
such a tree. It is not a routine safety net for the staging loop — the internal check above is,
and it runs by default.

```sh
# The tree is at the pre-patch state: the check is meaningful here.
hunkpick select 1 -i patch.diff --verify-result-diff-git -C /path/to/clean/checkout
```

Use `-C <DIR>` to specify the working tree directory the check runs against (default:
current directory). `-C` requires `--verify-result-diff-git`; passing `-C` alone is a usage
error.

```sh
hunkpick select 1 -i patch.diff --verify-result-diff-git -C /path/to/clean/checkout
```

### Verification failure

On any verification failure, `hunkpick` writes a diagnostic to stderr, writes
nothing to stdout, and exits with code **70**. A `git` that cannot be started at all is a
different matter — the check never ran, so that is exit **74**, not a verdict on the diff.

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

Run without a pipe and without `-i` and hunkpick reads the terminal, as any filter does, after
writing one line to stderr saying so — a paste-and-Ctrl-D still works, but a forgotten pipe no
longer looks like a hang. In a pipeline stdin is not a terminal, so nothing is printed.

### What the input may be

hunkpick reads a two-sided unified diff: what `git diff`, `git format-patch`, `diff -u`, and
Mercurial or Subversion produce. Within that, the input is passed through unchanged — what
comes in comes back out byte for byte, including CRLF endings, a `\ No newline at end of file`
marker, the mail head and footer of a `format-patch` output, a full binary patch
(`git diff --binary`), and a diff that arrived without a final newline.

The diff has to arrive as a UTF-8 (or otherwise ASCII-compatible) byte stream. A UTF-8 BOM is
harmless and comes back out unchanged, but a UTF-16 or UTF-32 one is refused with a message
naming the encoding — `git diff > patch.diff` in Windows PowerShell 5.1 produces UTF-16LE, and
`iconv -f UTF-16LE -t UTF-8` makes it readable. hunkpick does not re-encode input: the whole
point of the byte-for-byte pass-through is that what comes out is what went in.

One format is deliberately not read: the **combined diff** git writes for a merge commit
(`git show <merge>`, `git diff --cc`, `@@@` headers). Its body has one marker column per
parent, so a sub-hunk of it is not a two-sided change and cannot be addressed or sliced.
hunkpick rejects such input as a usage error (exit code 2) rather than reading it as
something it is not.

### Size limit

Input (from stdin or a file) is capped at **64 MiB** by default to guard against an
accidentally unbounded stream. Exceeding the limit is a usage error (exit code 2).
Override with `--max-input-bytes N`; `0` disables the limit.

The limit bounds the input, not the memory: hunkpick keeps the whole parsed diff in memory
with one allocation per line, so peak RSS is a multiple of the input size, and the multiple
grows as the average line gets shorter. Measured on a release build (`/usr/bin/time -f %M`):

| Input                          | `select '*'` | `list --json` |
|--------------------------------|-------------:|--------------:|
| 18 MB, lines of ~13 bytes      |    196 MiB   |      349 MiB  |
| 61 MiB, lines of ~60 bytes     |    341 MiB   |      534 MiB  |
| 63 MiB (at the limit), ~40 B   |    427 MiB   |      713 MiB  |

That is 6x–19x the input, and the run at the default limit takes well under a second
(0.49 s and 0.88 s for the row above). Plan for around a gigabyte when the lines are short,
and raise `--max-input-bytes` only with that in mind.

```sh
hunkpick list --max-input-bytes 268435456 -i huge.diff   # raise to 256 MiB
hunkpick list --max-input-bytes 0 -i huge.diff           # no limit
```

Note: the working-set memory is several times the input size (the input buffer, the
parsed model, and the emitted diff coexist), so a 64 MiB input corresponds to a few
hundred MiB of peak RAM. Lower the limit if you run in a memory-constrained environment.

One other limit exists and is not configurable: the selectors of one invocation may name at
most **1 048 576** (2^20) indices *between them*, so `1-99999999` is a usage error (exit code 2)
rather than an allocation, and so is a long list of smaller ranges that adds up past the
ceiling. The allowance is shared because the number of selectors is bounded only by the length
of the command line. It is far above the sub-hunk count of any real diff — there is no
legitimate reason to raise it, hence no flag.

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
|   70 | Verification failure (internal consistency, or `git apply --check` rejecting the result) |
|   74 | I/O error: reading stdin, writing stdout, or being unable to run `git` for `--verify-result-diff-git` |
|  130 | Interrupted by SIGINT (default signal disposition: 128 + 2)     |
|  143 | Terminated by SIGTERM (default signal disposition: 128 + 15)    |

A reader that closes the pipe first (`hunkpick list | head`) is not an error: the write
ends the run with code 0, so the tool composes with `set -o pipefail`.

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
| Split any sub-hunk by individual changed lines  |     ❌     |    ✅    |

## Development

Contributions are welcome. The crate has no build-time code generation and no external
runtime dependencies, so the standard cargo workflow applies.

```sh
# Run the unit and integration tests through nextest (the documented runner).
cargo t

# Run the doc tests; nextest does not execute them.
cargo t-doc

# Lint with all warnings denied (the CI gate).
cargo clippy --all-targets --all-features -- -D warnings

# Check formatting (CI verifies this; use `cargo fmt --all` to apply).
cargo fmt --all --check

# Verify the code still compiles on the minimum supported Rust version (1.85).
# `--all-targets` includes the tests, so the dev-dependencies are checked too.
cargo +1.85 check --all-targets --all-features
```

`t` and `t-doc` are aliases from [`.cargo/config.toml`](https://github.com/VitalyOstanin/hunkpick/blob/master/.cargo/config.toml); the CI workflow
([`.github/workflows/ci.yml`](https://github.com/VitalyOstanin/hunkpick/blob/master/.github/workflows/ci.yml)) runs the same checks with
[`cargo-nextest`](https://nexte.st/) for the unit/integration tests and `cargo test --doc` for
doc tests. Test runner limits (per-test timeout and thread count) live in
[`.config/nextest.toml`](https://github.com/VitalyOstanin/hunkpick/blob/master/.config/nextest.toml) and apply only under nextest; without it
installed, use `cargo test --all-features -- --test-threads=4`. Please keep tests fast and
hermetic — several tests shell out to `git apply --check` and require `git` on `PATH`.

`cargo t` includes generated tests: a differential suite that compares hunkpick with real git
over generated diffs, and property tests over shapes git will not produce on demand. The fuzz
targets in [`fuzz/`](https://github.com/VitalyOstanin/hunkpick/blob/master/fuzz) need nightly and are run separately, through
[`scripts/fuzz-all.sh`](https://github.com/VitalyOstanin/hunkpick/blob/master/scripts/fuzz-all.sh) (`FUZZ_SECONDS=60 scripts/fuzz-all.sh parse`
for one target, one minute); CI builds each target on every push and runs a longer search twice
a week. See [`CONTRIBUTING.md`](https://github.com/VitalyOstanin/hunkpick/blob/master/CONTRIBUTING.md) for what each kind covers, why the
command needs a toolchain override, an explicit triple, a corpus directory and a hang timeout,
and how a failure is reproduced.

## License

MIT. See [LICENSE](LICENSE).

The released binary statically links third-party crates (all MIT / Apache-2.0 / Unicode-3.0).
Their license texts and copyright notices ship inside every release archive as
`THIRD-PARTY-NOTICES.md`, generated at release time by
[`scripts/generate-notices.sh`](https://github.com/VitalyOstanin/hunkpick/blob/master/scripts/generate-notices.sh) from
[`about.toml`](https://github.com/VitalyOstanin/hunkpick/blob/master/about.toml). Installing from crates.io needs no such file — cargo resolves the
dependencies' own licenses from `Cargo.lock`.
