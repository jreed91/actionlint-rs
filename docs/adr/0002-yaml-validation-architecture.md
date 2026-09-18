# 2. Structural validation architecture: off-the-shelf validator + span-mapped YAML

Date: 2026-09-18
Status: Accepted

## Context

v1 validates workflow **structure** against SchemaStore's `github-workflow.json`
(draft-07). Exact `line:column` source positions are a v1 requirement (see CONTEXT.md).

A JSON Schema validator operates on a JSON value model and locates errors by **JSON
Pointer** (e.g. `/jobs/build/runs-on`). It does not know source positions. YAML parsers
that produce a plain value drop source spans. So the core engineering problem is mapping
a JSON-Pointer-located error back to a YAML source span.

Verified against current docs (2026-09-18, Context7 + docs.rs):
- **`jsonschema` crate** (`/stranger6667/jsonschema`): supports `Draft::Draft7`; each
  error exposes `instance_path()` — a JSON Pointer to the failing location.
- **`saphyr` crate** (v0.0.12): provides `MarkedYaml`, a node tree where each node carries
  a `Span` (points to the **start** of the node) via `Marker`. Successor to yaml-rust.

## Decision

**Architecture A — two parses, JSON-Pointer-keyed span lookup:**

1. Parse the workflow YAML to `serde_json::Value` (or equivalent) → feed to the
   `jsonschema` draft-07 validator. Validator stays an off-the-shelf black box.
2. Parse the same YAML with `saphyr` to a `MarkedYaml` span tree.
3. On a validation error at pointer `/jobs/build/runs-on`, walk the `MarkedYaml` tree by
   that pointer to fetch the node's `Span`, and emit `file:line:col`.

Span-mapping (pointer → span) is isolated in one module.

## Consequences

- The schema validator remains swappable — protects the A+D thesis (swap/upgrade the
  schema freely without touching validation internals).
- The MVP's real complexity is the span-mapping module, not schema validation.
- **Dependency risk accepted:** `saphyr` is pre-1.0 (0.0.12); API churn is likely. Chosen
  over the deprecated/archived `serde_yaml` because that crate drops spans.
- **Open detail:** `saphyr` spans point to node *start*; per-key vs per-value granularity
  for map entries needs handling in the span module (point at value vs key correctly).
  End-of-span (full range vs single point) to be confirmed during build.

## Alternatives considered

- **B — custom span-carrying value model** consumed directly by the validator: rejected
  for v1 (tighter coupling to validator internals; would need `jsonschema` to be generic
  over the value type). Revisit only if two-parse proves too lossy.
- **Hand-written draft-07 validator:** rejected — rabbit hole orthogonal to the thesis.
- **JSON-path-only errors (no line:col):** rejected — violates v1 exact-position
  requirement.
