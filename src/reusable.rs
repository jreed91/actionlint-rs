//! Reusable-workflow (`workflow_call`) call checking.
//!
//! A job that calls another workflow (`uses: ./path.yml` or `org/repo/....yml@ref`) passes
//! `with:` inputs and `secrets:`. When the callee is a **local** file (`./...`), we can read
//! its `on.workflow_call` contract and check the call against it — entirely offline:
//!
//! - every **required input** with no default is supplied by the caller's `with:`;
//! - no **unknown input** is passed in `with:`;
//! - every **required secret** is supplied (unless the caller uses `secrets: inherit`).
//!
//! Scope / zero-false-positive guards:
//! - **Remote** callees (`org/repo/...@ref`) need network to resolve — skipped.
//! - A callee we can't read or that isn't a `workflow_call` workflow is skipped (the `uses:`
//!   format check covers malformed refs; a missing file is reported once, not as spurious
//!   input errors).
//! - `secrets: inherit` disables secret checking (all secrets are forwarded).
//! - A `with:`/secret *value* containing `${{ }}` is still a provided value (we check
//!   presence, not the expression).

use std::path::Path;

use serde_json::Value;

use crate::checks::CheckFinding;

/// Check every reusable-workflow call in `workflow`, resolving local callees relative to
/// `caller_path`. Returns findings anchored at the caller job's pointer.
pub fn check(workflow: &Value, caller_path: &Path) -> Vec<CheckFinding> {
    let mut out = Vec::new();
    let Some(jobs) = workflow.get("jobs").and_then(|j| j.as_object()) else {
        return out;
    };
    for (job_id, job) in jobs {
        let Some(uses) = job.get("uses").and_then(|u| u.as_str()) else {
            continue; // not a reusable-workflow call
        };
        check_call(job_id, job, uses, caller_path, &mut out);
    }
    out
}

fn check_call(
    job_id: &str,
    job: &Value,
    uses: &str,
    caller_path: &Path,
    out: &mut Vec<CheckFinding>,
) {
    // Only local callees are resolvable offline.
    let Some(rel) = local_ref(uses) else {
        return;
    };
    // Resolve relative to the caller file's directory. A local reusable `uses:` is repo-root
    // relative in GitHub, but files commonly sit together under .github/workflows; we resolve
    // relative to the caller's directory when that exists, else give up quietly.
    let Some(callee_path) = resolve_local(caller_path, rel) else {
        return; // can't locate the callee offline — skip (no false errors)
    };
    let Ok(source) = std::fs::read_to_string(&callee_path) else {
        return;
    };
    let Ok(callee) = crate::yaml::parse(&source) else {
        return;
    };
    let Some(contract) = workflow_call_contract(&callee.json) else {
        return; // callee isn't a reusable workflow (no on.workflow_call)
    };

    let base = format!("/jobs/{}", escape(job_id));
    check_inputs(job, &contract, &base, out);
    check_secrets(job, &contract, &base, out);
}

/// The `workflow_call` contract extracted from a callee workflow.
struct Contract {
    /// input name -> required?
    inputs: Vec<(String, bool)>,
    /// secret name -> required?
    secrets: Vec<(String, bool)>,
}

fn workflow_call_contract(callee: &Value) -> Option<Contract> {
    let wc = callee.get("on")?.get("workflow_call")?;
    // `on: workflow_call` may be null (no inputs/secrets) — still a valid reusable workflow.
    let inputs = wc
        .get("inputs")
        .and_then(|i| i.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), is_required(v)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let secrets = wc
        .get("secrets")
        .and_then(|s| s.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), is_required(v)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(Contract { inputs, secrets })
}

/// An input/secret spec is required when `required: true` and (for inputs) it has no default.
///
/// `required` may arrive as a JSON boolean or as the string `"true"` — the workflow
/// reconciliation (ADR-0004) coerces `inputs`/`with` scalar values to strings, so we accept
/// both.
fn is_required(spec: &Value) -> bool {
    let required = match spec.get("required") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s == "true",
        _ => false,
    };
    let has_default = spec.get("default").is_some();
    required && !has_default
}

fn check_inputs(job: &Value, contract: &Contract, base: &str, out: &mut Vec<CheckFinding>) {
    let provided = job.get("with").and_then(|w| w.as_object());
    // Unknown inputs.
    if let Some(with) = provided {
        for key in with.keys() {
            if !contract.inputs.iter().any(|(name, _)| name == key) {
                out.push(CheckFinding {
                    pointer: format!("{base}/with/{}", escape(key)),
                    message: format!(
                        "unknown input `{key}` — the called workflow declares no such \
                         `workflow_call` input"
                    ),
                    rule_id: "reusable/input".to_string(),
                });
            }
        }
    }
    // Missing required inputs.
    for (name, required) in &contract.inputs {
        if *required && !provided.map(|w| w.contains_key(name)).unwrap_or(false) {
            out.push(CheckFinding {
                pointer: format!("{base}/with"),
                message: format!("missing required input `{name}` for the called workflow"),
                rule_id: "reusable/input".to_string(),
            });
        }
    }
}

fn check_secrets(job: &Value, contract: &Contract, base: &str, out: &mut Vec<CheckFinding>) {
    // `secrets: inherit` forwards everything — nothing to check.
    if job.get("secrets").and_then(|s| s.as_str()) == Some("inherit") {
        return;
    }
    let provided = job.get("secrets").and_then(|s| s.as_object());
    if let Some(secrets) = provided {
        for key in secrets.keys() {
            if !contract.secrets.iter().any(|(name, _)| name == key) {
                out.push(CheckFinding {
                    pointer: format!("{base}/secrets/{}", escape(key)),
                    message: format!(
                        "unknown secret `{key}` — the called workflow declares no such \
                         `workflow_call` secret"
                    ),
                    rule_id: "reusable/secret".to_string(),
                });
            }
        }
    }
    for (name, required) in &contract.secrets {
        if *required && !provided.map(|s| s.contains_key(name)).unwrap_or(false) {
            out.push(CheckFinding {
                pointer: format!("{base}/secrets"),
                message: format!("missing required secret `{name}` for the called workflow"),
                rule_id: "reusable/secret".to_string(),
            });
        }
    }
}

/// If `uses` is a local reusable-workflow reference (`./path` or `path` ending in a workflow
/// file, no `@ref`), return the path portion; else `None`.
fn local_ref(uses: &str) -> Option<&str> {
    // Local refs start with `./` (GitHub requires this for same-repo reusable workflows).
    // They carry no `@ref`. Dynamic values (`${{ }}`) aren't resolvable.
    if uses.contains("${{") {
        return None;
    }
    let rel = uses.strip_prefix("./")?;
    // Must look like a workflow file.
    if rel.ends_with(".yml") || rel.ends_with(".yaml") {
        Some(rel)
    } else {
        None
    }
}

/// Resolve a repo-root-relative local ref to an on-disk path, given the caller's path. GitHub
/// resolves `./x` from the repo root; we find the repo root by walking up from the caller to
/// the `.github` directory's parent. Falls back to caller-dir-relative if that fails.
fn resolve_local(caller_path: &Path, rel: &str) -> Option<std::path::PathBuf> {
    // Repo-root resolution: the caller is typically <root>/.github/workflows/<file>. Find the
    // ancestor that contains `.github` and resolve `rel` from there.
    let mut dir = caller_path.parent();
    while let Some(d) = dir {
        let candidate = d.join(rel);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Stop once we've passed a `.github` boundary's parent (the repo root).
        dir = d.parent();
    }
    None
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("alr-reuse-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(d.join(".github").join("workflows")).unwrap();
        d
    }

    #[test]
    fn local_ref_recognizes_local_workflows() {
        assert_eq!(local_ref("./.github/workflows/reuse.yml"), Some(".github/workflows/reuse.yml"));
        assert_eq!(local_ref("./ci/build.yaml"), Some("ci/build.yaml"));
        // Remote refs and non-workflow refs are not local.
        assert_eq!(local_ref("org/repo/.github/workflows/x.yml@main"), None);
        assert_eq!(local_ref("actions/checkout@v4"), None);
        assert_eq!(local_ref("./scripts/build.sh"), None);
        assert_eq!(local_ref("${{ inputs.wf }}"), None);
    }

    #[test]
    fn is_required_logic() {
        assert!(is_required(&json!({ "required": true })));
        // Reconciliation coerces scalars to strings under inputs — accept "true" too.
        assert!(is_required(&json!({ "required": "true" })));
        assert!(!is_required(&json!({ "required": true, "default": "x" })));
        assert!(!is_required(&json!({ "required": "true", "default": "x" })));
        assert!(!is_required(&json!({ "required": false })));
        assert!(!is_required(&json!({ "required": "false" })));
        assert!(!is_required(&json!({})));
    }

    /// Write a callee reusable workflow and return the caller path in the same dir.
    fn setup(tag: &str, callee_body: &str) -> (PathBuf, PathBuf) {
        let root = tmpdir(tag);
        let wf = root.join(".github").join("workflows");
        std::fs::write(wf.join("reuse.yml"), callee_body).unwrap();
        let caller = wf.join("caller.yml");
        (root, caller)
    }

    #[test]
    fn valid_call_is_clean() {
        let (root, caller) = setup(
            "ok",
            "on:\n  workflow_call:\n    inputs:\n      env:\n        required: true\n        type: string\n    secrets:\n      token:\n        required: true\n",
        );
        let wf = json!({
            "jobs": { "call": {
                "uses": "./.github/workflows/reuse.yml",
                "with": { "env": "prod" },
                "secrets": { "token": "${{ secrets.T }}" }
            } }
        });
        let f = check(&wf, &caller);
        assert!(f.is_empty(), "{f:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_required_input_is_flagged() {
        let (root, caller) = setup(
            "missing-in",
            "on:\n  workflow_call:\n    inputs:\n      env:\n        required: true\n",
        );
        let wf = json!({
            "jobs": { "call": { "uses": "./.github/workflows/reuse.yml" } }
        });
        let f = check(&wf, &caller);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "reusable/input");
        assert!(f[0].message.contains("missing required input `env`"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unknown_input_is_flagged() {
        let (root, caller) = setup(
            "unknown-in",
            "on:\n  workflow_call:\n    inputs:\n      env:\n        required: false\n",
        );
        let wf = json!({
            "jobs": { "call": {
                "uses": "./.github/workflows/reuse.yml",
                "with": { "env": "x", "bogus": "y" }
            } }
        });
        let f = check(&wf, &caller);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("unknown input `bogus`"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn required_input_with_default_is_not_required() {
        let (root, caller) = setup(
            "default-in",
            "on:\n  workflow_call:\n    inputs:\n      env:\n        required: true\n        default: dev\n",
        );
        let wf = json!({ "jobs": { "call": { "uses": "./.github/workflows/reuse.yml" } } });
        assert!(check(&wf, &caller).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_required_secret_is_flagged() {
        let (root, caller) = setup(
            "missing-sec",
            "on:\n  workflow_call:\n    secrets:\n      token:\n        required: true\n",
        );
        let wf = json!({ "jobs": { "call": { "uses": "./.github/workflows/reuse.yml" } } });
        let f = check(&wf, &caller);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "reusable/secret");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unknown_secret_is_flagged() {
        let (root, caller) = setup(
            "unknown-sec",
            "on:\n  workflow_call:\n    secrets:\n      token:\n        required: false\n",
        );
        let wf = json!({
            "jobs": { "call": {
                "uses": "./.github/workflows/reuse.yml",
                "secrets": { "token": "${{ secrets.T }}", "bogus": "${{ secrets.B }}" }
            } }
        });
        let f = check(&wf, &caller);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "reusable/secret");
        assert!(f[0].message.contains("unknown secret `bogus`"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn callee_with_null_workflow_call_and_no_contract_items_is_clean() {
        // `on: workflow_call:` with an empty/absent inputs+secrets is a valid reusable
        // workflow; a caller passing nothing is clean (exercises empty-contract path).
        let (root, caller) = setup("empty-contract", "on:\n  workflow_call:\n");
        let wf = json!({ "jobs": { "call": { "uses": "./.github/workflows/reuse.yml" } } });
        assert!(check(&wf, &caller).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn job_without_uses_is_skipped() {
        // A normal job (no `uses:`) is not a reusable call.
        let wf = json!({ "jobs": { "b": { "runs-on": "x", "steps": [] } } });
        assert!(check(&wf, Path::new("/tmp/x.yml")).is_empty());
    }

    #[test]
    fn secrets_inherit_disables_secret_checking() {
        let (root, caller) = setup(
            "inherit",
            "on:\n  workflow_call:\n    secrets:\n      token:\n        required: true\n",
        );
        let wf = json!({
            "jobs": { "call": { "uses": "./.github/workflows/reuse.yml", "secrets": "inherit" } }
        });
        assert!(check(&wf, &caller).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn remote_callee_is_skipped() {
        let wf = json!({
            "jobs": { "call": { "uses": "org/repo/.github/workflows/x.yml@main" } }
        });
        assert!(check(&wf, Path::new("/tmp/whatever.yml")).is_empty());
    }

    #[test]
    fn missing_callee_file_is_skipped_not_errored() {
        let root = tmpdir("nofile");
        let caller = root.join(".github").join("workflows").join("caller.yml");
        let wf = json!({
            "jobs": { "call": { "uses": "./.github/workflows/does-not-exist.yml" } }
        });
        assert!(check(&wf, &caller).is_empty(), "missing callee should be skipped");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn non_reusable_callee_is_skipped() {
        // Callee has no on.workflow_call — not a reusable workflow.
        let (root, caller) = setup(
            "not-reusable",
            "on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [{run: hi}]\n",
        );
        let wf = json!({
            "jobs": { "call": { "uses": "./.github/workflows/reuse.yml", "with": { "x": "y" } } }
        });
        assert!(check(&wf, &caller).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
