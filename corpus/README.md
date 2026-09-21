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
- `containers-and-services.yml` — job `container:` (image/env/ports/options/credentials) +
  `services:` (postgres/redis with ports, options, health).
- `concurrency-and-matrix.yml` — workflow- and job-level `concurrency:`, a matrix with
  `include`/`exclude`/`fail-fast`/`max-parallel`, and job `outputs`.
- `reusable-callee.yml` / `reusable-caller.yml` — a local reusable workflow (`workflow_call`
  with typed inputs/secrets/outputs) and a caller that supplies them, uses `needs`, and
  consumes the callee's output. Exercises the reusable-workflow typing pass (`src/reusable.rs`).

`bad/` (must produce >=1 diagnostic):
- `missing-runs-on.yml`, `on-wrong-type.yml`, `missing-jobs.yml`, `unknown-job-key.yml`,
  `step-has-both-run-and-uses.yml`.
- `unknown-event.yml` — a typo'd webhook event (`pull-request`); exercises `event/name`.
- `reusable-job-missing-uses.yml` — a job with `with:` but no `uses:`/`runs-on:`; exercises
  the humanizer's reusable-job-shape detection ("missing `uses`", not "missing runs-on").

## Snapshot gate

Beyond the coarse `run-corpus.sh` assertion (good → zero diagnostics, bad → ≥1), every corpus
file's **exact** linter output is pinned as an `insta` snapshot in `tests/corpus_snapshot.rs`
(`tests/snapshots/corpus_snapshot__{good,bad}__<file>.snap`). Any change to *what* is reported
— a new finding, a reworded message, a moved anchor — shows up as a reviewable snapshot diff.
Update intentionally with `cargo insta review` (or `INSTA_UPDATE=always cargo test`).

## Curation findings
- **SchemaStore false positive (null env values):** ripgrep's real CI declares `TARGET_FLAGS:`
  (empty env value = YAML null). SchemaStore rejects null env values; GitHub accepts them.
  Handled by a documented reconciliation in `src/reconcile.rs`, not by editing the vendored
  schema. `null-env-value.yml` guards it.

## TODO (roadmap)
- [x] Scrape a corpus of real known-good workflows from popular repos.
- [x] Broaden further (containers, services, concurrency, reusable-workflow callers) — see
      `good/containers-and-services.yml`, `good/concurrency-and-matrix.yml`, and the
      `reusable-callee.yml`/`reusable-caller.yml` pair, plus `bad/unknown-event.yml` and
      `bad/reusable-job-missing-uses.yml` for the new checks' negative paths.
- [ ] Import applicable MIT-licensed test fixtures from rhysd/actionlint. **Deferred: fetching
      upstream fixtures needs network access (out of scope for this offline environment), and
      most of actionlint's fixtures are Go-specific error snapshots rather than portable
      workflow YAML. Revisit when network is available; import only the reusable `.yml` inputs
      under MIT attribution.**
- [x] Add snapshot-based expected output per file (insta) rather than just non-empty — see the
      "Snapshot gate" section above (`tests/corpus_snapshot.rs`).
