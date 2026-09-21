//! Lint `run:` script blocks via external tools (ADR-0006 step 7): shellcheck for
//! bash/sh, pyflakes for python.
//!
//! Design:
//! - The external tools are **optional**. If the binary isn't found (or fails to spawn), we
//!   silently skip — a missing linter must never break linting or produce spurious errors.
//! - The shell for a `run:` step is its `shell:` key, else the job/workflow default, else
//!   bash (GitHub's default on Linux/macOS). We only lint shells we understand.
//! - Findings are anchored at the `run:` node (via its JSON pointer); precise line-within-
//!   the-script mapping is a future refinement (same rationale as expressions).

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

/// A `run:` finding: a message plus the JSON pointer it anchors at.
#[derive(Debug, Clone, PartialEq)]
pub struct RunFinding {
    pub pointer: String,
    pub message: String,
    pub rule_id: String,
}

/// Which external tools to invoke. Absent tools are skipped; this also lets tests inject
/// explicit paths / disable a tool.
#[derive(Debug, Clone)]
pub struct RunLinters {
    /// Path to the shellcheck binary, or `None` to disable.
    pub shellcheck: Option<String>,
    /// Path to the pyflakes binary, or `None` to disable.
    pub pyflakes: Option<String>,
}

impl Default for RunLinters {
    /// Auto-detect: use `shellcheck` / `pyflakes` from PATH if present.
    fn default() -> Self {
        RunLinters {
            shellcheck: which("shellcheck"),
            pyflakes: which("pyflakes"),
        }
    }
}

impl RunLinters {
    /// No external linters (used when neither is available or for pure tests).
    pub fn none() -> Self {
        RunLinters {
            shellcheck: None,
            pyflakes: None,
        }
    }

    fn any(&self) -> bool {
        self.shellcheck.is_some() || self.pyflakes.is_some()
    }
}

/// Lint every `run:` step in the workflow with the configured external tools.
pub fn check(workflow: &Value, linters: &RunLinters) -> Vec<RunFinding> {
    let mut out = Vec::new();
    if !linters.any() {
        return out;
    }
    let workflow_shell = default_shell(workflow);
    if let Some(jobs) = workflow.get("jobs").and_then(|j| j.as_object()) {
        for (job_id, job) in jobs {
            let job_shell = default_shell(job).or_else(|| workflow_shell.clone());
            if let Some(steps) = job.get("steps").and_then(|s| s.as_array()) {
                for (i, step) in steps.iter().enumerate() {
                    let Some(run) = step.get("run").and_then(|r| r.as_str()) else {
                        continue;
                    };
                    let shell = step_shell(step).or_else(|| job_shell.clone());
                    let pointer = format!("/jobs/{}/steps/{}/run", escape(job_id), i);
                    lint_run(run, shell.as_deref(), &pointer, linters, &mut out);
                }
            }
        }
    }
    out
}

/// The `defaults.run.shell` of a workflow/job node, if set.
fn default_shell(node: &Value) -> Option<String> {
    node.get("defaults")?
        .get("run")?
        .get("shell")?
        .as_str()
        .map(|s| s.to_string())
}

fn step_shell(step: &Value) -> Option<String> {
    step.get("shell").and_then(|s| s.as_str()).map(|s| s.to_string())
}

fn lint_run(
    script: &str,
    shell: Option<&str>,
    pointer: &str,
    linters: &RunLinters,
    out: &mut Vec<RunFinding>,
) {
    // Default shell is bash on Linux/macOS (GitHub's default runner shell).
    let shell = shell.unwrap_or("bash");
    match classify_shell(shell) {
        Some(ShellKind::Posix(dialect)) => {
            if let Some(sc) = &linters.shellcheck {
                out.extend(run_shellcheck(sc, script, dialect, pointer));
            }
        }
        Some(ShellKind::Python) => {
            if let Some(pf) = &linters.pyflakes {
                out.extend(run_pyflakes(pf, script, pointer));
            }
        }
        None => {} // pwsh, cmd, custom — not linted
    }
}

enum ShellKind {
    /// A POSIX-ish shell; the value is the shellcheck `--shell` dialect.
    Posix(&'static str),
    Python,
}

fn classify_shell(shell: &str) -> Option<ShellKind> {
    // GitHub `shell:` may be a name (`bash`) or a command template (`bash -e {0}`); take the
    // first word.
    let name = shell.split_whitespace().next().unwrap_or(shell);
    let base = name.rsplit('/').next().unwrap_or(name);
    match base {
        "bash" => Some(ShellKind::Posix("bash")),
        "sh" => Some(ShellKind::Posix("sh")),
        "dash" => Some(ShellKind::Posix("dash")),
        "ksh" => Some(ShellKind::Posix("ksh")),
        "python" | "python3" => Some(ShellKind::Python),
        _ => None,
    }
}

/// Run shellcheck over a script via stdin, parse its JSON, and map to findings.
fn run_shellcheck(bin: &str, script: &str, dialect: &str, pointer: &str) -> Vec<RunFinding> {
    let output = spawn_with_stdin(
        bin,
        &["--format=json", &format!("--shell={dialect}"), "-"],
        script,
    );
    let Some(stdout) = output else {
        return Vec::new(); // spawn failed — skip silently
    };
    let Ok(items) = serde_json::from_str::<Vec<Value>>(&stdout) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|it| {
            let code = it.get("code").and_then(|c| c.as_u64()).unwrap_or(0);
            let line = it.get("line").and_then(|l| l.as_u64()).unwrap_or(0);
            let msg = it.get("message").and_then(|m| m.as_str()).unwrap_or("");
            RunFinding {
                pointer: pointer.to_string(),
                message: format!("shellcheck SC{code} (line {line} of run): {msg}"),
                rule_id: "run/shellcheck".to_string(),
            }
        })
        .collect()
}

/// Run pyflakes over a script via stdin. pyflakes prints `<file>:<line>:<col> <message>`.
fn run_pyflakes(bin: &str, script: &str, pointer: &str) -> Vec<RunFinding> {
    // pyflakes reads stdin when given no file; label is `<stdin>`.
    let output = spawn_with_stdin(bin, &[], script);
    let Some(stdout) = output else {
        return Vec::new();
    };
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let detail = parse_pyflakes_line(l);
            RunFinding {
                pointer: pointer.to_string(),
                message: format!("pyflakes (run): {detail}"),
                rule_id: "run/pyflakes".to_string(),
            }
        })
        .collect()
}

/// Turn a pyflakes line into `line <n>: <message>`. pyflakes prints either
/// `<file>:<line>:<col> <message>` or `<file>:<line>: <message>` depending on version; we
/// take the second colon-separated field as the line number and everything after the
/// location prefix as the message.
fn parse_pyflakes_line(line: &str) -> String {
    let parts: Vec<&str> = line.splitn(3, ':').collect();
    if parts.len() == 3 {
        if let Ok(n) = parts[1].trim().parse::<u32>() {
            // parts[2] is `<col> <message>` or ` <message>`; drop a leading numeric col.
            let tail = parts[2].trim_start();
            let msg = match tail.split_once(' ') {
                Some((first, rest)) if first.parse::<u32>().is_ok() => rest,
                _ => tail,
            };
            return format!("line {n}: {}", msg.trim());
        }
    }
    line.trim().to_string()
}

/// Spawn `bin args...`, write `input` to its stdin, and return stdout (combined tools use
/// stdout). Returns `None` if the process can't be spawned (missing binary, etc).
fn spawn_with_stdin(bin: &str, args: &[&str], input: &str) -> Option<String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    String::from_utf8(output.stdout).ok()
}

/// Find a binary on PATH.
fn which(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
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

    #[test]
    fn classify_shells() {
        assert!(matches!(classify_shell("bash"), Some(ShellKind::Posix("bash"))));
        assert!(matches!(classify_shell("sh"), Some(ShellKind::Posix("sh"))));
        assert!(matches!(classify_shell("bash -e {0}"), Some(ShellKind::Posix("bash"))));
        assert!(matches!(classify_shell("/usr/bin/python3"), Some(ShellKind::Python)));
        assert!(classify_shell("pwsh").is_none());
        assert!(classify_shell("cmd").is_none());
    }

    #[test]
    fn no_linters_is_a_noop() {
        let wf = json!({
            "jobs": { "b": { "steps": [ { "run": "echo $undefined" } ] } }
        });
        assert!(check(&wf, &RunLinters::none()).is_empty());
    }

    #[test]
    fn parse_pyflakes_line_extracts_line_and_message() {
        assert_eq!(
            parse_pyflakes_line("<stdin>:3:1 undefined name 'x'"),
            "line 3: undefined name 'x'"
        );
        // A line without the expected prefix is passed through trimmed.
        assert_eq!(parse_pyflakes_line("  weird output "), "weird output");
    }

    #[test]
    fn default_and_step_shell_resolution() {
        let wf = json!({ "defaults": { "run": { "shell": "sh" } } });
        assert_eq!(default_shell(&wf).as_deref(), Some("sh"));
        let step = json!({ "shell": "python", "run": "x" });
        assert_eq!(step_shell(&step).as_deref(), Some("python"));
        assert_eq!(default_shell(&json!({})), None);
    }

    #[test]
    fn which_finds_a_real_binary_or_none() {
        // `sh` exists on any unix test host; a nonsense name does not.
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-binary-xyz").is_none());
    }

    // The following test only runs where shellcheck is installed; it verifies real
    // end-to-end behavior without making CI depend on the tool.
    #[test]
    fn shellcheck_flags_a_real_issue_when_available() {
        let Some(sc) = which("shellcheck") else {
            return; // skip on hosts without shellcheck
        };
        let linters = RunLinters {
            shellcheck: Some(sc),
            pyflakes: None,
        };
        let wf = json!({
            "jobs": {
                "b": {
                    "runs-on": "x",
                    "steps": [ { "run": "echo $undefinedvar" } ]
                }
            }
        });
        let findings = check(&wf, &linters);
        assert!(
            findings.iter().any(|f| f.message.contains("shellcheck SC")),
            "expected a shellcheck finding, got: {findings:?}"
        );
        assert_eq!(findings[0].pointer, "/jobs/b/steps/0/run");
    }

    #[test]
    fn clean_script_has_no_findings_when_shellcheck_available() {
        let Some(sc) = which("shellcheck") else { return };
        let linters = RunLinters { shellcheck: Some(sc), pyflakes: None };
        let wf = json!({
            "jobs": { "b": { "runs-on": "x", "steps": [ { "run": "echo hello" } ] } }
        });
        assert!(check(&wf, &linters).is_empty());
    }

    #[test]
    fn spawn_failure_is_silent() {
        // A nonexistent binary yields no findings rather than erroring.
        assert!(run_shellcheck("no-such-bin-xyz", "echo x", "bash", "/p").is_empty());
        assert!(run_pyflakes("no-such-bin-xyz", "import x", "/p").is_empty());
    }

    #[test]
    fn shellcheck_nonjson_output_is_ignored() {
        // `cat` echoes the script back — not valid shellcheck JSON — so no findings (the
        // JSON-parse-failure branch), exercised without needing shellcheck installed.
        assert!(run_shellcheck("/bin/cat", "echo hi", "bash", "/p").is_empty());
    }

    #[test]
    fn pyflakes_output_parsing_via_fake_binary() {
        // Simulate pyflakes by piping fixed lines through `cat`: run_pyflakes maps each
        // non-empty output line to a finding, exercising the parse/mapping path.
        let script = "<stdin>:3:1 undefined name 'x'\n<stdin>:5: imported but unused\n";
        let findings = run_pyflakes("/bin/cat", script, "/jobs/b/steps/0/run");
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert!(findings[0].message.contains("line 3: undefined name 'x'"));
        assert!(findings[1].message.contains("line 5:"));
        assert_eq!(findings[0].rule_id, "run/pyflakes");
    }

    #[test]
    fn non_posix_shell_is_not_linted() {
        let Some(sc) = which("shellcheck") else { return };
        let linters = RunLinters { shellcheck: Some(sc), pyflakes: None };
        let wf = json!({
            "jobs": {
                "b": {
                    "runs-on": "x",
                    "steps": [ { "shell": "pwsh", "run": "echo $undefinedvar" } ]
                }
            }
        });
        assert!(check(&wf, &linters).is_empty(), "pwsh should not be shellcheck'd");
    }
}
