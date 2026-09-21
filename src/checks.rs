//! Security and miscellaneous semantic checks (ADR-0006 step 8).
//!
//! Hand-coded checks that need domain knowledge beyond the schema/expression layers:
//! - **Script injection:** an untrusted `${{ github.event.* }}` value interpolated directly
//!   into a `run:` script (the classic GitHub Actions injection vector).
//! - **Hardcoded credentials:** a literal password in a `container`/`service` credentials
//!   block (should be a `${{ secrets.* }}`).
//! - **Deprecated workflow commands:** `::set-output::` / `::save-state::` in `run:` scripts.
//! - **cron syntax:** a malformed `schedule.cron` expression.

use serde_json::Value;

/// A check finding: a message, the JSON pointer to anchor at, and a rule id.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckFinding {
    pub pointer: String,
    pub message: String,
    pub rule_id: String,
}

/// Run all security + misc checks over a workflow.
pub fn check(workflow: &Value) -> Vec<CheckFinding> {
    let mut out = Vec::new();
    let mut pointer = String::new();
    walk(workflow, &mut pointer, &mut out);
    check_cron(workflow, &mut out);
    out
}

fn walk(value: &Value, pointer: &mut String, out: &mut Vec<CheckFinding>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let len = pointer.len();
                pointer.push('/');
                pointer.push_str(&escape(k));

                match k.as_str() {
                    "run" => {
                        if let Value::String(s) = v {
                            check_run_script(s, pointer, out);
                        }
                    }
                    "credentials" => check_credentials(v, pointer, out),
                    _ => {}
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

/// Checks over a `run:` script string.
fn check_run_script(script: &str, pointer: &str, out: &mut Vec<CheckFinding>) {
    // Script injection: an untrusted expression interpolated into the script.
    for expr in expressions_in(script) {
        let e = expr.trim();
        if is_untrusted_context(e) {
            out.push(CheckFinding {
                pointer: pointer.to_string(),
                message: format!(
                    "possible script injection: `${{{{ {e} }}}}` is untrusted input \
                     interpolated into a run script — pass it through an `env:` var instead"
                ),
                rule_id: "security/script-injection".to_string(),
            });
        }
    }
    // Deprecated workflow commands.
    for (needle, name) in [("::set-output", "set-output"), ("::save-state", "save-state")] {
        if script.contains(needle) {
            out.push(CheckFinding {
                pointer: pointer.to_string(),
                message: format!(
                    "deprecated workflow command `{name}` — use the `$GITHUB_OUTPUT` / \
                     `$GITHUB_STATE` environment files instead"
                ),
                rule_id: "deprecated/workflow-command".to_string(),
            });
        }
    }
}

/// A container/service `credentials` mapping with a literal (non-expression) password.
fn check_credentials(creds: &Value, pointer: &str, out: &mut Vec<CheckFinding>) {
    let Some(map) = creds.as_object() else { return };
    if let Some(Value::String(pw)) = map.get("password") {
        let t = pw.trim();
        if !t.is_empty() && !t.contains("${{") {
            out.push(CheckFinding {
                pointer: format!("{pointer}/password"),
                message: "hardcoded credential: `password` should reference a secret \
                          (`${{ secrets.* }}`), not a literal value"
                    .to_string(),
                rule_id: "security/hardcoded-credential".to_string(),
            });
        }
    }
}

/// Validate `on.schedule[*].cron` expressions.
fn check_cron(workflow: &Value, out: &mut Vec<CheckFinding>) {
    let Some(schedule) = workflow
        .get("on")
        .and_then(|o| o.get("schedule"))
        .and_then(|s| s.as_array())
    else {
        return;
    };
    for (i, entry) in schedule.iter().enumerate() {
        if let Some(cron) = entry.get("cron").and_then(|c| c.as_str()) {
            if let Some(reason) = cron_error(cron) {
                out.push(CheckFinding {
                    pointer: format!("/on/schedule/{i}/cron"),
                    message: format!("invalid cron `{cron}`: {reason}"),
                    rule_id: "misc/cron".to_string(),
                });
            }
        }
    }
}

/// Validate a POSIX 5-field cron expression. Returns an error reason if invalid.
fn cron_error(cron: &str) -> Option<String> {
    let fields: Vec<&str> = cron.split_whitespace().collect();
    if fields.len() != 5 {
        return Some(format!(
            "expected 5 space-separated fields, found {}",
            fields.len()
        ));
    }
    // Ranges per field: minute, hour, day-of-month, month, day-of-week.
    let ranges = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];
    for (field, (lo, hi)) in fields.iter().zip(ranges.iter()) {
        if !cron_field_ok(field, *lo, *hi) {
            return Some(format!("`{field}` is not a valid field (allowed {lo}-{hi})"));
        }
    }
    None
}

/// Validate a single cron field: `*`, `*/n`, `a`, `a-b`, `a-b/n`, or a comma list of those.
fn cron_field_ok(field: &str, lo: u32, hi: u32) -> bool {
    field.split(',').all(|part| cron_atom_ok(part, lo, hi))
}

fn cron_atom_ok(atom: &str, lo: u32, hi: u32) -> bool {
    // Optional step: `<range>/<n>`.
    let (range, step) = match atom.split_once('/') {
        Some((r, s)) => (r, Some(s)),
        None => (atom, None),
    };
    if let Some(s) = step {
        if s.parse::<u32>().map(|n| n == 0).unwrap_or(true) {
            return false; // step must be a positive integer
        }
    }
    if range == "*" {
        return true;
    }
    // `a` or `a-b`.
    match range.split_once('-') {
        Some((a, b)) => match (a.parse::<u32>(), b.parse::<u32>()) {
            (Ok(a), Ok(b)) => a >= lo && b <= hi && a <= b,
            _ => false,
        },
        None => range.parse::<u32>().map(|n| n >= lo && n <= hi).unwrap_or(false),
    }
}

/// Extract inner text of each `${{ ... }}` in a string.
fn expressions_in(s: &str) -> Vec<String> {
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

/// Whether an expression reads attacker-controllable **free-text** input — the real script
/// injection vectors. Deliberately narrow (matching actionlint's curated list): we flag
/// `github.head_ref` and `github.event.*` values whose *content* an attacker controls (PR/
/// issue titles and bodies, commit messages, author names/emails, branch/label names, ...),
/// but NOT constrained fields like `.sha` or `.number` that can't carry a shell payload.
fn is_untrusted_context(expr: &str) -> bool {
    let e = expr.trim();
    // `github.head_ref` is the attacker's branch name — always untrusted.
    if e == "github.head_ref" {
        return true;
    }
    // Otherwise it must read from `github.event.` ...
    let Some(rest) = e.strip_prefix("github.event.") else {
        return false;
    };
    // ... and end in a known attacker-controlled free-text property.
    const UNTRUSTED_SUFFIXES: &[&str] = &[
        ".title",
        ".body",
        ".message",
        ".name",
        ".email",
        ".description",
        ".label",
        ".default_branch",
        // Only the attacker's own branch/label (head.*), not base.* (the target branch).
        ".head.ref",
        ".head.label",
        ".head.repo.default_branch",
        ".author.email",
        ".author.name",
        ".authors", // commit authors list (used via .*.email/.name)
    ];
    // Match either an exact suffix or the property directly (e.g. `pull_request.title`).
    UNTRUSTED_SUFFIXES
        .iter()
        .any(|suf| rest.ends_with(suf) || rest.ends_with(suf.trim_start_matches('.')))
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn findings(wf: Value) -> Vec<CheckFinding> {
        check(&wf)
    }

    fn rules(wf: Value) -> Vec<String> {
        let mut r: Vec<String> = findings(wf).into_iter().map(|f| f.rule_id).collect();
        r.sort();
        r
    }

    #[test]
    fn script_injection_from_event_is_flagged() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [
                { "run": "echo ${{ github.event.pull_request.title }}" }
            ] } }
        });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "security/script-injection");
    }

    #[test]
    fn head_ref_injection_is_flagged() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [
                { "run": "git checkout ${{ github.head_ref }}" }
            ] } }
        });
        assert_eq!(rules(wf), vec!["security/script-injection"]);
    }

    #[test]
    fn trusted_context_in_run_is_clean() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [
                { "run": "echo ${{ github.sha }} ${{ runner.os }} ${{ env.X }}" }
            ] } }
        });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn constrained_event_fields_are_not_injection() {
        // SHAs and the base (target) branch ref are not free-text attacker payloads.
        for expr in [
            "github.event.pull_request.head.sha",
            "github.event.pull_request.base.sha",
            "github.event.pull_request.base.ref",
            "github.event.number",
        ] {
            assert!(!is_untrusted_context(expr), "{expr} should be trusted");
        }
        // But head.ref and free-text fields are untrusted.
        assert!(is_untrusted_context("github.event.pull_request.head.ref"));
        assert!(is_untrusted_context("github.head_ref"));
        assert!(is_untrusted_context("github.event.commits.0.message"));
    }

    #[test]
    fn deprecated_set_output_is_flagged() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [
                { "run": "echo ::set-output name=x::1" }
            ] } }
        });
        assert_eq!(rules(wf), vec!["deprecated/workflow-command"]);
    }

    #[test]
    fn deprecated_save_state_is_flagged() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [ { "run": "echo ::save-state name=x::1" } ] } }
        });
        assert_eq!(rules(wf), vec!["deprecated/workflow-command"]);
    }

    #[test]
    fn hardcoded_credential_is_flagged() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x",
                "container": { "image": "x", "credentials": { "username": "u", "password": "hunter2" } }
            } }
        });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "security/hardcoded-credential");
        assert!(f[0].pointer.ends_with("/password"));
    }

    #[test]
    fn secret_credential_is_clean() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x",
                "container": { "credentials": { "password": "${{ secrets.REGISTRY_PW }}" } }
            } }
        });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn valid_cron_is_clean() {
        let wf = json!({ "on": { "schedule": [ { "cron": "0 0 * * *" }, { "cron": "*/15 9-17 * * 1-5" } ] } });
        let f = findings(wf);
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn invalid_cron_field_count_is_flagged() {
        let wf = json!({ "on": { "schedule": [ { "cron": "0 0 * *" } ] } });
        let f = findings(wf);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rule_id, "misc/cron");
        assert!(f[0].message.contains("5 space-separated"));
    }

    #[test]
    fn invalid_cron_range_is_flagged() {
        let wf = json!({ "on": { "schedule": [ { "cron": "99 0 * * *" } ] } });
        assert_eq!(rules(wf), vec!["misc/cron"]);
    }

    #[test]
    fn cron_field_validation_units() {
        assert!(cron_error("0 0 * * *").is_none());
        assert!(cron_error("*/5 * * * *").is_none());
        assert!(cron_error("0 0 1-15 * 1,3,5").is_none());
        assert!(cron_error("60 0 * * *").is_some()); // minute out of range
        assert!(cron_error("0 0 0 * *").is_some()); // day-of-month min is 1
        assert!(cron_error("0 0 * * 7").is_some()); // dow max is 6
        assert!(cron_error("* * * *").is_some()); // too few fields
        assert!(cron_error("a b c d e").is_some()); // non-numeric
        assert!(cron_error("*/0 * * * *").is_some()); // zero step
        assert!(cron_error("5-2 * * * *").is_some()); // reversed range
    }

    #[test]
    fn multiple_findings_across_steps() {
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [
                { "run": "echo ${{ github.event.issue.title }}" },
                { "run": "echo ::set-output name=y::2" }
            ] } }
        });
        assert_eq!(
            rules(wf),
            vec!["deprecated/workflow-command", "security/script-injection"]
        );
    }
}
