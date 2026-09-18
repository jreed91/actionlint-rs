# 1. Schema-driven structural layer, sourced externally and auto-resynced

Date: 2026-09-18
Status: Accepted

## Context

We are building a Rust GitHub Actions workflow linter (a rewrite of rhysd/actionlint).
The differentiating thesis ("delta") over actionlint is **A+D**:

- **A — External source of truth:** drive the *structural* validation layer from the
  community-maintained SchemaStore `github-workflow.json`, rather than from our own
  curated, hand-maintained data files (which is what actionlint does).
- **D — Automated re-sync:** a CI pipeline watches upstream sources and opens PRs to
  keep our vendored copies current with zero human initiative.

Established facts (grilling research, 2026-09-18):
- GitHub publishes **no** first-party machine-readable *JSON Schema*.
- GitHub DOES publish a first-party structural model: `actions/languageservices` →
  `workflow-parser/src/workflow-v1.0.json` (MIT, what GitHub's VS Code extension validates
  against). But it is a **custom DSL** (`mapping`/`sequence`/`one-of`/`loose-key-type`),
  NOT JSON Schema — not usable in a JSON-Schema validator without transpilation.
- SchemaStore's schema is community-maintained, draft-07, **structure only**, and lags
  GitHub, catching up reactively via human PRs (recently *more* current than GitHub's DSL).
- Contexts / functions / events exist only as GitHub **prose docs**; actionlint scrapes
  github/docs Markdown to generate its knowledge. This is brittle and breaks on doc
  reorganization.
- actionlint deliberately does NOT use SchemaStore — it hand-tunes its own rules for
  precision.

## Decision

**Upstream = option C (grilling 2026-09-18):**
- v1 **validation** is driven by the external SchemaStore `github-workflow.json` (draft-07,
  vendored, not fetched at lint time) — the only drop-in JSON Schema.
- The resync gate **also cross-checks** GitHub's first-party `workflow-v1.0.json` as a
  second signal: "SchemaStore and GitHub's parser disagree here" becomes a reviewable alert.
  GitHub's schema is *advisory / authority benchmark* in v1, not the validation engine.
- The **expression, context, and semantic layers** remain hand-maintained knowledge
  (ADR-scope: this decision covers structure only).

**North star = option B (post-MVP, see ROADMAP):** switch the validation upstream to
GitHub's first-party `workflow-v1.0.json` by writing a DSL→JSON-Schema **transpiler** in
the resync pipeline, so validation itself becomes first-party-sourced. Deliberately NOT in
v1 — building the transpiler before proving the basic pipeline is the over-reach we avoid.

A CI re-sync pipeline (D) watches SchemaStore (and, later, github/docs + GitHub's DSL) and
auto-PRs updates.

## Consequences

- **Accepted downgrade of the original goal.** "Changes automatically addressed" becomes
  "auto-resynced from upstream sources, which themselves lag GitHub by days-to-weeks."
  This must be stated honestly in user-facing docs; not marketed as real-time or
  first-party-guaranteed currency.
- We **inherit SchemaStore's coverage and its lag** for structure — we trade actionlint's
  hand-tuned precision for not maintaining structural data ourselves.
- We inherit SchemaStore's limitations: it does not enumerate contexts or expression
  functions, so those layers get us nothing from the schema and must be built separately.
- Re-sync reduces but does not eliminate brittleness (scraped layers still break on doc
  reorg).

## Alternatives considered (upstream)

- **A — SchemaStore only:** simplest, but community-only; Scenario-1 criticism ("you track
  a community source, not GitHub") stands unanswered. Chose C to consume the first-party
  source as authority without owning a transpiler yet.
- **B — GitHub DSL + transpiler, in v1:** strongest thesis but too big for the MVP; owning
  a DSL→JSON-Schema transpiler before proving the pipeline is over-reach. Kept as north star.
- **Just-Rust port of actionlint's design:** rejected as the *primary* thesis (we get its
  benefits anyway); not novel.
- **Live schema fetch at lint time:** rejected — network dependency + reproducibility.
