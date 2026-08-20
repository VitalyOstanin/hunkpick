# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Contents

- [Unreleased](#unreleased)
- [0.8.1](#081---2026-08-18)
- [0.8.0](#080---2026-08-17)
- [0.7.0](#070---2026-07-29)
- [0.6.0](#060---2026-07-21)
- [0.5.1](#051---2026-07-21)
- [0.5.0](#050---2026-07-11)
- [0.4.0](#040---2026-07-05)
- [0.3.1](#031---2026-06-25)
- [0.3.0](#030---2026-06-24)
- [0.2.2](#022---2026-06-20)
- [0.2.1](#021---2026-06-20)
- [0.2.0](#020---2026-06-19)
- [0.1.0](#010---2026-06-19)

## [Unreleased]

### Fixed

- A hunk header whose ranges were separated by something other than one ASCII space — a tab, two
  spaces, or a non-ASCII whitespace character such as U+00A0 — parsed, and the header was then
  rebuilt on the way out with a plain space. A diff git calls a corrupt patch came back out as
  one git accepts, at exit 0. Such a header is now refused as malformed (exit 2), the same as any
  other header hunkpick cannot reproduce byte for byte.
- The sign of a hunk range was not checked at all: `@@ +1,3 +1,3 @@`, `@@ -1,3 1,3 @@`,
  `@@ -+1,3 +1,3 @@` and `@@ -1,+3 +1,3 @@` all parsed and were rendered back as
  `@@ -1,3 +1,3 @@` at exit 0, while git reads every one of them as a corrupt patch. The leading
  `-` and `+` are now required, and a range component has to be plain ASCII digits.
- Selecting by a content id shared by several sub-hunks scaled as the square of the number of
  matches, in both time and memory. Reading every id out of `list --json` and passing them back
  — the documented batch flow — names such an id once per sub-hunk, and each naming both
  appended a pick per match and re-ran the collision check over every match. A diff of 8 000
  identical changes (150 KB) took 10 s and 3 GB of RSS; it is now a fraction of a second. The
  same accumulator defect was reachable through a repeated `*`.
- The cap on how many indices a selector may materialise (`1048576`) applied to one selector
  while the number of selectors is bounded only by the length of the command line: 200 copies of
  `1-1048576` — 2.6 KB of arguments against a four-sub-hunk diff — reached 1.6 GB of RSS before
  any index was compared against the diff. The cap is now one allowance for the whole
  invocation, and its message says so.
- `split` now applies the same trailing-newline rule as `select`. A diff that arrived without
  its final newline keeps that property only while the result still ends on the line the input
  ended on, and a cut that drops the piece holding the tail does not: `hunkpick split 1 --at 3`
  on such a diff ended its output mid-line at exit 0, and `git apply` called the result a
  corrupt patch. `select` was taught this in 0.8.1; `split` was not.
- The defect a scheduled fuzzing run found — a hunk header declaring six old lines over a body
  of three, whose selection then failed the tool's own output check — was closed by making the
  fuzz target skip such input, and nothing recorded it afterwards. The precondition is now
  stated on `select`, the crate example screens its input the way the CLI does, and
  `select_carries_an_inconsistent_header_into_its_result` holds the shape, which until now
  existed only in one machine's `fuzz/artifacts/`.
- The `selectors` fuzz target recounts a hunk header from its body instead of skipping the
  input. Skipping cost roughly a third of the parsed stream — the mutations libFuzzer favours
  break the header-to-body correspondence more often than not — so most of what it generated no
  longer reached the selection code the target exists for.
- Fuzz seeds are checked by `tests/fuzz_seeds.rs`, which found three that had stopped reaching
  the code they were written for: `simple-content-id` named a made-up id, `two-files-path-index`
  named a path absent from its own diff, and `two-files-star` used a bare `*` on a multi-file
  diff, which is a usage error. Each was silently skipped by the target rather than reported. A
  seed for the `@id` collision path was added, and the byte-identical `parse/` and `roundtrip/`
  seed directories were merged into one `diff/`.
- The property tests reach two shapes they could not generate before: an edit whose changed
  lines repeat (so several sub-hunks share a content id, and a selection by that id is checked
  against `id_count` from `list --json`), and a `git format-patch` signature after the last
  hunk (so the trailing-line shift in `split_file_hunk` is covered by something other than one
  example).
- Fuzzing is run from `scripts/fuzz-all.sh` and `scripts/fuzz-repro.sh` instead of a command
  copied out of the guides. The documented command was missing `mkdir -p fuzz/corpus/<target>`,
  and the corpus directory is gitignored, so on a fresh clone libFuzzer refused to start; the
  scheduled CI run had already failed on exactly that and gained the step, while the guides did
  not. `fuzz/seeds/README.md` also omitted the nightly override and the explicit target triple
  that CONTRIBUTING.md calls mandatory, and the local command had no `-timeout=10`, so one hung
  input could swallow a short run's whole budget. The scripts carry all four, and a test checks
  that every script the documentation names exists and is executable.
- Documentation no longer contradicts the tool. Two README examples still recommended
  `--verify-result-diff-git` on a `git diff | hunkpick ... | git apply --cached` pipeline after
  the README gained the paragraph explaining that such a command reports a correct result as a
  failure (exit 70); both now show a patch file against a tree at the pre-patch state, and a test
  reads the shell blocks of README.md and CONTRIBUTING.md so the shape cannot come back.
  `hunkpick --help` claimed "every subset applies" for `@L` while `select --help` and the README
  already listed the two exceptions. The README described the selector index ceiling as per
  selector, which it no longer is, and called two of the four lossy JSON text fields lossy.

- Only git's own verdict now decides exit 70 for `--verify-result-diff-git`. `git apply --check`
  answers 1, and only 1, when a patch does not apply; every other status is git failing before it
  looked at the diff. A repository git refuses to read (`fatal: bad config line …`, exit 128) was
  reported as `git apply --check rejected the result diff`, and a git killed by a signal — the OOM
  killer, a timeout — produced the same sentence with nothing after the colon. Both now name the
  status and exit 74, so exit 70 keeps meaning what ADR 0013 reserves it for: a result hunkpick
  itself produced.
- `-C DIR` naming something that is not a directory is reported against that argument. A mistyped
  path reaches `Command::spawn` as the same `NotFound` a missing `git` binary does, so both printed
  `failed to run git: No such file or directory (os error 2)` at exit 74 and sent the caller after
  their git installation instead of after their own typo. The path is now checked before git is
  started and named in the message at exit 2; if git still fails to start, the message says which
  directory it was to run in.
- The one non-ASCII character in hunkpick's own output is gone. The encoding diagnostic added in
  this cycle carried an em dash, and it is addressed by construction to a Windows user, whose
  console on cp866 or cp1251 shows it as mojibake in the middle of the sentence. The guard added
  alongside it covered the help texts only, so the character came back in an error message the
  same week; it now covers every string literal of `src/` outside the test modules.
- One copy of the plumbing that runs `git` with a diff on its stdin. `gitenv::feed_and_wait`
  replaces three near-identical blocks — the result-diff check and two test helpers, two of them
  added last cycle — that had already drifted apart in how they treat a failed write. It also
  closes the failure path they shared: the child is now kept in hand and killed if waiting on it
  fails, because `std::thread::scope` parks until every thread it started has finished, and the
  writing thread can be blocked in a `write` nobody is going to read. The failure
  was a hang rather than a diagnostic, and the fallback test command documented in CONTRIBUTING
  has no per-test timeout. The stub file names that keep a developer's git configuration out of
  the test repositories moved to `gitenv::insulate_config`, next to the list of variables that
  was consolidated there last cycle, and `tests/git_config_isolation.rs` now poisons
  `GIT_CONFIG_SYSTEM` as well as `GIT_CONFIG_GLOBAL`, so dropping either one turns the suite red.

- The crates.io token no longer reaches `cargo publish --dry-run` in the release workflow. A dry
  run never asks the registry for credentials, but it does run a verification build, which
  compiles the package along with the build scripts and proc macros of its dependencies — all of
  them inheriting the step's environment. A rehearsal (`workflow_dispatch` with an empty tag)
  builds whatever branch the dialog names, so the step handed a credential good for every crate
  of the account, and outliving any access to this repository, to code from an arbitrary branch.
  The publishing step keeps its own copy of the token.

- The release pipeline checks the public API too. `cargo semver-checks check-release` was a CI
  gate only, and CI runs on branch pushes: a tag on a commit CI never saw, or one whose version
  was bumped after the last run, reached the irreversible `cargo publish` with the API unchecked.
  The release workflow now runs the same gate for the same reason its test, lint and MSRV jobs
  are duplicated there.
- The documented local loop covers what CI gates. The `fuzz` directory is a workspace of its own,
  so the `--all` in `cargo fmt`/`cargo clippy` never reached it, while CI lints it as a separate
  step: a fuzz target edited locally passed every documented command and failed the pull request.
  The two commands are in CONTRIBUTING.md now, along with `cargo-semver-checks` as a tool the
  project expects; RELEASING.md runs the semver and MSRV checks before the release commit, and
  the README says its list is the everyday loop and points at CONTRIBUTING.md for the full set
  rather than claiming to be it.

- RELEASING.md describes the pipeline that exists. Its table of jobs still began with `test` and
  attributed the tag, manifest, lock and changelog checks to `publish`, while they have their own
  first job (`verify-metadata`) that every other job waits on — the same document said so three
  sections further down, contradicting its own table, and an operator reading it would have gone
  looking in the wrong job's log. The intro counted four artefacts that must agree where there
  are five: `fuzz/Cargo.lock` is checked too. The job numbers in "If something fails" follow the
  table again.

- Six test gaps this cycle opened are closed. The four byte-order marks are checked one by one,
  by name: the single test asserting the message says "UTF-16" also passed when a UTF-32LE input
  was misdiagnosed as UTF-16LE, which is the mistake the arm order guards against. An I/O failure
  on stdout is now held by a test (`/dev/full`, exit 74 with the diagnostic) instead of by a
  comment. The seven repository-locating variables get a case each, with a count assertion that
  fails on a name added to the list and not to the table. The differential timings in the source
  cover all six tests that draw their case count from the same place, not three, and the
  documented soak (`HUNKPICK_DIFF_CASES=2000`) comes with the timeout it needs — measured, the
  slowest test goes from 4.5 s to 44.5 s against a 60 s kill. The linearity check takes the
  fastest of three runs per size, so a scheduler pause on a shared runner no longer moves the
  verdict. And a property-test counterexample found in CI is uploaded as an artifact, the way a
  fuzzer crash already was: the seed lives only on the runner, and a re-run draws a new one.

- CI jobs fetch dependencies through the retry action the release workflow has always used. A
  flaky crates.io download — the `curl 55` class that cargo's own `net.retry` does not treat as
  spurious — failed a push run and needed a manual re-run, while the workflow that runs a few
  times a month was protected and the one that runs on every push was not.
- The fuzz search accepts a budget it can finish. `minutes` had no upper bound while the job is
  killed at 90, so a larger value ended the run as a failure rather than as a completed search;
  it is now capped at 80. A manual dispatch also no longer cancels a scheduled search in
  progress: the corpus survives either way, but the search itself restarted from nothing.

### Changed

- A UTF-16 stream with no byte-order mark is now named as an encoding problem instead of being
  reported as binary input. `iconv -t UTF-16LE` and `UnicodeEncoding($false, $false)` write such
  a stream, and "binary input: NUL byte found" sent the reader looking for a binary file in the
  pipeline rather than at the encoding of their own patch. The diff is still not re-encoded
  (ADR 0005) — the message says what to run.
- **Library API.** `split::split_patch_hunk` is the entry point for splitting a hunk of a whole
  patch: it does what `split_file_hunk` does and additionally keeps `Patch::no_trailing_newline`
  honest, which a function taking one `FileDiff` cannot. `split_file_hunk` no longer panics on an
  address the patch does not have — a binary entry, or a hunk index past the end — but returns
  the new `SplitError::NotATextEntry` and `SplitError::OutOfBounds`. The CLI resolves addresses
  before it splits, so this only concerns a direct library caller; adding the two variants is a
  breaking change for a caller that matches `SplitError` exhaustively.
- **Library API.** `cli` and `gitenv` are `#[doc(hidden)]`: service modules the binary and the
  test crates need (neither can see a `pub(crate)` item), not a contract anyone should build on.
  The `cargo semver-checks` gate added in 0.8.1 counts a new CLI flag as a new public field of an
  exhaustively constructible struct, so without this every release that adds a flag would have to
  be a minor one on account of a type no library user calls. `parser::is_combined_marker`, made
  public in 0.8.0 for a guard in the binary, is private again; the guard itself moved into the
  library as `parser::looks_like_a_diff`, so the list of lines that open a diff lives next to the
  code that reads them.
- **Library API.** `gitenv` gained `feed_and_wait`, `FeedError`, `insulate_config` and the two
  file-name constants it uses. Additions only; nothing existing changed shape.
- **Library API.** `validate::GitCheckError::Spawn` carries the working directory alongside the
  `io::Error` (`Spawn { source, dir }` instead of `Spawn(io::Error)`), and the new variant
  `Failed { code, stderr }` reports a git that never reached a verdict. Both are breaking changes
  for a caller that constructs or exhaustively matches `GitCheckError`.
- The MSRV gate now builds on the minimum supported version it names. `rust-toolchain.toml`
  pins the repository to `stable`, and a toolchain file wins over an installed default, so both
  the CI job and the release job that `publish` waits on compiled on stable and reported success
  without ever seeing 1.85. Both now set `RUSTUP_TOOLCHAIN` first, as the fuzzing setup already
  did, and run `cargo check --locked --all-targets` so the dev-dependencies are compiled too —
  `proptest` is pinned to `~1.11` for an MSRV reason that `cargo build` never reached.
- `git apply --check`, run for `--verify-result-diff-git`, now runs in the C locale. Its stderr
  is shown next to hunkpick's own ASCII-English text, so an inherited locale used to put another
  language — or U+FFFD, where the bytes were not UTF-8 — in front of the user.
- The fuzzing dictionary and the committed seed corpus now carry bytes above ASCII: a path in
  git's quoted form with octal escapes, content that is not valid UTF-8, and the byte-order
  marks the input guard names. The header defect above was reachable only through such bytes and
  had survived fuzzing.

## [0.8.1] - 2026-08-18

### Fixed

- The two-round `@L` example in `select --help` and in the README could not be run: it reused
  the first round's line numbers (`1@L91-120`) after a re-diff that renumbers them, so the
  second command failed with exit 2. Both now show the corrected round and say why the numbers
  change; an integration test drives the selectors straight out of the help text.

### Changed

- Reading a diff from a terminal — no pipe, no `-i` — now says so in one line on stderr before
  it blocks. A forgotten pipe used to be indistinguishable from a hang. Nothing is printed when
  stdin is not a terminal, so pipelines are unaffected.
- Selecting by content id resolves through an index built once per invocation instead of a scan
  per id: on a 20 000 sub-hunk diff, selecting every id went from 0.22 s to 0.06 s.
- Release archives are now verified by running the packaged binary and comparing its
  `--version` with the archive name, so a packaging fault is caught before upload. CI gained a
  `cargo semver-checks` gate over the public API, the scheduled fuzz run bounds a single input
  with `-timeout`, and the versions of the tools CI installs are pinned.

## [0.8.0] - 2026-08-17

### Fixed

- The overlap check compared raw header numbers, ignoring git's convention that a side with
  no lines reports the line *before* its empty range. A hunk ending in a pure deletion —
  a file whose tail is removed — therefore looked like an overlap, and `select '*'` refused
  a diff git itself writes: exit code 70 on roughly one in ten ordinary diffs (measured over
  200 generated cases). Present since 0.7.0, when the new-side check was introduced.
- A full binary patch (`git diff --binary`) lost its structure: the payload lines
  (`literal <n>` and the base85 body) were filed as leading headers and emitted *above* the
  `GIT binary patch` marker, so `git apply` rejected the result as garbage while hunkpick
  exited 0. This is the form needed to stage a binary change, and the binary marker of a
  CRLF diff went unrecognised for the same reason.
- A sub-hunk with no old-side lines (an appended block) carried a header one line past
  where git puts it (`@@ -4,0 +5 @@` instead of `@@ -3,0 +4 @@`), and the new-side anchor
  inherited the shift.
- An `@L` slice of a whole-file deletion produced a self-contradictory patch: the header
  declares the file removed while the body keeps the unselected lines as context, which git
  rejects (`deleted file f still has contents`). Such a selection is now a usage error.
- A combined diff (`diff --cc`, `@@@` headers — what git writes for a merge) was read as a
  two-sided one: the hunk body was truncated at its first line and a `--- removed in both`
  line invented a file entry. It is now rejected as unsupported (exit 2) instead of losing
  data silently. See `docs/ADR/0012-two-sided-diffs-only.md`.
- The listing gave a CRLF diff's `@@` header a separating space and a raw CR — inside a JSON
  string field — where `emit` already knew the CR is the line ending, not section text.
- A context line for an empty source line is a lone space, and transports that strip
  trailing whitespace deliver it as a zero-length line. Parsing treated that line as the
  end of the hunk, dropped the rest of the body into the file's headers and emitted a
  diff `git apply` rejects as garbage — with exit code 0. Such a line is now read as
  context and emitted with its marker restored.
- Lines that follow a hunk body — a blank separator between hunks, the `-- ` signature
  `git format-patch` appends, a stray `Binary files ...` marker — were emitted with the
  leading headers, which moved them above the first `@@`. They now keep their position,
  and the binary marker is no longer dropped silently.
- `split` replaces one hunk with several but left those trailing lines at their old
  positions, so a signature recorded after the last hunk was emitted between the pieces.
  `git apply` rejected the result (`patch fragment without header`) while hunkpick exited 0.
  Each line now follows the hunk it followed before the split.
- Emitting those lines rescanned the whole list for every hunk, which is quadratic in their
  number: a 7 MB diff carrying a separator after each of its 128 000 hunks took 15 s to
  re-emit, against 0,2 s to list. One pass now walks the list alongside the hunks (0,18 s on
  the same input).
- A binary file, a pure rename and a mode-only change have no `---`/`+++` lines, so they
  had no path and could not be addressed in a multi-file diff. Their paths are now read
  from the `diff --git` line, and `*` takes a hunkless entry whole.
- Paths quoted and C-escaped by git (`"a/\303\251.txt"`, the `core.quotePath` default for
  non-ASCII names) are decoded, so a selector spelled with the real file name matches.
  The emitted diff keeps the original bytes.
- A CRLF diff left the CR in the file path (breaking `path:` selectors) and added a stray
  space to the hunk header, so the round-trip was not byte-identical.
- A deleted file was listed as `/dev/null`; it is now shown under its old name.
- Line numbers near `u32::MAX` from the input header overflowed while checking hunk
  overlap: a debug build panicked with exit 101, a release build wrapped and decided the
  check on a meaningless value. The bounds are computed in `u64`, and sub-hunk starts
  saturate rather than wrap.
- A defect of the input diff (a header that disagrees with its body, e.g. a truncated
  diff) was reported as a verification failure of hunkpick's own result: exit code 70 and
  a Debug dump of internal fields. It is now a usage error (exit 2) in prose, with the
  sub-hunk numbered from one as `list` numbers it.
- A reader that closes the pipe first (`hunkpick list | head`) ended the run with exit 74
  and a `Broken pipe` diagnostic; it is now a normal end of work (exit 0). Both this and the
  exit-code change above are recorded in `docs/ADR/0013-exit-code-contract.md`.
- In the `path:set` selector form a broken set was re-read together with the path, so
  `f:2-1` was reported as `not a number: f:2` instead of `reversed range`.
- `split` now recomputes new-side anchors like `select` does, so both commands treat a
  diff carved out of a larger one the same way.
- In a plain (non-git) diff a header-only entry absorbed the next file's marker lines.
- The human listing escapes text a terminal would act on (escape sequences, control
  bytes, bidirectional overrides) instead of passing it through.
- A file whose name is not valid UTF-8 (legal on Unix) could not be addressed: the
  argument was refused before hunkpick saw it. Selector paths are now taken as raw bytes,
  so such a file is reachable by name; only the set after the `:` must be ASCII.
- A selection that did not include the input's last line inherited its "no final newline"
  flag, so the result ended mid-line and `git apply` called it a corrupt patch. The flag now
  travels only when the result does end on that line.
- A hunk header carrying a token hunkpick cannot represent — `@@ -1,3,9 +1,3 @@`,
  `@@ -1,3 +1,3 junk @@` — was parsed as if the extra part were absent and emitted without it,
  at exit 0. Both are now parse errors (exit 2).
- A diff saved in UTF-16 (what `git diff > patch.diff` writes in Windows PowerShell 5.1) was
  reported as "binary input: NUL byte found", which points at the wrong thing. A UTF-16/UTF-32
  byte-order mark is now named, with the re-encoding command to fix it.
- A `git` that could not be started for `--verify-result-diff-git` was reported as a failed
  verification of the result diff (exit 70). The check never ran, so it is an environment
  failure: exit 74.
- Output written after the last newline could be lost silently: stdout is line-buffered and the
  runtime's implicit flush at exit discards its error. It is flushed explicitly, and a failure
  is exit 74.
- `list --json` did not end its output with a newline.
- A combined diff whose entries carry no `---`/`+++` pair (a file resolved the same way in both
  parents) was reported as "no diff markers found" rather than as the combined diff it is.

### Changed

- Auto-splitting a hunk is linear in the number of change runs; it re-counted the whole
  prefix per sub-hunk before. On a 1.9 MB one-hunk diff with 64 000 runs `list` went from
  3.9 s to 0.06 s.
- `split` no longer clones the whole parsed diff to rewrite one hunk.
- The `git apply --check` child process no longer inherits `GIT_DIR`, `GIT_WORK_TREE` and
  related variables, so `-C DIR` alone selects the repository.
- `--color` is described in `list --help`; the exit-code table in the README distinguishes
  SIGINT (130) from SIGTERM (143) and documents the closed-pipe case.
- Release archives carry `THIRD-PARTY-NOTICES.md` with the license texts of the crates
  linked into the binary, and the release pipeline builds and verifies every archive
  before the irreversible `cargo publish` rather than after it. The release procedure is
  written down in `RELEASING.md`.
- README states the measured peak memory (6x–19x the input, depending on average line
  length) instead of "a few hundred MiB", so the input limit is not read as a RAM ceiling.
- The crate is built on the Rust 2024 edition, formatted with the matching style edition.
  The minimum supported Rust version is unchanged (1.85, the release that stabilised the
  edition), so nothing is required of consumers; the dependency resolver now honours that
  minimum when picking versions. See `docs/ADR/0011-rust-2024-edition.md`.
- `path:` selectors resolve through an index built once per invocation instead of scanning the
  file list per selector: 16 000 selectors over 16 000 files went from 1.5 s to 0.04 s.
- `--verify-result-diff-git` documents what it actually checks — the working tree, which in the
  usual staging pipeline already holds the edits, so the flag reports a correct result as not
  applying. Same for the JSON listing, whose text fields are the diff's own content: not
  display-sanitised the way the human listing is, and lossy for non-UTF-8 bytes.
- The selector index limit (2^20 per selector) is named in the error message and in the README.
- The release pipeline checks the tag against the manifest, both lockfiles and the CHANGELOG in
  a job of its own, before anything is built, and a release can be rehearsed by running the
  workflow with no tag named, which reads the version from `Cargo.toml` and publishes nothing.
- Behavioural decisions of this release are recorded as ADRs: `docs/ADR/0012-two-sided-diffs-only.md`
  and `docs/ADR/0013-exit-code-contract.md`.

### Added

- Generated tests, because the defects above were found by generating inputs rather than by
  reading code: `tests/differential.rs` compares hunkpick with real git over generated diffs
  (a selection applies, staging one sub-hunk at a time converges on the target, the output is
  valid input for the next invocation), `tests/property.rs` uses `proptest` for the forms git
  will not produce on demand (CRLF, a missing final newline, a mail preamble), and `fuzz/`
  holds libFuzzer targets for parsing, the `parse . emit` fixed point and selector handling,
  with committed seeds (`fuzz/seeds/`) and a token dictionary. CI builds every fuzz target on
  each push; a scheduled workflow searches twice a week and keeps its corpus across runs.

### Changed (library API)

- `Selector::File.path` is `Option<Vec<u8>>` instead of `Option<String>`, and
  `select::parse_selectors` accepts anything convertible to `OsStr` (a `&[String]` still
  works). This is what lets a selector name a file whose path is not valid UTF-8.
  `select::resolve_file` takes `Option<&[u8]>` accordingly.
- `select::build_view` returns `Vec<Vec<Hunk>>`: the position in the result is the file
  index, so the redundant index in each tuple is gone.
- The crate denies undocumented public items (`#![warn(missing_docs)]`); every exported
  item now carries rustdoc.
- `Line::no_newline` is `Option<Vec<u8>>` (the marker as it arrived) rather than a flag, so a
  CRLF diff keeps the marker's line ending.
- `Patch` gained `preamble` (the lines before the first file entry — the mail head of a
  `format-patch` output, which used to be dropped while its footer was kept) and
  `no_trailing_newline` (an input that ended without one now leaves without one). Together
  with the two above, `emit` now round-trips its input byte for byte, not just a
  git-canonical diff.
- `ParseError` gained a `Combined` variant for merge diffs.
- `validate::validate_with_git` returns `Result<(), GitCheckError>` instead of
  `Result<(), String>`, so a caller can tell a rejected diff from a check that never ran.
- `validate::ValidationError::CountMismatch` carries `side: Side` in place of
  `field: &'static str`, along with the declared and actual counts.
- `select::resolve_hunk` takes `addr: &OsStr` instead of `&str`, the same widening the
  selector parser got above: a hunk address arrives from a command line, which on Unix is
  bytes.
- Added: `pub mod gitenv` (the environment variables that decide which repository a `git`
  child acts on) and `parser::is_combined_marker`. Both are service items rather than
  contract, and both are `#[doc(hidden)]` or private again in the next release.

## [0.7.0] - 2026-07-29

### Fixed

- `select` now recomputes the new-side (`+`) start of every emitted hunk instead of
  carrying over the value from the input diff. Leaving a sub-hunk out changes how many
  lines the result adds or removes above each later hunk, so the inherited anchors
  described a file the selection does not produce. `git apply` starts its search at the
  new-side position: with a drifted anchor a selection could be rejected
  (`patch does not apply`) or, where the surrounding context occurs more than once,
  applied cleanly to the wrong occurrence. Selecting several sub-hunks in one invocation
  no longer needs the `diff → stage → re-diff` loop as a workaround.

### Changed

- The default internal consistency check also verifies the new-side starts against the
  accumulated `added - deleted` of the preceding hunks, reported as a `StaleNewStart`
  error. A diff whose anchors are stale (for example, a result diff produced by an
  earlier version) is now rejected rather than passed on to `git apply`.

## [0.6.0] - 2026-07-21

### Added

- `CLICOLOR_FORCE` (any non-empty value) forces coloured output in the default
  `--color auto` mode even when stdout is not a terminal (e.g. a pipe).
  `NO_COLOR` still takes precedence when both are set, and an explicit
  `--color always|never` overrides both.

### Changed

- Result-diff validation now rejects a change-free (all-context) hunk with a
  dedicated `NoChangeHunk` error. Such a hunk balances the header counts but
  `git apply` rejects it; it is unreachable from a real git diff and only
  possible from a synthetic patch.
- The parser no longer appends body lines past the counts declared in a hunk
  header: once a side's declared count is exhausted, a further line of that kind
  ends the hunk instead of being absorbed into it. Well-formed git diffs are
  unaffected.

## [0.5.1] - 2026-07-21

### Fixed

- `parse` no longer mistakes a deletion line whose content begins with `-- `
  for a new-file header. Such a deletion renders as `--- <text>` in the hunk
  body; new-file detection previously triggered on any `--- ` line while inside
  a hunk, dropping the real change and emitting a phantom file. The parser now
  tracks the old/new line counts declared by the hunk header and only treats
  `--- ` as the next file once the current hunk body is fully consumed.
- `read_limited` no longer overflows `limit + 1` at `--max-input-bytes`
  `18446744073709551615` (`u64::MAX`), which wrapped to `0` in release builds
  and silently treated any input as empty. It now uses a saturating add.
- `split --at` on a hunk's first context line no longer emits a leading
  context-only piece with zero net change (a hunk `git apply` rejects); such a
  piece is dropped.

## [0.5.0] - 2026-07-11

### Removed

- **Breaking:** the `[path:]INDEX@lo-hi` added-line range selector (added in
  0.4.0) is removed. Its contiguous-slice implementation produced an
  unapplicable hunk for a leading or interior slice of an addition block placed
  mid-file: it kept the leading context but dropped the trailing context, so
  `git apply` could not anchor the piece (`patch does not apply`). The per-line
  `[path:]INDEX@L<set>` selector supersedes it: `@L` addresses any subset of a
  sub-hunk's changed lines, keeps both leading and trailing context (every
  subset applies), and can express any added-line range plus what `@lo-hi`
  could not (deletions, interior slices, replacements). A selector that still
  uses the `@lo-hi` form now fails with exit 2 and a message pointing at the
  `@L` replacement.

### Changed

- The human `list` marker for an all-additions sub-hunk is renamed from
  `[+range]` to `[+add]` (the `@lo-hi` range form it advertised is gone). The
  `addition_only` field in `list --json` is unchanged.

## [0.4.0] - 2026-07-05

### Added

- `select` accepts a per-line changed-line selector `[path:]INDEX@L<set>` that
  keeps an arbitrary subset of a sub-hunk's changed (`+`/`-`) lines, where
  `<set>` is an index list over the changed lines (`L1,3`, `L1-2,4`). Unlike
  `INDEX@RANGE` (added-side only, cut between two `+` lines, deletions atomic),
  it has no boundary restriction: a deletion surrounded by additions
  (`+x -y +z`) can be isolated, and the deletions and additions of a
  replacement can be separated across commits via the diff → stage → re-diff
  loop. Unselected deletions are kept as context (anchoring the hunk, so the
  old-side footprint is invariant and no `--unidiff-zero` is needed);
  unselected additions are dropped. A sub-hunk addressed by `@L` must be
  addressed once per invocation (exit 2 otherwise).
- `list --json` reports a `changed_lines` array per sub-hunk —
  `[{i, kind, text}]` with 1-based indices shared across deletions and
  additions — the machine-readable source for building `@L` selectors without
  parsing the diff body.

### Tests

- Added unit and end-to-end coverage for the line-set selector: separating
  deletions from additions, isolating a deletion among additions, full-selection
  round-trip, the no-newline-at-EOF re-add edge, the once-per-invocation rule,
  and the `changed_lines` JSON numbering.

## [0.3.1] - 2026-06-25

### Fixed

- `select` now orders multiple `INDEX@RANGE` selectors that address the same
  sub-hunk by their first added line, so two disjoint ranges given in any order
  (e.g. `1@3-4 1@1-2`) emit in ascending new-file order and apply cleanly
  instead of being rejected as overlapping.
- Overlapping added-line ranges of one sub-hunk (e.g. `1@1-3 1@2-4`) are now
  rejected as a selector error (exit 2) before emission, so they can no longer
  slip past `--no-verify-result-diff-internal` as a corrupt diff.
- A malformed selector now reports the specific reason it was rejected — a
  reversed range, a zero bound, or a non-numeric bound — instead of a bare
  `bad selector: <text>`.

### Tests

- Added unit and end-to-end coverage for range ordering, overlap rejection
  (including under `--no-verify-result-diff-internal`), and the specific
  bad-selector reasons.

## [0.3.0] - 2026-06-24

### Added

- `select` accepts a per-line range selector `[path:]INDEX@RANGE` that cuts one
  sub-hunk to a range of its added (`+`) lines, where `RANGE` is `lo-hi`, `lo-`
  (to the last added line), `-hi` (from the first), or a single `N`. This makes
  an otherwise atomic addition-only sub-hunk — a block of new functions, or a
  file-creation diff `@@ -0,0 +1,N @@` — splittable across commits. The cut is
  allowed only between two added lines; only a numeric index may precede `@`
  (content ids and `*` are not accepted as the address of a range). See
  [ADR 0008](docs/ADR/0008-added-line-range-addressing.md).
- `list` reports whether a sub-hunk is all additions (and therefore freely
  cuttable at any added line): an `addition_only` boolean in `--json` output and
  a `[+range]` marker in the human listing.

### Changed

- The `@id` selector now requires a non-empty hex id: a non-hex `@token` is
  rejected at parse time as a bad selector instead of failing later at resolve
  time. No valid 16-hex content id is affected.

### Tests

- Added unit and end-to-end coverage for the range selector, including a git
  round-trip that stages part of a file-creation diff, the addition|addition
  boundary rule, open-ended ranges, and round-trip reconstruction of a sub-hunk
  from its pieces.

## [0.2.2] - 2026-06-20

### Changed

- Expanded the examples in `hunkpick --help` and the README: selecting several
  `@<id>`s at once (mixed with `path:` selectors), the full `list --json` once →
  `select @id` staging loop ending with `*`, multi-file diffs
  (`git diff file1 file2 fileN`) addressed per `path:`, and content ids across a
  multi-file diff (the file path is part of the id, so an id addresses the change
  in its own file). Examples that read ids before selecting use the
  machine-readable `list --json`.

### Tests

- Added integration tests for multi-file diffs (`tests/multi_file.rs`): per-`path:`
  selection across several files with a `git apply` check, `path:*` scoping to one
  file, rejection of a bare selector on a multi-file diff (exit code 2), and
  `@<id>` addressing its own file in a multi-file diff.

## [0.2.1] - 2026-06-20

### Changed

- `hunkpick --help` now shows usage examples and a content-id (`@<id>`) section
  inline, so the common recipes are visible without drilling into each
  subcommand's `--help`. The short `-h` stays a compact summary and points to
  `--help` for the examples.

## [0.2.0] - 2026-06-19

### Added

- Content ids for sub-hunks. `list` (human and `--json`) now reports a 16-hex `id`
  per sub-hunk, derived from the file paths and the sub-hunk's changed (`+`/`-`) lines
  only — independent of its context lines, the `@@` line numbers, and the section header.
  The id is stable across a re-diff in the common agent loop: it is unchanged both when an
  unrelated edit only shifts a change's line numbers and when staging a neighbour rewrites
  the change's surrounding context (or re-splits the enclosing hunk). It changes only when
  the change's own `+`/`-` lines change. `list --json` also reports `id_count` per
  sub-hunk — how many sub-hunks share that id (`1` = unique), so a consumer can tell
  whether `@<id>` addresses one sub-hunk or several.
- `@<id>` selector for `select`: emits every sub-hunk whose content id equals `<id>`
  (matched case-insensitively). Changes with identical `+`/`-` lines share an id and are
  selected together (use `path:N` to pick one, guided by `id_count`); an id shared by
  changes whose `+`/`-` lines genuinely differ (an accidental hash collision) is reported
  and exits with code 2.
- `*` selector: `path:*` selects every sub-hunk of a file; bare `*` selects every
  sub-hunk of a single-file diff. Removes the need to first read the sub-hunk count
  from `list`.

## [0.1.0] - 2026-06-19

### Added

- Unified-diff parser with full round-trip emitter: parses `git diff`, `diff -u`,
  rename/mode/new-file/deleted-file/binary headers, no-newline markers, CRLF line
  endings, and plain (non-git) diffs; emits a semantically equivalent patch.
- `list` subcommand: auto-splits each hunk into minimal sub-hunks and lists them per
  file with a 1-based per-file index. Human-readable output by default; `--json` emits
  a stable machine schema with `path`, `binary`, `index`, `old_start`, `old_lines`,
  `new_start`, `new_lines`, `added`, `deleted`, `header`, and `preview` fields.
  `--color auto|always|never` controls ANSI colour (respects `NO_COLOR`).
- `select` subcommand: emits only the chosen sub-hunks as a valid unified diff.
  Selector syntax: bare `1,3` or `2-4` for single-file diffs; `path:1,3` and
  `path:2-4` for multi-file diffs; multiple selectors may be combined. A binary file
  referenced by any selector is emitted whole.
- `split` subcommand: explicitly splits one original hunk (addressed `path:N` or `N`)
  at given new-file line numbers (must be context lines); emits the whole patch with
  that hunk replaced by the pieces.
- Auto-split semantics with non-overlapping old-file ranges: boundary context between
  adjacent change runs becomes trailing context of the earlier sub-hunk; later
  sub-hunks start at their change run. Selecting all sub-hunks is apply-equivalent to
  the original patch.
- Result-diff verification for `select` and `split`: internal consistency check (hunk
  header counts match body, hunks are ordered and non-overlapping) runs by default;
  disable with `--no-verify-result-diff-internal`. Optional `git apply --check` via
  `--verify-result-diff-git`; `-C <DIR>` sets the working tree directory and requires
  `--verify-result-diff-git`.
- Git-agnostic design: reads stdin, writes stdout; does not call `git diff` itself.
  Works with diffs from any source (git, Mercurial, SVN, plain `diff -u`).
- Encoding-agnostic byte-oriented core: diff content is parsed, processed, and emitted
  as raw bytes, so any encoding (including invalid UTF-8) round-trips byte-for-byte.
  Only `list` paths/previews are decoded lossily for display.
- Input validation before parsing: empty/whitespace-only input is a no-op (exit 0);
  binary input (NUL byte) and text with no diff markers are rejected with exit code 2.
- Input source selection: `-i, --input FILE` reads the diff from a file (`-` means
  stdin) on every subcommand; stdin remains the default.
- Input size limit: `--max-input-bytes N` caps the input (default 64 MiB; `0` disables);
  exceeding it is a usage error (exit code 2). The input buffer is freed after parsing so
  it does not coexist with the result diff on the heap.
- Edge-case support: rename diffs, mode-only changes, new-file and deleted-file
  patches, binary file entries, `\ No newline at end of file` markers, CRLF line
  endings, and plain (non-extended-header) unified diffs.
- Structured exit codes: 0 success, 2 usage/parse error, 70 verification failure,
  74 I/O error, 130 signal termination.
- Prebuilt binaries on GitHub Releases for `x86_64-unknown-linux-gnu`,
  `aarch64-apple-darwin`, `x86_64-apple-darwin`, and `x86_64-pc-windows-msvc`,
  installable with `cargo binstall hunkpick`.
