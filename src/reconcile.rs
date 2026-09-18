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
    stringify_scalar_values_under(root, "env");
    stringify_scalar_values_under(root, "with");
}

/// **Reconciliation: scalar coercion under `env:` and `with:`.**
///
/// GitHub Actions treats every `env:` value and action `with:` input as a **string**, and
/// coerces whatever the user wrote: `FOO:` (YAML null) → `""`, `RUST_BACKTRACE: 1` → `"1"`,
/// `submodules: true` → `"true"`, `fetch-depth: 0` → `"0"`. Both structural schemas are
/// stricter than this:
/// - SchemaStore rejects a null env value (real example: ripgrep's `TARGET_FLAGS:`).
/// - GitHub's own first-party schema (option-B) types these as `string`, so it rejects the
///   number/bool forms that appear in nearly every real workflow (`fetch-depth: 0`,
///   `node-version: 20`, ...).
///
/// We mirror GitHub's coercion: stringify every null/number/boolean value directly under a
/// `<key>` mapping, at any nesting level (workflow-, job-, and step-level all qualify).
/// Nested objects/arrays under the key are left alone (they are already invalid, and the
/// schema should report them honestly).
fn stringify_scalar_values_under(node: &mut Value, key: &str) {
    match node {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if k == key {
                    if let Value::Object(values) = child {
                        for v in values.values_mut() {
                            coerce_scalar_to_string(v);
                        }
                    }
                }
                // Recurse so nested job/step blocks are covered.
                stringify_scalar_values_under(child, key);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                stringify_scalar_values_under(item, key);
            }
        }
        _ => {}
    }
}

/// Coerce a single null/number/boolean value to its GitHub string form, in place. Strings,
/// objects, and arrays are left unchanged.
fn coerce_scalar_to_string(v: &mut Value) {
    let replacement = match v {
        Value::Null => Some(String::new()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    if let Some(s) = replacement {
        *v = Value::String(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn coerces_scalar_env_values_to_strings() {
        let mut v = json!({ "env": { "A": null, "N": 1, "B": true, "S": "x" } });
        reconcile(&mut v);
        assert_eq!(v["env"]["A"], json!(""));
        assert_eq!(v["env"]["N"], json!("1"));
        assert_eq!(v["env"]["B"], json!("true"));
        assert_eq!(v["env"]["S"], json!("x"), "existing strings are unchanged");
    }

    #[test]
    fn coerces_scalar_with_inputs_to_strings() {
        // Real-world: fetch-depth: 0, submodules: true under a step's `with:`.
        let mut v = json!({
            "jobs": { "b": { "steps": [ { "with": { "fetch-depth": 0, "submodules": true } } ] } }
        });
        reconcile(&mut v);
        assert_eq!(v["jobs"]["b"]["steps"][0]["with"]["fetch-depth"], json!("0"));
        assert_eq!(v["jobs"]["b"]["steps"][0]["with"]["submodules"], json!("true"));
    }

    #[test]
    fn coerces_env_nested_in_jobs_and_steps() {
        let mut v = json!({
            "jobs": {
                "b": {
                    "env": { "X": 5 },
                    "steps": [ { "env": { "Y": null } } ]
                }
            }
        });
        reconcile(&mut v);
        assert_eq!(v["jobs"]["b"]["env"]["X"], json!("5"));
        assert_eq!(v["jobs"]["b"]["steps"][0]["env"]["Y"], json!(""));
    }

    #[test]
    fn leaves_non_env_scalars_untouched() {
        // A scalar that is NOT under env/with must not be coerced — other schema violations
        // are still reported honestly.
        let mut v = json!({ "on": 42, "env": { "A": 1 } });
        reconcile(&mut v);
        assert_eq!(v["on"], json!(42), "non-env scalar must be preserved");
        assert_eq!(v["env"]["A"], json!("1"));
    }

    #[test]
    fn nested_object_under_env_is_left_alone() {
        // A non-scalar value under env (itself invalid) is not touched; schema reports it.
        let mut v = json!({ "env": { "BAD": { "nested": 1 } } });
        reconcile(&mut v);
        assert_eq!(v["env"]["BAD"], json!({ "nested": 1 }));
    }

    #[test]
    fn env_that_is_not_a_mapping_is_left_alone() {
        // `env` given as a string (itself invalid) is not touched here; the schema reports it.
        let mut v = json!({ "env": "not-a-map" });
        reconcile(&mut v);
        assert_eq!(v["env"], json!("not-a-map"));
    }
}
