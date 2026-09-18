# Golden corpus (regression gate)

This corpus is the trust mechanism for the auto-resync pipeline (ADR-0003). Every resync
PR runs the linter over this corpus; if results change, a human reviews the diff before
merging. It is **required**, not optional.

Seeding strategy = **option C** (grilling 2026-09-18):

- `good/` — real-world, known-GOOD workflows that must produce **zero** diagnostics.
  Proves the schema + span-mapping don't false-positive on legitimate workflows. Seed
  these by scraping `.github/workflows` from popular public repos.
- `bad/` — hand-authored known-BAD workflows that must produce **at least one**
  diagnostic. Proves the linter actually catches structural errors. Each `bad/<name>.yml`
  should have a sibling `bad/<name>.expected` describing what must be caught (informational
  in v1; the gate only asserts "non-empty").

Expected results are "golden by inspection": run the linter once, review the output, and
commit it as the baseline. When a resync changes a baseline, that change is what the human
reviews.

## Current contents

`good/` (must lint clean):
- `minimal.yml`, `matrix-and-expressions.yml` — hand-authored basics (+ opaque expressions).
- `null-env-value.yml` — regression for the null-env reconciliation (see `src/reconcile.rs`).
- `real-*.yml` — curated real-world workflows scraped from popular public repos
  (actions/checkout, rust-lang/cargo, rust-lang/mdBook, BurntSushi/ripgrep, sharkdp/bat,
  cli/cli). Each was linted and confirmed clean during curation.

`bad/` (must produce >=1 diagnostic):
- `missing-runs-on.yml`, `on-wrong-type.yml`, `missing-jobs.yml`, `unknown-job-key.yml`,
  `step-has-both-run-and-uses.yml`.

## Curation findings
- **SchemaStore false positive (null env values):** ripgrep's real CI declares `TARGET_FLAGS:`
  (empty env value = YAML null). SchemaStore rejects null env values; GitHub accepts them.
  Handled by a documented reconciliation in `src/reconcile.rs`, not by editing the vendored
  schema. `null-env-value.yml` guards it.

## TODO (roadmap)
- [x] Scrape a corpus of real known-good workflows from popular repos.
- [ ] Broaden further (containers, services, concurrency, reusable-workflow callers).
- [ ] Import applicable MIT-licensed test fixtures from rhysd/actionlint.
- [ ] Add snapshot-based expected output per file (insta) rather than just non-empty.
