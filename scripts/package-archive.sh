#!/usr/bin/env bash
# Package a release archive for a given target.
#
# Produces a contract-compliant archive in OUTPUT_DIR:
#   - hunkpick-${VER}-${TARGET}.${ARCHIVE_EXT}
#   - hunkpick-${VER}-${TARGET}.${ARCHIVE_EXT}.sha256
#
# The archive contains a single top-level directory matching the archive
# stem; inside it: the binary, README.md, LICENSE.
#
# Required env:
#   VER          version (e.g. 0.1.0)
#   TARGET       target triple (e.g. x86_64-unknown-linux-gnu)
#   ARCHIVE_EXT  tar.gz | zip
#   BIN_NAME     binary file name (hunkpick or hunkpick.exe)
#
# Optional env (with defaults):
#   BIN_PATH     path to the prebuilt binary (default target/release/$BIN_NAME)
#   RUNNER_TEMP  parent directory for the staging tree (default $(mktemp -d))
#   OUTPUT_DIR   where to write the asset + .sha256 (default $PWD)
#
# Stdout: a single line `asset=<filename>` suitable for $GITHUB_OUTPUT.

set -euo pipefail

: "${VER:?VER required}"
: "${TARGET:?TARGET required}"
: "${ARCHIVE_EXT:?ARCHIVE_EXT required}"
: "${BIN_NAME:?BIN_NAME required}"

BIN_PATH=${BIN_PATH:-target/release/${BIN_NAME}}
RUNNER_TEMP=${RUNNER_TEMP:-$(mktemp -d)}
OUTPUT_DIR=${OUTPUT_DIR:-$PWD}

if [ ! -f "$BIN_PATH" ]; then
  echo "error: binary not found at $BIN_PATH" >&2
  exit 1
fi
if [ ! -d "$OUTPUT_DIR" ]; then
  echo "error: OUTPUT_DIR does not exist: $OUTPUT_DIR" >&2
  exit 1
fi

stem="hunkpick-${VER}-${TARGET}"
asset="${stem}.${ARCHIVE_EXT}"
output_path="${OUTPUT_DIR}/${asset}"

# Stage the payload in a clean directory under RUNNER_TEMP so the archive
# root is a single ${stem}/ folder containing binary + README + LICENSE.
stage="${RUNNER_TEMP}/${stem}"
rm -rf "$stage"
mkdir -p "$stage"
cp "$BIN_PATH" "${stage}/${BIN_NAME}"
cp README.md LICENSE "$stage/"

case "$ARCHIVE_EXT" in
  tar.gz)
    # Reproducible: sorted entries, fixed owner/mtime so a re-run of the
    # workflow on the same commit produces a byte-identical archive. The
    # --sort=name / --owner / --group / --numeric-owner / --mtime flags are
    # GNU-tar specific; BSD tar (macOS' default `tar`) does not accept them.
    # Prefer `gtar` when present (installed via `brew install gnu-tar` in
    # CI's macOS runner); fall back to `tar` on Linux/Windows runners where
    # GNU tar is already the default.
    if command -v gtar >/dev/null 2>&1; then
      TAR_BIN=gtar
    else
      TAR_BIN=tar
    fi
    "$TAR_BIN" --sort=name --owner=0 --group=0 --numeric-owner --mtime='@0' \
      -czf "$output_path" -C "$RUNNER_TEMP" "$stem"
    ;;
  zip)
    if ! command -v 7z >/dev/null 2>&1; then
      echo "error: 7z is required to produce a .zip archive but was not found" >&2
      exit 1
    fi
    # IMPORTANT: cd into RUNNER_TEMP and pass "$stem" (the directory name),
    # NOT "${stage}/*" — the latter expands to absolute paths and 7z stores
    # the files flat at the archive root, breaking the documented
    # single-top-level-directory contract.
    ( cd "$RUNNER_TEMP" && 7z a -tzip -mtc=off "$output_path" "$stem" >/dev/null )
    ;;
  *)
    echo "error: unknown archive extension: $ARCHIVE_EXT" >&2
    exit 1
    ;;
esac

( cd "$OUTPUT_DIR" && sha256sum "$asset" > "${asset}.sha256" )

echo "asset=${asset}"
