# ADR 0005 — Byte-oriented core and input validation

Date: 2026-06-19

## Status

Accepted

## Context

Two correctness gaps were identified after the initial implementation.

**Encoding.** The original core read stdin with `read_to_string` and stored line
content, hunk sections, headers, and paths as `String`. `String` requires valid UTF-8,
so any diff containing non-UTF-8 bytes — a latin-1 file, a shift-JIS file, or any
arbitrary byte sequence git happily diffs as text — was rejected at read time with
`stream did not contain valid UTF-8` (exit 74). A diff filter must not corrupt or
reject content based on its encoding: git itself treats lines as bytes and only looks
for the `\n` separator.

Empirically (git 2.53.0): a valid multibyte UTF-8 file round-tripped, but a single
latin-1 byte `0xE9` on a content line aborted the program before any processing.

**Input validation.** The tool parsed any input as a (possibly empty) diff. Feeding a
binary file or unrelated text produced either an empty result or partial garbage rather
than a clear diagnostic. A non-interactive tool whose first consumer is an automated
agent should fail fast and unambiguously on input that is not a unified diff.

## Decision

**Byte-oriented core.** All diff content is modelled and processed as raw bytes:

- `Line.text`, `Hunk.section`, `FileDiff.headers`, `FileDiff.old_path`,
  `FileDiff.new_path`, and `FileContent::Binary` hold `Vec<u8>` (or `Vec<Vec<u8>>`).
- `parse` takes `&[u8]`; `emit` returns `Vec<u8>`.
- `main` reads stdin with `read_to_end` and writes raw bytes to stdout.
- Structural tokens (`diff --git `, `@@ `, the `-`/`+`/` ` line markers, the numeric
  hunk-header ranges) are ASCII and are matched as byte literals. The numeric range
  portion of a hunk header is the only part decoded as UTF-8, and only to parse
  integers; a non-ASCII range is reported as a bad hunk header.
- Paths and previews surfaced by `list` / `list --json` are decoded with
  `String::from_utf8_lossy` for display and addressing only. This never affects the
  emitted diff, which stays byte-exact.

**Input validation.** Before parsing, `main` classifies the raw input:

- Empty or whitespace-only input is a no-op: nothing is written, exit code 0, for every
  subcommand.
- Input containing a NUL byte is treated as binary and rejected (exit code 2).
- Non-empty input with no line beginning with a known diff marker
  (`diff --git `, `--- `, `+++ `, `@@ `, `Binary files `) is rejected (exit code 2).

## Consequences

- Diffs in any encoding, including invalid UTF-8, round-trip byte-for-byte. A regression
  test feeds a diff with a lone `0xE9` byte and asserts the byte survives selection.
- The tool is usable as a general diff filter, not just for UTF-8 content.
- Malformed input fails fast with exit code 2 and a one-line diagnostic instead of
  silently producing empty or partial output.
- Empty-input no-op keeps the tool composable in pipelines where an upstream `git diff`
  legitimately produces nothing.
- The empty-input short-circuit runs before selector validation, so `select <sel>` on
  empty input exits 0 rather than reporting a selector error. This is intentional: no
  input means no work, regardless of the selector.
- `list` paths/previews are display-decoded lossily, so a path with invalid UTF-8 shows
  replacement characters in the listing while the emitted diff retains the original
  bytes.
