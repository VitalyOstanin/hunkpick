# ADR 0004 — Two-tier result-diff verification: internal by default, git apply on demand

Date: 2026-06-19

## Status

Accepted

## Context

`select` and `split` both produce a result diff. Several failure modes can produce a
malformed or inapplicable result:

1. A bug in hunkpick's split or emit logic produces incorrect `@@` counts.
2. Sub-hunks within a file are emitted out of order or with overlapping old-file
   ranges, causing `git apply` to reject the patch.
3. The result diff is internally consistent but does not apply to the working tree
   (e.g. the file has since changed).

Two verification strategies address these at different cost and dependency levels:

**Internal consistency check**: verifies `@@` header counts against body line counts,
hunk ordering within each file, and non-overlap of old-file ranges. Requires no git
repository and runs in microseconds. Catches categories 1 and 2.

**`git apply --check`**: runs `git apply --check` on the result diff against the
working tree. Requires git and a repository. Catches all three categories, including
category 3 (stale working tree). Slower and has an external dependency.

## Decision

- The **internal consistency check runs by default** after `select` and `split`.
  It can be disabled with `--no-verify-result-diff-internal` when the caller has
  already verified the result or needs maximum throughput.
- The **`git apply --check` verification is opt-in** via `--verify-result-diff-git`.
  It is useful in scripts that need confirmation the result applies before piping it
  further.
- The `-C <DIR>` flag sets the working tree directory for the git check (default:
  current directory). `-C` is declared in clap as requiring `--verify-result-diff-git`;
  passing `-C` without the git flag is a usage error (exit code 2), not a silent
  no-op.
- On any verification failure: a diagnostic is written to stderr, nothing is written
  to stdout, and the process exits with code **70**.

## Consequences

- By default, `hunkpick` catches its own internal errors (miscounted `@@`, overlap)
  without requiring git, making it usable as a pure diff-processing library.
- The default check adds negligible latency and no external dependencies.
- Users who want the strongest guarantee (the patch will apply) can add
  `--verify-result-diff-git` at the cost of one extra `git apply --check` invocation.
- Disabling the internal check (`--no-verify-result-diff-internal`) is available for
  performance-sensitive pipelines where the caller controls the input quality.
- The dependency constraint that `-C` requires `--verify-result-diff-git` is enforced
  by clap at argument-parse time, providing a clear error message before any diff
  processing begins.
