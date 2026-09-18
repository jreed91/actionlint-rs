//! The vendored structural schema(s) and the compiled validator.
//!
//! Two upstreams are supported:
//! - **SchemaStore** (`github-workflow.json`, draft-07) — the default (ADR-0003).
//! - **First-party** (`workflow-v1.0.json`, GitHub's DSL) — transpiled to draft-07 at
//!   runtime by `crate::transpile` (the option-B north star, ADR-0001).
//!
//! Both are embedded at compile time via `include_str!` (no build/lint-time fetch); the
//! auto-resync pipeline updates the committed files via PR.

use anyhow::{Context, Result};
use jsonschema::{Draft, Validator};

/// The SchemaStore GitHub-workflow JSON Schema, baked into the binary.
///
/// Kept current by the auto-resync pipeline (docs/ROADMAP.md), gated by the golden corpus.
pub const WORKFLOW_SCHEMA_JSON: &str = include_str!("../schemas/github-workflow.json");

/// GitHub's first-party workflow DSL, baked in. Transpiled to JSON Schema at runtime.
pub const FIRST_PARTY_DSL_JSON: &str = include_str!("../schemas/workflow-v1.0.json");

/// Which structural schema source to validate against.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum SchemaSource {
    /// Community SchemaStore JSON Schema (default).
    #[default]
    SchemaStore,
    /// GitHub's first-party DSL, transpiled to JSON Schema.
    FirstParty,
}

/// Compile the default (SchemaStore) schema into a reusable draft-07 validator.
pub fn build_validator() -> Result<Validator> {
    build_validator_from(WORKFLOW_SCHEMA_JSON)
}

/// Compile a validator for the given schema source.
pub fn build_validator_for(source: SchemaSource) -> Result<Validator> {
    match source {
        SchemaSource::SchemaStore => build_validator(),
        SchemaSource::FirstParty => build_first_party_validator(),
    }
}

/// Transpile GitHub's first-party DSL to JSON Schema and compile it (option-B north star).
pub fn build_first_party_validator() -> Result<Validator> {
    let dsl: serde_json::Value = serde_json::from_str(FIRST_PARTY_DSL_JSON)
        .context("embedded first-party DSL is not valid JSON (vendoring bug)")?;
    let transpiled = crate::transpile::transpile(&dsl)
        .context("failed to transpile first-party DSL to JSON Schema")?;
    jsonschema::options()
        .with_draft(Draft::Draft7)
        .build(&transpiled.schema)
        .map_err(|e| anyhow::anyhow!("failed to compile transpiled first-party schema: {e}"))
}

/// Compile an arbitrary draft-07 schema JSON string into a validator.
///
/// Split out from [`build_validator`] so both the JSON-parse and schema-compile error
/// paths are testable with injected inputs.
fn build_validator_from(schema_json: &str) -> Result<Validator> {
    let schema: serde_json::Value = serde_json::from_str(schema_json)
        .context("workflow schema is not valid JSON (vendoring bug)")?;

    jsonschema::options()
        .with_draft(Draft::Draft7)
        .build(&schema)
        // ValidationError borrows from `schema`; capture its text so the error can escape.
        .map_err(|e| anyhow::anyhow!("failed to compile workflow schema: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_schema_is_valid_json_and_compiles() {
        // Guards against a corrupt vendored file landing via the resync pipeline.
        build_validator().expect("embedded schema should compile as draft-07");
    }

    #[test]
    fn invalid_json_schema_is_a_parse_error() {
        let err = build_validator_from("{ not json").unwrap_err();
        assert!(err.to_string().contains("not valid JSON"));
    }

    #[test]
    fn uncompilable_schema_is_a_compile_error() {
        // `type` must be a string or array of strings; a number makes compilation fail.
        let err = build_validator_from(r#"{"type": 123}"#).unwrap_err();
        assert!(err.to_string().contains("failed to compile"));
    }

    #[test]
    fn valid_injected_schema_compiles() {
        assert!(build_validator_from(r#"{"type": "object"}"#).is_ok());
    }

    #[test]
    fn embedded_schema_is_draft07() {
        let schema: serde_json::Value = serde_json::from_str(WORKFLOW_SCHEMA_JSON).unwrap();
        let declared = schema.get("$schema").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            declared.contains("draft-07"),
            "expected draft-07 schema, got `{declared}` — resync may have pulled a different draft"
        );
    }

    #[test]
    fn first_party_dsl_transpiles_and_compiles() {
        // Guards the option-B path: the embedded DSL must transpile to a schema that
        // compiles as draft-07 (catches a bad vendored DSL or a transpiler regression).
        build_first_party_validator()
            .expect("first-party DSL should transpile + compile as draft-07");
    }

    #[test]
    fn first_party_validator_accepts_good_and_rejects_bad() {
        let v = build_first_party_validator().unwrap();
        let good: serde_json::Value = serde_json::from_str(
            r#"{"on":"push","jobs":{"b":{"runs-on":"ubuntu-latest","steps":[{"run":"hi"}]}}}"#,
        )
        .unwrap();
        assert!(v.is_valid(&good), "valid workflow should pass first-party schema");

        let bad: serde_json::Value =
            serde_json::from_str(r#"{"on":"push","jobs":{"b":{"steps":[{"run":"hi"}]}}}"#).unwrap();
        assert!(!v.is_valid(&bad), "job missing runs-on should fail first-party schema");
    }

    #[test]
    fn build_validator_for_dispatches_on_source() {
        assert!(build_validator_for(SchemaSource::SchemaStore).is_ok());
        assert!(build_validator_for(SchemaSource::FirstParty).is_ok());
    }
}
