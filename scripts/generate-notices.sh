#!/usr/bin/env bash
# Generate THIRD-PARTY-NOTICES.md: the license texts and copyright notices of the crates
# statically linked into the released binary.
#
# The MIT license of most dependencies (and the MIT branch of the MIT-OR-Apache-2.0 ones)
# requires their copyright notice to be included in "all copies or substantial portions of the
# Software", which covers a binary distribution. The generated file travels inside every
# release archive next to LICENSE.
#
# Requires `cargo-about` (https://github.com/EmbarkStudios/cargo-about). The set of accepted
# licenses and the targets the notices cover live in about.toml; the layout is
# scripts/templates/notices.hbs.
#
# Doubles as the licence gate: cargo-about refuses to generate when a crate's licence is
# outside the `accepted` list in about.toml, so CI runs this on every pull request.
#
# Optional env:
#   OUT   output path (default THIRD-PARTY-NOTICES.md in the repository root)

set -euo pipefail

OUT=${OUT:-THIRD-PARTY-NOTICES.md}

if ! command -v cargo-about >/dev/null 2>&1; then
    echo "error: cargo-about is required; install it with 'cargo binstall cargo-about'" >&2
    exit 2
fi

cargo about generate scripts/templates/notices.hbs -o "$OUT"
echo "wrote $OUT" >&2
