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

# Keep Cargo.lock's own-package version in sync with Cargo.toml. A mismatch makes
# `cargo build --locked` (used by the release build job) fail, so this must be reliable — we
# edit the lockfile directly rather than depend on `cargo update` succeeding in CI.
#
# The lockfile lists our package as:
#   [[package]]
#   name = "actionlint-rs"
#   version = "<x.y.z>"
# Update only the `version` line that immediately follows our package's `name` line.
if [[ -f Cargo.lock ]]; then
  tmp="$(mktemp)"
  awk -v v="$VERSION" '
    /^name = "actionlint-rs"$/ { found=1 }
    found && /^version = "[^"]+"/ { sub(/"[^"]+"/, "\"" v "\""); found=0 }
    { print }
  ' Cargo.lock > "$tmp"
  mv "$tmp" Cargo.lock
  echo "set-version.sh: Cargo.lock own-package version -> $VERSION"
  # Best-effort validation that the two now agree (never fatal in CI).
  if command -v cargo >/dev/null 2>&1; then
    cargo update --offline -p actionlint-rs --precise "$VERSION" >/dev/null 2>&1 || true
  fi
fi
