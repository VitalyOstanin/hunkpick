#!/usr/bin/env bash
# Replay every crashing input under fuzz/artifacts/, one by one.
#
# Usage: scripts/fuzz-repro.sh
#
# Drop a crash file into fuzz/artifacts/<target>/ and run this: each input is
# replayed against the target it belongs to, with the same compiler the search
# used. The first failure stops the run, which is the point -- the input that
# still crashes is the one to look at.
#
# RUSTUP_TOOLCHAIN and the spelled-out triple are needed here for the reasons
# CONTRIBUTING.md gives in its "Fuzzing" section, the same ones fuzz-all.sh runs
# under.
set -uo pipefail

TRIPLE="${FUZZ_TRIPLE:-x86_64-unknown-linux-gnu}"
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly}"

for artifact in fuzz/artifacts/*/*; do
    [ -e "$artifact" ] || continue
    target="$(basename "$(dirname "$artifact")")"
    printf '== %s: %s\n' "$target" "$artifact"
    cargo fuzz run --target "$TRIPLE" "$target" "$artifact" || exit 1
done
