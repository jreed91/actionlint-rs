# 6. Pursue full actionlint check parity (supersedes the MVP scope boundary)

Date: 2026-09-18
Status: Accepted

## Context

The project began (ADR-0001, CONTEXT.md, ROADMAP.md) with a deliberately narrow thesis: the
schema drives the **structural** layer (~10% of actionlint's value); the hand-coded ~90%
(expression type-checking, dataflow, external linters, action metadata, security) was
explicitly OUT of scope. That MVP is complete and shipped: structural validation from
GitHub's first-party schema (default) + SchemaStore, humanized diagnostics, SARIF, `--ignore`,
Docker action, auto-resync, cross-validation gate.

The user has now decided to pursue **full check parity with actionlint** — reproducing the
hand-coded checks in Rust.

## Decision

Build toward feature parity with actionlint's checks, beyond the structural layer. This
**supersedes the "~90% out of scope" boundary** in ADR-0001 / ROADMAP.md. The schema-driven
structural layer remains and stays the default; the hand-coded checks are added alongside it.

Sequencing (each is its own module + tests, delivered incrementally):

1. **Expression engine** — a `${{ }}` lexer, parser, and AST. FOUNDATION: expression
   type-checking, context-availability, and `if:` analysis all depend on it. (This ADR's
   first deliverable.)
2. **Expression type system** — types for contexts (`github`, `env`, `steps`, ...) and
   built-in functions (`contains`, `format`, ...); type-check expressions.
3. **Context availability / dataflow** — which contexts are valid where; step-output
   existence; `needs` results.
4. **Graph checks** — `needs:` cycles / undefined jobs; matrix consistency.
5. **`uses:` / action metadata** — resolve `action.yml`, type-check `with:` inputs.
6. **External linters** — shellcheck (bash) and pyflakes (python) on `run:` blocks.
7. **Security & misc** — script-injection, hardcoded credentials, glob/cron, deprecations.

## Consequences

- **Scope, effort, identity all change.** This is a months-scale effort; "schema-driven,
  auto-resyncing" becomes one feature among many rather than the headline. Recorded so the
  reversal is conscious, not accidental.
- Expressions stop being opaque (reverses a grilling decision + a documented v1 limitation).
  The transpiler's expression-position widening (ADR-0005) stays for the *structural* pass;
  the new expression engine does the semantic work.
- The 98% coverage bar and the gates (corpus, cross-validation) continue to apply to every
  new module.
- Where actionlint hand-maintains knowledge by scraping GitHub docs (contexts, events,
  popular actions), we will do the same or vendor+resync it — consistent with the "D"
  pipeline already in place.

## Alternatives considered

- **CLI/UX/config parity only:** keep the thesis, match only actionlint's interface. Rejected
  by the user in favor of full check parity.
- **Stay MVP-scoped:** rejected — the user wants the full linter.
