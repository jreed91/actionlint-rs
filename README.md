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
- **Action references** — `uses:` must be a valid `owner/repo@ref`, `./local`, or
  `docker://image` (a pinned ref is required for repository actions).
- **Scripts** — if `shellcheck` / `pyflakes` are installed, `run:` blocks are linted through
  them (disable with `--no-external`).
- **Security & misc** — script injection from untrusted input (`${{ github.event.*.title }}`,
  `github.head_ref`, …), hardcoded credentials, deprecated `::set-output::` / `::save-state::`
  commands, and cron syntax.

Every check is on by default and validated against a corpus of real-world workflows for
false positives.

## Usage

```sh
actionlint-rs                          # lint .github/workflows/*.{yml,yaml}
actionlint-rs path/to/workflow.yml     # lint specific files
cat workflow.yml | actionlint-rs -     # lint stdin
```

Common flags:

```sh
--format sarif            # SARIF 2.1.0 output (for GitHub code scanning)
--schema schemastore      # validate against the community SchemaStore schema instead
--ignore <REGEX>          # suppress diagnostics whose message matches REGEX (repeatable)
--no-external             # skip shellcheck / pyflakes
```

Exit codes: `0` clean, `1` problems found, `2` usage/IO error.

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
