# Architecture Decision Records

This directory records the significant design decisions behind `hunkpick` as
[Nygard-style](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions)
ADRs. Each file is immutable once accepted; a later decision that changes course is
added as a new ADR that supersedes the earlier one rather than editing it in place.

| ADR                                            | Title                                                                 | Status   | Date       |
|------------------------------------------------|-----------------------------------------------------------------------|----------|------------|
| [0001](0001-pure-diff-filter.md)               | hunkpick is a pure stdin→stdout unified-diff filter                   | Accepted | 2026-06-19 |
| [0002](0002-per-file-hunk-addressing.md)       | Per-file sub-hunk addressing with `path:index` selector syntax       | Accepted | 2026-06-19 |
| [0003](0003-non-overlapping-auto-split.md)     | Non-overlapping auto-split: boundary context goes to the earlier sub-hunk | Accepted | 2026-06-19 |
| [0004](0004-result-diff-verification.md)       | Two-tier result-diff verification: internal by default, git apply on demand | Accepted | 2026-06-19 |
| [0005](0005-byte-core-and-input-validation.md) | Byte-oriented core and input validation                              | Accepted | 2026-06-19 |
| [0006](0006-input-source-and-size-limit.md)    | Input source selection and size limit                                | Accepted | 2026-06-19 |
| [0007](0007-content-id-and-wildcard-addressing.md) | Content-id (`@id`) and wildcard (`*`) sub-hunk addressing        | Accepted | 2026-06-19 |
| [0008](0008-added-line-range-addressing.md)        | Added-line range addressing (`INDEX@RANGE`) in `select`          | Superseded by 0009 | 2026-06-24 |
| [0009](0009-changed-line-addressing-supersedes-range.md) | Changed-line addressing (`INDEX@L<set>`) supersedes and removes `INDEX@RANGE` | Accepted | 2026-07-11 |

## Adding a new ADR

1. Copy the structure of an existing record: a `# ADR NNNN — <title>` heading, a
   `Date:` line, then `## Status`, `## Context`, `## Decision`, and `## Consequences`
   sections.
2. Use the next free zero-padded number.
3. Add a row to the table above.
