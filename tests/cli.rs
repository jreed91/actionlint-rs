//! Integration tests for the CLI (src/main.rs), which is excluded from library coverage.
//! These run the built binary to exercise arg parsing, stdin, discovery, and exit codes.

use std::io::Write;
use std::process::{Command, Stdio};

/// Path to the binary built by `cargo test` (Cargo sets CARGO_BIN_EXE_<name>).
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_actionlint-rs")
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("alr-it-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn good_file_exits_zero() {
    let dir = tmpdir("good");
    let f = dir.join("wf.yml");
    std::fs::write(&f, "on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [{run: hi}]\n").unwrap();
    let out = Command::new(bin()).arg(&f).output().unwrap();
    assert!(out.status.success(), "expected exit 0, stderr: {}", String::from_utf8_lossy(&out.stderr));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn bad_file_exits_one_and_prints_position() {
    let dir = tmpdir("bad");
    let f = dir.join("wf.yml");
    std::fs::write(&f, "on: push\njobs:\n  b:\n    steps: [{run: hi}]\n").unwrap();
    let out = Command::new(bin()).arg(&f).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(":"), "expected file:line:col diagnostic, got: {stdout}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn stdin_dash_is_linted() {
    let mut child = Command::new(bin())
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"on: push\njobs:\n  b:\n    steps: [{run: hi}]\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("<stdin>"));
}

#[test]
fn discovers_workflows_dir() {
    let dir = tmpdir("discover");
    let wf = dir.join(".github").join("workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(wf.join("ci.yml"), "on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [{run: hi}]\n").unwrap();
    let out = Command::new(bin()).current_dir(&dir).output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_workflows_found_is_not_an_error() {
    let dir = tmpdir("empty");
    let out = Command::new(bin()).current_dir(&dir).output().unwrap();
    // No .github/workflows -> informational, exit 0.
    assert!(out.status.success());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn empty_string_arg_falls_through_to_discovery() {
    // The Docker action passes `${{ inputs.files }}`, which is an empty arg `""` when the
    // input is unset. It must behave like "no args" (discover), not error.
    let dir = tmpdir("emptyarg");
    let wf = dir.join(".github").join("workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(
        wf.join("ci.yml"),
        "on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [{run: hi}]\n",
    )
    .unwrap();
    let out = Command::new(bin()).arg("").current_dir(&dir).output().unwrap();
    assert!(
        out.status.success(),
        "empty arg should discover, not error. stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_file_is_usage_error_exit_two() {
    let out = Command::new(bin()).arg("/no/such/wf.yml").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn version_flag_reports_crate_version() {
    // Release assets are named by this; it must reflect the Cargo.toml version that
    // semantic-release bumps. `--version` exits 0 and prints `actionlint-rs <version>`.
    let out = Command::new(bin()).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("actionlint-rs "), "got: {stdout}");
    assert!(
        stdout.trim().contains(env!("CARGO_PKG_VERSION")),
        "version output {stdout:?} should contain crate version {}",
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn ignore_pattern_suppresses_matching_diagnostics() {
    let dir = tmpdir("ignore");
    let f = dir.join("wf.yml");
    std::fs::write(&f, "on: push\njobs:\n  b:\n    steps: [{run: hi}]\n").unwrap();
    // Without --ignore: flagged (exit 1).
    let out = Command::new(bin()).arg(&f).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    // With a matching --ignore: suppressed (exit 0, no output).
    let out = Command::new(bin())
        .arg(&f)
        .arg("--ignore")
        .arg("runs-on")
        .output()
        .unwrap();
    assert!(out.status.success(), "matching --ignore should suppress the only finding");
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn invalid_ignore_pattern_is_usage_error_exit_two() {
    let out = Command::new(bin())
        .arg("-")
        .arg("--ignore")
        .arg("(")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid --ignore pattern"));
}

#[test]
fn config_ignore_suppresses_diagnostics() {
    // A committed `.github/actionlint.yaml` `ignore:` entry suppresses like --ignore.
    let dir = tmpdir("cfg-ignore");
    let wf = dir.join(".github").join("workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(wf.join("ci.yml"), "on: push\njobs:\n  b:\n    steps: [{run: hi}]\n").unwrap();
    // Without config: exit 1 (missing runs-on).
    let out = Command::new(bin()).current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    // With config ignoring the message: exit 0.
    std::fs::write(
        dir.join(".github").join("actionlint.yaml"),
        "ignore:\n  - 'runs-on'\n",
    )
    .unwrap();
    let out = Command::new(bin()).current_dir(&dir).output().unwrap();
    assert!(out.status.success(), "config ignore should suppress. stderr: {}", String::from_utf8_lossy(&out.stderr));
    // And --no-config restores the finding.
    let out = Command::new(bin()).arg("--no-config").current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn malformed_config_is_usage_error_exit_two() {
    let dir = tmpdir("cfg-bad");
    let wf = dir.join(".github").join("workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(wf.join("ci.yml"), "on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [{run: hi}]\n").unwrap();
    std::fs::write(
        dir.join(".github").join("actionlint.yaml"),
        "self-hosted-runner:\n  labels: not-a-list\n",
    )
    .unwrap();
    let out = Command::new(bin()).current_dir(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("must be a list"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn runner_label_check_is_opt_in_and_config_aware() {
    let dir = tmpdir("runner");
    let f = dir.join("wf.yml");
    std::fs::write(
        &f,
        "on: push\njobs:\n  b:\n    runs-on: totally-made-up-label\n    steps: [{run: hi}]\n",
    )
    .unwrap();
    // Default: not checked, exit 0.
    let out = Command::new(bin()).arg(&f).output().unwrap();
    assert!(out.status.success(), "runner check should be off by default");
    // Opt-in: flagged, exit 1.
    let out = Command::new(bin()).arg(&f).arg("--check-runner-labels").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("unknown runner label"));
    // Opt-in + config declaring the label: clean.
    let cfg = dir.join("cfg.yaml");
    std::fs::write(&cfg, "self-hosted-runner:\n  labels: [totally-made-up-label]\n").unwrap();
    let out = Command::new(bin())
        .arg(&f)
        .arg("--check-runner-labels")
        .arg("--config")
        .arg(&cfg)
        .output()
        .unwrap();
    assert!(out.status.success(), "declared label should be accepted. stdout: {}", String::from_utf8_lossy(&out.stdout));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_explicit_config_is_usage_error() {
    let out = Command::new(bin())
        .arg("-")
        .arg("--config")
        .arg("/no/such/config.yaml")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn sarif_format_emits_valid_sarif_and_exit_one_on_problems() {
    let dir = tmpdir("sarif");
    let f = dir.join("wf.yml");
    std::fs::write(&f, "on: push\njobs:\n  b:\n    steps: [{run: hi}]\n").unwrap();
    let out = Command::new(bin())
        .arg(&f)
        .arg("--format")
        .arg("sarif")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("SARIF output must be valid JSON");
    assert_eq!(v["version"], "2.1.0");
    assert_eq!(v["runs"][0]["results"][0]["ruleId"], "structure/required");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn json_format_emits_array_of_diagnostics_and_exit_one() {
    let dir = tmpdir("json");
    let f = dir.join("wf.yml");
    std::fs::write(&f, "on: push\njobs:\n  b:\n    steps: [{run: hi}]\n").unwrap();
    let out = Command::new(bin())
        .arg(&f)
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("valid JSON");
    let arr = v.as_array().expect("top-level array");
    assert!(!arr.is_empty());
    assert_eq!(arr[0]["rule"], "structure/required");
    assert!(arr[0]["line"].is_number());
    assert!(arr[0]["message"].is_string());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn json_format_on_clean_file_is_empty_array_exit_zero() {
    let dir = tmpdir("json-clean");
    let f = dir.join("wf.yml");
    std::fs::write(
        &f,
        "on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [{run: hi}]\n",
    )
    .unwrap();
    let out = Command::new(bin()).arg(&f).arg("--format").arg("json").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 0);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn sarif_format_on_clean_file_is_valid_and_exit_zero() {
    let dir = tmpdir("sarif-clean");
    let f = dir.join("wf.yml");
    std::fs::write(
        &f,
        "on: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps: [{run: hi}]\n",
    )
    .unwrap();
    let out = Command::new(bin()).arg(&f).arg("--format").arg("sarif").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v["runs"][0]["results"].as_array().unwrap().len(), 0);
    std::fs::remove_dir_all(&dir).ok();
}
