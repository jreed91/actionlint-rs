//! Orchestration: parse a workflow, validate its structure and its `${{ }}` expressions,
//! and produce diagnostics.
//!
//! Two passes run over the parsed workflow: structural validation against the schema
//! (`humanize`d), and expression type-checking (`expr_lint`, ADR-0006). Their diagnostics are
//! merged and sorted by source position.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jsonschema::Validator;

use crate::config::Config;
use crate::diagnostic::{Diagnostic, Position};
use crate::run_lint::{self, RunLinters};
use crate::{
    checks, dataflow, enums, events, expr_lint, graph, humanize, reusable, runner, span, uses,
    yaml,
};

/// Lint one workflow file's source text. Uses auto-detected external `run:` linters
/// (shellcheck/pyflakes if installed). For explicit control, use [`lint_source_with`].
pub fn lint_source(validator: &Validator, path: &Path, source: &str) -> Result<Vec<Diagnostic>> {
    lint_source_with(validator, path, source, &RunLinters::default())
}

/// Lint one workflow file's source text, with explicit control over external `run:` linters.
///
/// `path` is used only for diagnostic display; `source` is the file contents.
pub fn lint_source_with(
    validator: &Validator,
    path: &Path,
    source: &str,
    run_linters: &RunLinters,
) -> Result<Vec<Diagnostic>> {
    lint_source_full(validator, path, source, run_linters, &Config::default())
}

/// Lint one workflow file's source text, with explicit external-linter control **and** a
/// loaded [`Config`] (`.github/actionlint.yaml`). The config supplies self-hosted runner
/// labels for `runs-on` checking; message-regex `ignore` patterns are applied by the caller
/// (they compose with `--ignore`), not here.
///
/// `path` is used only for diagnostic display; `source` is the file contents.
pub fn lint_source_full(
    validator: &Validator,
    path: &Path,
    source: &str,
    run_linters: &RunLinters,
    config: &Config,
) -> Result<Vec<Diagnostic>> {
    let parsed = yaml::parse(source)
        .with_context(|| format!("could not parse {}", path.display()))?;

    let mut diagnostics: Vec<Diagnostic> = validator
        .iter_errors(&parsed.json)
        .map(|error| {
            // Turn the raw schema error into plain language, re-anchoring to the deepest
            // relevant node (humanize may descend into `oneOf` branches for a better path).
            let h = humanize::humanize(&error);
            // Resolve the full span; fall back to the document start if it doesn't resolve.
            let (pos, end) = match span::range_for_pointer(&parsed.marked, &h.pointer) {
                Some((start, end)) => (start, Some(end)),
                None => (Position::new(1, 1), None),
            };
            Diagnostic::new(path.to_path_buf(), pos, h.pointer, h.message)
                .with_end(end)
                .with_rule_id(h.rule_id)
        })
        .collect();

    // Expression pass: type-check every `${{ }}` embedded in a string value.
    diagnostics.extend(expr_lint::lint_expressions(&parsed.json, &parsed.marked, path));

    // Graph pass: `needs:` referencing undefined jobs, and cycles.
    for f in graph::check(&parsed.json) {
        let pos = span::position_for_pointer(&parsed.marked, &f.pointer)
            .unwrap_or_else(|| Position::new(1, 1));
        diagnostics.push(
            Diagnostic::new(path.to_path_buf(), pos, f.pointer, f.message)
                .with_rule_id("graph/needs"),
        );
    }

    // `uses:` pass: validate action-reference format.
    for f in uses::check(&parsed.json) {
        let pos = span::position_for_pointer(&parsed.marked, &f.pointer)
            .unwrap_or_else(|| Position::new(1, 1));
        diagnostics.push(
            Diagnostic::new(path.to_path_buf(), pos, f.pointer, f.message)
                .with_rule_id("uses/format"),
        );
    }

    // Runner-label pass (opt-in): `runs-on` labels must be known GitHub-hosted labels or
    // declared self-hosted labels (from config). Off by default — see the `runner` module.
    if config.check_runner_labels {
        for f in runner::check(&parsed.json, &config.self_hosted_runner_labels) {
            let pos = span::position_for_pointer(&parsed.marked, &f.pointer)
                .unwrap_or_else(|| Position::new(1, 1));
            diagnostics.push(
                Diagnostic::new(path.to_path_buf(), pos, f.pointer, f.message)
                    .with_rule_id(f.rule_id),
            );
        }
    }

    // Dataflow pass: undefined step-id references and non-needed job references.
    for f in dataflow::check(&parsed.json) {
        let pos = span::position_for_pointer(&parsed.marked, &f.pointer)
            .unwrap_or_else(|| Position::new(1, 1));
        diagnostics.push(
            Diagnostic::new(path.to_path_buf(), pos, f.pointer, f.message).with_rule_id(f.rule_id),
        );
    }

    // Reusable-workflow pass: check `with:`/`secrets:` of local `workflow_call` jobs against
    // the callee's declared contract (local `./` callees only; remote needs network).
    for f in reusable::check(&parsed.json, path) {
        let pos = span::position_for_pointer(&parsed.marked, &f.pointer)
            .unwrap_or_else(|| Position::new(1, 1));
        diagnostics.push(
            Diagnostic::new(path.to_path_buf(), pos, f.pointer, f.message).with_rule_id(f.rule_id),
        );
    }

    // Event pass: `on:` event names and activity types.
    for f in events::check(&parsed.json) {
        let pos = span::position_for_pointer(&parsed.marked, &f.pointer)
            .unwrap_or_else(|| Position::new(1, 1));
        diagnostics.push(
            Diagnostic::new(path.to_path_buf(), pos, f.pointer, f.message).with_rule_id(f.rule_id),
        );
    }

    // Enum pass: permissions scopes/levels and shell keywords (bounded, on by default).
    for f in enums::check(&parsed.json) {
        let pos = span::position_for_pointer(&parsed.marked, &f.pointer)
            .unwrap_or_else(|| Position::new(1, 1));
        diagnostics.push(
            Diagnostic::new(path.to_path_buf(), pos, f.pointer, f.message).with_rule_id(f.rule_id),
        );
    }

    // Security + misc pass: script injection, hardcoded creds, deprecated commands, cron.
    for f in checks::check(&parsed.json) {
        let pos = span::position_for_pointer(&parsed.marked, &f.pointer)
            .unwrap_or_else(|| Position::new(1, 1));
        diagnostics.push(
            Diagnostic::new(path.to_path_buf(), pos, f.pointer, f.message).with_rule_id(f.rule_id),
        );
    }

    // `run:` pass: lint shell/python scripts via external tools (skipped if not installed).
    for f in run_lint::check(&parsed.json, run_linters) {
        let pos = span::position_for_pointer(&parsed.marked, &f.pointer)
            .unwrap_or_else(|| Position::new(1, 1));
        diagnostics.push(
            Diagnostic::new(path.to_path_buf(), pos, f.pointer, f.message).with_rule_id(f.rule_id),
        );
    }

    // Stable, source-order output: sort by position, then by pointer, then message for
    // determinism (a scalar can carry both structural and expression diagnostics).
    diagnostics.sort_by(|a, b| {
        (a.pos.line, a.pos.col, &a.pointer, &a.message).cmp(&(
            b.pos.line,
            b.pos.col,
            &b.pointer,
            &b.message,
        ))
    });
    Ok(diagnostics)
}

/// Lint a file on disk, with explicit external-linter control.
pub fn lint_file_with(
    validator: &Validator,
    path: &Path,
    run_linters: &RunLinters,
) -> Result<Vec<Diagnostic>> {
    lint_file_full(validator, path, run_linters, &Config::default())
}

/// Lint a file on disk, with explicit external-linter control and a loaded [`Config`].
pub fn lint_file_full(
    validator: &Validator,
    path: &Path,
    run_linters: &RunLinters,
    config: &Config,
) -> Result<Vec<Diagnostic>> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    lint_source_full(validator, path, &source, run_linters, config)
}

/// Lint a file on disk (auto-detected external linters).
pub fn lint_file(validator: &Validator, path: &Path) -> Result<Vec<Diagnostic>> {
    lint_file_with(validator, path, &RunLinters::default())
}

/// Lint stdin, labeling diagnostics as coming from `<stdin>`, with explicit linter control.
pub fn lint_stdin_with(
    validator: &Validator,
    source: &str,
    run_linters: &RunLinters,
) -> Result<Vec<Diagnostic>> {
    lint_stdin_full(validator, source, run_linters, &Config::default())
}

/// Lint stdin with explicit linter control and a loaded [`Config`].
pub fn lint_stdin_full(
    validator: &Validator,
    source: &str,
    run_linters: &RunLinters,
    config: &Config,
) -> Result<Vec<Diagnostic>> {
    lint_source_full(validator, &PathBuf::from("<stdin>"), source, run_linters, config)
}

/// Lint stdin (auto-detected external linters).
pub fn lint_stdin(validator: &Validator, source: &str) -> Result<Vec<Diagnostic>> {
    lint_stdin_with(validator, source, &RunLinters::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema;

    fn validator() -> Validator {
        schema::build_validator().unwrap()
    }

    #[test]
    fn valid_workflow_has_no_diagnostics() {
        let src = "\
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
";
        let diags = lint_source_with(&validator(), Path::new("wf.yml"), src, &RunLinters::none()).unwrap();
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
    }

    #[test]
    fn missing_runs_on_is_flagged() {
        // A normal job requires runs-on; omitting it is a structural error.
        let src = "\
on: push
jobs:
  build:
    steps:
      - run: echo hi
";
        let diags = lint_source_with(&validator(), Path::new("wf.yml"), src, &RunLinters::none()).unwrap();
        assert!(!diags.is_empty(), "expected a diagnostic for missing runs-on");
    }

    #[test]
    fn lint_file_reads_from_disk() {
        let dir = std::env::temp_dir().join(format!("alr-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("wf.yml");
        std::fs::write(&file, "on: push\njobs:\n  b:\n    steps: []\n").unwrap();
        let diags = lint_file(&validator(), &file).unwrap();
        assert!(!diags.is_empty(), "job without runs-on should be flagged");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lint_file_errors_on_missing_file() {
        let err = lint_file(&validator(), Path::new("/no/such/file.yml")).unwrap_err();
        assert!(err.to_string().contains("could not read"));
    }

    #[test]
    fn lint_stdin_labels_source_as_stdin() {
        let diags = lint_stdin(&validator(), "on: push\njobs:\n  b:\n    steps: []\n").unwrap();
        assert!(!diags.is_empty());
        assert_eq!(diags[0].file.to_string_lossy(), "<stdin>");
    }

    #[test]
    fn lint_source_errors_on_unparseable_yaml() {
        let err = lint_source_with(&validator(), Path::new("wf.yml"), "on: [push", &RunLinters::none()).unwrap_err();
        assert!(err.to_string().contains("could not parse"));
    }

    #[test]
    fn diagnostics_are_sorted_by_position() {
        // Two distinct structural problems; output must be position-ordered.
        let src = "on: 42\njobs: []\n";
        let diags = lint_source_with(&validator(), Path::new("wf.yml"), src, &RunLinters::none()).unwrap();
        assert!(diags.len() >= 2, "expected multiple diagnostics: {diags:?}");
        for pair in diags.windows(2) {
            let a = (pair[0].pos.line, pair[0].pos.col);
            let b = (pair[1].pos.line, pair[1].pos.col);
            assert!(a <= b, "diagnostics not sorted: {a:?} then {b:?}");
        }
    }

    #[test]
    fn broken_expression_is_flagged() {
        // ADR-0006: expressions are no longer opaque — `this` is an unknown context.
        let src = "\
on: push
jobs:
  build:
    runs-on: ${{ this.is.not.validated }}
    steps:
      - run: echo hi
";
        let diags = lint_source_with(&validator(), Path::new("wf.yml"), src, &RunLinters::none()).unwrap();
        assert!(
            diags.iter().any(|d| d.message.contains("unknown context `this`")),
            "expected an expression diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn valid_expression_and_structure_is_clean() {
        let src = "\
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    if: ${{ github.event_name == 'push' }}
    steps:
      - run: echo ${{ runner.os }}
";
        let diags = lint_source_with(&validator(), Path::new("wf.yml"), src, &RunLinters::none()).unwrap();
        assert!(diags.is_empty(), "got: {diags:?}");
    }

    #[test]
    fn structural_and_expression_diagnostics_coexist() {
        // Missing runs-on (structural) AND an unknown context (expression).
        let src = "\
on: push
jobs:
  build:
    if: ${{ bogus.x }}
    steps:
      - run: echo hi
";
        let diags = lint_source_with(&validator(), Path::new("wf.yml"), src, &RunLinters::none()).unwrap();
        assert!(diags.iter().any(|d| d.message.contains("runs-on")));
        assert!(diags.iter().any(|d| d.message.contains("unknown context `bogus`")));
    }

    #[test]
    fn graph_and_uses_diagnostics_are_emitted() {
        // undefined `needs` + unpinned `uses` come through lint_source_with.
        let src = "\
on: push
jobs:
  a:
    runs-on: ubuntu-latest
    needs: ghost
    steps:
      - uses: actions/checkout
";
        let diags = lint_source_with(&validator(), Path::new("wf.yml"), src, &RunLinters::none()).unwrap();
        assert!(diags.iter().any(|d| d.rule_id == "graph/needs"), "{diags:?}");
        assert!(diags.iter().any(|d| d.rule_id == "uses/format"), "{diags:?}");
    }

    #[test]
    fn run_pass_emits_findings_via_fake_linter() {
        // Force a shellcheck-shaped run via a fake binary (`/bin/cat` echoes non-JSON, so no
        // findings) AND a pyflakes fake that echoes a finding line — exercises the run pass
        // wiring in lint_source_with regardless of installed tools.
        let linters = RunLinters {
            shellcheck: None,
            pyflakes: Some("/bin/cat".to_string()),
        };
        let src = "\
on: push
jobs:
  b:
    runs-on: ubuntu-latest
    steps:
      - shell: python
        run: \"<stdin>:2:1 undefined name 'x'\"
";
        let diags = lint_source_with(&validator(), Path::new("wf.yml"), src, &linters).unwrap();
        assert!(
            diags.iter().any(|d| d.rule_id == "run/pyflakes"),
            "expected a run/pyflakes finding, got: {diags:?}"
        );
    }

    #[test]
    fn lint_file_and_stdin_default_wrappers_work() {
        // Exercise the auto-detecting default wrappers (lint_file / lint_stdin).
        let dir = std::env::temp_dir().join(format!("alr-lint-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("wf.yml");
        std::fs::write(&f, "on: push\njobs:\n  b:\n    steps: [{run: hi}]\n").unwrap();
        let diags = lint_file(&validator(), &f).unwrap();
        assert!(diags.iter().any(|d| d.message.contains("runs-on")));
        std::fs::remove_dir_all(&dir).ok();

        let diags = lint_stdin(&validator(), "on: push\njobs:\n  b:\n    steps: [{run: hi}]\n").unwrap();
        assert!(diags[0].file.to_string_lossy().contains("stdin"));
    }
}

