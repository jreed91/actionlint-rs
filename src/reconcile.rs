//! Documented reconciliations between what GitHub Actions actually accepts and what the
//! vendored SchemaStore schema encodes.
//!
//! DESIGN TENSION (see ADR-0001): the thesis is that validation is *driven by the external
//! schema*, and we deliberately avoid actionlint-style hand-tuning. Every rule here is a
//! knowing exception to that principle — a place where SchemaStore is provably stricter
//! than GitHub, causing a false positive on a real, valid workflow. To keep the exceptions
//! honest and from sprawling silently:
//!
//!   1. Each reconciliation lives HERE, in one auditable place — not scattered in the parser.
//!   2. Each has a comment citing the concrete divergence and, ideally, a real workflow that
//!      tripped it.
//!   3. Each is a candidate to DELETE once fixed upstream (a SchemaStore PR) or once the
//!      option-B first-party-schema path lands (which should not have the divergence).
//!
//! The resync cross-check (option C) is where such divergences are meant to surface; this
//! module is the pragmatic stopgap so users aren't shown false positives in the meantime.

use serde_json::Value;

/// Apply all documented reconciliations to the parsed workflow JSON, in place, before it is
/// handed to the schema validator.
pub fn reconcile(root: &mut Value) {
    null_env_values_to_empty_string(root);
}

/// **Reconciliation: null env values.**
///
/// GitHub Actions accepts an env var declared with no value (`FOO:` → YAML null), treating
/// it as an empty string. SchemaStore requires env values to be `string | number | boolean`
/// and rejects null. Real-world example: BurntSushi/ripgrep's CI declares `TARGET_FLAGS:`.
///
/// We coerce null values under any `env:` mapping to `""` so the schema accepts them.
/// Scope is deliberately narrow — only direct children of an `env` mapping, at any nesting
/// level (workflow-, job-, and step-level `env` all qualify).
fn null_env_values_to_empty_string(node: &mut Value) {
    match node {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "env" {
                    if let Value::Object(env) = child {
                        for v in env.values_mut() {
                            if v.is_null() {
                                *v = Value::String(String::new());
                            }
                        }
                    }
                }
                // Recurse into every child so nested job/step `env` blocks are covered.
                null_env_values_to_empty_string(child);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                null_env_values_to_empty_string(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coerces_null_env_value_at_workflow_level() {
        let mut v = json!({ "env": { "A": null, "B": "x" } });
        reconcile(&mut v);
        assert_eq!(v["env"]["A"], json!(""));
        assert_eq!(v["env"]["B"], json!("x"));
    }

    #[test]
    fn coerces_null_env_value_nested_in_jobs_and_steps() {
        let mut v = json!({
            "jobs": {
                "b": {
                    "env": { "X": null },
                    "steps": [ { "env": { "Y": null } } ]
                }
            }
        });
        reconcile(&mut v);
        assert_eq!(v["jobs"]["b"]["env"]["X"], json!(""));
        assert_eq!(v["jobs"]["b"]["steps"][0]["env"]["Y"], json!(""));
    }

    #[test]
    fn leaves_non_env_nulls_untouched() {
        // A null that is NOT an env value must not be silently coerced — only env is
        // reconciled, so other schema violations are still reported honestly.
        let mut v = json!({ "on": null, "env": { "A": null } });
        reconcile(&mut v);
        assert_eq!(v["on"], json!(null), "non-env null must be preserved");
        assert_eq!(v["env"]["A"], json!(""));
    }

    #[test]
    fn env_that_is_not_a_mapping_is_left_alone() {
        // `env` given as a string (itself invalid) is not touched here; the schema reports it.
        let mut v = json!({ "env": "not-a-map" });
        reconcile(&mut v);
        assert_eq!(v["env"], json!("not-a-map"));
    }
}
