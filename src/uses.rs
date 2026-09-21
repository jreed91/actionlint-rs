//! `uses:` reference validation (ADR-0006 step 6).
//!
//! Validates the *format* of every `uses:` value in the workflow — the part that can be
//! checked offline, which catches the most common mistakes. Three valid forms (GitHub docs):
//! - **Repository action:** `owner/repo@ref` or `owner/repo/path@ref` — a ref is REQUIRED.
//! - **Local action:** `./path` (relative to the repo root) — no ref.
//! - **Docker action:** `docker://image[:tag]`.
//!
//! Input validation against a resolved `action.yml` needs a network fetch and is a documented
//! follow-up (kept offline here for reproducibility).

use serde_json::Value;

/// A `uses:` finding: a message plus the JSON pointer it anchors at.
#[derive(Debug, Clone, PartialEq)]
pub struct UsesFinding {
    pub pointer: String,
    pub message: String,
}

/// Check every `uses:` in the workflow (any step, at any depth).
pub fn check(workflow: &Value) -> Vec<UsesFinding> {
    let mut findings = Vec::new();
    let mut pointer = String::new();
    walk(workflow, &mut pointer, &mut findings);
    findings
}

fn walk(value: &Value, pointer: &mut String, out: &mut Vec<UsesFinding>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let len = pointer.len();
                pointer.push('/');
                pointer.push_str(&escape(k));
                if k == "uses" {
                    if let Value::String(s) = v {
                        if let Some(msg) = validate_uses(s) {
                            out.push(UsesFinding {
                                pointer: pointer.clone(),
                                message: msg,
                            });
                        }
                    }
                }
                walk(v, pointer, out);
                pointer.truncate(len);
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let len = pointer.len();
                pointer.push('/');
                pointer.push_str(&i.to_string());
                walk(item, pointer, out);
                pointer.truncate(len);
            }
        }
        _ => {}
    }
}

/// Validate one `uses:` string; returns an error message if malformed, else `None`.
fn validate_uses(s: &str) -> Option<String> {
    let u = s.trim();
    if u.is_empty() {
        return Some("`uses` is empty".into());
    }
    // A value containing an expression / variable substitution is dynamic — its final form
    // isn't known statically, so we don't format-check it. This covers `${{ }}` interpolation
    // and the bare-`$` substitution some real workflows use (e.g. home-assistant CI's
    // `uses: $/.github/actions/...`), avoiding false positives on working workflows.
    if u.contains('$') {
        return None;
    }
    // Local action: `./path` or `../path`.
    if u.starts_with("./") || u.starts_with("../") {
        return None;
    }
    // Docker action: `docker://image[:tag]`.
    if let Some(rest) = u.strip_prefix("docker://") {
        if rest.is_empty() {
            return Some("`uses: docker://` is missing an image".into());
        }
        return None;
    }
    // Otherwise it must be a repository action `owner/repo[/path]@ref`.
    validate_repo_action(u)
}

fn validate_repo_action(u: &str) -> Option<String> {
    let Some((path, git_ref)) = u.split_once('@') else {
        return Some(format!(
            "`uses: {u}` is missing a version — repository actions must be pinned with `@<ref>` \
             (e.g. `owner/repo@v4`)"
        ));
    };
    if git_ref.is_empty() {
        return Some(format!("`uses: {u}` has an empty ref after `@`"));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 2 || segments[0].is_empty() || segments[1].is_empty() {
        return Some(format!(
            "`uses: {u}` is not a valid action reference (expected `owner/repo@ref`)"
        ));
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

    fn msg(uses: &str) -> Option<String> {
        validate_uses(uses)
    }

    #[test]
    fn valid_repository_actions() {
        assert!(msg("actions/checkout@v4").is_none());
        assert!(msg("actions/checkout@8f4b7f8").is_none());
        assert!(msg("owner/repo/path/to/action@v1.2.3").is_none());
        assert!(msg("owner/repo@main").is_none());
    }

    #[test]
    fn valid_local_and_docker_and_expr() {
        assert!(msg("./.github/actions/my-action").is_none());
        assert!(msg("../shared/action").is_none());
        assert!(msg("docker://alpine:3.19").is_none());
        assert!(msg("docker://ghcr.io/owner/img").is_none());
        assert!(msg("${{ matrix.action }}").is_none());
        // Dynamic values containing a substitution are not format-checked.
        assert!(msg("${{ github.workspace }}/.github/actions/x").is_none());
        assert!(msg("$/.github/actions/x").is_none());
    }

    #[test]
    fn missing_ref_is_flagged() {
        let m = msg("actions/checkout").unwrap();
        assert!(m.contains("missing a version"), "{m}");
    }

    #[test]
    fn empty_ref_is_flagged() {
        let m = msg("actions/checkout@").unwrap();
        assert!(m.contains("empty ref"), "{m}");
    }

    #[test]
    fn not_owner_repo_is_flagged() {
        let m = msg("checkout@v4").unwrap();
        assert!(m.contains("not a valid action reference"), "{m}");
    }

    #[test]
    fn empty_uses_is_flagged() {
        assert!(msg("").unwrap().contains("empty"));
        assert!(msg("   ").unwrap().contains("empty"));
    }

    #[test]
    fn empty_docker_image_is_flagged() {
        assert!(msg("docker://").unwrap().contains("missing an image"));
    }

    #[test]
    fn check_walks_steps_and_anchors_pointer() {
        let wf = json!({
            "jobs": {
                "b": {
                    "runs-on": "x",
                    "steps": [
                        { "uses": "actions/checkout@v4" },
                        { "uses": "actions/setup-node" }
                    ]
                }
            }
        });
        let f = check(&wf);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].pointer, "/jobs/b/steps/1/uses");
        assert!(f[0].message.contains("missing a version"));
    }

    #[test]
    fn non_string_uses_is_ignored() {
        let wf = json!({ "jobs": { "b": { "steps": [ { "uses": 42 } ] } } });
        assert!(check(&wf).is_empty());
    }
}
