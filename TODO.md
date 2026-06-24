# TODO

## Contents

- [Classify overlapping-selection errors as usage errors](#classify-overlapping-selection-errors-as-usage-errors)

## Classify overlapping-selection errors as usage errors

### Problem

When a user passes selectors that overlap within one sub-hunk — a whole sub-hunk
and a range of it (`select 1 1@1-2`), or two overlapping ranges of one sub-hunk
(`select 1@1-3 1@2-4`) — `select` emits overlapping hunks. The downstream
`validate_internal` correctly rejects the result (`OverlappingHunks`), so no bad
patch is produced, but it surfaces as `AppError::Verify` ("internal consistency
check failed", exit code 70) rather than a usage error (exit code 2). The message
reads as an internal bug instead of pointing at the conflicting arguments.

Non-overlapping ranges of the same sub-hunk (`select 1@1-2 1@3-4`) are legitimate
and must keep working, so a plain "duplicate index" rejection is wrong — the check
must compare the actual covered line ranges.

### Requirement

Detect, in `select`, when the chosen pieces of one sub-hunk overlap (a `Whole`
plus any range of the same index, or two ranges with intersecting added-line
spans) and return a usage-level error naming the conflicting selectors, before the
result reaches `validate_internal`. Keep non-overlapping ranges of the same
sub-hunk valid.
