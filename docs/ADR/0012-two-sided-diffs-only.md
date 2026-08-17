# ADR 0012 — Only two-sided unified diffs; a combined diff is rejected

Date: 2026-08-17

## Status

Accepted

## Context

`git diff` for a merge commit, `git show` of a merge, and `git diff` against the index during a
conflict all write a *combined* diff: `diff --cc <path>` (or `diff --combined`) instead of
`diff --git a/… b/…`, and hunk headers with one `@@` per parent — `@@@ -1,4 -1,6 +1,5 @@@` for two
parents. Body lines carry one marker column per parent, so `- x` and `-- x` mean different things,
and `--- removed in both` is a body line, not a file header.

hunkpick's model is two-sided: a hunk has an old side and a new side, a body line is context, an
addition or a deletion, and `--- ` / `+++ ` start a file entry. Fed a combined diff, the parser
read the first `@@@` line as a header with a trailing `@` and stopped the body at the first line
whose marker it did not recognise. What followed was filed as leading headers, `--- removed in
both` was taken for a file marker, and the result was an entry invented out of body text. The tool
exited 0 and emitted a diff that describes something the input never said.

Supporting combined diffs properly is not a parsing detail: a selection would have to name a
parent, `git apply` does not accept a combined diff at all (`git apply` refuses `--cc` output), and
the whole `diff → stage → re-diff` workflow has no meaning against a merge.

## Decision

- **A combined diff is unsupported input, detected and refused.** A `diff --cc` / `diff --combined`
  file header, or a hunk header opening with `@@@`, is a usage error: exit code 2 with a diagnostic
  naming the form, before any hunk is parsed.
- **The refusal is part of input validation**, not of result verification: the input is at fault,
  not hunkpick's output, so it belongs with the other exit-2 rejections (binary input, non-diff
  text, oversized input) rather than with the exit-70 checks of ADR 0004.
- The two-sided model stays as it is. No parent-aware addressing, no `-c`/`--cc` mode.

## Consequences

- Piping `git show <merge>` into hunkpick reports what is wrong instead of producing a diff that
  silently drops most of the change. The failure is loud and immediate.
- Someone who wants to stage part of a merge resolution still can: `git diff` against the index in
  a conflicted worktree writes an ordinary two-sided diff once the file is resolved, and that is
  what hunkpick reads.
- If combined diffs are ever supported, it will be an additive decision recorded in a new ADR —
  the addressing grammar would have to name a parent, which the current syntax has no room for.
- The check costs one comparison per file header and per hunk header, so it does not affect the
  linear parsing cost of ordinary input.
