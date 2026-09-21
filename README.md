# actionlint-rs

A fast, schema-driven linter for GitHub Actions workflow files, written in Rust.

Inspired by [rhysd/actionlint](https://github.com/rhysd/actionlint) (MIT) — a fresh
implementation, not a fork. Its distinguishing idea is that the **structural** layer is
driven by GitHub's own machine-readable schema and kept current by an automated,
regression-gated resync pipeline, rather than hand-coded rules. The semantic checks
(expressions, job graph, security) are layered on top.

## What it checks

- **Structure** — validates the workflow against GitHub's first-party schema
  (`actions/languageservices` `workflow-v1.0.json`, transpiled to JSON Schema at runtime),
  with plain-language diagnostics: a job missing `runs-on` reports
  `` `build` is missing required key `runs-on` `` rather than raw JSON-Schema `oneOf` jargon.
- **Expressions** (`${{ }}`) — type-checks contexts, properties, functions, and arity;
  flags unknown contexts (`gihtub.sha`), unknown properties (`runner.oss`), and syntax
  errors, and enforces **context availability by position** (e.g. `secrets` is not available
  in `runs-on`).
- **Job graph** — `needs:` referencing undefined jobs, and dependency cycles.
- **Dataflow** — `steps.<id>...` must name a step defined in the same job; `needs.<job>...`
  must name a job listed in `needs:`.
- **Reusable workflows** — a job calling a *local* `workflow_call` workflow
  (`uses: ./....yml`) is checked against the callee's declared inputs/secrets (required
  supplied, no unknowns; `secrets: inherit` respected). Remote callees need network, skipped.
- **Action references** — `uses:` must be a valid `owner/repo@ref`, `./local`, or
  `docker://image` (a pinned ref is required for repository actions).
- **Scripts** — if `shellcheck` / `pyflakes` are installed, `run:` blocks are linted through
  them (disable with `--no-external`).
- **Security & misc** — script injection from untrusted input (`${{ github.event.*.title }}`,
  `github.head_ref`, …), hardcoded credentials, deprecated `::set-output::` / `::save-state::`
  commands, cron syntax, and constant `if:` conditions (`if: false` never runs).
- **Events** — `on:` event names (a typo like `pull-request` for `pull_request`) and activity
  `types:` (`opend` for `opened`).
- **Enums** — `permissions` scopes and levels (derived from GitHub's own schema, so
  per-scope: `id-token` accepts only `write`/`none`) and `shell` keywords.
- **Runner labels** (opt-in, `--check-runner-labels`) — `runs-on` labels checked against the
  known GitHub-hosted set plus self-hosted labels declared in config.

Every check (except runner labels) is on by default and validated against a corpus of
real-world workflows for false positives.

## Usage

```sh
actionlint-rs                          # lint .github/workflows/*.{yml,yaml}
actionlint-rs path/to/workflow.yml     # lint specific files
cat workflow.yml | actionlint-rs -     # lint stdin
```

Common flags:

```sh
--format sarif            # SARIF 2.1.0 output (for GitHub code scanning)
--format json             # plain JSON array of diagnostics (for jq / scripting)
--schema schemastore      # validate against the community SchemaStore schema instead
--ignore <REGEX>          # suppress diagnostics whose message matches REGEX (repeatable)
--no-external             # skip shellcheck / pyflakes
--config <FILE>           # use a specific config file (default: .github/actionlint.yaml)
--no-config               # ignore any config file
--check-runner-labels     # also validate runs-on labels (needs config for self-hosted)
```

Exit codes: `0` clean, `1` problems found, `2` usage/IO error.

### Configuration

An optional `.github/actionlint.yaml` (actionlint-compatible) is auto-discovered:

```yaml
self-hosted-runner:
  labels:            # custom labels accepted by --check-runner-labels
    - linux-arm64-16core
ignore:              # message-regex suppressions, committed to the repo
  - 'unknown runner label'
```

### As a pre-commit hook

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/jreed91/actionlint-rs
    rev: v0.1.0
    hooks:
      - id: actionlint-rs
```

### As a GitHub Action

```yaml
- uses: your-org/actionlint-rs@v1
  # optional: files: ".github/workflows/ci.yml"
```

### Inline PR annotations via SARIF

```yaml
- name: Lint workflows
  run: actionlint-rs --format sarif > actionlint.sarif
- uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: actionlint.sarif
```

## Schema source

By default the structural layer validates against **GitHub's own first-party schema**,
transpiled to JSON Schema at runtime (`src/transpile.rs`). `--schema schemastore` opts out
to the community [SchemaStore](https://json.schemastore.org/github-workflow.json) schema.

There is no first-party *machine-readable JSON Schema* published by GitHub, so both sources
are vendored and kept current by an automated resync pipeline (`.github/workflows/`), gated
by a golden corpus and a cross-validation check between the two schemas. See
[`docs/adr/`](docs/adr/) for the design decisions and [`docs/ROADMAP.md`](docs/ROADMAP.md)
for what's built and what's next.

## Development

```sh
cargo test                 # unit + integration tests
./scripts/run-corpus.sh    # golden-corpus regression gate
cargo test --test xval     # first-party vs SchemaStore cross-validation gate
```

## License

MIT — see [LICENSE](LICENSE).
