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

## TODO (roadmap)
- [ ] Scrape a corpus of real known-good workflows from popular repos.
- [ ] Import applicable MIT-licensed test fixtures from rhysd/actionlint.
- [ ] Add snapshot-based expected output per file (insta) rather than just non-empty.
