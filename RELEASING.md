# Releasing hunkpick

The release pipeline is [`.github/workflows/release.yml`](.github/workflows/release.yml). It is
triggered by pushing a `v*` tag (or by `workflow_dispatch` on a tag that already exists) and
enforces that five artefacts agree: the tag, `Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.lock` and
`CHANGELOG.md`. This document is the checklist for producing a release that satisfies those
checks on the first run.

## Contents

- [What the pipeline does](#what-the-pipeline-does)
- [Preparing the release commit](#preparing-the-release-commit)
- [Rehearsing with a dry run](#rehearsing-with-a-dry-run)
- [Cutting the release](#cutting-the-release)
- [If something fails](#if-something-fails)

## What the pipeline does

Jobs run in this order; everything reversible happens before the one step that is not
(`cargo publish` can only be undone with `cargo yank`, which does not free the version):

| № | Job                | What it does                                                                   |
|---|--------------------|--------------------------------------------------------------------------------|
| 1 | `verify-metadata`  | Resolves the tag and checks it against the manifest, both locks and the changelog, before anything is compiled; every other job waits on it |
| 2 | `test`             | Test suite on ubuntu / macOS / Windows at the tagged commit                     |
| 3 | `lint`             | `clippy -D warnings`, `cargo fmt --check`, docs build                           |
| 4 | `msrv`             | `cargo check --locked --all-targets` on the minimum supported Rust version      |
| 5 | `semver`           | `cargo semver-checks check-release`: the public API against the released version |
| 6 | `package-binaries` | Builds each target, generates the notices, packs and verifies every archive     |
| 7 | `publish`          | `cargo publish --locked`, then creates the GitHub Release from the changelog    |
| 8 | `upload-assets`    | Attaches the archives (built in step 6) to that Release                         |

Steps 2 to 5 run in parallel; `package-binaries` waits on 1 to 4, and `publish` on everything
before it. The tests, lints and gates duplicate CI on purpose: the workflow is triggered by a tag
push, so a green run on `master` says nothing about the commit being released.

Consistency checks inside `verify-metadata`: the tag matches
`v<MAJOR>.<MINOR>.<PATCH>[-pre+build]` ([`scripts/release-validate-tag.sh`](scripts/release-validate-tag.sh)),
the version in `Cargo.toml` equals the tag, both `Cargo.lock` and `fuzz/Cargo.lock` record that
same version for the `hunkpick` package, and `CHANGELOG.md` has a `## [X.Y.Z]` section
([`scripts/check-changelog.sh`](scripts/check-changelog.sh)).

## Preparing the release commit

1. Refresh the `dtolnay/rust-toolchain` pins. They point at commit SHAs of the rolling
   `stable` and `1.85.0` refs, which Dependabot deliberately ignores, so they drift silently:

   ```sh
   gh api repos/dtolnay/rust-toolchain/branches/stable --jq '.commit.sha'
   gh api repos/dtolnay/rust-toolchain/branches/1.85.0 --jq '.commit.sha'
   ```

   Update `.github/workflows/{ci,release}.yml` if either differs from what is pinned there.
2. Make sure `master` is green and up to date with `origin`.
3. Bump `version` in [`Cargo.toml`](Cargo.toml).
4. Refresh the lock entry in both workspaces so each carries the new version. The fuzz
   workspace has its own lock, and leaving it behind makes every cargo command in `fuzz/`
   rewrite it later:

   ```sh
   cargo update -p hunkpick
   (cd fuzz && cargo update -p hunkpick)
   ```

5. In [`CHANGELOG.md`](CHANGELOG.md), turn the `## [Unreleased]` section into
   `## [X.Y.Z] - YYYY-MM-DD` (ASCII hyphen, ISO date) and add a fresh empty `Unreleased`
   section above it. Add the new version to the table of contents at the top; the file has no
   link definitions at the bottom — its entries link to the heading anchors.
6. Verify locally:

   ```sh
   cargo t && cargo t-doc                                      # tests
   cargo clippy --all-targets --all-features -- -D warnings    # lint
   cargo fmt --all --check                                     # formatting
   cargo +1.85 check --all-targets --all-features              # MSRV
   cargo semver-checks check-release                           # public API vs the released version
   bash scripts/check-changelog.sh X.Y.Z                       # changelog section exists
   cargo publish --dry-run --locked                            # package contents
   ```

   The semver check is run here, after step 3 raised the version: it compares this tree against
   the last version on crates.io and passes only if the number covers what the API did. Running
   it before the bump reports the change against the version it is replacing. The full local
   loop, including the two commands that lint the `fuzz` workspace, is in
   [CONTRIBUTING.md](CONTRIBUTING.md).

7. Commit as `chore(release): X.Y.Z` and push to `master`.

## Rehearsing with a dry run

Before tagging, the whole pipeline can be exercised without publishing anything: run the Release
workflow manually (`gh workflow run release.yml --ref master`) and leave the `tag` input **empty**.
The version is then read from `Cargo.toml`, the run checks out the selected branch, and it builds
every target, generates the third-party notices and verifies each archive — but skips
`cargo publish`, the Release creation and the asset upload.

Do not rehearse by pushing the tag: the trigger is `push: tags: ['v*']`, so the push *is* the
release, and `cargo publish` is irreversible. The `dry_run: true` input applies to the other manual
form — a run that names an existing tag — and is for re-checking a tag that is already public.

To reproduce the packaging step on your own machine:

```sh
cargo build --release
bash scripts/generate-notices.sh                                    # needs cargo-about
VER=X.Y.Z TARGET=x86_64-unknown-linux-gnu ARCHIVE_EXT=tar.gz \
  BIN_NAME=hunkpick bash scripts/package-archive.sh
bash scripts/verify-archive.sh hunkpick-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz hunkpick
```

## Cutting the release

```sh
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

The tag push starts the pipeline. Its first job, `Verify tag, manifest, locks and CHANGELOG`,
takes seconds and stops the run before anything is compiled if the tag, `Cargo.toml`, either
lockfile or the CHANGELOG disagree. When the pipeline finishes, check that the Release carries one
archive plus one `.sha256` per target and that `cargo binstall hunkpick` picks up the new version.

There is no approval step: pushing the tag publishes, with `CARGO_REGISTRY_TOKEN` read from the
repository secrets. That is a deliberate trade — a mistaken tag burns a version number on
crates.io, which is cheaper here than a manual approval on every release. To change it, put the
token in a GitHub Environment with a required reviewer and name that environment on the `publish`
job; the run then pauses before the one irreversible step.

## If something fails

- **Before `publish`** (jobs 1–6): nothing is public. Fix the problem on `master`, delete the
  tag locally and remotely (`git push origin :refs/tags/vX.Y.Z`), and re-tag the new commit.
- **After `publish`** (jobs 7–8): the crates.io version is permanent. A failed
  `upload-assets` can simply be re-run — the upload is idempotent (`--clobber`), and the
  archives are kept as workflow artifacts for 7 days. If the published version itself is
  broken, `cargo yank --version X.Y.Z` stops new dependents from picking it up, and the fix
  ships as the next patch version.
