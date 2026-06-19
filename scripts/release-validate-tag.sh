#!/usr/bin/env bash
# Validate a release tag follows the project's `v<MAJOR>.<MINOR>.<PATCH>`
# form, with an optional pre-release / build-metadata suffix introduced by
# `-` or `+`. Suffix characters are restricted to the SemVer-compatible
# alphabet `[A-Za-z0-9.+-]`.
#
# Used by `.github/workflows/release.yml` to harden both the
# `workflow_dispatch.inputs.tag` and the pushed-tag code paths. The tag
# value flows through an `env:` block first (so YAML expansion cannot
# inject shell payloads) and is then sanity-checked here as defense in
# depth, per the GitHub Actions security-hardening guidance on
# "Untrusted input".
#
# Usage:
#   scripts/release-validate-tag.sh <tag>
#
# Exits 0 when the tag matches the format, 1 otherwise (with a one-line
# explanation on stderr).

set -euo pipefail

tag="${1-}"

if [ -z "$tag" ]; then
    echo "release-validate-tag.sh: empty tag" >&2
    exit 1
fi

# `[[ =~ ]]` matches against the entire string (no implicit line-by-line
# loop the way `grep` does), so an injection payload that smuggles a
# newline cannot satisfy `^...$` for a benign-looking first line.
# SemVer-style separators: `-` for pre-release, `+` for build metadata.
# Anything else (a fourth dotted component, a space, a shell metacharacter)
# is refused.
re='^v[0-9]+\.[0-9]+\.[0-9]+([-+][A-Za-z0-9.+-]+)?$'
if [[ ! $tag =~ $re ]]; then
    echo "release-validate-tag.sh: tag '$tag' does not match expected format vX.Y.Z[-pre+build]" >&2
    exit 1
fi
