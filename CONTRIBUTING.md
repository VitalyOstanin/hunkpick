# Contributing to hunkpick

Thanks for your interest in improving `hunkpick`. This document describes how to build
the project, run the checks, and submit changes.

## Development environment

- A Rust toolchain meeting the project's minimum supported version (**1.85**). The
  [`rust-toolchain.toml`](rust-toolchain.toml) pins the components (`rustfmt`, `clippy`).
- `git` on `PATH`: several integration tests shell out to `git apply --check`, so they
  require a working `git` binary.
- Optionally [`cargo-nextest`](https://nexte.st/) for a faster test run; CI uses it, but
  `cargo test` works as well.

## Development loop

Run these before opening a pull request; they mirror the CI gates in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml):

```sh
cargo test --all-features                                   # unit + integration + doc tests
cargo clippy --all-targets --all-features -- -D warnings    # lint, warnings denied
cargo fmt --all --check                                     # formatting (apply with `cargo fmt --all`)
cargo +1.85 build --all-features                            # MSRV build
```

Test runner limits (per-test timeout and thread count) live in
[`.config/nextest.toml`](.config/nextest.toml). Keep tests fast and hermetic.

## Pull requests

- Keep each pull request focused on a single concern.
- Add or update tests for any behaviour change. The codebase follows a test-first
  approach: a change to parsing, selection, splitting, or emission should come with a
  test that exercises it, and where applicable a `git apply --check` round-trip.
- Update [`CHANGELOG.md`](CHANGELOG.md) under an `Unreleased` section when your change is
  user-visible (new flag, changed output, bug fix).
- Update the [README](README.md) and, for design decisions, add an
  [ADR](docs/ADR/README.md) when the change alters externally observable behaviour or a
  core invariant.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/) (`fix:`, `feat:`,
`refactor:`, `docs:`, `ci:`, `test:`, `chore:`). Write messages in English, imperative
mood, with a body explaining the why when it is not obvious from the subject.

## Reporting issues

Open a GitHub issue with a minimal reproduction: the input diff (or a redacted excerpt),
the exact command line, the observed output and exit code, and what you expected instead.
