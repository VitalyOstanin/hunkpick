# Contributing to hunkpick

Thanks for your interest in improving `hunkpick`. This document describes how to build
the project, run the checks, and submit changes.

## Development environment

- A Rust toolchain meeting the project's minimum supported version (**1.85**). The
  [`rust-toolchain.toml`](rust-toolchain.toml) pins the components (`rustfmt`, `clippy`).
  The MSRV check below runs on that exact version, so install it once:
  `rustup toolchain install 1.85`.
- `git` on `PATH`: several integration tests shell out to `git apply --check`, so they
  require a working `git` binary.
- [`cargo-nextest`](https://nexte.st/): the documented way to run the tests, locally and in
  CI. Its profile bounds per-test time and parallelism; plain `cargo test` has neither.

## Development loop

Run these before opening a pull request; they mirror the CI gates in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml):

```sh
cargo t                                                     # unit + integration tests (nextest)
cargo t-doc                                                 # doc tests (nextest does not run them)
cargo clippy --all-targets --all-features -- -D warnings    # lint, warnings denied
cargo fmt --all --check                                     # formatting (apply with `cargo fmt --all`)
cargo +1.85 build --all-features                            # MSRV build
```

`t` and `t-doc` are aliases defined in [`.cargo/config.toml`](.cargo/config.toml). Test runner
limits (per-test timeout and thread count) live in
[`.config/nextest.toml`](.config/nextest.toml), and they apply only under nextest. Without
`cargo-nextest` installed, the fallback is `cargo test --all-features -- --test-threads=4` —
it still has no per-test timeout, so a hung test has to be interrupted by hand. Keep tests fast
and hermetic.

## Generated tests

Three kinds of test run beside the hand-written ones. They exist because the defects that
survived several reviews were found by generating inputs, not by reading code.

| № | Where                     | What it does                                                                                          | Cost                          |
|---|---------------------------|-------------------------------------------------------------------------------------------------------|-------------------------------|
| 1 | `tests/differential.rs`   | Generates diffs with real git and asserts hunkpick agrees with it: a selection applies, staging one sub-hunk at a time converges, the output is valid input | seconds; part of `cargo t`    |
| 2 | `tests/property.rs`       | Generates diffs directly, including forms git will not produce on demand (CRLF, missing final newline, a mail preamble), and checks round-trip, idempotence and the internal invariants | fast; part of `cargo t`        |
| 3 | `fuzz/`                   | libFuzzer targets over arbitrary bytes: parsing must not panic, `parse . emit` must be a fixed point, a successful selection must pass its own checks | open-ended; run on demand     |

Cases 1 and 2 are deterministic: a failure names the seed (differential) or shrinks to the
smallest failing shape (property), so it reproduces. The differential suite runs 200 generated
cases by default (30 on Windows, where process spawns cost an order of magnitude more);
`HUNKPICK_DIFF_CASES=2000 cargo t` soaks it harder without editing the source. A shrunk case that turns out to be a real
defect is worth adding to the hand-written tests as well — `proptest-regressions` files are
generated locally and not committed, because a saved seed says nothing about what broke.

The fuzz targets need nightly (libFuzzer uses `-Z` flags) and a C++ toolchain, so they are not
part of the normal loop:

```sh
cargo +nightly fuzz run parse -- -max_total_time=60      # one target, one minute
cargo +nightly fuzz run parse fuzz/artifacts/parse/<id>  # replay a crash
```

CI runs a 60-second smoke pass of each target on every push, and
[`.github/workflows/fuzz.yml`](.github/workflows/fuzz.yml) runs a longer weekly search that
keeps its corpus between runs.

## Releases

Cutting a release is described in [`RELEASING.md`](RELEASING.md): what the pipeline checks,
how to prepare the release commit, and how to rehearse it with a dry run.

## Pull requests

- Keep each pull request focused on a single concern.
- Add or update tests for any behaviour change. The codebase follows a test-first
  approach: a change to parsing, selection, splitting, or emission should come with a
  test that exercises it, and where applicable a `git apply --check` round-trip.
- Update [`CHANGELOG.md`](CHANGELOG.md) under an `Unreleased` section when your change is
  user-visible (new flag, changed output, bug fix).
- Adding or removing a dependency changes the third-party notices shipped in release
  archives. They are generated at release time by
  [`scripts/generate-notices.sh`](scripts/generate-notices.sh) (needs
  [`cargo-about`](https://github.com/EmbarkStudios/cargo-about)); if the new crate's license is
  not in the `accepted` list of [`about.toml`](about.toml), generation fails — review the
  license before adding it there.
- Update the [README](README.md) and, for design decisions, add an
  [ADR](docs/ADR/README.md) when the change alters externally observable behaviour or a
  core invariant.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/) (`fix:`, `feat:`,
`refactor:`, `docs:`, `ci:`, `test:`, `chore:`). Write messages in English, imperative
mood, with a body explaining the why when it is not obvious from the subject.

The release commit has one fixed spelling: `chore(release): X.Y.Z` (see
[`RELEASING.md`](RELEASING.md)). Earlier releases used `chore: release X.Y.Z (vX.Y.Z)`; that
form is retired, so the history reads uniformly from 0.6.0 onwards.

## Reporting issues

Open a GitHub issue with a minimal reproduction: the input diff (or a redacted excerpt),
the exact command line, the observed output and exit code, and what you expected instead.
