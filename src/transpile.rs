//! Transpile GitHub's first-party workflow DSL (`actions/languageservices`
//! `workflow-parser/src/workflow-v1.0.json`) into a JSON Schema (draft-07) that our
//! existing `jsonschema` validator can consume.
//!
//! This is the option-B "north star" (ADR-0001): validation sourced from GitHub's own
//! first-party structural model instead of the community SchemaStore schema. GitHub's model
//! is a custom DSL, not JSON Schema, so it cannot be fed to a validator directly — this
//! module bridges that gap.
//!
//! ## The DSL, and how each construct maps
//!
//! A DSL document is `{ "version", "definitions": { <name>: <node> } }`. Every node is one
//! of:
//! - `null` — a primitive whose meaning comes from its *name* (`string`/`number`/
//!   `boolean`/`null`). Non-primitive `null` bodies are treated as "any".
//! - a bare string `"foo"` — a reference to definition `foo` → `{"$ref": "#/definitions/foo"}`.
//! - `{ "string": <opts> }` → `{"type": "string"}`, plus:
//!     - `constant: X` → `{"const": X}`
//!     - `allowed-values: [...]` → `{"enum": [...]}`
//!     - `require-non-empty: true` → `minLength: 1`
//! - `{ "number": {} }` / `{ "boolean": {} }` / `{ "null": {} }` → the matching `type`.
//! - `{ "sequence": { "item-type": T } }` → `{"type":"array","items": <T>}`.
//! - `{ "mapping": { "properties": {...}, "loose-key-type": K, "loose-value-type": V } }`
//!     → `{"type":"object","properties":{...},"required":[...],"additionalProperties": <V or true>}`.
//!     A property value may be a bare string (ref), or `{ "type": T, "required": bool, ... }`.
//! - `{ "one-of": [a, b, ...] }` → `{"oneOf": [<a>, <b>, ...]}`.
//! - `{ "type": T }` (as a value) → `<T>` (a ref or inline).
//! - `context: [...]` — an expression-context annotation. It carries no *structural* shape,
//!   but its presence means GitHub allows a `${{ }}` expression in that position, so a node
//!   carrying `context` is widened to `anyOf[<base>, <expression string>]` (see
//!   `allow_expression`). A node that is *only* a context annotation with `string: {}`
//!   becomes `anyOf[string, expression]`.

use std::collections::BTreeSet;

use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

/// Transpile a DSL document (parsed `workflow-v1.0.json`) into a draft-07 JSON Schema whose
/// root validates a workflow file.
///
/// The result references `#/definitions/workflow-root` as its top-level shape. Unsupported
/// constructs are collected and returned so callers can surface them rather than silently
/// dropping fidelity.
pub fn transpile(dsl: &Value) -> Result<Transpiled> {
    let defs = dsl
        .get("definitions")
        .and_then(|d| d.as_object())
        .ok_or_else(|| anyhow!("DSL has no `definitions` object"))?;

    let mut ctx = Ctx::default();
    let mut out_defs = Map::new();
    for (name, node) in defs {
        let schema = ctx.node_to_schema(name, node);
        out_defs.insert(name.clone(), schema);
    }

    // Several primitives are implicit keywords in the DSL — referenced by name (as a `type`,
    // `item-type`, `loose-value-type`, one-of branch, ...) but never declared under
    // `definitions`. Inject them so our `$ref`s resolve. Only add ones the DSL didn't define.
    for (prim, schema) in [
        ("string", json!({ "type": "string" })),
        // number/boolean self-widen: GitHub accepts a `${{ }}` expression anywhere a scalar
        // is expected (ADR-0005), so these definitions accept the literal type OR an
        // expression string.
        ("number", number_schema()),
        ("boolean", boolean_schema()),
        ("null", json!({ "type": "null" })),
        ("mapping", json!({ "type": "object" })),
        // A sequence position can also hold a `${{ }}` expression that resolves to a list at
        // runtime (e.g. a matrix variable `x: ${{ fromJSON(...) }}`), so `sequence` widens too.
        ("sequence", json!({ "anyOf": [{ "type": "array" }, expression_schema()] })),
        // `any` matches any value.
        ("any", json!({})),
    ] {
        out_defs.entry(prim.to_string()).or_insert(schema);
    }

    if !defs.contains_key("workflow-root") {
        return Err(anyhow!("DSL is missing the `workflow-root` definition"));
    }

    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$id": "https://github.com/actions/languageservices/workflow-v1.0.transpiled.json",
        "$ref": "#/definitions/workflow-root",
        "definitions": Value::Object(out_defs),
    });

    Ok(Transpiled {
        schema,
        unsupported: ctx.unsupported.into_iter().collect(),
    })
}

/// The transpiled schema plus a record of any DSL constructs we could not fully translate.
pub struct Transpiled {
    pub schema: Value,
    /// Distinct unsupported-construct descriptions, for honest reporting (should be empty
    /// for the current DSL).
    pub unsupported: Vec<String>,
}

#[derive(Default)]
struct Ctx {
    unsupported: BTreeSet<String>,
}

impl Ctx {
    /// Convert one DSL node into a JSON Schema fragment. `name` is only used for the
    /// primitive-by-name convention (`string`/`number`/`boolean`/`null` with a null body).
    fn node_to_schema(&mut self, name: &str, node: &Value) -> Value {
        match node {
            // A bare-null body: primitive-by-name, else "any".
            Value::Null => match name {
                "string" => json!({ "type": "string" }),
                "number" => number_schema(),
                "boolean" => boolean_schema(),
                "null" => json!({ "type": "null" }),
                _ => json!({}),
            },
            // A bare string is a reference to another definition.
            Value::String(reference) => ref_to(reference),
            Value::Object(map) => self.object_node(map),
            other => {
                self.unsupported
                    .insert(format!("unexpected node shape: {other}"));
                json!({})
            }
        }
    }

    /// Handle an object-shaped node: transpile its structural keyword, then — if the node
    /// carries a `context` annotation — widen it to also accept a `${{ }}` expression.
    ///
    /// A `context: [...]` annotation on a DSL node means GitHub allows an expression in that
    /// position instead of the literal value (e.g. `continue-on-error: ${{ matrix.x }}`,
    /// `timeout-minutes: ${{ inputs.t }}`). We model that as `anyOf[<base>, <expression>]` so
    /// the transpiled first-party schema stops rejecting valid expression usage.
    fn object_node(&mut self, map: &Map<String, Value>) -> Value {
        let base = self.base_object_node(map);
        // Scalars (boolean/number) already self-widen to accept expressions everywhere (see
        // `boolean_schema`/`number_schema`), and a string trivially matches an expression —
        // so only wrap non-scalar bases (one-of / mapping) when the node carries `context`.
        if map.contains_key("context") && !already_accepts_expression(map) {
            allow_expression(base)
        } else {
            base
        }
    }

    /// Transpile an object node by its structural keyword, ignoring any `context` annotation.
    fn base_object_node(&mut self, map: &Map<String, Value>) -> Value {
        if let Some(schema) = self.string_node(map) {
            return schema;
        }
        if map.contains_key("number") {
            return number_schema();
        }
        if map.contains_key("boolean") {
            return boolean_schema();
        }
        if map.contains_key("null") {
            return json!({ "type": "null" });
        }
        if let Some(seq) = map.get("sequence") {
            return self.sequence_node(seq);
        }
        if let Some(mapping) = map.get("mapping") {
            return self.mapping_node(mapping);
        }
        if let Some(one_of) = map.get("one-of") {
            return self.one_of_node(one_of);
        }
        // `{ "type": T, ... }` as a value — resolve to a ref/inline for T.
        if let Some(t) = map.get("type") {
            return self.type_value(t);
        }
        // Only a description and/or context, nothing structural → accept anything.
        json!({})
    }

    /// `{ "string": { constant | allowed-values | require-non-empty } }`.
    fn string_node(&mut self, map: &Map<String, Value>) -> Option<Value> {
        let opts = map.get("string")?;
        let mut schema = Map::new();
        schema.insert("type".into(), json!("string"));
        if let Some(obj) = opts.as_object() {
            if let Some(c) = obj.get("constant") {
                // A constant fully determines the value.
                return Some(json!({ "const": c }));
            }
            if let Some(vals) = obj.get("allowed-values").and_then(|v| v.as_array()) {
                return Some(json!({ "enum": vals }));
            }
            if obj.get("require-non-empty").and_then(|v| v.as_bool()) == Some(true) {
                schema.insert("minLength".into(), json!(1));
            }
        }
        Some(Value::Object(schema))
    }

    fn sequence_node(&mut self, seq: &Value) -> Value {
        let items = seq
            .get("item-type")
            .map(|t| self.type_value(t))
            .unwrap_or_else(|| json!({}));
        json!({ "type": "array", "items": items })
    }

    fn mapping_node(&mut self, mapping: &Value) -> Value {
        let Some(m) = mapping.as_object() else {
            return json!({ "type": "object" });
        };
        let mut props = Map::new();
        let mut required: Vec<Value> = Vec::new();

        if let Some(properties) = m.get("properties").and_then(|p| p.as_object()) {
            for (key, val) in properties {
                let (schema, is_required) = self.property_value(val);
                if is_required {
                    required.push(json!(key));
                }
                props.insert(key.clone(), schema);
            }
        }

        let mut obj = Map::new();
        obj.insert("type".into(), json!("object"));
        if !props.is_empty() {
            obj.insert("properties".into(), Value::Object(props));
        }
        if !required.is_empty() {
            obj.insert("required".into(), Value::Array(required));
        }

        // loose-value-type controls additionalProperties. Absent + closed properties means
        // the DSL still allows unknown keys unless it is a *strict* mapping; the base DSL
        // uses loose-key/value-type to model open maps (e.g. `jobs`, `env`). When neither is
        // present we default to permissive (additionalProperties: true) to match GitHub,
        // which does not reject unknown top-level keys the way SchemaStore does.
        match m.get("loose-value-type") {
            Some(v) => {
                obj.insert("additionalProperties".into(), self.type_value(v));
            }
            None => {
                obj.insert("additionalProperties".into(), json!(true));
            }
        }
        Value::Object(obj)
    }

    fn one_of_node(&mut self, one_of: &Value) -> Value {
        let branches: Vec<Value> = one_of
            .as_array()
            .map(|arr| arr.iter().map(|b| self.type_value(b)).collect())
            .unwrap_or_default();
        json!({ "oneOf": branches })
    }

    /// A mapping property value: either a bare ref string, or `{ type, required, ... }`.
    /// Returns the schema and whether the property is required.
    fn property_value(&mut self, val: &Value) -> (Value, bool) {
        match val {
            Value::String(reference) => (ref_to(reference), false),
            Value::Object(obj) => {
                let required = obj.get("required").and_then(|r| r.as_bool()).unwrap_or(false);
                // A property object usually names its type via `type`; otherwise it may be an
                // inline node (mapping/one-of/etc.).
                let schema = if let Some(t) = obj.get("type") {
                    self.type_value(t)
                } else {
                    self.object_node(obj)
                };
                (schema, required)
            }
            other => {
                self.unsupported
                    .insert(format!("unexpected property value: {other}"));
                (json!({}), false)
            }
        }
    }

    /// A `type` value or `item-type`: a bare string is a ref; an object is an inline node.
    fn type_value(&mut self, t: &Value) -> Value {
        match t {
            Value::String(reference) => ref_to(reference),
            Value::Object(map) => self.object_node(map),
            Value::Null => json!({}),
            other => {
                self.unsupported
                    .insert(format!("unexpected type value: {other}"));
                json!({})
            }
        }
    }
}

/// A JSON-Schema `$ref` to a definition by name. Primitive names resolve through their
/// (also-emitted) definitions, so a single mechanism covers everything.
fn ref_to(name: &str) -> Value {
    json!({ "$ref": format!("#/definitions/{name}") })
}

/// A JSON-Schema fragment matching a whole-value `${{ ... }}` expression string. Mirrors
/// SchemaStore's `expressionSyntax` pattern so both schemas treat expressions the same way.
fn expression_schema() -> Value {
    json!({ "type": "string", "pattern": r"^\$\{\{(.|[\r\n])*\}\}$" })
}

/// A boolean value OR a `${{ }}` expression. GitHub accepts an expression anywhere a boolean
/// is expected (`continue-on-error: ${{ ... }}`, `cancel-in-progress: ${{ ... }}`, ...), so
/// the transpiled schema must too (ADR-0005).
fn boolean_schema() -> Value {
    json!({ "anyOf": [{ "type": "boolean" }, expression_schema()] })
}

/// A number value OR a `${{ }}` expression (e.g. `timeout-minutes: ${{ inputs.t }}`).
fn number_schema() -> Value {
    json!({ "anyOf": [{ "type": "number" }, expression_schema()] })
}

/// Whether a DSL node's base schema already accepts an expression (a scalar node), so the
/// node-level context widening would be redundant.
fn already_accepts_expression(map: &Map<String, Value>) -> bool {
    map.contains_key("string") || map.contains_key("number") || map.contains_key("boolean")
}

/// Widen a schema to also accept a `${{ }}` expression string, i.e. "base OR expression".
///
/// We use `anyOf`, not `oneOf`: an expression like `${{ matrix.os }}` is also a valid
/// non-empty string, so it would match *two* branches of a `oneOf` (the string branch and
/// the expression branch) and be rejected as ambiguous (`oneOfMultipleValid`). `anyOf`
/// accepts a value that matches one or more branches, which is what we want here.
fn allow_expression(base: Value) -> Value {
    json!({ "anyOf": [base, expression_schema()] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_by_name() {
        let mut c = Ctx::default();
        assert_eq!(c.node_to_schema("string", &Value::Null), json!({"type":"string"}));
        // number/boolean self-widen to accept `${{ }}` expressions (ADR-0005).
        assert_eq!(c.node_to_schema("number", &Value::Null), number_schema());
        assert_eq!(c.node_to_schema("boolean", &Value::Null), boolean_schema());
        assert_eq!(c.node_to_schema("null", &Value::Null), json!({"type":"null"}));
        // Unknown null body → any.
        assert_eq!(c.node_to_schema("mystery", &Value::Null), json!({}));
    }

    #[test]
    fn bare_string_becomes_ref() {
        let mut c = Ctx::default();
        assert_eq!(
            c.node_to_schema("x", &json!("job-id")),
            json!({"$ref":"#/definitions/job-id"})
        );
    }

    #[test]
    fn string_constant_becomes_const() {
        let mut c = Ctx::default();
        let n = json!({ "string": { "constant": "read" } });
        assert_eq!(c.node_to_schema("permission-level-read", &n), json!({"const":"read"}));
    }

    #[test]
    fn string_allowed_values_becomes_enum() {
        let mut c = Ctx::default();
        let n = json!({ "string": { "allowed-values": ["a", "b"] } });
        assert_eq!(c.node_to_schema("x", &n), json!({"enum":["a","b"]}));
    }

    #[test]
    fn require_non_empty_becomes_min_length() {
        let mut c = Ctx::default();
        let n = json!({ "string": { "require-non-empty": true } });
        assert_eq!(c.node_to_schema("x", &n), json!({"type":"string","minLength":1}));
    }

    #[test]
    fn sequence_becomes_array_with_items_ref() {
        let mut c = Ctx::default();
        let n = json!({ "sequence": { "item-type": "string" } });
        assert_eq!(
            c.node_to_schema("x", &n),
            json!({"type":"array","items":{"$ref":"#/definitions/string"}})
        );
    }

    #[test]
    fn mapping_with_required_property() {
        let mut c = Ctx::default();
        let n = json!({
            "mapping": {
                "properties": {
                    "runs-on": { "type": "runs-on", "required": true },
                    "name": "string"
                }
            }
        });
        let s = c.node_to_schema("job", &n);
        assert_eq!(s["type"], "object");
        assert_eq!(s["properties"]["runs-on"], json!({"$ref":"#/definitions/runs-on"}));
        assert_eq!(s["properties"]["name"], json!({"$ref":"#/definitions/string"}));
        assert_eq!(s["required"], json!(["runs-on"]));
    }

    #[test]
    fn mapping_loose_value_type_becomes_additional_properties() {
        let mut c = Ctx::default();
        let n = json!({
            "mapping": { "loose-key-type": "job-id", "loose-value-type": "job" }
        });
        let s = c.node_to_schema("jobs", &n);
        assert_eq!(s["additionalProperties"], json!({"$ref":"#/definitions/job"}));
    }

    #[test]
    fn one_of_becomes_one_of() {
        let mut c = Ctx::default();
        let n = json!({ "one-of": ["a", "b"] });
        assert_eq!(
            c.node_to_schema("x", &n),
            json!({"oneOf":[{"$ref":"#/definitions/a"},{"$ref":"#/definitions/b"}]})
        );
    }

    #[test]
    fn context_string_is_a_plain_string() {
        // A `string` base already matches an expression (an expression IS a string), so a
        // context annotation adds no extra wrapping — it stays a plain string.
        let mut c = Ctx::default();
        let n = json!({ "context": ["github", "inputs"], "string": {} });
        assert_eq!(c.node_to_schema("job-if", &n), json!({"type":"string"}));
    }

    #[test]
    fn boolean_self_widens_to_accept_expression_even_without_context() {
        // GitHub accepts an expression anywhere a boolean is expected, so `boolean` widens
        // globally (not just under a context annotation).
        let mut c = Ctx::default();
        let s = c.node_to_schema("x", &json!({ "boolean": {} }));
        assert_eq!(s, boolean_schema());
        assert_eq!(s["anyOf"][0], json!({"type":"boolean"}));
        assert!(s["anyOf"][1]["pattern"].is_string());
    }

    #[test]
    fn number_self_widens_to_accept_expression() {
        let mut c = Ctx::default();
        assert_eq!(c.node_to_schema("x", &json!({ "number": {} })), number_schema());
    }

    #[test]
    fn context_one_of_appends_expression_branch() {
        // A context-annotated one-of (like runs-on) becomes anyOf[<the oneOf>, expression].
        let mut c = Ctx::default();
        let n = json!({ "context": ["matrix"], "one-of": ["a", "b"] });
        let s = c.node_to_schema("runs-on", &n);
        assert!(s["anyOf"][0]["oneOf"].is_array(), "base oneOf preserved under anyOf");
        assert!(s["anyOf"][1]["pattern"].is_string(), "expression branch added");
    }

    #[test]
    fn context_mapping_appends_expression_branch() {
        // A context-annotated mapping (like strategy/concurrency) also accepts a whole-value
        // expression.
        let mut c = Ctx::default();
        let n = json!({ "context": ["matrix"], "mapping": { "properties": {} } });
        let s = c.node_to_schema("strategy", &n);
        assert_eq!(s["anyOf"][0]["type"], "object");
        assert!(s["anyOf"][1]["pattern"].is_string());
    }

    #[test]
    fn full_document_transpiles_and_has_root_ref() {
        let doc = json!({
            "version": "workflow-v1.0",
            "definitions": {
                "string": null,
                "workflow-root": {
                    "mapping": {
                        "properties": { "jobs": { "type": "jobs", "required": true } }
                    }
                },
                "jobs": { "mapping": { "loose-value-type": "string" } }
            }
        });
        let t = transpile(&doc).unwrap();
        assert_eq!(t.schema["$ref"], "#/definitions/workflow-root");
        assert!(t.schema["definitions"]["workflow-root"].is_object());
        assert!(t.unsupported.is_empty());
    }

    #[test]
    fn missing_definitions_is_an_error() {
        assert!(transpile(&json!({ "version": "x" })).is_err());
    }

    #[test]
    fn missing_workflow_root_is_an_error() {
        let doc = json!({ "definitions": { "string": null } });
        assert!(transpile(&doc).is_err());
    }

    #[test]
    fn type_value_node_resolves_to_ref() {
        // A node that is `{ "type": "X" }` (with a description) resolves to a ref to X.
        let mut c = Ctx::default();
        let n = json!({ "description": "d", "type": "runs-on" });
        assert_eq!(c.node_to_schema("x", &n), json!({"$ref":"#/definitions/runs-on"}));
    }

    #[test]
    fn inline_property_object_without_type() {
        // A property value that is an inline node (no `type` key) is transpiled in place.
        let mut c = Ctx::default();
        let n = json!({
            "mapping": { "properties": { "x": { "one-of": ["a", "b"] } } }
        });
        let s = c.node_to_schema("m", &n);
        assert_eq!(
            s["properties"]["x"],
            json!({"oneOf":[{"$ref":"#/definitions/a"},{"$ref":"#/definitions/b"}]})
        );
    }

    #[test]
    fn non_object_mapping_body_is_generic_object() {
        let mut c = Ctx::default();
        let n = json!({ "mapping": "weird" });
        assert_eq!(c.node_to_schema("m", &n), json!({"type":"object"}));
    }

    #[test]
    fn type_value_null_and_object_forms() {
        let mut c = Ctx::default();
        // item-type given as null → any.
        let seq = json!({ "sequence": { "item-type": null } });
        assert_eq!(c.node_to_schema("s", &seq), json!({"type":"array","items":{}}));
        // item-type given as an inline object node.
        let seq2 = json!({ "sequence": { "item-type": { "string": {} } } });
        assert_eq!(
            c.node_to_schema("s", &seq2),
            json!({"type":"array","items":{"type":"string"}})
        );
    }

    #[test]
    fn unsupported_node_shapes_are_recorded_not_dropped_silently() {
        let mut c = Ctx::default();
        // A node that is a bare number is not a valid DSL node shape.
        let _ = c.node_to_schema("x", &json!(42));
        // A property value that is a bare number.
        let _ = c.property_value(&json!(7));
        // A type value that is a bare number.
        let _ = c.type_value(&json!(9));
        assert_eq!(c.unsupported.len(), 3, "each odd shape should be recorded once");
    }

    #[test]
    fn transpiled_definitions_include_injected_primitives() {
        let doc = json!({
            "definitions": {
                "workflow-root": { "mapping": { "properties": {} } }
            }
        });
        let t = transpile(&doc).unwrap();
        for prim in ["string", "number", "boolean", "null", "mapping", "sequence", "any"] {
            assert!(
                t.schema["definitions"][prim].is_object(),
                "primitive `{prim}` should be injected"
            );
        }
    }
}
