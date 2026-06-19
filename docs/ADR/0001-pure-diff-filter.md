# ADR 0001 — hunkpick is a pure stdin→stdout unified-diff filter

Date: 2026-06-19

## Status

Accepted

## Context

`hunkpick` needs to select and split hunks from a unified diff. The design space
includes two broad approaches:

1. A git-integrated tool that calls `git diff` internally and writes directly to the
   git index.
2. A pure filter that reads a unified diff from stdin and writes a unified diff to
   stdout, leaving application to the caller.

Option 1 would couple the tool to git: it would require a repository, would not work
with diffs from Mercurial, SVN, or plain `diff -u`, and would need its own staging
logic (`git apply --cached`), complicating the implementation and reducing
composability.

## Decision

`hunkpick` is a pure stdin→stdout filter. It:

- reads a unified diff from stdin,
- performs selection or splitting,
- writes the result diff to stdout.

It does not call `git diff` itself. Applying the result diff to the index is the
caller's responsibility, typically via `git apply --cached` or `git apply`. The
optional `--verify-result-diff-git` flag shells out to `git apply --check` purely
for verification (no side effects on the index).

## Consequences

- Works with any diff source: git, Mercurial, SVN, plain `diff -u`, or diffs stored
  in files.
- No repository coupling: the tool can run outside a git working tree (except when
  `--verify-result-diff-git` is requested).
- Composable in shell pipelines: `git diff path | hunkpick select 1,3 | git apply --cached`.
- Application semantics (cached vs. working tree, `--3way`, etc.) remain under the
  caller's control.
- The tool does not need to understand git object storage, refs, or config.
