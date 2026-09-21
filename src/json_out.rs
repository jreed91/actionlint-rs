//! Plain JSON output (`--format json`).
//!
//! A lighter, tool-agnostic alternative to SARIF: a flat array of diagnostics, one object
//! each, easy to pipe into `jq` or consume from a script. Unlike SARIF (which targets
//! GitHub code-scanning and carries a rule catalog + fingerprints), this is just the
//! findings, with every field a [`Diagnostic`] carries.
//!
//! Shape (stable, documented for consumers):
//! ```json
//! [
//!   {
//!     "file": ".github/workflows/ci.yml",
//!     "line": 4,
//!     "column": 4,
//!     "end_line": 4,
//!     "end_column": 12,
//!     "rule": "structure/required",
//!     "pointer": "/jobs/build/runs-on",
//!     "message": "`build` is missing required key `runs-on`"
//!   }
//! ]
//! ```
//! `end_line`/`end_column` are `null` when the span end is unknown. A clean run emits `[]`.

use serde_json::{json, Value};

use crate::diagnostic::Diagnostic;

/// Serialize diagnostics to a pretty-printed JSON array.
pub fn to_json(diagnostics: &[Diagnostic]) -> String {
    let items: Vec<Value> = diagnostics.iter().map(diagnostic_object).collect();
    serde_json::to_string_pretty(&Value::Array(items))
        .expect("JSON diagnostics are always serializable")
}

fn diagnostic_object(d: &Diagnostic) -> Value {
    json!({
        "file": d.file.to_string_lossy(),
        "line": d.pos.line,
        "column": d.pos.col,
        "end_line": d.end.map(|e| e.line),
        "end_column": d.end.map(|e| e.col),
        "rule": d.rule_id,
        "pointer": d.pointer,
        "message": d.message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Position;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("JSON output must be valid JSON")
    }

    #[test]
    fn empty_diagnostics_is_empty_array() {
        let v = parse(&to_json(&[]));
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[test]
    fn emits_all_fields() {
        let d = Diagnostic::new("wf.yml", Position::new(4, 4), "/jobs/b/runs-on", "boom")
            .with_end(Some(Position::new(4, 12)))
            .with_rule_id("structure/required");
        let v = parse(&to_json(&[d]));
        let o = &v[0];
        assert_eq!(o["file"], "wf.yml");
        assert_eq!(o["line"], 4);
        assert_eq!(o["column"], 4);
        assert_eq!(o["end_line"], 4);
        assert_eq!(o["end_column"], 12);
        assert_eq!(o["rule"], "structure/required");
        assert_eq!(o["pointer"], "/jobs/b/runs-on");
        assert_eq!(o["message"], "boom");
    }

    #[test]
    fn missing_end_is_null() {
        let d = Diagnostic::new("wf.yml", Position::new(2, 3), "/x", "m");
        let v = parse(&to_json(&[d]));
        assert!(v[0]["end_line"].is_null());
        assert!(v[0]["end_column"].is_null());
    }

    #[test]
    fn preserves_order_and_count() {
        let a = Diagnostic::new("f", Position::new(1, 1), "/a", "first");
        let b = Diagnostic::new("f", Position::new(2, 1), "/b", "second");
        let v = parse(&to_json(&[a, b]));
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["message"], "first");
        assert_eq!(arr[1]["message"], "second");
    }
}
