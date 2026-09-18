# 3. Schema vendoring and the auto-resync trust gate (the "D" in A+D)

Date: 2026-09-18
Status: Accepted

## Context

The thesis includes **D**: auto-resync so the structural schema never rots without human
initiative. But upstream SchemaStore is community-maintained, hand-patched, and now merges
AI-authored PRs — so a resync could restructure schema, tighten an enum, or ship a bug
that makes our linter flag valid workflows or crash. "Automated" is worthless if it isn't
"trustworthy."

## Decision

**Vendoring:** the schema is a **committed file** in the repo (e.g.
`schemas/github-workflow.json`), embedded into the binary at compile time
(`include_str!`). No build-time or lint-time fetch.

**Resync pipeline:** a scheduled CI job fetches upstream SchemaStore, diffs against the
committed file, and if changed, opens a **PR** updating the committed file.

**Trust gate (the load-bearing part):** a **golden corpus** of real workflow files with
known-expected lint results runs in CI on every resync PR. The PR runs the linter with the
**new** schema over the corpus and surfaces any diff in results. **Never auto-merge** — a
human reviews the results diff and approves. "Auto-resynced, every resync regression-tested
against a real-workflow corpus before it can land."

## Consequences

- **New commitment:** we must **build and maintain a golden corpus** of real workflows +
  expected lint output. This is real, ongoing work — added to the roadmap. Without it, D is
  not defensible.
- Builds stay reproducible and git-auditable; the PR diff shows exactly what changed.
- "Zero human initiative" downgrades honestly to "zero initiative to *detect and prepare*
  the update; a human still approves the merge." This is the right trade for trust.
- Resync currency is still bounded by SchemaStore's own lag (see ADR-0001).

## Alternatives considered

- **Build-time fetch:** rejected — non-reproducible, no audit trail.
- **Auto-merge if unit tests pass:** rejected — misses behavioral regressions from schema
  changes, which are exactly the risk.
- **Manual review, no corpus:** rejected — undercuts the D selling point and doesn't scale.
