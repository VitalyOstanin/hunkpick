# TODO

Open items only. What has been done is recorded in [CHANGELOG.md](CHANGELOG.md) and, for the
decisions behind it, in [docs/ADR/](docs/ADR/README.md) — this file used to repeat both and went
stale as a result.

## Contents

- [Per-line cutting: remaining edges](#per-line-cutting-remaining-edges)
- [Listing: deferred review items](#listing-deferred-review-items)

## Per-line cutting: remaining edges

- **Multiple `@L` pieces of the same sub-hunk in one invocation.** Currently a
  usage error: separate pieces would carry mutually inconsistent new-side line
  numbers. Combining them in one emitted diff would need each piece's new-side
  anchor recomputed against the intermediate file the earlier pieces produce. The
  supported path today is the `diff → stage → re-diff` loop (one piece per round).
  Lift this only if a single-invocation multi-piece cut proves worth the anchor
  bookkeeping.

- **Genuinely zero-context edges.** The convert-unselected-deletions-to-context
  rule removes most zero-context cases, but a context-less run (a whole-file
  replacement, a file creation/deletion) can still yield a piece git needs
  `--unidiff-zero` for. If such cases matter, add an explicit `--unidiff-zero`
  opt-in (git does not content-verify those hunks, so keep it off by default).

### Fundamental limits (out of scope)

- a single changed line is the atom — half a line cannot be staged;
- a unified diff does not record which deletion pairs with which addition, so a
  "semantically correct" split of a replacement is inherently ambiguous;
- some intermediate states are unbuildable — a property of any line-wise staging.

## Listing: deferred review items

Raised by the review of `list --lines` / `list --only` and left out of that change on purpose.

- **Say in the human listing that an id is shared.** `id_count` exists only in `--json`. Before
  `--only`, a repeated id was visible by eye on neighbouring lines; a narrowed listing hides the
  other bearers, so `select @<id>` can take a sub-hunk the listing never showed. A marker beside
  a shared id would say it in the form a person reads, at the cost of a column that means
  nothing on the overwhelming majority of listings.

- **Check the README's output blocks against a real run.** The blocks that show `list` output are
  prose today: `tests/docs_examples.rs` reads only `sh` blocks, and nothing compares a printed
  listing with what the binary prints. One example already drifted that way (a preview line that
  the tool cannot produce). A check would have to pin the content ids the listing shows, which
  is why it is not a two-line test.

- **Positional `jq` recipes break on a narrowed JSON listing.** `hunks[7]` is the eighth listed
  sub-hunk, not sub-hunk 8, once `--only` is in play. The listing carries `index` for exactly
  this reason; whether the README should stop showing positional recipes altogether is open.

- **Smaller points from the same review.** `write_changed_lines` panics through `unreachable!`
  (exit 101) where the crate's contract is an exit code; `ListOptions` is not `#[non_exhaustive]`,
  so adding a field is breaking for an external caller that builds it literally; there is no
  shell-completion generation (`clap_complete`); the `@L` out-of-range message could name
  `list --lines --only N`, the command that prints the numbering.
