//! Enum tightening the structural schema leaves loose (permissions scopes/levels, shells).
//!
//! The SchemaStore schema types these fields permissively (a `permissions:` map allows any
//! key; `shell:` is a free string). GitHub only accepts a fixed, documented set of values.
//! These are bounded, stable enums — ideal for on-by-default checking with zero false
//! positives (unlike runner labels, whose set is unbounded).
//!
//! Checks:
//! - **`permissions`**: string form must be `read-all`/`write-all`; map form's keys must be
//!   known scopes and values must be an allowed level *for that scope*. The scope list and
//!   each scope's allowed levels are **derived from the embedded first-party DSL**
//!   (`schemas/workflow-v1.0.json`), so they resync automatically rather than rotting in a
//!   hardcoded list — the project's core thesis (ADR-0001). This makes the check more precise
//!   than a flat scope list: e.g. `id-token` accepts only `write`/`none`, never `read`.
//! - **`shell`**: a `shell:` value (step/job/workflow default) must be a known shell keyword,
//!   or a custom command template (contains whitespace, e.g. `bash -e {0}`), which GitHub
//!   allows and we don't second-guess. (GitHub types `shell` as a free string in the DSL, so
//!   this list is documented-but-not-schema'd and stays hand-maintained.)

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::checks::CheckFinding;
use crate::schema::FIRST_PARTY_DSL_JSON;

/// Known `shell:` keywords GitHub documents. Not enumerated in the DSL (typed as a free
/// string there), so this stays hand-maintained.
const SHELLS: &[&str] = &["bash", "sh", "pwsh", "powershell", "cmd", "python"];

/// Permission scopes → the set of levels allowed for that scope, derived once from the
/// embedded first-party DSL. Empty if the DSL shape changes (fail-open: no false positives).
fn permission_scopes() -> &'static BTreeMap<String, Vec<String>> {
    static SCOPES: OnceLock<BTreeMap<String, Vec<String>>> = OnceLock::new();
    SCOPES.get_or_init(derive_permission_scopes)
}

/// Parse `/definitions/permissions-mapping/mapping/properties` from the DSL, resolving each
/// scope's level-type (`permission-level-any` etc.) to concrete `read`/`write`/`none` values.
fn derive_permission_scopes() -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Ok(dsl) = serde_json::from_str::<Value>(FIRST_PARTY_DSL_JSON) else {
        return out;
    };
    let defs = dsl.get("definitions");
    let Some(props) = defs
        .and_then(|d| d.get("permissions-mapping"))
        .and_then(|m| m.get("mapping"))
        .and_then(|m| m.get("properties"))
        .and_then(|p| p.as_object())
    else {
        return out;
    };
    for (scope, spec) in props {
        let level_type = spec.get("type").and_then(|t| t.as_str());
        let levels = level_type
            .map(|t| resolve_levels(defs, t))
            .unwrap_or_default();
        out.insert(scope.clone(), levels);
    }
    out
}

/// Resolve a `permission-level-*` type name to its concrete allowed level strings, following
/// the DSL's `one-of` composites down to the `string.constant` leaves.
fn resolve_levels(defs: Option<&Value>, type_name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Some(def) = defs.and_then(|d| d.get(type_name)) else {
        return out;
    };
    // Leaf: a constant string level.
    if let Some(c) = def
        .get("string")
        .and_then(|s| s.get("constant"))
        .and_then(|c| c.as_str())
    {
        out.push(c.to_string());
        return out;
    }
    // Composite: a one-of over other level types.
    if let Some(variants) = def.get("one-of").and_then(|o| o.as_array()) {
        for v in variants {
            if let Some(name) = v.as_str() {
                out.extend(resolve_levels(defs, name));
            }
        }
    }
    out
}

/// Run enum checks over a workflow.
pub fn check(workflow: &Value) -> Vec<CheckFinding> {
    let mut out = Vec::new();

    // Top-level permissions + defaults shell.
    check_permissions(workflow.get("permissions"), "/permissions", &mut out);
    check_defaults_shell(workflow, "", &mut out);

    if let Some(jobs) = workflow.get("jobs").and_then(|j| j.as_object()) {
        for (job_id, job) in jobs {
            let jp = format!("/jobs/{}/permissions", escape(job_id));
            check_permissions(job.get("permissions"), &jp, &mut out);
            check_defaults_shell(job, &format!("/jobs/{}", escape(job_id)), &mut out);

            if let Some(steps) = job.get("steps").and_then(|s| s.as_array()) {
                for (i, step) in steps.iter().enumerate() {
                    if let Some(shell) = step.get("shell") {
                        let ptr = format!("/jobs/{}/steps/{}/shell", escape(job_id), i);
                        check_shell(shell, &ptr, &mut out);
                    }
                }
            }
        }
    }
    out
}

fn check_permissions(perms: Option<&Value>, pointer: &str, out: &mut Vec<CheckFinding>) {
    let Some(perms) = perms else { return };
    match perms {
        Value::String(s) => {
            if s != "read-all" && s != "write-all" && !s.is_empty() {
                out.push(CheckFinding {
                    pointer: pointer.to_string(),
                    message: format!(
                        "invalid permissions `{s}` — the string form must be `read-all` or \
                         `write-all`"
                    ),
                    rule_id: "enum/permissions".to_string(),
                });
            }
        }
        Value::Object(map) => {
            let scopes = permission_scopes();
            // Fail-open: if the DSL couldn't be parsed we derive no scopes; skip rather than
            // flag everything (preserving the zero-false-positive guarantee).
            if scopes.is_empty() {
                return;
            }
            for (scope, level) in map {
                let Some(allowed) = scopes.get(scope) else {
                    out.push(CheckFinding {
                        pointer: format!("{pointer}/{}", escape(scope)),
                        message: format!("unknown permission scope `{scope}`"),
                        rule_id: "enum/permissions".to_string(),
                    });
                    continue;
                };
                if let Value::String(lvl) = level {
                    if !lvl.contains("${{") && !allowed.iter().any(|a| a == lvl) {
                        let allowed_list = allowed
                            .iter()
                            .map(|a| format!("`{a}`"))
                            .collect::<Vec<_>>()
                            .join(", ");
                        out.push(CheckFinding {
                            pointer: format!("{pointer}/{}", escape(scope)),
                            message: format!(
                                "invalid permission level `{lvl}` for `{scope}` — must be one \
                                 of {allowed_list}"
                            ),
                            rule_id: "enum/permissions".to_string(),
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

/// Check a `defaults.run.shell` on a workflow or job node.
fn check_defaults_shell(node: &Value, base_pointer: &str, out: &mut Vec<CheckFinding>) {
    if let Some(shell) = node
        .get("defaults")
        .and_then(|d| d.get("run"))
        .and_then(|r| r.get("shell"))
    {
        let ptr = format!("{base_pointer}/defaults/run/shell");
        check_shell(shell, &ptr, out);
    }
}

fn check_shell(shell: &Value, pointer: &str, out: &mut Vec<CheckFinding>) {
    let Value::String(s) = shell else { return };
    // A custom shell command template (e.g. `bash -e {0}`, `perl {0}`) contains whitespace;
    // GitHub allows arbitrary custom shells this way, so we don't validate those.
    if s.split_whitespace().count() != 1 {
        return;
    }
    if s.contains("${{") {
        return; // dynamic
    }
    if !SHELLS.contains(&s.as_str()) {
        out.push(CheckFinding {
            pointer: pointer.to_string(),
            message: format!(
                "unknown shell `{s}` — expected one of bash, sh, pwsh, powershell, cmd, \
                 python (or a custom `<cmd> {{0}}` template)"
            ),
            rule_id: "enum/shell".to_string(),
        });
    }
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules(wf: Value) -> Vec<String> {
        let mut r: Vec<String> = check(&wf).into_iter().map(|f| f.rule_id).collect();
        r.sort();
        r
    }

    fn findings(wf: Value) -> Vec<CheckFinding> {
        check(&wf)
    }

    #[test]
    fn valid_permissions_string_is_clean() {
        for s in ["read-all", "write-all"] {
            assert!(findings(json!({ "permissions": s })).is_empty(), "{s}");
        }
    }

    #[test]
    fn invalid_permissions_string_is_flagged() {
        let f = findings(json!({ "permissions": "read" }));
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "enum/permissions");
    }

    #[test]
    fn valid_permissions_map_is_clean() {
        let wf = json!({ "permissions": { "contents": "read", "id-token": "write", "issues": "none" } });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn scopes_are_derived_from_the_dsl() {
        let scopes = permission_scopes();
        assert!(!scopes.is_empty(), "should derive scopes from embedded DSL");
        // `code-quality` is a real scope present in the DSL — must not be flagged (regression:
        // it was missing from the old hardcoded list).
        assert!(scopes.contains_key("code-quality"));
        assert!(scopes.contains_key("contents"));
        // Levels resolve to concrete strings.
        assert_eq!(scopes["contents"], vec!["read", "write", "none"]);
    }

    #[test]
    fn code_quality_scope_is_accepted() {
        let wf = json!({ "permissions": { "code-quality": "write" } });
        assert!(findings(wf).is_empty(), "code-quality is a real scope");
    }

    #[test]
    fn per_scope_level_constraint_is_enforced() {
        // `id-token` accepts only write/none, never read (from the DSL).
        assert_eq!(permission_scopes()["id-token"], vec!["write", "none"]);
        let f = findings(json!({ "permissions": { "id-token": "read" } }));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("id-token"));
        // But write is fine.
        assert!(findings(json!({ "permissions": { "id-token": "write" } })).is_empty());
    }

    #[test]
    fn unknown_scope_is_flagged() {
        let wf = json!({ "permissions": { "contents": "read", "bogus-scope": "write" } });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("bogus-scope"));
        assert!(f[0].pointer.ends_with("/bogus-scope"));
    }

    #[test]
    fn invalid_level_is_flagged() {
        let wf = json!({ "permissions": { "contents": "readonly" } });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("readonly"));
    }

    #[test]
    fn expression_level_is_skipped() {
        let wf = json!({ "permissions": { "contents": "${{ inputs.level }}" } });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn job_level_permissions_are_checked() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "permissions": { "nope": "read" }, "steps": [] } }
        });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].pointer.starts_with("/jobs/b/permissions"));
    }

    #[test]
    fn valid_shells_are_clean() {
        for s in ["bash", "sh", "pwsh", "powershell", "cmd", "python"] {
            let wf = json!({ "jobs": { "b": { "runs-on": "x", "steps": [ { "shell": s, "run": "x" } ] } } });
            assert!(findings(wf).is_empty(), "{s} should be a valid shell");
        }
    }

    #[test]
    fn custom_shell_template_is_allowed() {
        let wf = json!({ "jobs": { "b": { "runs-on": "x", "steps": [ { "shell": "perl {0}", "run": "x" } ] } } });
        assert!(findings(wf).is_empty(), "custom shell template should be allowed");
    }

    #[test]
    fn unknown_shell_is_flagged() {
        let wf = json!({ "jobs": { "b": { "runs-on": "x", "steps": [ { "shell": "fish", "run": "x" } ] } } });
        assert_eq!(rules(wf), vec!["enum/shell"]);
    }

    #[test]
    fn defaults_shell_is_checked_at_workflow_and_job() {
        let wf = json!({ "defaults": { "run": { "shell": "zsh" } } });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].pointer, "/defaults/run/shell");

        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "defaults": { "run": { "shell": "zsh" } }, "steps": [] } }
        });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].pointer, "/jobs/b/defaults/run/shell");
    }
}
