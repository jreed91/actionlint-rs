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
