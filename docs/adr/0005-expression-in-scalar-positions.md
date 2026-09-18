# 5. Widen context-annotated DSL nodes to accept `${{ }}` expressions

Date: 2026-09-18
Status: Accepted

## Context

GitHub Actions allows a `${{ <expression> }}` in many value positions instead of a literal
(e.g. `continue-on-error: ${{ matrix.x }}`, `runs-on: ${{ matrix.os }}`,
`timeout-minutes: ${{ inputs.t }}`). This is universal in real workflows.

The first-party transpiler (ADR-0001, option B) originally dropped the DSL's `context: [...]`
annotation as "non-structural". But that annotation is exactly the signal that expressions
are permitted at a node. Dropping it made the transpiled first-party schema type such fields
by their literal type only (boolean/number/string/one-of), so it **rejected valid
expression usage**. The cross-validation gate (ADR / tests/xval.rs) caught this: axum and
clap CI both diverged from SchemaStore on `continue-on-error`/`runs-on` expressions.

SchemaStore models the same thing by unioning the field with an `expressionSyntax` pattern
(`^\$\{\{(.|[\r\n])*\}\}$`).

## Decision

When a DSL node carries a `context` annotation, widen its transpiled schema to
**`anyOf[<base schema>, <expression string>]`**, where the expression string matches
SchemaStore's `expressionSyntax` pattern. Applied to **all** context-annotated nodes
(scalars, one-ofs, and mappings) — the annotation means expressions are allowed there,
regardless of base shape.

**`anyOf`, not `oneOf`:** an expression like `${{ matrix.os }}` is *also* a valid non-empty
string, so it matches multiple branches; `oneOf` would reject it as ambiguous
(`oneOfMultipleValid`). `anyOf` accepts a value matching one or more branches.

## Consequences

- The first-party (default) schema now accepts expressions wherever GitHub does; all real
  cross-validation-corpus workflows validate under both schemas (baseline is empty).
- **Reduced strictness where an expression is used.** A field written as an expression is no
  longer type-checked against its literal type (we can't evaluate the expression — expression
  typing is explicitly out of scope, CONTEXT.md). A *literal* wrong value is still caught.
- Error messages for a genuinely-bad value at a widened node become the generic `anyOf`
  "is invalid" rather than "must be a boolean" — a small message-quality cost of the union.
- Bounded by the DSL: only nodes GitHub marks with `context` are widened.

## Alternatives considered

- **Scalars only (string/boolean/number):** would still leave one-of/mapping nodes like
  `runs-on` rejecting expressions. Rejected in favor of honoring the annotation uniformly.
- **Model expression types properly:** evaluate what an expression resolves to and type-check
  it. Rejected — that is the expression layer the thesis scoped out (CONTEXT.md).
- **Track as a permanent known divergence (baseline):** rejected once a clean, faithful fix
  existed; a real fix beats a documented false positive.
