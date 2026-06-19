# ADR 0006 — Input source selection and size limit

Date: 2026-06-19

## Status

Accepted

## Context

Two input-handling needs were raised after the initial implementation:

1. **Read from a file, not only stdin.** Callers want `hunkpick ... -i changes.diff`
   without a shell redirection or `cat`.
2. **Bound the input size.** A pure stdin filter will read an arbitrarily large stream
   into memory. An accidental unbounded source (a wrong pipe, a huge generated diff)
   should fail fast rather than exhaust RAM.

**Argument placement.** A positional `[FILE]` argument cannot be used: `select` takes a
variadic positional list of selectors and `split` takes a positional hunk address, so a
positional file would be ambiguous. The input source must therefore be a flag.

**Memory cost.** The core is not zero-copy: the parser copies each line into an owned
`Vec<u8>`, and `select` / `split` clone the chosen hunks into a new `Patch` that `emit`
serialises into a fresh `Vec<u8>`. At peak, the raw input buffer, the parsed model, the
output `Patch`, and the emitted bytes can all be live. For a 64 MiB input this is on the
order of a few hundred MiB of peak RAM (the multiple depends on average line length and
allocator overhead). The size *limit* therefore does not equal the RAM ceiling; it is a
linear control over it.

## Decision

- **Source flag**: `-i, --input FILE` on every subcommand reads the diff from `FILE`.
  `-i -` and the absence of the flag both mean stdin. The flag is flattened into each
  subcommand (via an `InputOpts` clap group), so it may appear after the subcommand:
  `hunkpick select 1,3 -i changes.diff`.
- **Size limit**: `--max-input-bytes N` caps the input at `N` bytes (default
  `DEFAULT_MAX_INPUT_BYTES` = 64 MiB). `N = 0` disables the limit. The cap applies
  uniformly to stdin and file input. Exceeding it is a usage error (exit code 2) with a
  diagnostic naming the limit and the override flag.
- **Enforcement**: the reader uses `Read::take(limit + 1)` and reports an error when more
  than `limit` bytes arrive, so an oversized stream is not fully buffered before being
  rejected, and an input of exactly `limit` bytes is accepted.
- **Buffer lifetime**: reading, validation, and parsing are done in a helper that returns
  only the parsed `Patch`. The raw input `Vec<u8>` is dropped when that helper returns, so
  it does not coexist on the heap with the output `Patch` and emitted bytes during
  `select` / `split` / `emit`. This removes one input-sized copy from the peak.
- A missing or unreadable input file is an I/O error (exit code 74), distinct from the
  size-limit usage error.

## Consequences

- `hunkpick` can read a diff from a file directly, composing with tools that write diffs
  to disk, without changing the default stdin behaviour.
- An unbounded or oversized input fails fast at a predictable byte count instead of
  driving the process into the OOM killer.
- The default 64 MiB covers very large real diffs while bounding worst-case RAM;
  memory-constrained callers can lower it, and callers who intentionally process huge
  diffs can raise or disable it.
- The limit is a byte count, not a RAM figure; the README documents the multiplier so the
  default is not mistaken for a 64 MiB memory ceiling.
- A future zero-copy parser (borrowing slices of the input buffer instead of copying)
  would cut the multiple but is out of scope here; this ADR's buffer-drop decision is the
  cheap partial mitigation.
