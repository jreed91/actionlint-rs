//! `runs-on` runner-label checking (enum tightening the schema leaves loose).
//!
//! The structural schema types `runs-on` as "string or list of strings" — it can't know
//! *which* labels are real. GitHub-hosted runners advertise a fixed, published set of labels
//! (`ubuntu-latest`, `windows-2022`, ...); anything else is either a self-hosted label or a
//! typo. We flag labels that are neither a known GitHub-hosted label nor declared in the
//! `.github/actionlint.yaml` `self-hosted-runner.labels` list.
//!
//! **Opt-in** (`--check-runner-labels`). Unlike the other checks, this one is *off by
//! default*: larger GitHub runners (`ubuntu-latest-8core`, ...) and self-hosted labels are
//! project-specific and unbounded, so a static allowlist can't meet the project's
//! zero-false-positive bar without per-repo config. Enabling it pairs naturally with
//! declaring the repo's real labels under `self-hosted-runner.labels`.
//!
//! Conservative by design:
//! - `self-hosted` and the common arch/OS grouping labels (`linux`, `x64`, ...) are always
//!   allowed — they're the documented self-hosted convention.
//! - A `runs-on` value containing an expression (`${{ }}`) or a matrix reference is skipped
//!   entirely (its real value isn't known statically).
//! - The `runs-on: { group:, labels: }` object form is handled: `group` is a runner-group
//!   name (never validated), `labels` are validated like the list form.

use serde_json::Value;

use crate::checks::CheckFinding;

/// GitHub-hosted runner labels (as of 2026-01; resync-able from GitHub's runner-images docs).
/// Includes the `-latest` aliases and the currently-supported pinned versions.
const GITHUB_HOSTED_LABELS: &[&str] = &[
    // Ubuntu
    "ubuntu-latest",
    "ubuntu-24.04",
    "ubuntu-22.04",
    "ubuntu-20.04",
    "ubuntu-24.04-arm",
    "ubuntu-22.04-arm",
    // Windows
    "windows-latest",
    "windows-2025",
    "windows-2022",
    "windows-2019",
    "windows-11-arm",
    // macOS
    "macos-latest",
    "macos-latest-large",
    "macos-latest-xlarge",
    "macos-15",
    "macos-15-large",
    "macos-15-xlarge",
    "macos-14",
    "macos-14-large",
    "macos-14-xlarge",
    "macos-13",
    "macos-13-large",
    "macos-13-xlarge",
    "macos-12",
];

/// Self-hosted convention labels that are always acceptable without explicit config: the
/// `self-hosted` marker plus the documented OS/arch grouping labels.
const SELF_HOSTED_CONVENTION: &[&str] = &[
    "self-hosted",
    // OS
    "linux",
    "windows",
    "macos",
    // Architecture
    "x64",
    "arm",
    "arm64",
];

/// Check every job's `runs-on` labels against known GitHub-hosted labels plus the
/// config-declared self-hosted labels.
pub fn check(workflow: &Value, self_hosted_labels: &[String]) -> Vec<CheckFinding> {
    let mut out = Vec::new();
    let Some(jobs) = workflow.get("jobs").and_then(|j| j.as_object()) else {
        return out;
    };
    for (job_id, job) in jobs {
        let Some(runs_on) = job.get("runs-on") else {
            continue;
        };
        let ptr = format!("/jobs/{}/runs-on", escape(job_id));
        check_runs_on(runs_on, &ptr, self_hosted_labels, &mut out);
    }
    out
}

fn check_runs_on(
    runs_on: &Value,
    pointer: &str,
    self_hosted_labels: &[String],
    out: &mut Vec<CheckFinding>,
) {
    match runs_on {
        Value::String(label) => check_label(label, pointer, self_hosted_labels, out),
        Value::Array(labels) => {
            for (i, item) in labels.iter().enumerate() {
                if let Value::String(label) = item {
                    check_label(label, &format!("{pointer}/{i}"), self_hosted_labels, out);
                }
            }
        }
        Value::Object(map) => {
            // `{ group:, labels: }` form. Only `labels` is a label set; `group` is a
            // runner-group name we never validate.
            if let Some(labels) = map.get("labels") {
                let labels_ptr = format!("{pointer}/labels");
                check_runs_on(labels, &labels_ptr, self_hosted_labels, out);
            }
        }
        _ => {}
    }
}

fn check_label(
    label: &str,
    pointer: &str,
    self_hosted_labels: &[String],
    out: &mut Vec<CheckFinding>,
) {
    // Skip dynamic values: expressions and matrix placeholders can't be checked statically.
    if label.contains("${{") || label.trim().is_empty() {
        return;
    }
    if is_known(label, self_hosted_labels) {
        return;
    }
    out.push(CheckFinding {
        pointer: pointer.to_string(),
        message: format!(
            "unknown runner label `{label}` — not a GitHub-hosted runner label; if this is a \
             self-hosted runner, declare it under `self-hosted-runner.labels` in \
             `.github/actionlint.yaml`"
        ),
        rule_id: "runner/label".to_string(),
    });
}

fn is_known(label: &str, self_hosted_labels: &[String]) -> bool {
    // GitHub matches runner labels case-insensitively, so `Linux` == `linux`,
    // `ubuntu-LATEST` == `ubuntu-latest`, etc. Compare everything case-insensitively.
    GITHUB_HOSTED_LABELS
        .iter()
        .any(|l| l.eq_ignore_ascii_case(label))
        || SELF_HOSTED_CONVENTION
            .iter()
            .any(|l| l.eq_ignore_ascii_case(label))
        || self_hosted_labels
            .iter()
            .any(|l| l.eq_ignore_ascii_case(label))
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rules(wf: Value, labels: &[String]) -> Vec<String> {
        let mut r: Vec<String> = check(&wf, labels)
            .into_iter()
            .map(|f| f.rule_id)
            .collect();
        r.sort();
        r
    }

    fn findings(wf: Value) -> Vec<CheckFinding> {
        check(&wf, &[])
    }

    #[test]
    fn github_hosted_labels_are_clean() {
        for label in ["ubuntu-latest", "windows-2022", "macos-14", "ubuntu-22.04-arm"] {
            let wf = json!({ "jobs": { "b": { "runs-on": label } } });
            assert!(findings(wf).is_empty(), "{label} should be known");
        }
    }

    #[test]
    fn unknown_label_is_flagged() {
        let wf = json!({ "jobs": { "b": { "runs-on": "ubunto-latest" } } });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "runner/label");
        assert!(f[0].message.contains("ubunto-latest"));
        assert_eq!(f[0].pointer, "/jobs/b/runs-on");
    }

    #[test]
    fn self_hosted_convention_labels_are_clean() {
        let wf = json!({ "jobs": { "b": { "runs-on": ["self-hosted", "linux", "x64"] } } });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn labels_match_case_insensitively() {
        // GitHub matches labels case-insensitively: `Linux`, `Self-Hosted`, `Ubuntu-Latest`
        // are all known.
        for label in ["Linux", "Self-Hosted", "Ubuntu-Latest", "WINDOWS", "ARM64"] {
            let wf = json!({ "jobs": { "b": { "runs-on": label } } });
            assert!(findings(wf).is_empty(), "{label} should be known (case-insensitive)");
        }
    }

    #[test]
    fn configured_self_hosted_label_is_clean() {
        let wf = json!({ "jobs": { "b": { "runs-on": ["self-hosted", "gpu-16core"] } } });
        // Without config: flagged.
        assert_eq!(rules(wf.clone(), &[]), vec!["runner/label"]);
        // With config: clean.
        assert!(check(&wf, &["gpu-16core".to_string()]).is_empty());
    }

    #[test]
    fn configured_label_matches_case_insensitively() {
        let wf = json!({ "jobs": { "b": { "runs-on": "GPU-Big" } } });
        assert!(check(&wf, &["gpu-big".to_string()]).is_empty());
    }

    #[test]
    fn expression_runs_on_is_skipped() {
        let wf = json!({ "jobs": { "b": { "runs-on": "${{ matrix.os }}" } } });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn group_and_labels_object_form() {
        // group is never validated; labels are.
        let wf = json!({
            "jobs": { "b": { "runs-on": { "group": "my-group", "labels": ["ubuntu-latest"] } } }
        });
        assert!(findings(wf).is_empty());

        let wf = json!({
            "jobs": { "b": { "runs-on": { "group": "my-group", "labels": ["typo-label"] } } }
        });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].pointer.ends_with("/labels/0"));
    }

    #[test]
    fn job_without_runs_on_is_skipped() {
        // Reusable jobs (uses:) have no runs-on; not our concern here.
        let wf = json!({ "jobs": { "b": { "uses": "org/repo/.github/workflows/x.yml@main" } } });
        assert!(findings(wf).is_empty());
    }
}
