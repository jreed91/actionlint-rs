//! Turn raw JSON-Schema validation errors into human-readable diagnostics.
//!
//! The SchemaStore workflow schema leans heavily on `oneOf` (jobs are
//! `normalJob | reusableWorkflowCallJob`; steps are `run | uses | ...`; many scalar fields
//! are `<value> | expressionSyntax`). The `jsonschema` crate reports these as
//! `OneOfNotValid { context }`, whose default message ("not valid under any of the schemas
//! listed in the 'oneOf' keyword") is useless. This module descends into the `oneOf`
//! branches, picks the branch the user most likely intended, and renders the underlying
//! failure in plain language — re-anchoring to the deepest instance path so the reported
//! span points at the actual offending node.

use jsonschema::error::{TypeKind, ValidationErrorKind};
use jsonschema::ValidationError;

/// A humanized message plus the JSON Pointer it should be anchored at (which may be deeper
/// than the top-level error's path) and a stable rule id for grouping (SARIF `ruleId`).
pub struct Humanized {
    pub pointer: String,
    pub message: String,
    pub rule_id: String,
}

/// Humanize one top-level validation error.
pub fn humanize(error: &ValidationError<'_>) -> Humanized {
    let pointer = error.instance_path().to_string();
    match error.kind() {
        ValidationErrorKind::OneOfNotValid { context } => humanize_one_of(&pointer, context),
        other => Humanized {
            pointer,
            message: render_leaf(other, &error.instance_path().to_string()),
            rule_id: rule_id_for(other),
        },
    }
}

/// A stable, kebab-case rule identifier for an error kind, used as the SARIF `ruleId` so
/// findings group into rules. All are under the `structure` layer for v1.
fn rule_id_for(kind: &ValidationErrorKind) -> String {
    let slug = match kind {
        ValidationErrorKind::Required { .. } => "required",
        ValidationErrorKind::Type { .. } => "type",
        ValidationErrorKind::AdditionalProperties { .. } => "additional-properties",
        ValidationErrorKind::Enum { .. } => "enum",
        ValidationErrorKind::Pattern { .. } => "pattern",
        ValidationErrorKind::OneOfNotValid { .. }
        | ValidationErrorKind::OneOfMultipleValid { .. } => "one-of",
        _ => "structure",
    };
    format!("structure/{slug}")
}

/// Choose the most relevant branch of a failed `oneOf` and render it.
///
/// Heuristic: the branch the user *meant* is the one that failed by the smallest, most
/// specific margin — typically a single `Required`/`Type`/`Pattern` error — rather than a
/// wholesale `AdditionalProperties` rejection (which is what an entirely wrong branch, like
/// `reusableWorkflowCallJob` for a normal job, produces). We score each branch and keep the
/// best; ties keep the earliest branch (schema-authored order).
fn humanize_one_of(pointer: &str, context: &[Vec<ValidationError<'_>>]) -> Humanized {
    let best = context
        .iter()
        .filter(|branch| !branch.is_empty())
        .min_by_key(|branch| branch_key(branch));

    match best {
        Some(branch) => {
            // Render the least-penalized error(s) in the chosen branch. A branch may itself
            // fail via a nested oneOf (e.g. a step's run/uses oneOf) — recurse.
            let leaf = branch
                .iter()
                .min_by_key(|e| {
                    // Prefer the deepest, most specific error in the branch.
                    let depth = e.instance_path().to_string().matches('/').count() as i32;
                    (-depth, leaf_penalty(e.kind()))
                })
                .expect("non-empty branch");
            match leaf.kind() {
                ValidationErrorKind::OneOfNotValid { context } => {
                    humanize_one_of(&leaf.instance_path().to_string(), context)
                }
                kind => Humanized {
                    pointer: leaf.instance_path().to_string(),
                    message: render_leaf(kind, &leaf.instance_path().to_string()),
                    rule_id: rule_id_for(kind),
                },
            }
        }
        // No branches at all (shouldn't happen): fall back to a generic message.
        None => Humanized {
            pointer: pointer.to_string(),
            message: "value does not match any allowed form".to_string(),
            rule_id: "structure/one-of".to_string(),
        },
    }
}

/// Sort key for choosing the branch the user most likely intended. Smaller is better.
///
/// The intended branch is the one that got the *most* structure right and failed only on a
/// nested detail (a bad step, a wrong scalar type) — so its errors reach **deepest** into
/// the instance. A wrong branch (e.g. `reusableWorkflowCallJob` for a normal job) fails
/// **shallow**, rejecting the instance's own top-level keys. We therefore rank primarily by
/// deepest error path (deeper = better), then by how specific/actionable that error is.
///
/// Returned tuple: `(negated max depth, best leaf penalty)` — both compared ascending, so a
/// deeper branch wins, and among equal depths the more specific error wins.
fn branch_key(branch: &[ValidationError<'_>]) -> (i32, u32) {
    let max_depth = branch
        .iter()
        .map(|e| e.instance_path().to_string().matches('/').count() as i32)
        .max()
        .unwrap_or(0);
    let best_leaf = branch
        .iter()
        .map(|e| leaf_penalty(e.kind()))
        .min()
        .unwrap_or(u32::MAX);
    (-max_depth, best_leaf)
}

/// Lower = more likely what the user intended / more specific and actionable.
///
/// Within an already-chosen branch (wrong branches are filtered out by `branch_penalty`),
/// an `AdditionalProperties` error names a genuine typo'd key and is the single most useful
/// thing to report — so it ranks above a nested `oneOf` whose branches would only say
/// "missing run/uses". A concrete `Required`/`Type` is best of all.
fn leaf_penalty(kind: &ValidationErrorKind) -> u32 {
    match kind {
        // A missing required key or a wrong scalar type is a precise, likely-intended fix.
        ValidationErrorKind::Required { .. } => 0,
        ValidationErrorKind::Type { .. } => 1,
        ValidationErrorKind::AdditionalProperties { .. } => 1,
        ValidationErrorKind::Pattern { .. } => 2,
        ValidationErrorKind::Enum { .. } => 2,
        // A nested oneOf is worth descending into, but only if nothing more concrete exists.
        ValidationErrorKind::OneOfNotValid { .. } => 3,
        _ => 4,
    }
}

/// Render a single (non-oneOf) error kind as a plain-language sentence.
fn render_leaf(kind: &ValidationErrorKind, pointer: &str) -> String {
    let at = field_name(pointer);
    match kind {
        ValidationErrorKind::Required { property } => {
            let prop = property.as_str().unwrap_or("a required property");
            match at {
                Some(name) => format!("`{name}` is missing required key `{prop}`"),
                None => format!("missing required key `{prop}`"),
            }
        }
        ValidationErrorKind::Type { kind } => {
            let want = type_kind_str(kind);
            match at {
                Some(name) => format!("`{name}` must be {want}"),
                None => format!("value must be {want}"),
            }
        }
        ValidationErrorKind::AdditionalProperties { unexpected } => {
            let list = unexpected
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ");
            match at {
                Some(name) => format!("`{name}` has unexpected key(s): {list}"),
                None => format!("unexpected key(s): {list}"),
            }
        }
        ValidationErrorKind::Enum { .. } => match at {
            Some(name) => format!("`{name}` is not one of the allowed values"),
            None => "value is not one of the allowed values".to_string(),
        },
        ValidationErrorKind::Pattern { .. } => match at {
            Some(name) => format!("`{name}` does not match the required format"),
            None => "value does not match the required format".to_string(),
        },
        // Any other kind: fall back to a reasonable generic phrasing.
        _ => match at {
            Some(name) => format!("`{name}` is invalid"),
            None => "value is invalid".to_string(),
        },
    }
}

/// The last segment of a JSON Pointer, e.g. `/jobs/build/runs-on` -> `runs-on`. Numeric
/// array indices are skipped in favor of the enclosing key (`/on/1` -> `on`).
fn field_name(pointer: &str) -> Option<String> {
    pointer
        .rsplit('/')
        .find(|seg| !seg.is_empty() && seg.parse::<usize>().is_err())
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
}

fn type_kind_str(kind: &TypeKind) -> String {
    match kind {
        TypeKind::Single(t) => a_or_an(&t.to_string()),
        TypeKind::Multiple(set) => {
            let names: Vec<String> = set.iter().map(|t| t.to_string()).collect();
            format!("one of: {}", names.join(", "))
        }
    }
}

fn a_or_an(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    let article = matches!(lower.chars().next(), Some('a' | 'e' | 'i' | 'o' | 'u'));
    if article {
        format!("an {lower}")
    } else {
        format!("a {lower}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema;
    use crate::yaml;

    fn first_message(src: &str) -> Humanized {
        let v = schema::build_validator().unwrap();
        let p = yaml::parse(src).unwrap();
        let err = v.iter_errors(&p.json).next().expect("expected an error");
        humanize(&err)
    }

    #[test]
    fn missing_runs_on_names_the_key() {
        let h = first_message("on: push\njobs:\n  b:\n    steps: [{run: hi}]\n");
        assert_eq!(h.message, "`b` is missing required key `runs-on`");
    }

    #[test]
    fn wrong_scalar_type_is_readable() {
        let h = first_message("on: 42\njobs:\n  b:\n    runs-on: x\n    steps: [{run: hi}]\n");
        assert_eq!(h.message, "`on` must be a string");
    }

    #[test]
    fn rule_ids_cover_the_common_kinds() {
        use jsonschema::error::TypeKind;
        use jsonschema::JsonType;
        use serde_json::json;
        assert_eq!(
            rule_id_for(&ValidationErrorKind::Required { property: json!("x") }),
            "structure/required"
        );
        assert_eq!(
            rule_id_for(&ValidationErrorKind::Type {
                kind: TypeKind::Single(JsonType::String)
            }),
            "structure/type"
        );
        assert_eq!(
            rule_id_for(&ValidationErrorKind::AdditionalProperties { unexpected: vec![] }),
            "structure/additional-properties"
        );
        assert_eq!(
            rule_id_for(&ValidationErrorKind::Enum { options: json!([]) }),
            "structure/enum"
        );
        assert_eq!(
            rule_id_for(&ValidationErrorKind::Pattern { pattern: "x".into() }),
            "structure/pattern"
        );
        // Both oneOf variants map to the same rule id (covers the OneOfMultipleValid arm).
        assert_eq!(
            rule_id_for(&ValidationErrorKind::OneOfNotValid { context: vec![] }),
            "structure/one-of"
        );
        assert_eq!(
            rule_id_for(&ValidationErrorKind::OneOfMultipleValid { context: vec![] }),
            "structure/one-of"
        );
        // Any other kind falls back to the generic structure rule.
        assert_eq!(
            rule_id_for(&ValidationErrorKind::MaxLength { limit: 3 }),
            "structure/structure"
        );
    }

    #[test]
    fn humanized_carries_rule_id() {
        let h = first_message("on: push\njobs:\n  b:\n    steps: [{run: hi}]\n");
        assert_eq!(h.rule_id, "structure/required");
    }

    #[test]
    fn wrong_type_on_timeout_reports_number() {
        let h = first_message(
            "on: push\njobs:\n  b:\n    runs-on: x\n    timeout-minutes: abc\n    steps: [{run: hi}]\n",
        );
        assert_eq!(h.message, "`timeout-minutes` must be a number");
        assert!(h.pointer.ends_with("/timeout-minutes"));
    }

    #[test]
    fn unexpected_step_key_names_the_key_and_reanchors() {
        // The bogus key is inside steps[0]: the message should name it and point there,
        // not surface the misleading "missing uses" from the wrong job branch.
        let h = first_message(
            "on: push\njobs:\n  b:\n    runs-on: x\n    steps: [{bogus: 1}]\n",
        );
        assert!(
            h.message.contains("bogus"),
            "should name the unexpected key, got: {}",
            h.message
        );
        assert!(h.pointer.contains("steps"), "pointer: {}", h.pointer);
        assert!(!h.message.contains("uses"), "should not blame missing uses: {}", h.message);
    }

    #[test]
    fn no_message_contains_schema_jargon() {
        for src in [
            "on: push\njobs:\n  b:\n    steps: [{run: hi}]\n",
            "on: 42\njobs:\n  b:\n    runs-on: x\n    steps: [{run: hi}]\n",
            "on: push\njobs:\n  b:\n    runs-on: x\n    steps: [{bogus: 1}]\n",
        ] {
            let h = first_message(src);
            assert!(!h.message.contains("oneOf"), "jargon in: {}", h.message);
            assert!(!h.message.contains("schemas listed"), "jargon in: {}", h.message);
        }
    }

    #[test]
    fn multiple_type_kind_is_rendered() {
        // Directly exercise the Multiple-type rendering path via a tiny ad-hoc schema.
        let v = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft7)
            .build(&serde_json::json!({
                "properties": { "x": { "type": ["string", "number"] } }
            }))
            .unwrap();
        let instance = serde_json::json!({ "x": true });
        let err = v.iter_errors(&instance).next().unwrap();
        let h = humanize(&err);
        assert!(h.message.contains("one of"), "got: {}", h.message);
    }

    #[test]
    fn enum_and_pattern_render_without_jargon() {
        // Enum: an invalid activity type; Pattern: a malformed value in a patterned field.
        let v = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft7)
            .build(&serde_json::json!({
                "properties": {
                    "color": { "enum": ["red", "green"] },
                    "code": { "pattern": "^[0-9]+$" }
                }
            }))
            .unwrap();
        let enum_err = humanize(
            &v.iter_errors(&serde_json::json!({ "color": "blue" })).next().unwrap(),
        );
        assert!(enum_err.message.contains("allowed values"), "got: {}", enum_err.message);
        let pat_err = humanize(
            &v.iter_errors(&serde_json::json!({ "code": "abc" })).next().unwrap(),
        );
        assert!(pat_err.message.contains("required format"), "got: {}", pat_err.message);
    }

    #[test]
    fn field_name_skips_array_index() {
        assert_eq!(field_name("/on/1").as_deref(), Some("on"));
        assert_eq!(field_name("/jobs/build/runs-on").as_deref(), Some("runs-on"));
        assert_eq!(field_name(""), None);
    }

    #[test]
    fn article_selection() {
        assert_eq!(a_or_an("string"), "a string");
        assert_eq!(a_or_an("Array"), "an array");
        assert_eq!(a_or_an("object"), "an object");
    }

    // Exercise the root-level (`pointer == ""`, no field name) rendering arms directly, so
    // every branch of `render_leaf` is covered even though workflow errors are rarely at the
    // document root.
    #[test]
    fn render_leaf_without_field_name() {
        use serde_json::json;
        let req = ValidationErrorKind::Required { property: json!("uses") };
        assert_eq!(render_leaf(&req, ""), "missing required key `uses`");

        let addl = ValidationErrorKind::AdditionalProperties { unexpected: vec!["x".into()] };
        assert_eq!(render_leaf(&addl, ""), "unexpected key(s): `x`");

        let en = ValidationErrorKind::Enum { options: json!(["a"]) };
        assert_eq!(render_leaf(&en, ""), "value is not one of the allowed values");

        let pat = ValidationErrorKind::Pattern { pattern: "^x$".into() };
        assert_eq!(render_leaf(&pat, ""), "value does not match the required format");

        // A kind with no dedicated arm falls back to the generic phrasing.
        let other = ValidationErrorKind::MaxLength { limit: 3 };
        assert_eq!(render_leaf(&other, ""), "value is invalid");
        assert_eq!(render_leaf(&other, "/a/b"), "`b` is invalid");
    }

    #[test]
    fn render_leaf_type_without_field_name() {
        use jsonschema::error::TypeKind;
        use jsonschema::JsonType;
        let t = ValidationErrorKind::Type { kind: TypeKind::Single(JsonType::Boolean) };
        assert_eq!(render_leaf(&t, ""), "value must be a boolean");
    }

    #[test]
    fn empty_one_of_context_falls_back() {
        // Defensive: an OneOf with only empty branches yields the generic message.
        let h = humanize_one_of("/x", &[vec![], vec![]]);
        assert_eq!(h.message, "value does not match any allowed form");
        assert_eq!(h.pointer, "/x");
    }
}

