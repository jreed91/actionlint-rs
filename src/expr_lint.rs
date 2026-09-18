//! Lint the `${{ }}` expressions embedded in a workflow's string values.
//!
//! This is the wiring that turns the expression engine (`crate::expr`) into real diagnostics
//! (ADR-0006 step 3). It walks the parsed workflow JSON, finds every string value, extracts
//! each `${{ ... }}` expression, parses + type-checks it, and emits a [`Diagnostic`] anchored
//! at the containing value node (resolved through the span tree by JSON pointer).
//!
//! Position note: a diagnostic is anchored at the START of the containing scalar (reliable
//! from saphyr's span tree); the offending expression text is included in the message.
//! Precise column-within-the-string is a documented future refinement — mapping an offset
//! through block/folded/quoted scalars is fragile with start-only spans.

use saphyr::MarkedYaml;
use std::sync::OnceLock;

use serde_json::Value;

use crate::diagnostic::{Diagnostic, Position};
use crate::expr::{self, AvailabilityTable};
use crate::schema::FIRST_PARTY_DSL_JSON;
use crate::span;

/// The context-availability table, derived once from the embedded first-party DSL.
fn availability() -> &'static AvailabilityTable {
    static TABLE: OnceLock<AvailabilityTable> = OnceLock::new();
    TABLE.get_or_init(|| match serde_json::from_str::<Value>(FIRST_PARTY_DSL_JSON) {
        Ok(dsl) => AvailabilityTable::from_dsl(&dsl),
        // A bad embedded DSL is caught elsewhere (schema tests); degrade to no availability
        // restriction rather than panic.
        Err(_) => AvailabilityTable::default(),
    })
}

/// Produce expression diagnostics for a parsed workflow.
///
/// `json` is the workflow value; `marked` is the span tree for the same document (both come
/// from `crate::yaml::parse`). `path` is the file label for diagnostics.
pub fn lint_expressions(
    json: &Value,
    marked: &MarkedYaml<'_>,
    path: &std::path::Path,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut pointer = String::new();
    walk(json, &mut pointer, marked, path, &mut out);
    out
}

/// Recursively walk the JSON value, tracking the current JSON pointer, and check any string.
fn walk(
    value: &Value,
    pointer: &mut String,
    marked: &MarkedYaml<'_>,
    path: &std::path::Path,
    out: &mut Vec<Diagnostic>,
) {
    match value {
        Value::String(s) => check_string(s, pointer, marked, path, out),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let len = pointer.len();
                pointer.push('/');
                pointer.push_str(&i.to_string());
                walk(item, pointer, marked, path, out);
                pointer.truncate(len);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                let len = pointer.len();
                pointer.push('/');
                pointer.push_str(&escape_token(k));
                walk(v, pointer, marked, path, out);
                pointer.truncate(len);
            }
        }
        _ => {}
    }
}

/// Check every `${{ }}` expression in one string value.
fn check_string(
    s: &str,
    pointer: &str,
    marked: &MarkedYaml<'_>,
    path: &std::path::Path,
    out: &mut Vec<Diagnostic>,
) {
    let exprs = extract_expressions(s);
    if exprs.is_empty() {
        return;
    }
    // Resolve the containing scalar's position once; all its expressions anchor there.
    let pos = span::position_for_pointer(marked, pointer).unwrap_or_else(|| Position::new(1, 1));

    // If this position constrains context availability, check against the allowed set.
    let allowed = availability().allowed_for_pointer(pointer);

    for src in exprs {
        // Parse errors and type errors are both reported; a parse error precludes checking.
        match expr::parse(&src) {
            Ok(ast) => {
                let errs = match allowed {
                    Some(set) => expr::check_with_availability(&ast, set),
                    None => expr::check(&ast),
                };
                for err in errs {
                    out.push(
                        Diagnostic::new(
                            path.to_path_buf(),
                            pos,
                            pointer.to_string(),
                            format!("in `${{{{ {} }}}}`: {}", src.trim(), err.message),
                        )
                        .with_rule_id("expression/type"),
                    );
                }
            }
            Err(e) => {
                out.push(
                    Diagnostic::new(
                        path.to_path_buf(),
                        pos,
                        pointer.to_string(),
                        format!("in `${{{{ {} }}}}`: syntax error: {}", src.trim(), e.message),
                    )
                    .with_rule_id("expression/syntax"),
                );
            }
        }
    }
}

/// Extract the inner text of each `${{ ... }}` occurrence in `s` (without the delimiters).
/// Nested `}}` inside strings is not a concern for GitHub's grammar; we take the shortest
/// `}}` after each `${{`.
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
            None => break, // unterminated `${{` — leave to structural checks
        }
    }
    out
}

/// Escape a JSON-pointer reference token (RFC 6901): `~` -> `~0`, `/` -> `~1`.
fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml;
    use std::path::Path;

    fn lint(src: &str) -> Vec<String> {
        let parsed = yaml::parse(src).unwrap();
        lint_expressions(&parsed.json, &parsed.marked, Path::new("wf.yml"))
            .into_iter()
            .map(|d| d.message)
            .collect()
    }

    #[test]
    fn extract_handles_multiple_and_surrounding_text() {
        assert_eq!(
            extract_expressions("a-${{ x }}-${{ y }}-b"),
            vec![" x ".to_string(), " y ".to_string()]
        );
        assert!(extract_expressions("no expressions here").is_empty());
        assert!(extract_expressions("unterminated ${{ x").is_empty());
    }

    #[test]
    fn valid_expressions_produce_no_diagnostics() {
        let src = "\
on: push
jobs:
  b:
    runs-on: ubuntu-latest
    if: ${{ github.event_name == 'push' }}
    steps:
      - run: echo ${{ runner.os }}
";
        assert!(lint(src).is_empty(), "got: {:?}", lint(src));
    }

    #[test]
    fn unknown_context_in_if_is_flagged() {
        let src = "\
on: push
jobs:
  b:
    runs-on: ubuntu-latest
    if: ${{ gihtub.event_name == 'push' }}
    steps:
      - run: echo hi
";
        let msgs = lint(src);
        assert_eq!(msgs.len(), 1, "got: {:?}", msgs);
        assert!(msgs[0].contains("unknown context `gihtub`"), "got: {:?}", msgs);
        assert!(msgs[0].contains("${{"), "message should quote the expression: {:?}", msgs);
    }

    #[test]
    fn unknown_property_in_run_is_flagged() {
        let src = "\
on: push
jobs:
  b:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ github.nonsense }}
";
        let msgs = lint(src);
        assert_eq!(msgs.len(), 1, "got: {:?}", msgs);
        assert!(msgs[0].contains("not a valid property"), "got: {:?}", msgs);
    }

    #[test]
    fn syntax_error_in_expression_is_flagged() {
        let src = "\
on: push
jobs:
  b:
    runs-on: ubuntu-latest
    if: ${{ github. }}
    steps:
      - run: echo hi
";
        let msgs = lint(src);
        assert_eq!(msgs.len(), 1, "got: {:?}", msgs);
        assert!(msgs[0].contains("syntax error"), "got: {:?}", msgs);
    }

    #[test]
    fn multiple_expressions_in_one_string_each_checked() {
        let src = "\
on: push
jobs:
  b:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ bad1.x }} and ${{ bad2.y }}
";
        assert_eq!(lint(src).len(), 2);
    }

    #[test]
    fn escape_token_escapes_pointer_specials() {
        assert_eq!(escape_token("a/b"), "a~1b");
        assert_eq!(escape_token("a~b"), "a~0b");
        assert_eq!(escape_token("plain"), "plain");
    }

    #[test]
    fn key_with_slash_still_resolves_and_checks() {
        // A mapping key containing `/` exercises escape_token in the walk.
        let src = "\
on: push
env:
  \"a/b\": ${{ nope.x }}
jobs:
  j:
    runs-on: ubuntu-latest
    steps: [{run: hi}]
";
        assert_eq!(lint(src).len(), 1, "got: {:?}", lint(src));
    }

    #[test]
    fn unterminated_expression_is_ignored_here() {
        // `${{` with no closing `}}` yields no expression diagnostics (left to other checks).
        let src = "\
on: push
jobs:
  j:
    runs-on: ubuntu-latest
    steps:
      - run: echo ${{ github.sha
";
        assert!(lint(src).is_empty(), "got: {:?}", lint(src));
    }

    #[test]
    fn expressions_in_array_values_are_checked() {
        let src = "\
on: push
jobs:
  b:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
        env:
          LIST: ${{ nope.x }}
";
        assert_eq!(lint(src).len(), 1);
    }
}
