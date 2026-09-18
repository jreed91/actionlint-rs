# Roadmap: from thesis-only MVP to full actionlint replacement

This tracks the distance between the v1 MVP (structural validation + A+D auto-resync)
and a complete actionlint replacement. Each item notes whether the **schema-driven
thesis helps** — because most of the remaining value does NOT come from a schema, and
pretending otherwise is how this project would mislead its users.

## v1 (MVP) — in scope
- [ ] Parse workflow YAML into a form we can validate.
- [ ] Validate **structure** against vendored SchemaStore `github-workflow.json`.
- [ ] Map schema validation errors to good diagnostics (file:line:col, message).
- [ ] Schema vendored as committed file, embedded via `include_str!`.
- [ ] A+D **auto-resync pipeline**: scheduled CI fetches SchemaStore, diffs, opens PR.
- [ ] Resync gate **cross-checks** GitHub's first-party `workflow-v1.0.json` (advisory
      signal: flag SchemaStore-vs-GitHub disagreements for review).
- [ ] **Golden corpus** of workflows + expected lint results (trust gate). REQUIRED
      for the resync gate — real ongoing commitment (ADR-0003). Seed = **option C**:
      hand-authored known-BAD files (prove we catch structural errors) + scraped real-world
      known-GOOD workflows from popular public repos (prove no false positives on
      legit workflows) + actionlint's MIT test fixtures. Expected results are "golden by
      inspection" (run once, human-review the baseline).
- [ ] Resync PR runs linter w/ new schema over corpus, surfaces diff, human approves merge.
- [ ] **CLI** binary — the real primitive (lint files/dirs/stdin, exit codes, diagnostics).
- [ ] **Docker-based GitHub Action** (thin wrapper over the CLI) — v1 distribution.

## Deferred — the "other 90%" (post-MVP)

### Schema thesis helps here (structural/data, resync-able)
- [ ] Webhook **event / activity-type** validation — resync from octokit/openapi-webhooks
      or github/docs. (Data-driven; fits A+D.)
- [ ] Enum tightening the SchemaStore schema leaves loose (permissions scopes, shells,
      runner labels). (Data-driven.)

### Schema thesis does NOT help — hand-coded, the hard part
- [ ] **Expression `${{ }}` lexer/parser/AST.** (Syntax errors in expressions.)
- [ ] **Expression type system** — type-check `contains(...)`, `format(...)`, etc.
      against context types. (actionlint's `expr_sema.go` / `expr_type.go`; hand-coded.)
- [ ] **Context availability + dataflow** — which contexts/props are valid under which
      keys; step-output existence; `secrets`/`needs` availability. (Scraped from
      github/docs `contexts.md` — resync-able for the *table*, but the *analysis* is
      hand-coded.)
- [ ] **`needs:` graph** — cycle detection, undefined-job references.
- [ ] **`uses:` / action-metadata validation** — resolve `action.yml`, type-check `with:`
      inputs; popular-action metadata (actionlint's generated `popular_actions.json`).
- [ ] **Reusable workflow (`workflow_call`)** input/output/secret typing.
- [ ] **shellcheck** integration for `run:` bash/sh. (Spawn external binary.)
- [ ] **pyflakes** integration for `run:` python. (Spawn external binary.)
- [ ] **Security checks** — script-injection from untrusted input; hardcoded credentials.
- [ ] **Misc semantic** — glob syntax, cron syntax, deprecated actions/commands,
      constant `if:` conditions.

### Diagnostic quality (post-MVP polish)
- [x] **`oneOf` error messages humanized** (`src/humanize.rs`). Descends failed `oneOf`
      branches, picks the branch the user intended (deepest error path wins; wrong branches
      fail shallow by rejecting the instance's own keys), and renders the leaf in plain
      language, re-anchored to the deepest node. E.g. a job missing `runs-on` now reports
      `` `build` is missing required key `runs-on` `` instead of the schema jargon.
  - [ ] Follow-up: ambiguous job branches (only `with:`, no `runs-on`/`uses`) report
        "missing runs-on"; could detect `with:`/`secrets:` as a reusable-job signal.

### Distribution / ecosystem parity (post-MVP)
- [ ] **SARIF / JSON output** + problem matcher — enables inline PR annotations.
- [ ] `-ignore` filtering.
- [ ] Composite action + multi-target release binaries (cross-compilation) — faster,
      cross-OS UX vs the v1 Docker action.
- [ ] pre-commit hook, Docker image, editor integrations, WASM playground.

## North star (option B) — the compelling post-MVP direction
- [ ] **DSL→JSON-Schema transpiler** in the resync pipeline: fetch GitHub's first-party
      `actions/languageservices` `workflow-v1.0.json`, transpile its custom DSL
      (`mapping`/`sequence`/`one-of`/`loose-key-type`) into JSON Schema, embed, and
      validate against THAT. Makes validation itself first-party-sourced — the strongest
      form of "schema-driven, tracks GitHub." Owns a transpiler that can break on new DSL
      keywords (that's the cost). (ADR-0001.)

## Honest note
Roughly **~10%** of actionlint's user value (structure) is what the thesis addresses.
Reaching full replacement means reimplementing the hand-coded ~90%, which is orthogonal
to the schema-driven idea. The MVP proves the thesis; it does not replace actionlint.
