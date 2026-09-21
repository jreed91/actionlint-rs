//! Dataflow checks over `${{ }}` references: step-output and needs-output existence.
//!
//! The expression type-checker (`crate::expr`) treats `steps`, `needs`, and `jobs` as open
//! maps — it can't know which step IDs or needed jobs exist, because that's a property of the
//! *workflow structure*, not the expression in isolation. This pass supplies that context:
//!
//! - **`steps.<id>...`** — `<id>` must be the `id:` of a step *earlier in the same job*. A
//!   reference to an undefined step id is a guaranteed empty value (a bug).
//! - **`needs.<job>...`** — `<job>` must be listed in the referencing job's `needs:`. Even if
//!   the job exists elsewhere, its outputs are only available when declared as a dependency.
//!
//! Deliberately scoped to keep zero false positives:
//! - We check *id / job existence*, not output-*name* existence: a `run:` step's outputs are
//!   written dynamically to `$GITHUB_OUTPUT`, and a `uses:` step's outputs need the action's
//!   `action.yml` (not available offline) — neither is knowable statically.
//! - Any expression we can't parse is skipped (the expression pass reports syntax errors).
//! - Dynamic step ids can't occur (ids are static YAML), so there's nothing to skip there.

use serde_json::Value;

use crate::checks::CheckFinding;
use crate::expr::{self, Expr};

/// Run dataflow checks over a workflow.
pub fn check(workflow: &Value) -> Vec<CheckFinding> {
    let mut out = Vec::new();
    let Some(jobs) = workflow.get("jobs").and_then(|j| j.as_object()) else {
        return out;
    };
    for (job_id, job) in jobs {
        check_job(job_id, job, &mut out);
    }
    out
}

fn check_job(job_id: &str, job: &Value, out: &mut Vec<CheckFinding>) {
    let step_ids = collect_step_ids(job);
    let needs = collect_needs(job);
    let base = format!("/jobs/{}", escape(job_id));

    // Walk the whole job subtree, checking every string for offending references. We keep the
    // JSON pointer so findings anchor at the containing scalar.
    walk_strings(job, &base, &mut |s, pointer| {
        for src in extract_expressions(s) {
            let Ok(ast) = expr::parse(&src) else { continue };
            collect_refs(&ast, &step_ids, &needs, pointer, out);
        }
    });
}

/// Collect the set of `id:` values declared by steps in this job.
fn collect_step_ids(job: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(steps) = job.get("steps").and_then(|s| s.as_array()) {
        for step in steps {
            if let Some(id) = step.get("id").and_then(|i| i.as_str()) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

/// Collect the set of job ids this job declares in `needs:` (string or list form).
fn collect_needs(job: &Value) -> Vec<String> {
    match job.get("needs") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Traverse an expression AST, flagging `steps.<id>` / `needs.<job>` roots that reference an
/// undefined id / non-needed job.
fn collect_refs(
    expr: &Expr,
    step_ids: &[String],
    needs: &[String],
    pointer: &str,
    out: &mut Vec<CheckFinding>,
) {
    // Look for `Index { target: Ident("steps"|"needs"), index: Str(name) }` patterns.
    if let Expr::Index { target, index } = expr {
        if let (Expr::Ident(root), Expr::Str(name)) = (target.as_ref(), index.as_ref()) {
            match root.as_str() {
                "steps" => {
                    if !step_ids.iter().any(|id| id == name) {
                        out.push(CheckFinding {
                            pointer: pointer.to_string(),
                            message: format!(
                                "reference to undefined step id `{name}` \
                                 (`steps.{name}` — no step in this job has `id: {name}`)"
                            ),
                            rule_id: "dataflow/step-output".to_string(),
                        });
                    }
                }
                "needs" => {
                    if !needs.iter().any(|j| j == name) {
                        out.push(CheckFinding {
                            pointer: pointer.to_string(),
                            message: format!(
                                "reference to job `{name}` not in `needs` \
                                 (`needs.{name}` — add `{name}` to this job's `needs:` to use \
                                 its outputs)"
                            ),
                            rule_id: "dataflow/needs-output".to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    // Recurse into all subexpressions.
    for child in children(expr) {
        collect_refs(child, step_ids, needs, pointer, out);
    }
}

/// The direct child expressions of a node.
fn children(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::Index { target, index } => vec![target, index],
        Expr::Star(inner) => vec![inner],
        Expr::Call { args, .. } => args.iter().collect(),
        Expr::Unary { operand, .. } => vec![operand],
        Expr::Binary { left, right, .. } => vec![left, right],
        _ => Vec::new(),
    }
}

/// Walk every string value under `node`, invoking `f(string, json_pointer)`.
fn walk_strings(node: &Value, pointer: &str, f: &mut impl FnMut(&str, &str)) {
    match node {
        Value::String(s) => f(s, pointer),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk_strings(item, &format!("{pointer}/{i}"), f);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                walk_strings(v, &format!("{pointer}/{}", escape(k)), f);
            }
        }
        _ => {}
    }
}

/// Extract inner text of each `${{ ... }}` in a string.
fn extract_expressions(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("${{") {
        let after = &rest[start + 3..];
        match after.find("}}") {
            Some(end) => {
                out.push(after[..end].to_string());
                rest = &after[end + 2..];
            }
            None => break,
        }
    }
    out
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
    fn defined_step_output_is_clean() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [
                { "id": "build", "run": "echo x" },
                { "run": "echo ${{ steps.build.outputs.version }}" }
            ] } }
        });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn undefined_step_id_is_flagged() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [
                { "id": "build", "run": "echo x" },
                { "run": "echo ${{ steps.biuld.outputs.version }}" }
            ] } }
        });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "dataflow/step-output");
        assert!(f[0].message.contains("biuld"));
    }

    #[test]
    fn needed_job_output_is_clean() {
        let wf = json!({
            "jobs": {
                "a": { "runs-on": "x", "outputs": { "v": "1" }, "steps": [] },
                "b": { "runs-on": "x", "needs": ["a"],
                    "steps": [ { "run": "echo ${{ needs.a.outputs.v }}" } ] }
            }
        });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn non_needed_job_reference_is_flagged() {
        // Job `a` exists but isn't in `b`'s needs — its outputs aren't available.
        let wf = json!({
            "jobs": {
                "a": { "runs-on": "x", "steps": [] },
                "b": { "runs-on": "x",
                    "steps": [ { "run": "echo ${{ needs.a.outputs.v }}" } ] }
            }
        });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "dataflow/needs-output");
        assert!(f[0].message.contains("needs.a"));
    }

    #[test]
    fn needs_string_form_is_handled() {
        let wf = json!({
            "jobs": {
                "a": { "runs-on": "x", "steps": [] },
                "b": { "runs-on": "x", "needs": "a",
                    "steps": [ { "run": "echo ${{ needs.a.result }}" } ] }
            }
        });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn step_reference_in_if_and_with_is_checked() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [
                { "id": "s1", "run": "echo x" },
                { "if": "${{ steps.s1.outputs.ok == 'true' }}",
                  "uses": "actions/foo@v1",
                  "with": { "val": "${{ steps.ghost.outputs.x }}" } }
            ] } }
        });
        // s1 is fine; ghost is undefined -> exactly one finding, anchored in `with`.
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].pointer.contains("/with/val"), "{}", f[0].pointer);
    }

    #[test]
    fn steps_context_without_index_is_ignored() {
        // A bare `steps` (no property) or a dynamic index shouldn't crash or false-positive.
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [
                { "run": "echo ${{ toJSON(steps) }}" }
            ] } }
        });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn each_job_scopes_its_own_step_ids() {
        // `steps.build` is defined in job a but referenced in job b -> flagged in b.
        let wf = json!({
            "jobs": {
                "a": { "runs-on": "x", "steps": [ { "id": "build", "run": "x" } ] },
                "b": { "runs-on": "x", "steps": [ { "run": "echo ${{ steps.build.outputs.v }}" } ] }
            }
        });
        assert_eq!(rules(wf), vec!["dataflow/step-output"]);
    }
}
