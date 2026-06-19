#!/usr/bin/env bash
# Verify that CHANGELOG.md carries a section for the given version before a
# release is published.
#
# Usage: scripts/check-changelog.sh <VERSION>     (e.g. 0.1.0, no "v" prefix)
#
# hunkpick's CHANGELOG follows Keep a Changelog with `## [X.Y.Z] - YYYY-MM-DD`
# headings (ASCII hyphen separator, no `## [Unreleased]` placeholder). This
# check only asserts that a `## [<VERSION>]` heading exists, so a tag cannot
# ship a version that has no changelog entry. The version is matched
# literally (metacharacters escaped) so "0.1.0" does not match "## [0X1X0]".
#
# On failure it writes a diagnostic to stderr and exits non-zero. It NEVER
# prints to stdout, so it is safe inside `id:`-tagged workflow steps.

set -euo pipefail

if [ $# -lt 1 ] || [ -z "${1:-}" ]; then
    echo "usage: $0 <VERSION>" >&2
    exit 2
fi

VERSION="$1"
CHANGELOG="${CHANGELOG:-CHANGELOG.md}"

if [ ! -f "$CHANGELOG" ]; then
    echo "error: $CHANGELOG not found" >&2
    exit 2
fi

# Literal-match VERSION as a regex by escaping every metacharacter we care
# about. In practice only "." appears in semver, but escape conservatively.
escape_re() {
    # SC2001: sed is the clearest way to escape an arbitrary metacharacter set.
    # SC2016: the `$` inside the single-quoted sed program is a literal regex
    # metacharacter in the character class, not a shell expansion.
    # shellcheck disable=SC2001,SC2016
    printf '%s' "$1" | sed 's/[.[\*^$()+?{|]/\\&/g'
}
VERSION_RE=$(escape_re "$VERSION")

if ! grep -qE "^## \[${VERSION_RE}\]" "$CHANGELOG"; then
    echo "error: $CHANGELOG has no '## [${VERSION}]' section" >&2
    echo "       add a '## [${VERSION}] - YYYY-MM-DD' heading before tagging" >&2
    exit 1
fi
