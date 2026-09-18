#!/usr/bin/env bash
# Golden-corpus regression gate (ADR-0003).
#   - every corpus/good/*.yml MUST produce zero diagnostics (exit 0)
#   - every corpus/bad/*.yml  MUST produce >=1 diagnostic (exit 1)
# Any deviation fails, so a resync PR that changes results cannot pass without a human
# updating the corpus and reviewing the diff.
set -euo pipefail

cd "$(dirname "$0")/.."
BIN="${ACTIONLINT_RS_BIN:-}"
if [[ -z "$BIN" ]]; then
  cargo build --release >/dev/null
  BIN="./target/release/actionlint-rs"
fi

fail=0

for f in corpus/good/*.yml corpus/good/*.yaml; do
  [[ -e "$f" ]] || continue
  if "$BIN" "$f" >/tmp/out 2>&1; then
    echo "GOOD ok:   $f"
  else
    echo "GOOD FAIL: $f (expected zero diagnostics, got):"
    sed 's/^/    /' /tmp/out
    fail=1
  fi
done

for f in corpus/bad/*.yml corpus/bad/*.yaml; do
  [[ -e "$f" ]] || continue
  if "$BIN" "$f" >/tmp/out 2>&1; then
    echo "BAD  FAIL: $f (expected >=1 diagnostic, got none)"
    fail=1
  else
    echo "BAD  ok:   $f"
  fi
done

if [[ "$fail" -ne 0 ]]; then
  echo ""
  echo "Corpus regression gate FAILED."
  exit 1
fi
echo ""
echo "Corpus regression gate passed."
