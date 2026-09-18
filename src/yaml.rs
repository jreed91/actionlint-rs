//! Parse workflow YAML once into a span-carrying tree, and derive the JSON value the
//! validator consumes from that same tree.
//!
//! Architecture A (ADR-0002): rather than two independent parses, we parse once with
//! saphyr into `MarkedYaml` (every node carries a `Span`) and convert that tree into a
//! `serde_json::Value`. Deriving both views from one parse guarantees they agree
//! structurally, so a JSON Pointer from a validation error resolves against the span tree.

use anyhow::{anyhow, Result};
use saphyr::{LoadableYamlNode, MarkedYaml, Scalar, YamlData};

/// The parsed document: the span-carrying tree plus the JSON value derived from it.
pub struct ParsedWorkflow<'a> {
    /// saphyr's marked tree — the source of truth for positions.
    pub marked: MarkedYaml<'a>,
    /// The JSON projection fed to the schema validator.
    pub json: serde_json::Value,
}

/// Parse a single-document workflow YAML string.
///
/// Workflows are always a single YAML document; if the input contains more than one
/// document we lint the first and ignore the rest (documented v1 behavior).
pub fn parse(source: &str) -> Result<ParsedWorkflow<'_>> {
    let mut docs = MarkedYaml::load_from_str(source)
        .map_err(|e| anyhow!("YAML parse error: {e}"))?;

    if docs.is_empty() {
        return Err(anyhow!("empty YAML document"));
    }
    let marked = docs.swap_remove(0);
    let json = to_json(&marked);
    Ok(ParsedWorkflow { marked, json })
}

/// Convert a `MarkedYaml` node into a `serde_json::Value`, discarding spans.
///
/// Mapping keys are coerced to strings (JSON object keys are always strings; workflow
/// keys are always strings in practice). Non-string keys are rendered via their scalar
/// text so the JSON stays well-formed rather than silently dropping entries.
fn to_json(node: &MarkedYaml<'_>) -> serde_json::Value {
    use serde_json::Value;

    match &node.data {
        YamlData::Value(scalar) => scalar_to_json(scalar),
        YamlData::Sequence(items) => Value::Array(items.iter().map(to_json).collect()),
        YamlData::Mapping(map) => {
            let mut obj = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                obj.insert(key_to_string(k), to_json(v));
            }
            Value::Object(obj)
        }
        // A custom-tagged node — validate against its inner value for v1.
        YamlData::Tagged(_tag, inner) => to_json(inner),
        // Unreachable via the default loader (parses scalars to `Value`, resolves
        // aliases in place, errors rather than emitting `BadValue`). Defensive.
        _ => Value::Null,
    }
}

fn scalar_to_json(scalar: &Scalar<'_>) -> serde_json::Value {
    use serde_json::Value;
    match scalar {
        Scalar::Null => Value::Null,
        Scalar::Boolean(b) => Value::Bool(*b),
        Scalar::Integer(i) => Value::Number((*i).into()),
        Scalar::FloatingPoint(f) => serde_json::Number::from_f64(f.into_inner())
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Scalar::String(s) => Value::String(s.to_string()),
    }
}

/// Render a mapping key as a JSON object key string.
fn key_to_string(node: &MarkedYaml<'_>) -> String {
    match &node.data {
        YamlData::Value(Scalar::String(s)) => s.to_string(),
        YamlData::Value(Scalar::Boolean(b)) => b.to_string(),
        YamlData::Value(Scalar::Integer(i)) => i.to_string(),
        YamlData::Value(Scalar::Null) => "null".to_string(),
        YamlData::Value(Scalar::FloatingPoint(f)) => f.into_inner().to_string(),
        // Unreachable via the default loader (see `to_json`); non-scalar keys are invalid
        // in workflow YAML anyway. Defensive fallback.
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_workflow_to_json() {
        let src = "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n";
        let parsed = parse(src).unwrap();
        assert_eq!(parsed.json["on"], serde_json::json!("push"));
        assert_eq!(
            parsed.json["jobs"]["build"]["runs-on"],
            serde_json::json!("ubuntu-latest")
        );
    }

    #[test]
    fn preserves_scalar_types() {
        let src = "n: 3\nf: 1.5\nb: true\nnull_key:\n";
        let parsed = parse(src).unwrap();
        assert_eq!(parsed.json["n"], serde_json::json!(3));
        assert_eq!(parsed.json["f"], serde_json::json!(1.5));
        assert_eq!(parsed.json["b"], serde_json::json!(true));
        assert_eq!(parsed.json["null_key"], serde_json::Value::Null);
    }

    #[test]
    fn reports_parse_error_on_bad_yaml() {
        // An unterminated flow sequence is a genuine scan error (saphyr is otherwise
        // lenient — plain `:::` parses fine).
        let src = "on: [push, pull_request";
        assert!(parse(src).is_err());
    }

    #[test]
    fn empty_document_is_an_error() {
        // A comment-only / whitespace source yields no documents.
        assert!(parse("# just a comment\n").is_err());
    }

    #[test]
    fn tagged_scalar_uses_inner_value() {
        // A custom tag produces a `Tagged` node (a `!!str` core tag resolves directly to a
        // scalar); we validate against the inner value.
        let parsed = parse("v: !mytag hello\n").unwrap();
        assert_eq!(parsed.json["v"], serde_json::json!("hello"));
    }

    #[test]
    fn tagged_mapping_recurses_into_inner() {
        let parsed = parse("v: !mytag {a: 1}\n").unwrap();
        assert_eq!(parsed.json["v"]["a"], serde_json::json!(1));
    }

    #[test]
    fn non_string_keys_are_stringified() {
        // Integer, boolean, and null keys all become JSON string keys.
        let parsed = parse("1: one\ntrue: yes\nnull: n\n").unwrap();
        assert_eq!(parsed.json["1"], serde_json::json!("one"));
        assert_eq!(parsed.json["true"], serde_json::json!("yes"));
        assert_eq!(parsed.json["null"], serde_json::json!("n"));
    }

    #[test]
    fn float_key_is_stringified() {
        let parsed = parse("1.5: half\n").unwrap();
        assert_eq!(parsed.json["1.5"], serde_json::json!("half"));
    }

    #[test]
    fn sequence_of_scalars_becomes_json_array() {
        let parsed = parse("- 1\n- two\n- true\n").unwrap();
        assert_eq!(parsed.json, serde_json::json!([1, "two", true]));
    }

    #[test]
    fn only_first_document_is_used() {
        let parsed = parse("on: push\n---\non: pull_request\n").unwrap();
        assert_eq!(parsed.json["on"], serde_json::json!("push"));
    }
}
