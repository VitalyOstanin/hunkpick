# TODO

Open items only. What has been done is recorded in [CHANGELOG.md](CHANGELOG.md) and, for the
decisions behind it, in [docs/ADR/](docs/ADR/README.md) — this file used to repeat both and went
stale as a result.

## Contents

- [Per-line cutting: remaining edges](#per-line-cutting-remaining-edges)

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
