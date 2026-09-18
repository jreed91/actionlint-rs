# 4. Reconcile documented GitHub-vs-SchemaStore divergences before validation

Date: 2026-09-18
Status: Accepted

## Context

ADR-0001 commits us to validation *driven by the external SchemaStore schema*, explicitly
avoiding actionlint-style hand-tuning. But SchemaStore is a community artifact that can be
**stricter than GitHub Actions actually is**. Corpus curation (scraping real workflows from
popular repos) surfaced a concrete case:

- BurntSushi/ripgrep's CI declares `TARGET_FLAGS:` — an env var with no value (YAML null).
  GitHub Actions accepts this (empty env var); SchemaStore requires env values to be
  `string | number | boolean` and rejects null. Result: a **false positive** on a real,
  valid workflow.

A linter that flags valid workflows is worse than useless to its users, so we cannot simply
inherit every SchemaStore strictness.

## Decision

Introduce a single, centralized, documented reconciliation step (`src/reconcile.rs`),
applied to the parsed JSON *before* schema validation. Each reconciliation is a knowing,
narrowly-scoped exception where SchemaStore is provably stricter than GitHub.

Rule: **scalar coercion under `env:` and `with:`** — stringify null/number/boolean values
(GitHub coerces `RUST_BACKTRACE: 1` → `"1"`, `fetch-depth: 0` → `"0"`, `submodules: true` →
`"true"`, `FOO:` → `""`). This subsumes the original null-env rule and also unblocks the
first-party (option-B) schema, which types these values as `string` and so would otherwise
flag nearly every real workflow.

## Consequences

- **This deliberately diverges from "pure schema-driven" (ADR-0001).** We accept the
  divergence because false positives on valid workflows defeat the tool's purpose.
- **Sprawl is the risk.** To contain it: all reconciliations live in one module, each cites
  the concrete divergence (and a real workflow that tripped it), and each is a candidate to
  DELETE once fixed upstream (a SchemaStore PR) or once the option-B first-party-schema path
  lands (which shouldn't have the divergence).
- The advisory cross-check (`scripts/schema-crosscheck.sh`) is the systematic way we detect
  such divergences over time; reconciliations are the stopgap for ones that hurt users now.

## Alternatives considered

- **Do nothing / document only:** rejected — leaves a false positive that flags a common,
  valid pattern.
- **Patch the vendored schema:** rejected — a local fork breaks the auto-resync contract
  (next resync overwrites it) and undermines "driven by the external schema".
- **Coerce inline in the parser:** rejected — scatters divergences invisibly; the whole
  point is one auditable place.
