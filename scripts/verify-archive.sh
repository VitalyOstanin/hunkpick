#!/usr/bin/env bash
# Verify a release archive against the downstream-packager contract.
#
# Usage: verify-archive.sh <asset_path> <bin_name>
#
# Checks:
#   1. Filename matches hunkpick-<version>-<target>.<tar.gz|zip>
#   2. Sibling .sha256 exists and `sha256sum -c` passes
#   3. Archive extracts to a single top-level directory matching the stem
#   4. That directory contains exactly: <bin_name>, README.md, LICENSE

set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "Usage: $0 <asset_path> <bin_name>" >&2
  exit 64
fi

ASSET=$1
BIN_NAME=$2

if [ ! -f "$ASSET" ]; then
  echo "error: asset not found: $ASSET" >&2
  exit 1
fi

asset_dir=$(cd "$(dirname "$ASSET")" && pwd)
asset_file=$(basename "$ASSET")

# 1. Filename pattern. The version segment accepts SemVer plus an optional
#    pre-release / build suffix so a future v0.2.0-rc.1 still passes.
pattern='^hunkpick-([0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.+-]+)?)-([A-Za-z0-9_+-]+)\.(tar\.gz|zip)$'
if ! [[ "$asset_file" =~ $pattern ]]; then
  echo "error: asset filename does not match template: $asset_file" >&2
  echo "  expected pattern: hunkpick-<version>-<target>.(tar.gz|zip)" >&2
  exit 1
fi
version="${BASH_REMATCH[1]}"
target="${BASH_REMATCH[3]}"
ext="${BASH_REMATCH[4]}"
stem="hunkpick-${version}-${target}"

# 2. Sibling .sha256 must exist and verify cleanly. sha256sum -c reads the
#    archive name from the file, so it must be run with cwd at the asset's
#    directory.
sha_file="${asset_file}.sha256"
if [ ! -s "${asset_dir}/${sha_file}" ]; then
  echo "error: missing or empty sha256 companion: ${sha_file}" >&2
  exit 1
fi
if ! ( cd "$asset_dir" && sha256sum -c "$sha_file" >/dev/null 2>&1 ); then
  echo "error: sha256sum -c failed for ${sha_file}" >&2
  ( cd "$asset_dir" && sha256sum -c "$sha_file" ) >&2 || true
  exit 1
fi

# 3 + 4. Extract and inspect the layout. Tempdir cleanup via trap so a
# failed assertion still removes the extracted tree.
extract_dir=$(mktemp -d)
trap 'rm -rf "$extract_dir"' EXIT

case "$ext" in
  tar.gz)
    tar -xzf "$ASSET" -C "$extract_dir"
    ;;
  zip)
    if ! command -v 7z >/dev/null 2>&1; then
      echo "error: 7z is required to verify a .zip archive but was not found" >&2
      exit 1
    fi
    7z x "$ASSET" -o"$extract_dir" -y >/dev/null
    ;;
  *)
    echo "error: unknown archive extension: $ext" >&2
    exit 1
    ;;
esac

# Top-level entries: must be exactly one, a directory named $stem.
# Use POSIX find + sed instead of GNU-only `-printf` so the script runs on
# the macos-latest GHA runner (BSD find). `mapfile`/`readarray` is bash-4+
# (Linux runners), but macos-latest ships system bash 3.2, so use a
# portable while-read loop instead.
top_entries=()
while IFS= read -r line; do
  top_entries+=("$line")
done < <(
  cd "$extract_dir" && find . -mindepth 1 -maxdepth 1 | sed 's|^\./||' | sort
)
if [ "${#top_entries[@]}" -ne 1 ] || [ "${top_entries[0]}" != "$stem" ]; then
  echo "error: archive must contain exactly one top-level directory named '${stem}'" >&2
  echo "  found ${#top_entries[@]} top-level entries:" >&2
  for e in "${top_entries[@]}"; do
    echo "    $e" >&2
  done
  exit 1
fi

root="${extract_dir}/${stem}"
if [ ! -d "$root" ]; then
  echo "error: top-level entry '${stem}' is not a directory" >&2
  exit 1
fi

# Required files: exactly <bin_name>, README.md, LICENSE -- no extras.
required=("$BIN_NAME" "README.md" "LICENSE")
for f in "${required[@]}"; do
  if [ ! -f "${root}/${f}" ]; then
    echo "error: missing required file in archive: ${f}" >&2
    exit 1
  fi
done

actual=()
while IFS= read -r line; do
  actual+=("$line")
done < <(
  cd "$root" && find . -mindepth 1 -maxdepth 1 | sed 's|^\./||' | sort
)
expected_sorted=$(printf '%s\n' "${required[@]}" | sort)
actual_joined=$(printf '%s\n' "${actual[@]}")
if [ "$actual_joined" != "$expected_sorted" ]; then
  echo "error: unexpected files in archive root" >&2
  echo "  expected:" >&2
  printf '%s\n' "$expected_sorted" | sed 's/^/    /' >&2
  echo "  actual:" >&2
  printf '%s\n' "$actual_joined" | sed 's/^/    /' >&2
  exit 1
fi

echo "ok: ${asset_file}" >&2
