# Context — actionlint-rs (working name)

A Rust rewrite of [actionlint](https://github.com/rhysd/actionlint): a linter for
GitHub Actions workflow files. Primary goal is a **schema-driven** design so that
structural changes to GitHub's workflow format are tracked automatically rather
than hand-coded.

## Glossary

### Workflow
A GitHub Actions workflow file: a YAML document (typically under `.github/workflows/`)
describing triggers (`on:`), `jobs`, and `steps`. The primary artifact this tool lints.

### Structural layer
The part of validation that a JSON Schema *can* describe: which keys exist, their
types, required/optional, allowed enum values, nesting shape. Example:
"`jobs.<id>.runs-on` is required and is a string or list."

### Expression layer
The `${{ }}` mini-language: typed expressions over contexts and built-in functions
(`contains`, `format`, ...). **Not schema-describable.** A schema says nothing about
whether `contains(github.event.x, 'y')` type-checks.

### Context layer
Availability and shape of context objects (`github`, `env`, `steps`, `needs`, ...)
and their properties, including dataflow rules (a step output only exists if an
earlier step produced it; `secrets` unavailable in some positions). **Not
schema-describable** — this is dataflow analysis.

### "The schema" (there is no single one)
There is **no** first-party, machine-readable GitHub schema for workflows. What exists:
- **SchemaStore `github-workflow.json`** — community-maintained, draft-07, **structure only**,
  lags GitHub and catches up reactively via human PRs. (The de-facto structural schema.)
- **Contexts / functions / events** — GitHub publishes these as **prose docs only**.
  Every current tool (actionlint, octokit) derives machine-readable form by **scraping**
  GitHub's Markdown/HTML docs — brittle, breaks on doc reorganization.
- **Webhook payloads** — `octokit/openapi-webhooks` (OpenAPI, docs-derived, community).
So "the schema" is a **layered, partly-scraped, partly-hand-maintained artifact**, not one file.

### Semantic layer
Rules requiring domain knowledge beyond structure: deprecated action versions,
unreachable glob patterns, shell bugs (via shellcheck), Python bugs (via pyflakes).
Hand-coded and/or delegated to external tools.

### Span / source position
The `line:column` range in the source YAML that a diagnostic points at. Precise spans are
a **v1 requirement**. Consequence: the hard part of the MVP is mapping a schema-validation
error (which lives at a JSON path like `/jobs/build/runs-on`) back to the exact YAML span —
NOT the schema validation itself, which is an off-the-shelf crate.

## Decisions (see docs/adr/ for the hard ones)

- **Scope of "schema-driven":** The schema drives the **structural layer only**.
  The expression, context, and semantic layers remain hand-maintained knowledge.
  The win is that the structural layer stops rotting — NOT that the whole linter
  becomes maintenance-free. (Grilling session 2026-09-18.)
- **Primary motivation:** Prove the schema-driven *thesis* (goal type D). Rust and
  "newest schema compatibility" are in service of that thesis, not ends in themselves.
- **Not a literal fork.** A *fresh* repo that credits actionlint (MIT) as inspiration —
  not a git fork of the Go repo (no shared code; entirely different architecture).
- **Working name:** `actionlint-rs` (placeholder; distinct from actionlint, not implying
  official-successor status).
- **License:** MIT (matches actionlint; conventional; permits borrowing test/corpus ideas).
- **v1 output:** human-readable `file:line:col: message` only. SARIF/JSON (and the inline
  PR annotations SARIF enables) deferred post-MVP.
- **Expressions opaque in v1.** `${{ }}` is treated as an opaque string; v1 does NOT
  validate inside it. A structurally-valid workflow with a broken expression PASSES v1.
  Documented known limitation.
- **CLI is the primitive.** The linter is a CLI first; the GitHub Action is a thin
  wrapper over it. v1 Action = **Docker-based** (`runs: using: docker`), fewest moving
  parts. Composite + multi-target release binaries deferred (post-MVP UX/speed).
  (Grilling session 2026-09-18.)
- **v1 scope = thesis-only MVP.** v1 validates workflow **structure** against the
  vendored SchemaStore schema + ships the A+D auto-resync pipeline, and nothing else.
  Deliberately a worse linter than actionlint; the point is to prove schema-driven +
  auto-resync works and stays current. Path to full replacement tracked in
  `docs/ROADMAP.md`. (Grilling session 2026-09-18.)
