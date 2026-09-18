# Roadmap: from thesis-only MVP to full actionlint replacement

This tracks the distance between the v1 MVP (structural validation + A+D auto-resync)
and a complete actionlint replacement. Each item notes whether the **schema-driven
thesis helps** — because most of the remaining value does NOT come from a schema, and
pretending otherwise is how this project would mislead its users.

## v1 (MVP) — in scope
- [x] Parse workflow YAML into a form we can validate. (`src/yaml.rs`)
- [x] Validate **structure** against vendored SchemaStore `github-workflow.json`.
- [x] Map schema validation errors to good diagnostics (file:line:col, message) — with
      humanized `oneOf` messages (`src/humanize.rs`) and 1-indexed positions.
- [x] Schema vendored as committed file, embedded via `include_str!`. (`src/schema.rs`)
- [x] A+D **auto-resync pipeline**: scheduled CI fetches SchemaStore, diffs, opens PR.
      (`.github/workflows/resync-schema.yml`)
- [x] Resync gate **cross-checks** GitHub's first-party `workflow-v1.0.json` (advisory
      signal). (`scripts/schema-crosscheck.sh`, run in resync workflow.)
- [x] **Golden corpus** (option C): 6 curated real-world known-GOOD workflows + hand-authored
      known-GOOD + known-BAD cases. Curation surfaced a real SchemaStore false positive
      (null env values), handled in `src/reconcile.rs`.
- [x] Resync PR runs linter over corpus (`scripts/run-corpus.sh` in CI), human approves merge.
- [x] **CLI** binary — the real primitive (lint files/dirs/stdin, exit codes). (`src/main.rs`)
- [x] **Docker-based GitHub Action** (thin wrapper over the CLI). (`action.yml`, `Dockerfile`)

- [x] **Docker action tested end-to-end in CI** (`.github/workflows/ci.yml` `docker-action`
      job): builds the image, runs it over the workspace (default empty-input → discovery,
      exit 0), an explicit good file, a broken workflow (expect exit 1), and via `uses: ./`
      to exercise `action.yml` wiring. Fixed a real bug it surfaced: the default action
      invocation passed an empty-string arg that clap rejected — now filtered to fall
      through to discovery.

**MVP is complete.** Remaining optional v1 hardening: broaden the corpus further.

## Deferred — the "other 90%" (post-MVP)

### Schema thesis helps here (structural/data, resync-able)
- [ ] Webhook **event / activity-type** validation — resync from octokit/openapi-webhooks
      or github/docs. (Data-driven; fits A+D.)
- [ ] Enum tightening the SchemaStore schema leaves loose (permissions scopes, shells,
      runner labels). (Data-driven.)

### Full check parity (ADR-0006) — hand-coded, in progress
- [x] **Expression `${{ }}` lexer/parser/AST** (`src/expr/`, step 1).
- [x] **Expression type system** — contexts + built-in function signatures, type-checking
      (`src/expr/ty.rs`, `builtins.rs`, `check.rs`, step 2).
- [x] **Expression checking wired into the linter** (`src/expr_lint.rs`, step 3): every
      `${{ }}` in a workflow string is parsed + type-checked; diagnostics anchored at the
      containing scalar. On by default; ZERO false positives across the 29-workflow xval
      corpus. (Precise intra-string column is a future refinement.)
- [x] Context **availability by position** (`src/expr/availability.rs`, step 4): DSL-derived
      table (position → allowed contexts, from the DSL's own `context:` annotations, so it
      resyncs) + a pointer→position classifier; the checker flags a context used where it
      isn't available (e.g. `secrets` in `runs-on`). On by default; ZERO false positives
      across the 29-workflow xval corpus.
  - [ ] Follow-up: step-output existence (`steps.<id>.outputs.<name>`), needs-output typing.
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
- [x] **SARIF 2.1.0 output** (`--format sarif`, `src/sarif.rs`) — enables inline PR
      annotations via `github/codeql-action/upload-sarif`. Each finding carries a grouped
      `ruleId` (structure/required, /type, ...), a full region (start+end from the span),
      and a stable partial fingerprint. Verified against GitHub's SARIF-support docs.
  - [ ] Follow-up: plain JSON output; a problem-matcher path as a lighter alternative.
- [ ] `-ignore` filtering.
- [ ] Composite action + multi-target release binaries (cross-compilation) — faster,
      cross-OS UX vs the v1 Docker action.
- [ ] pre-commit hook, Docker image, editor integrations, WASM playground.

## North star (option B) — DONE (first cut)
- [x] **DSL→JSON-Schema transpiler** (`src/transpile.rs`): full-fidelity transpile of
      GitHub's first-party `workflow-v1.0.json` DSL (all keywords: `string`/`number`/
      `boolean`/`null`, `mapping`, `sequence`/`item-type`, `one-of`, `constant`,
      `allowed-values`, `require-non-empty`, `loose-key/value-type`, `$ref`s, `required`,
      injected implicit primitives). Validated against the real 304-definition DSL with zero
      unsupported constructs.
- [x] Wired into the linter: `--schema first-party` (`schema::build_first_party_validator`),
      embedded via `include_str!`. Makes validation first-party-sourced (ADR-0001).
- [x] Proven usable on real workflows: all good-corpus files pass, all bad flagged, under
      BOTH schema sources. Surfaced (and reconciled) the first-party schema's stricter
      scalar typing for `env:`/`with:` (ADR-0004).
- [x] **First-party is now the DEFAULT schema source** (`SchemaSource::default()` =
      `FirstParty`; `--schema schemastore` opts out). The corpus gate runs first-party by
      default, giving the north star ongoing regression coverage.
- [x] **First-party DSL added to the resync pipeline** (`.github/workflows/resync-schema.yml`):
      one combined weekly job now fetches BOTH `github-workflow.json` and `workflow-v1.0.json`,
      opens a single PR when either changed (flagging a first-party change as the higher-stakes
      one, since it drives default validation), and verifies before opening the PR that the new
      schemas build + transpile + pass the corpus gate.
- [x] **Cross-validation gate over a broad corpus** (`tests/xval.rs`, `corpus/xval/`):
      29 real workflows across ecosystems (Rust/Node/Python/Go/containers/deploy/pages)
      validated under BOTH schemas; a checked-in
      baseline (`corpus/xval/BASELINE.txt`) records accepted disagreements, and the gate
      fails on any drift (new or resolved). Wired into CI; proven to catch drift.
- [x] **Expression-in-value-position fidelity fixed** (ADR-0005). The transpiler widens
      boolean/number/sequence globally, and context-annotated one-of/mapping nodes per-node,
      to `anyOf[<base>, <${{ }}> expression]`. Broadening the corpus to 29 workflows drove
      expanding this from context-only to global-scalar (it found expressions at typed leaves
      like `concurrency.cancel-in-progress` and matrix values). Only remaining cross-schema
      divergence is `requests-tests.yml` — SchemaStore wrongly requires `strategy.matrix`;
      first-party (correctly) does not (baselined).

## Honest note
Roughly **~10%** of actionlint's user value (structure) is what the thesis addresses.
Reaching full replacement means reimplementing the hand-coded ~90%, which is orthogonal
to the schema-driven idea. The MVP proves the thesis; it does not replace actionlint.
