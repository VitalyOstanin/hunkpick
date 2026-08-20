# Contributing to hunkpick

Thanks for your interest in improving `hunkpick`. This document describes how to build
the project, run the checks, and submit changes.

## Contents

- [Development environment](#development-environment)
- [Development loop](#development-loop)
- [Generated tests](#generated-tests)
- [Releases](#releases)
- [Branch protection](#branch-protection)
- [Pull requests](#pull-requests)
- [Commit messages](#commit-messages)
- [Reporting issues](#reporting-issues)

## Development environment

- A Rust toolchain meeting the project's minimum supported version (**1.85**). The
  [`rust-toolchain.toml`](rust-toolchain.toml) pins the components (`rustfmt`, `clippy`).
  The MSRV check below runs on that exact version, so install it once:
  `rustup toolchain install 1.85`.
- `git` on `PATH`: several integration tests shell out to `git apply --check`, so they
  require a working `git` binary.
- [`cargo-nextest`](https://nexte.st/): the documented way to run the tests, locally and in
  CI. Its profile bounds per-test time and parallelism; plain `cargo test` has neither.
- [`cargo-semver-checks`](https://github.com/obi1kenobi/cargo-semver-checks): the public API
  gate below. It needs a network connection — it fetches the released version from crates.io.

## Development loop

Run these before opening a pull request; they mirror the CI gates in
[`.github/workflows/ci.yml`](.github/workflows/ci.yml):

```sh
cargo t                                                     # unit + integration tests (nextest)
cargo t-doc                                                 # doc tests (nextest does not run them)
cargo clippy --all-targets --all-features -- -D warnings    # lint, warnings denied
cargo fmt --all --check                                     # formatting (apply with `cargo fmt --all`)
cargo +1.85 check --all-targets --all-features              # MSRV check (incl. dev-deps)
cargo semver-checks check-release                           # public API vs the released version
cargo fmt --manifest-path fuzz/Cargo.toml --all --check     # the fuzz workspace, separately
cargo clippy --manifest-path fuzz/Cargo.toml --all-targets -- -D warnings
```

The two `--manifest-path fuzz/Cargo.toml` lines are not redundant: `fuzz/Cargo.toml` declares a
`[workspace]` of its own, so neither `--all` above reaches it (`cargo metadata --no-deps` in the
root lists one member). CI lints it as a separate step for the same reason, and a fuzz target
edited without these passes the local loop and fails the pull request.

`cargo semver-checks check-release` needs
[`cargo-semver-checks`](https://github.com/obi1kenobi/cargo-semver-checks)
and a network connection: it builds the crate twice — this tree and the version on crates.io —
and reports any incompatibility the version in `Cargo.toml` does not account for. Before 1.0 a
breaking change is allowed, provided the minor version goes up with it; that is what the check
enforces, and CI gates on it.

The gate checks the version number, not the record. A change to the public API also belongs in
the `Changed (library API)` section of `CHANGELOG.md` — that section is the only description of
the contract a consumer has, since the README documents the command line and not the crate. A
green gate says the number is right, and says nothing about whether the change was written down.

Two modules are outside that contract and marked `#[doc(hidden)]`: `cli`, the argument types
clap derives the command line from, and `gitenv`, the git plumbing the tests share with the tool.
The binary and the integration tests are separate crates and cannot see `pub(crate)` items, so
`pub` is the only way to reach them — but a new CLI flag is a new public field, and without the
attribute `cargo semver-checks` would call it a breaking change of the library. Add to them
freely; anything else public is a promise.

The versions of the tools CI installs (`cargo-nextest`, `cargo-about`, `cargo-fuzz`,
`cargo-llvm-cov`, `cargo-semver-checks`) are pinned in the `tool:` inputs of
`taiki-e/install-action` across [`ci.yml`](.github/workflows/ci.yml),
[`release.yml`](.github/workflows/release.yml) and
[`fuzz-setup`](.github/actions/fuzz-setup/action.yml). Without a pin an upstream release lands
in the next run and can turn CI red with no change in this repository; bump them deliberately,
the same way action SHAs are bumped.

`t` and `t-doc` are aliases defined in [`.cargo/config.toml`](.cargo/config.toml). Test runner
limits (per-test timeout and thread count) live in
[`.config/nextest.toml`](.config/nextest.toml), and they apply only under nextest. Without
`cargo-nextest` installed, the fallback is `cargo test --all-features -- --test-threads=4` —
it still has no per-test timeout, so a hung test has to be interrupted by hand. Keep tests fast
and hermetic.

Everything the binary prints stays ASCII: help, errors, hints. Rust writes stdout and stderr as
UTF-8 whatever the console is set to, and the release ships an `x86_64-pc-windows-msvc` binary,
so a console on cp866 or cp1251 turns a typographic dash into mojibake mid-sentence. Write `--`
for a dash, `"` for quotes and `...` for an ellipsis. Two tests hold the rule:
`every_help_text_is_ascii` (`src/cli.rs`) renders every help page, and
`no_source_string_literal_leaves_ascii` (`tests/ascii_output.rs`) scans the string literals of
`src/` outside the test modules. Test fixtures are exempt on purpose -- a path such as `é.txt`
is exactly the input the parser has to carry through.

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
`HUNKPICK_DIFF_CASES=2000` soaks it harder without editing the source (a value that is not a
number between 1 and 1000000 is reported on stderr and the default is used), but the count and the
per-test timeout have to move together — at 2000 cases the slowest of those tests takes about
45 s where it takes 4.5 s at the default, and the profile kills a test at 60 s, which reads as a
hang rather than as a finished soak:

```sh
HUNKPICK_DIFF_CASES=2000 cargo nextest run --all-features \
  --slow-timeout period=300s,terminate-after=2
```
 A shrunk case that turns out to be a real
defect is worth adding to the hand-written tests as well — `proptest-regressions` files are
generated locally and not committed, because a saved seed says nothing about what broke.

The fuzz targets need nightly (libFuzzer uses `-Z` flags) and a C++ toolchain, so they are not
part of the normal loop. Two scripts hold the command:

```sh
scripts/fuzz-all.sh                          # every target, five minutes each
FUZZ_SECONDS=60 scripts/fuzz-all.sh parse    # one target, one minute
scripts/fuzz-repro.sh                        # replay every crash under fuzz/artifacts/
```

They are scripts rather than a command to copy because four parts of that command are not
obvious, and each costs a confusing failure on its own:

- `RUSTUP_TOOLCHAIN=nightly`, not `cargo +nightly`. [`rust-toolchain.toml`](rust-toolchain.toml)
  pins the repository to stable and that file wins over an installed toolchain, so cargo-fuzz
  ends up handing the build to stable rustc, which rejects the `-Z` sanitizer flags.
- `--target x86_64-unknown-linux-gnu` spelled out. cargo-fuzz defaults to the triple it was
  itself built for, and the prebuilt binary `cargo binstall cargo-fuzz` installs is a static
  musl build; ASan cannot work against a statically linked libc, so the default fails with
  `sanitizer is incompatible with statically linked libc`.
- `mkdir -p fuzz/corpus/<target>` before the run. The corpus is gitignored, so a fresh clone
  does not carry it, and libFuzzer refuses to start when the writable corpus directory is
  missing — it does not create it. This is what the first scheduled run in CI failed on.
- `-timeout=10`. The libFuzzer default is 1200 s, so one hung input would silently eat the
  whole budget of a short run; parsing an input of the size libFuzzer generates takes
  milliseconds, which makes ten seconds a wide margin and still reports a hang as a crash.

`fuzz/seeds/` holds a committed starting point per target (see
[`fuzz/seeds/README.md`](fuzz/seeds/README.md)) and `fuzz/dictionaries/diff.dict` the tokens a
diff is made of; `fuzz/corpus/` is machine-local and grows as the fuzzer works.

CI builds every target on each push (advisory: the nightly it needs is whatever shipped today),
and [`.github/workflows/fuzz.yml`](.github/workflows/fuzz.yml) runs a search twice a week that
keeps its corpus between runs.

## Releases

Cutting a release is described in [`RELEASING.md`](RELEASING.md): what the pipeline checks,
how to prepare the release commit, and how to rehearse it with a dry run.

## Branch protection

`master` is not protected, and changes land on it by direct push. That is deliberate for a
single-maintainer project: a required-status gate would put a pull request and a wait in front of
every change without adding a reviewer. The consequence is equally deliberate — CI is a signal,
not a gate. A run can go red on a commit that is already in the branch, and did: a test that
built a command line too long for Windows failed after the push, not before it.

So the rule that replaces the gate is: a red run on `master` is fixed by the next commit, not by
the next batch of work. GitHub notifies the author of the push by default; that notification is
the whole mechanism.

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
