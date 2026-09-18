//! The vendored structural schema and the compiled validator.
//!
//! v1 upstream is SchemaStore's `github-workflow.json` (draft-07), embedded at compile
//! time via `include_str!` (see ADR-0003: committed file + embed, no build/lint-time
//! fetch). The auto-resync pipeline updates the committed file via PR.

use anyhow::{Context, Result};
use jsonschema::{Draft, Validator};

/// The SchemaStore GitHub-workflow JSON Schema, baked into the binary.
///
/// Kept current by the auto-resync pipeline (docs/ROADMAP.md), gated by the golden corpus.
pub const WORKFLOW_SCHEMA_JSON: &str = include_str!("../schemas/github-workflow.json");

/// Compile the embedded schema into a reusable draft-07 validator.
pub fn build_validator() -> Result<Validator> {
    build_validator_from(WORKFLOW_SCHEMA_JSON)
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
}
