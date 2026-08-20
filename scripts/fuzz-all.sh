#!/usr/bin/env bash
# Run every fuzz target in turn; the first crash stops the run.
#
# Usage: scripts/fuzz-all.sh
#        scripts/fuzz-all.sh parse selectors
#        FUZZ_SECONDS=600 scripts/fuzz-all.sh
#
# Needs a nightly toolchain and cargo-fuzz: the sanitizer coverage libFuzzer
# steers by is a nightly flag.
#
# RUSTUP_TOOLCHAIN, not `cargo +nightly`: rust-toolchain.toml pins the whole
# repository to stable, and a toolchain file wins over the toolchain named on
# the command line for the crate it covers. Without the override cargo-fuzz
# hands the build to stable rustc, which rejects the -Z sanitizer flags.
#
# mkdir first, because the corpus is gitignored: a tree that has never fuzzed
# locally does not carry it, and libFuzzer refuses to start when the writable
# corpus directory is missing.
#
# The target triple is spelled out because cargo-fuzz otherwise builds for the
# triple it was itself built for, and a statically linked libc cannot carry
# AddressSanitizer.
#
# A crash leaves its input in fuzz/artifacts/, which does not always travel
# back from wherever the run happened -- a crash is by definition the run that
# did not succeed. The base64 dump into the log is enough to reconstruct the
# input elsewhere.
set -uo pipefail

TARGETS=(parse roundtrip selectors)
if [ "$#" -gt 0 ]; then
    TARGETS=("$@")
fi
TRIPLE="${FUZZ_TRIPLE:-x86_64-unknown-linux-gnu}"
SECONDS_PER_TARGET="${FUZZ_SECONDS:-300}"
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-nightly}"

for target in "${TARGETS[@]}"; do
    mkdir -p "fuzz/corpus/${target}" || exit 1
    # A target reads either its own seeds (selectors: diff, NUL, selector lines) or the
    # shared set of plain diffs, which parse and roundtrip both take as-is.
    seeds="fuzz/seeds/${target}"
    [ -d "$seeds" ] || seeds="fuzz/seeds/diff"
    if ! cargo fuzz run --target "$TRIPLE" "$target" \
        "fuzz/corpus/${target}" "$seeds" \
        -- -max_total_time="$SECONDS_PER_TARGET" \
        -dict=fuzz/dictionaries/diff.dict \
        -print_final_stats=1 \
        -timeout=10
    then
        printf 'crashing inputs for %s:\n' "$target"
        for artifact in "fuzz/artifacts/${target}"/*; do
            [ -e "$artifact" ] || continue
            printf '== %s\n' "$artifact"
            base64 -w0 "$artifact"
            printf '\n'
        done
        exit 1
    fi
done
