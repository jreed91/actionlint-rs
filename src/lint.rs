//! Orchestration: parse a workflow, validate its structure and its `${{ }}` expressions,
//! and produce diagnostics.
//!
//! Two passes run over the parsed workflow: structural validation against the schema
//! (`humanize`d), and expression type-checking (`expr_lint`, ADR-0006). Their diagnostics are
//! merged and sorted by source position.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jsonschema::Validator;

use crate::diagnostic::{Diagnostic, Position};
use crate::{expr_lint, humanize, span, yaml};

/// Lint one workflow file's source text against the compiled schema.
///
/// `path` is used only for diagnostic display; `source` is the file contents.
pub fn lint_source(validator: &Validator, path: &Path, source: &str) -> Result<Vec<Diagnostic>> {
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

/// Lint a file on disk.
pub fn lint_file(validator: &Validator, path: &Path) -> Result<Vec<Diagnostic>> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    lint_source(validator, path, &source)
}

/// Lint stdin, labeling diagnostics as coming from `<stdin>`.
pub fn lint_stdin(validator: &Validator, source: &str) -> Result<Vec<Diagnostic>> {
    lint_source(validator, &PathBuf::from("<stdin>"), source)
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
        let diags = lint_source(&validator(), Path::new("wf.yml"), src).unwrap();
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
        let diags = lint_source(&validator(), Path::new("wf.yml"), src).unwrap();
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
        let err = lint_source(&validator(), Path::new("wf.yml"), "on: [push").unwrap_err();
        assert!(err.to_string().contains("could not parse"));
    }

    #[test]
    fn diagnostics_are_sorted_by_position() {
        // Two distinct structural problems; output must be position-ordered.
        let src = "on: 42\njobs: []\n";
        let diags = lint_source(&validator(), Path::new("wf.yml"), src).unwrap();
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
        let diags = lint_source(&validator(), Path::new("wf.yml"), src).unwrap();
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
        let diags = lint_source(&validator(), Path::new("wf.yml"), src).unwrap();
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
        let diags = lint_source(&validator(), Path::new("wf.yml"), src).unwrap();
        assert!(diags.iter().any(|d| d.message.contains("runs-on")));
        assert!(diags.iter().any(|d| d.message.contains("unknown context `bogus`")));
    }
}

