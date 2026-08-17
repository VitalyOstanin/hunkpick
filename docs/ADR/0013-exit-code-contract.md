# ADR 0013 — Exit-code contract: the input's fault, the tool's fault, and a closed pipe

Date: 2026-08-17

## Status

Accepted

## Context

hunkpick is used inside shell pipelines (`git diff | hunkpick select 1 | git apply --cached`), so
its exit code is a control-flow signal, not decoration. Three codes carry distinct meanings:

- `2` — usage error: the caller gave something hunkpick will not work on (bad flag, bad selector,
  input that is not a two-sided unified diff, input over the size limit);
- `70` — verification failure: hunkpick's own result did not pass the checks of ADR 0004, which is
  a defect in hunkpick, not in the call;
- `74` — I/O error reading input or writing output.

Two cases were classified against that distinction and had to be corrected.

**A malformed input diff was reported as a verification failure.** A hunk header disagreeing with
its body — a truncated diff, a header edited by hand — was carried through and caught by the
internal check on the way out, so the caller saw exit 70 and a `Debug` dump of hunkpick's internal
structs. That says "the tool is broken" about an input the tool merely copied. It also blurred the
one signal a user has for telling "my diff is bad" from "file a bug".

**A reader closing the pipe was reported as an I/O error.** `hunkpick list | head` makes the writer
see `EPIPE` on a later write. The run had already done what it was asked to do; the reader simply
stopped listening. Exiting 74 with a `Broken pipe` diagnostic makes an ordinary shell idiom look
like a failure, and under `set -o pipefail` it fails the whole pipeline. Rust's default SIGPIPE
handling (the signal is ignored, writes return the error) is what surfaces it to the program at
all.

## Decision

- **The input's defects are usage errors.** Structural checks that can be decided on the input —
  a header whose counts disagree with its body, overlapping or misordered hunks, an unsupported
  form — run on the input and report exit 2 with a prose message, naming the file and the sub-hunk
  numbered from one, the way `list` numbers it. Exit 70 is reserved for a result hunkpick itself
  produced.
- **A closed reader is a normal end of work.** A write failing with `ErrorKind::BrokenPipe` ends
  the run with exit 0 and no diagnostic. Every other I/O failure keeps exit 74.
- **The contract is documented where callers look**: the exit-code table in the README, which also
  distinguishes the signal-terminated cases (130 for SIGINT, 143 for SIGTERM) that come from the
  default signal disposition rather than from hunkpick.

## Consequences

- `hunkpick … | head` composes with `set -o pipefail`; scripts do not need to special-case it.
- A user piping a truncated diff gets a sentence about their diff and exit 2. Exit 70 now means
  what it is meant to mean — hunkpick produced something that fails its own check — and is worth
  reporting as a bug.
- The two checks share their implementation but not their classification: the same predicate runs
  over the input (exit 2) and over the result (exit 70), which keeps them from drifting apart while
  still telling the caller whose fault it is.
- A caller that relied on exit 70 for a malformed input, or on exit 74 for a closed pipe, sees
  different codes. Both changed in 0.8.0 and are recorded in the CHANGELOG.
