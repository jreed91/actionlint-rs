//! SARIF 2.1.0 output, for GitHub code-scanning inline PR annotations.
//!
//! GitHub renders SARIF uploaded via `github/codeql-action/upload-sarif`. Required shape
//! (verified against GitHub's SARIF-support docs, 2026-09-18):
//! - top level: `$schema`, `version` = "2.1.0", `runs[]`
//! - `runs[].tool.driver`: `name`, `rules[]` (each: `id`, `shortDescription.text`,
//!   `fullDescription.text`, `help.text`)
//! - `results[]`: `message.text`, `locations[]`, `partialFingerprints`; `ruleId` links to a
//!   rule; `level` in {error, warning, note}
//! - `physicalLocation`: `artifactLocation.uri` (relative to repo root) + `region`
//!   (`startLine`, `startColumn`, `endLine`, `endColumn`)

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::diagnostic::Diagnostic;

const TOOL_NAME: &str = "actionlint-rs";
const INFO_URI: &str = "https://github.com/jreed91/actionlint-rs";

/// Serialize diagnostics to a pretty-printed SARIF 2.1.0 document.
///
/// All findings are structural schema violations, reported at `error` level. `ruleId`s come
/// from [`Diagnostic::rule_id`]; each distinct id becomes one entry in `tool.driver.rules`.
pub fn to_sarif(diagnostics: &[Diagnostic]) -> String {
    // Collect the distinct rules, in stable (sorted) order.
    let mut rule_index: BTreeMap<&str, usize> = BTreeMap::new();
    for d in diagnostics {
        let next = rule_index.len();
        rule_index.entry(d.rule_id.as_str()).or_insert(next);
    }
    // Reassign indices by sorted key order so output is deterministic.
    let ordered_rules: Vec<&str> = rule_index.keys().copied().collect();
    let rule_pos: BTreeMap<&str, usize> = ordered_rules
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();

    let rules: Vec<Value> = ordered_rules.iter().map(|id| rule_object(id)).collect();

    let results: Vec<Value> = diagnostics
        .iter()
        .map(|d| result_object(d, rule_pos[d.rule_id.as_str()]))
        .collect();

    let doc = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": TOOL_NAME,
                    "informationUri": INFO_URI,
                    "rules": rules,
                }
            },
            "results": results,
        }],
    });

    serde_json::to_string_pretty(&doc).expect("SARIF document is always serializable")
}

fn rule_object(id: &str) -> Value {
    // v1 has one description per rule id; keep short/full/help populated (all required).
    let desc = rule_description(id);
    json!({
        "id": id,
        "shortDescription": { "text": desc },
        "fullDescription": { "text": desc },
        "help": { "text": desc },
    })
}

fn rule_description(id: &str) -> &'static str {
    match id {
        "structure/required" => "A required key is missing.",
        "structure/type" => "A value has the wrong type.",
        "structure/additional-properties" => "An unexpected/unknown key is present.",
        "structure/enum" => "A value is not one of the allowed values.",
        "structure/pattern" => "A value does not match the required format.",
        "structure/one-of" => "A value does not match any allowed form.",
        _ => "A workflow structural schema violation.",
    }
}

fn result_object(d: &Diagnostic, rule_index: usize) -> Value {
    let end = d.end.unwrap_or(d.pos);
    let uri = normalize_uri(&d.file.to_string_lossy());
    json!({
        "ruleId": d.rule_id,
        "ruleIndex": rule_index,
        "level": "error",
        "message": { "text": d.message },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": { "uri": uri },
                "region": {
                    "startLine": d.pos.line,
                    "startColumn": d.pos.col,
                    "endLine": end.line,
                    "endColumn": end.col,
                }
            }
        }],
        // Stable across runs for the same finding location so GitHub can de-dupe.
        "partialFingerprints": {
            "actionlintRs/v1": fingerprint(d),
        },
    })
}

/// A relative URI for the artifact location. GitHub interprets relative paths against the
/// repo root; strip a leading `./` and leave other paths as-is.
fn normalize_uri(path: &str) -> String {
    path.strip_prefix("./").unwrap_or(path).to_string()
}

/// A stable fingerprint for a finding: rule + pointer + start line. Deliberately excludes
/// the message text so wording tweaks don't churn fingerprints.
fn fingerprint(d: &Diagnostic) -> String {
    format!("{}:{}:{}", d.rule_id, d.pointer, d.pos.line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Position;

    fn diag() -> Diagnostic {
        Diagnostic::new("wf.yml", Position::new(4, 4), "/jobs/b/runs-on", "boom")
            .with_end(Some(Position::new(4, 12)))
            .with_rule_id("structure/required")
    }

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("SARIF output must be valid JSON")
    }

    #[test]
    fn produces_valid_sarif_skeleton() {
        let v = parse(&to_sarif(&[diag()]));
        assert_eq!(v["version"], "2.1.0");
        assert!(v["$schema"].as_str().unwrap().contains("sarif-2.1.0"));
        assert_eq!(v["runs"][0]["tool"]["driver"]["name"], TOOL_NAME);
    }

    #[test]
    fn result_has_required_region_fields() {
        let v = parse(&to_sarif(&[diag()]));
        let region = &v["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"];
        assert_eq!(region["startLine"], 4);
        assert_eq!(region["startColumn"], 4);
        assert_eq!(region["endLine"], 4);
        assert_eq!(region["endColumn"], 12);
    }

    #[test]
    fn result_links_to_a_declared_rule() {
        let v = parse(&to_sarif(&[diag()]));
        let result = &v["runs"][0]["results"][0];
        assert_eq!(result["ruleId"], "structure/required");
        let idx = result["ruleIndex"].as_u64().unwrap() as usize;
        let rules = v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
        assert_eq!(rules[idx]["id"], "structure/required");
        // Rule has all GitHub-required description fields.
        assert!(rules[idx]["shortDescription"]["text"].is_string());
        assert!(rules[idx]["fullDescription"]["text"].is_string());
        assert!(rules[idx]["help"]["text"].is_string());
    }

    #[test]
    fn distinct_rule_ids_become_distinct_rules() {
        let d1 = diag();
        let d2 = Diagnostic::new("wf.yml", Position::new(1, 1), "/on", "bad on")
            .with_rule_id("structure/type");
        let v = parse(&to_sarif(&[d1, d2]));
        let rules = v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn missing_end_falls_back_to_start() {
        let d = Diagnostic::new("wf.yml", Position::new(2, 3), "/x", "m"); // no end
        let v = parse(&to_sarif(&[d]));
        let region = &v["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"];
        assert_eq!(region["startLine"], 2);
        assert_eq!(region["endLine"], 2);
        assert_eq!(region["endColumn"], 3);
    }

    #[test]
    fn uri_strips_leading_dot_slash() {
        let d = Diagnostic::new("./.github/workflows/ci.yml", Position::new(1, 1), "", "m");
        let v = parse(&to_sarif(&[d]));
        let uri = v["runs"][0]["results"][0]["locations"][0]["physicalLocation"]
            ["artifactLocation"]["uri"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(uri, ".github/workflows/ci.yml");
    }

    #[test]
    fn empty_diagnostics_still_valid_sarif() {
        let v = parse(&to_sarif(&[]));
        assert_eq!(v["version"], "2.1.0");
        assert_eq!(v["runs"][0]["results"].as_array().unwrap().len(), 0);
        assert_eq!(v["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn fingerprint_ignores_message_text() {
        let a = Diagnostic::new("f", Position::new(1, 1), "/x", "message A")
            .with_rule_id("structure/type");
        let b = Diagnostic::new("f", Position::new(1, 1), "/x", "message B DIFFERENT")
            .with_rule_id("structure/type");
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }
}
