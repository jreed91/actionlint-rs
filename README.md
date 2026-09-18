# actionlint-rs

A **schema-driven** linter for GitHub Actions workflow files, written in Rust.

Inspired by [rhysd/actionlint](https://github.com/rhysd/actionlint) (MIT). This is **not a
fork** — it's a fresh implementation with a different architecture. actionlint hand-codes
its structural rules in Go; actionlint-rs drives its **structural** checks from an external
JSON Schema and keeps that schema current with an automated, regression-gated resync
pipeline.

## What v1 does (and honestly, what it doesn't)

**v1 is a thesis proof, not yet a daily replacement for actionlint.** It validates the
**structure** of workflow files against the community
[SchemaStore](https://json.schemastore.org/github-workflow.json) schema (JSON Schema
draft-07) and reports precise `file:line:col` diagnostics in plain language — e.g. a job
missing `runs-on` reports `` `build` is missing required key `runs-on` `` rather than raw
JSON-Schema `oneOf` jargon (see `src/humanize.rs`).

v1 **does not** (yet) check:
- expressions inside `${{ }}` — they are treated as opaque strings;
- context/property availability or dataflow (`steps.x.outputs`, `needs`, `secrets`);
- shell (`shellcheck`) or Python (`pyflakes`) inside `run:`;
- `uses:`/action-metadata, deprecated actions, security/script-injection.

See [`docs/ROADMAP.md`](docs/ROADMAP.md) for the path to full coverage and
[`docs/adr/`](docs/adr/) for the design decisions.

> **On "tracks GitHub automatically":** there is no first-party machine-readable GitHub
> schema. v1 auto-**resyncs** from SchemaStore (community-maintained, lags GitHub by
> days-to-weeks) and cross-checks GitHub's first-party
> [`workflow-v1.0.json`](https://github.com/actions/languageservices) as an advisory
> signal. The north star is to drive validation from that first-party schema directly.

## Usage

CLI (the primitive):

```sh
actionlint-rs                        # lint .github/workflows/*.{yml,yaml}
actionlint-rs path/to/workflow.yml   # lint specific files
cat workflow.yml | actionlint-rs -   # lint stdin
actionlint-rs --format sarif         # emit SARIF 2.1.0 (for code scanning)
```

Exit codes: `0` clean, `1` problems found, `2` usage/IO error.

As a GitHub Action (v1 = Docker-based):

```yaml
- uses: your-org/actionlint-rs@v1
  # optional: files: ".github/workflows/ci.yml"
```

### Inline PR annotations via SARIF

Emit SARIF and upload it to GitHub code scanning to get inline annotations on PRs:

```yaml
- name: Lint workflows (SARIF)
  run: actionlint-rs --format sarif > actionlint.sarif
- uses: github/codeql-action/upload-sarif@v3
  with:
    sarif_file: actionlint.sarif
```

## Development

```sh
cargo test              # unit tests
./scripts/run-corpus.sh # golden-corpus regression gate
```

## License

MIT — see [LICENSE](LICENSE).
