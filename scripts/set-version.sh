#!/usr/bin/env bash
# Set the crate version in Cargo.toml (and refresh Cargo.lock) to the version semantic-release
# computed. Called by @semantic-release/exec's prepareCmd as: set-version.sh <version>.
#
# Only the [package] `version` is touched — the sed is anchored to the first `version = "..."`
# line, which in this single-crate manifest is the package version (dependency versions come
# later and are not at column 0 in the same shape). Guarded so a malformed arg or an
# unexpected manifest fails loudly rather than silently corrupting the file.
set -euo pipefail

VERSION="${1:?usage: set-version.sh <x.y.z>}"

# Validate a semver-ish x.y.z (optionally with a prerelease/build suffix).
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+].+)?$ ]]; then
  echo "set-version.sh: refusing to set non-semver version '$VERSION'" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

if ! grep -qE '^version = "[^"]+"' Cargo.toml; then
  echo "set-version.sh: could not find a package version line in Cargo.toml" >&2
  exit 1
fi

# Replace only the FIRST `version = "..."` (the [package] version). Use a portable sed form
# that works on both GNU (Linux CI) and BSD (macOS) sed.
tmp="$(mktemp)"
awk -v v="$VERSION" '
  !done && /^version = "[^"]+"/ { sub(/"[^"]+"/, "\"" v "\""); done=1 }
  { print }
' Cargo.toml > "$tmp"
mv "$tmp" Cargo.toml

echo "set-version.sh: Cargo.toml version -> $VERSION"

# Refresh Cargo.lock so the locked package version matches. `--offline` avoids a network hit;
# the lockfile already has every dependency, only our own package version changes.
if command -v cargo >/dev/null 2>&1; then
  cargo update --offline -p actionlint-rs --precise "$VERSION" 2>/dev/null \
    || cargo generate-lockfile --offline 2>/dev/null \
    || true
  echo "set-version.sh: Cargo.lock refreshed"
fi
