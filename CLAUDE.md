# hunkpick — project guidance

`hunkpick` is a non-interactive unified-diff hunk picker and splitter: a pure
stdin→stdout filter with no network, database, async runtime, or UI. It ships both a
library (`src/lib.rs`) and a binary (`src/main.rs`).

## Conventions

- All committed content (code, comments, tests, docs, commit messages) is in **English**.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/);
  do not add `Co-Authored-By` trailers.
- The core processes the diff as **raw bytes** end to end; only paths and previews shown
  by `list` are decoded lossily. Keep new code byte-oriented — do not assume UTF-8.
- Minimum supported Rust version is **1.85** (pinned in `clippy.toml` and
  `rust-toolchain.toml`); design decisions in `cargo +1.85 build` must keep passing.

## Development loop

See [CONTRIBUTING.md](CONTRIBUTING.md). In short: `cargo test --all-features`,
`cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all --check`.
Some integration tests require `git` on `PATH`.

## Project decisions

- **No `CODE_OF_CONDUCT.md`.** Intentionally omitted: this is a single-maintainer
  utility crate. Contribution norms live in [CONTRIBUTING.md](CONTRIBUTING.md); a separate
  code of conduct is not maintained. Do not add one without an explicit request.
- Design rationale is captured as ADRs in [docs/ADR/](docs/ADR/README.md). Add a new ADR
  when changing externally observable behaviour or a core invariant.
