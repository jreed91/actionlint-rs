//! Context availability by workflow position.
//!
//! GitHub restricts which contexts may appear at each position — e.g. `secrets` is not
//! available in `runs-on`, `steps.*` is only available at step level, `matrix` only in a
//! matrix job. GitHub's first-party DSL encodes this as a `context: [...]` annotation on each
//! positional definition (see `workflow-v1.0.json`), so we DERIVE the table from the DSL
//! rather than hand-coding it (consistent with the schema-driven thesis — it resyncs with the
//! DSL).
//!
//! Two pieces:
//! 1. [`AvailabilityTable::from_dsl`] extracts `definition-name -> allowed-context-set` from
//!    the DSL.
//! 2. [`position_for_pointer`] classifies a workflow JSON pointer (e.g. `/jobs/b/steps/0/if`)
//!    to the DSL definition name that governs it (`step-if`), so a caller can look up the
//!    allowed set for an expression's position.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// A map from DSL positional-definition name to the set of context roots allowed there.
#[derive(Debug, Clone, Default)]
pub struct AvailabilityTable {
    by_definition: BTreeMap<String, BTreeSet<String>>,
}

impl AvailabilityTable {
    /// Build the table from a parsed first-party DSL document by collecting every definition
    /// that carries a `context: [...]` annotation. Function-style entries (e.g.
    /// `always(0,0)`) are dropped — they are functions, not contexts.
    pub fn from_dsl(dsl: &Value) -> Self {
        let mut by_definition = BTreeMap::new();
        if let Some(defs) = dsl.get("definitions").and_then(|d| d.as_object()) {
            for (name, node) in defs {
                if let Some(ctx) = node.get("context").and_then(|c| c.as_array()) {
                    let set: BTreeSet<String> = ctx
                        .iter()
                        .filter_map(|v| v.as_str())
                        .filter(|s| !s.contains('(')) // drop functions
                        .map(|s| s.to_string())
                        .collect();
                    by_definition.insert(name.clone(), set);
                }
            }
        }
        AvailabilityTable { by_definition }
    }

    /// The allowed context set for a DSL definition name, if the DSL constrains it.
    pub fn allowed_for_definition(&self, definition: &str) -> Option<&BTreeSet<String>> {
        self.by_definition.get(definition)
    }

    /// The allowed context set for a workflow JSON pointer, if its position is one we
    /// classify and the DSL constrains it.
    pub fn allowed_for_pointer(&self, pointer: &str) -> Option<&BTreeSet<String>> {
        let def = position_for_pointer(pointer)?;
        self.allowed_for_definition(def)
    }

    pub fn is_empty(&self) -> bool {
        self.by_definition.is_empty()
    }
}

/// Classify a workflow JSON pointer to the DSL positional-definition name that governs
/// expressions there, or `None` if we don't constrain that position (so the caller applies no
/// availability restriction — conservative).
///
/// Only the well-known positions where users actually write expressions are classified;
/// everything else returns `None` (no restriction), which never false-positives.
pub fn position_for_pointer(pointer: &str) -> Option<&'static str> {
    let segs: Vec<&str> = pointer.split('/').filter(|s| !s.is_empty()).collect();
    let last = *segs.last()?;
    let in_step = segs.iter().any(|s| *s == "steps");
    // Whether the pointer is under `/jobs/<id>/...` (job level) vs top-level workflow.
    let in_job = segs.first() == Some(&"jobs") && segs.len() >= 2;

    Some(match last {
        "if" => {
            if in_step {
                "step-if"
            } else {
                "job-if"
            }
        }
        "runs-on" => "runs-on",
        "timeout-minutes" if in_step => "step-timeout-minutes",
        "continue-on-error" if in_step => "step-continue-on-error",
        "name" if in_step => "step-name",
        // `run-name` is a top-level workflow key.
        "run-name" => "run-name",
        _ => {
            // Positions identified by an ancestor segment rather than the leaf:
            // env values: `.../env/<KEY>` (job vs step vs workflow level).
            if segs.len() >= 2 && segs[segs.len() - 2] == "env" {
                if in_step {
                    "step-env"
                } else if in_job {
                    "job-env"
                } else {
                    "workflow-env"
                }
            } else if in_step && segs.iter().any(|s| *s == "with") {
                // step `with:` input values.
                "step-with"
            } else {
                return None;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> AvailabilityTable {
        let dsl: Value =
            serde_json::from_str(include_str!("../../schemas/workflow-v1.0.json")).unwrap();
        AvailabilityTable::from_dsl(&dsl)
    }

    #[test]
    fn extracts_availability_from_dsl() {
        let t = table();
        assert!(!t.is_empty());
        let runs_on = t.allowed_for_definition("runs-on").unwrap();
        assert!(runs_on.contains("github"));
        assert!(runs_on.contains("matrix"));
        // secrets is NOT available in runs-on.
        assert!(!runs_on.contains("secrets"));
        // Functions are dropped, not stored as contexts.
        assert!(!runs_on.iter().any(|c| c.contains('(')));
    }

    #[test]
    fn classifies_common_positions() {
        assert_eq!(position_for_pointer("/jobs/b/if"), Some("job-if"));
        assert_eq!(position_for_pointer("/jobs/b/steps/0/if"), Some("step-if"));
        assert_eq!(position_for_pointer("/jobs/b/runs-on"), Some("runs-on"));
        assert_eq!(
            position_for_pointer("/jobs/b/steps/0/timeout-minutes"),
            Some("step-timeout-minutes")
        );
        assert_eq!(
            position_for_pointer("/jobs/b/steps/0/continue-on-error"),
            Some("step-continue-on-error")
        );
        assert_eq!(position_for_pointer("/run-name"), Some("run-name"));
    }

    #[test]
    fn classifies_env_by_level() {
        assert_eq!(position_for_pointer("/env/MY_VAR"), Some("workflow-env"));
        assert_eq!(position_for_pointer("/jobs/b/env/MY_VAR"), Some("job-env"));
        assert_eq!(
            position_for_pointer("/jobs/b/steps/0/env/MY_VAR"),
            Some("step-env")
        );
    }

    #[test]
    fn classifies_step_with() {
        assert_eq!(
            position_for_pointer("/jobs/b/steps/0/with/token"),
            Some("step-with")
        );
    }

    #[test]
    fn unclassified_positions_return_none() {
        // A position we don't restrict — no availability applied (conservative).
        assert_eq!(position_for_pointer("/jobs/b/steps/0/run"), None);
        assert_eq!(position_for_pointer(""), None);
    }

    #[test]
    fn allowed_for_pointer_end_to_end() {
        let t = table();
        // runs-on disallows secrets.
        let runs_on = t.allowed_for_pointer("/jobs/b/runs-on").unwrap();
        assert!(!runs_on.contains("secrets"));
        // step-env allows secrets.
        let step_env = t.allowed_for_pointer("/jobs/b/steps/0/env/X").unwrap();
        assert!(step_env.contains("secrets"));
        // an unclassified position has no restriction.
        assert!(t.allowed_for_pointer("/jobs/b/steps/0/run").is_none());
    }
}
