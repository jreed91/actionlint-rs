#!/usr/bin/env bash
# Advisory cross-check (option C, ADR-0001): compare the vendored SchemaStore schema against
# GitHub's first-party workflow-v1.0.json and report divergences for HUMAN review.
#
# The two schemas are different formats (SchemaStore is JSON Schema; GitHub's is a custom
# DSL), so we cannot diff them directly. Instead we extract a comparable *signal* — the set
# of webhook event names each recognizes — and report names present in one but not the other.
# A divergence is not a failure; it flags that SchemaStore may be lagging GitHub (or vice
# versa), worth a look when reviewing a resync PR.
#
# Exit status is always 0 (advisory only).
set -euo pipefail
cd "$(dirname "$0")/.."

SCHEMASTORE="schemas/github-workflow.json"
FIRSTPARTY_URL="https://raw.githubusercontent.com/actions/languageservices/main/workflow-parser/src/workflow-v1.0.json"
FIRSTPARTY="${FIRSTPARTY_JSON:-/tmp/workflow-v1.0.json}"

if [[ ! -f "$FIRSTPARTY" ]]; then
  echo "Fetching first-party schema..."
  curl -sfL "$FIRSTPARTY_URL" -o "$FIRSTPARTY" || {
    echo "advisory cross-check: could not fetch first-party schema (skipping)"
    exit 0
  }
fi

python3 - "$SCHEMASTORE" "$FIRSTPARTY" <<'PY'
import json, sys

ss = json.load(open(sys.argv[1]))
fp = json.load(open(sys.argv[2]))

# SchemaStore: event names are an enum under definitions.event.
try:
    ss_ev = set(ss["definitions"]["event"]["enum"])
except Exception:
    ss_ev = set()

# First-party DSL: event names are the property keys of `on-mapping-strict`.
try:
    fp_ev = set(fp["definitions"]["on-mapping-strict"]["mapping"]["properties"].keys())
except Exception:
    fp_ev = set()

print("=== Schema cross-check (advisory) ===")
print(f"SchemaStore events:  {len(ss_ev)}")
print(f"First-party events:  {len(fp_ev)}")

only_fp = sorted(fp_ev - ss_ev)
only_ss = sorted(ss_ev - fp_ev)

if only_fp:
    print("\n[REVIEW] Events GitHub's first-party schema has but SchemaStore lacks")
    print("         (SchemaStore may be lagging — verify against GitHub docs):")
    for e in only_fp:
        print(f"  + {e}")
if only_ss:
    print("\n[REVIEW] Events SchemaStore has but the first-party schema lacks")
    print("         (SchemaStore may be ahead, or naming differs):")
    for e in only_ss:
        print(f"  - {e}")
if not only_fp and not only_ss:
    print("\nEvent sets agree. No divergence.")

print("\nNote: schemas use different formats; this compares event-name signals only.")
print("Divergences are advisory — investigate, do not block the resync on them.")
PY

exit 0
