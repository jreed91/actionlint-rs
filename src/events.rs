//! Webhook event + activity-type validation for `on:`.
//!
//! The default (first-party DSL) schema loose-types `on:`, so it accepts unknown event names
//! (`on: pull-request` — a typo for `pull_request`) and invalid activity types
//! (`types: [opend]`). This pass tightens both:
//!
//! - **Event names** — validated against the known set, which is **derived from the vendored
//!   SchemaStore schema's `event` enum** (`schemas/github-workflow.json`), so it resyncs with
//!   the schema rather than rotting in a hardcoded list.
//! - **Activity types** — for the events that declare them, `types:` values are validated
//!   against a curated table (GitHub's activity types are stable, documented facts; they
//!   aren't cleanly addressable in either vendored schema, so this table is hand-maintained).
//!
//! Handles the three `on:` forms: a string (`on: push`), a sequence (`on: [push, pull_request]`),
//! and a mapping (`on: { pull_request: { types: [opened] } }`). Events carrying a `${{ }}`
//! expression can't occur (`on:` isn't expression-evaluated), so there's nothing to skip.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde_json::Value;

use crate::checks::CheckFinding;
use crate::schema::WORKFLOW_SCHEMA_JSON;

/// Curated activity types per event (GitHub's documented sets). Hand-maintained: these aren't
/// cleanly derivable from the vendored schemas. Events not listed here take no `types:`.
const ACTIVITY_TYPES: &[(&str, &[&str])] = &[
    ("branch_protection_rule", &["created", "edited", "deleted"]),
    ("check_run", &["created", "rerequested", "completed", "requested_action"]),
    ("check_suite", &["completed"]),
    ("discussion", &[
        "created", "edited", "deleted", "transferred", "pinned", "unpinned", "labeled",
        "unlabeled", "locked", "unlocked", "category_changed", "answered", "unanswered",
    ]),
    ("discussion_comment", &["created", "edited", "deleted"]),
    ("issue_comment", &["created", "edited", "deleted"]),
    ("issues", &[
        "opened", "edited", "deleted", "transferred", "pinned", "unpinned", "closed",
        "reopened", "assigned", "unassigned", "labeled", "unlabeled", "locked", "unlocked",
        "milestoned", "demilestoned", "typed", "untyped",
    ]),
    ("label", &["created", "edited", "deleted"]),
    ("merge_group", &["checks_requested"]),
    ("milestone", &["created", "closed", "opened", "edited", "deleted"]),
    ("project", &["created", "closed", "reopened", "edited", "deleted"]),
    ("project_card", &["created", "moved", "converted", "edited", "deleted"]),
    ("project_column", &["created", "updated", "moved", "deleted"]),
    ("pull_request", &[
        "assigned", "unassigned", "labeled", "unlabeled", "opened", "edited", "closed",
        "reopened", "synchronize", "converted_to_draft", "ready_for_review", "locked",
        "unlocked", "review_requested", "review_request_removed", "auto_merge_enabled",
        "auto_merge_disabled", "enqueued", "dequeued", "milestoned", "demilestoned",
    ]),
    ("pull_request_review", &["submitted", "edited", "dismissed"]),
    ("pull_request_review_comment", &["created", "edited", "deleted"]),
    ("pull_request_target", &[
        "assigned", "unassigned", "labeled", "unlabeled", "opened", "edited", "closed",
        "reopened", "synchronize", "converted_to_draft", "ready_for_review", "locked",
        "unlocked", "review_requested", "review_request_removed", "auto_merge_enabled",
        "auto_merge_disabled", "enqueued", "dequeued", "milestoned", "demilestoned",
    ]),
    ("registry_package", &["published", "updated"]),
    ("release", &[
        "published", "unpublished", "created", "edited", "deleted", "prereleased", "released",
    ]),
    ("watch", &["started"]),
    ("workflow_run", &["completed", "requested", "in_progress"]),
];

/// The known event-name set, derived once from the SchemaStore schema's `event` enum.
fn known_events() -> &'static BTreeSet<String> {
    static EVENTS: OnceLock<BTreeSet<String>> = OnceLock::new();
    EVENTS.get_or_init(derive_events)
}

fn derive_events() -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let Ok(schema) = serde_json::from_str::<Value>(WORKFLOW_SCHEMA_JSON) else {
        return set;
    };
    if let Some(enum_vals) = schema
        .get("definitions")
        .and_then(|d| d.get("event"))
        .and_then(|e| e.get("enum"))
        .and_then(|e| e.as_array())
    {
        for v in enum_vals {
            if let Some(s) = v.as_str() {
                set.insert(s.to_string());
            }
        }
    }
    // `schedule` and `workflow_dispatch`/`workflow_call` are events too; the enum includes the
    // dispatch/call ones. `schedule` is a special config key (cron) — always valid.
    set.insert("schedule".to_string());
    set
}

fn activity_types_for(event: &str) -> Option<&'static [&'static str]> {
    ACTIVITY_TYPES
        .iter()
        .find(|(e, _)| *e == event)
        .map(|(_, t)| *t)
}

/// Run event/activity-type checks over a workflow.
pub fn check(workflow: &Value) -> Vec<CheckFinding> {
    let mut out = Vec::new();
    let Some(on) = workflow.get("on") else {
        return out;
    };
    // Fail-open if the event set couldn't be derived (preserve zero-FP).
    if known_events().is_empty() {
        return out;
    }
    match on {
        Value::String(ev) => {
            check_event_name(ev, "/on", &mut out);
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                if let Value::String(ev) = item {
                    check_event_name(ev, &format!("/on/{i}"), &mut out);
                }
            }
        }
        Value::Object(map) => {
            for (ev, config) in map {
                let ptr = format!("/on/{}", escape(ev));
                if check_event_name(ev, &ptr, &mut out) {
                    // Only check activity types when the event name itself is valid.
                    check_activity_types(ev, config, &ptr, &mut out);
                }
            }
        }
        _ => {}
    }
    out
}

/// Returns true when the event name is known.
fn check_event_name(event: &str, pointer: &str, out: &mut Vec<CheckFinding>) -> bool {
    if known_events().contains(event) {
        return true;
    }
    out.push(CheckFinding {
        pointer: pointer.to_string(),
        message: format!("unknown workflow event `{event}`"),
        rule_id: "event/name".to_string(),
    });
    false
}

fn check_activity_types(event: &str, config: &Value, pointer: &str, out: &mut Vec<CheckFinding>) {
    let Some(types) = config.get("types") else {
        return;
    };
    let Some(allowed) = activity_types_for(event) else {
        // Event takes no activity types, but one was specified.
        out.push(CheckFinding {
            pointer: format!("{pointer}/types"),
            message: format!("event `{event}` does not support `types:`"),
            rule_id: "event/types".to_string(),
        });
        return;
    };
    // `types:` may be a single string or a list.
    let values: Vec<(&str, String)> = match types {
        Value::String(s) => vec![(s.as_str(), format!("{pointer}/types"))],
        Value::Array(items) => items
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.as_str().map(|s| (s, format!("{pointer}/types/{i}"))))
            .collect(),
        _ => return,
    };
    for (t, ptr) in values {
        if !allowed.contains(&t) {
            out.push(CheckFinding {
                pointer: ptr,
                message: format!("unknown activity type `{t}` for event `{event}`"),
                rule_id: "event/types".to_string(),
            });
        }
    }
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn findings(wf: Value) -> Vec<CheckFinding> {
        check(&wf)
    }

    fn rules(wf: Value) -> Vec<String> {
        let mut r: Vec<String> = findings(wf).into_iter().map(|f| f.rule_id).collect();
        r.sort();
        r
    }

    #[test]
    fn events_derived_from_schema() {
        let ev = known_events();
        assert!(ev.contains("push"));
        assert!(ev.contains("pull_request"));
        assert!(ev.contains("workflow_dispatch"));
        assert!(ev.contains("schedule"));
        assert!(ev.len() > 20, "should derive the full event set, got {}", ev.len());
    }

    #[test]
    fn valid_string_event_is_clean() {
        assert!(findings(json!({ "on": "push" })).is_empty());
    }

    #[test]
    fn unknown_string_event_is_flagged() {
        let f = findings(json!({ "on": "pull-request" })); // typo for pull_request
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "event/name");
        assert!(f[0].message.contains("pull-request"));
    }

    #[test]
    fn valid_sequence_of_events_is_clean() {
        assert!(findings(json!({ "on": ["push", "pull_request", "workflow_dispatch"] })).is_empty());
    }

    #[test]
    fn unknown_event_in_sequence_is_flagged() {
        let f = findings(json!({ "on": ["push", "bogus"] }));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].pointer.ends_with("/1"));
    }

    #[test]
    fn valid_mapping_with_types_is_clean() {
        let wf = json!({ "on": { "pull_request": { "types": ["opened", "synchronize"] } } });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn unknown_activity_type_is_flagged() {
        let wf = json!({ "on": { "pull_request": { "types": ["opend"] } } });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "event/types");
        assert!(f[0].message.contains("opend"));
    }

    #[test]
    fn types_on_event_that_forbids_them_is_flagged() {
        let wf = json!({ "on": { "push": { "types": ["opened"] } } });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("does not support"));
    }

    #[test]
    fn unknown_event_name_suppresses_type_check() {
        // A bogus event name yields exactly one finding (the name), not also a types error.
        let wf = json!({ "on": { "bogus_event": { "types": ["x"] } } });
        assert_eq!(rules(wf), vec!["event/name"]);
    }

    #[test]
    fn schedule_and_workflow_dispatch_configs_are_clean() {
        let wf = json!({
            "on": {
                "schedule": [ { "cron": "0 0 * * *" } ],
                "workflow_dispatch": { "inputs": { "x": { "type": "string" } } }
            }
        });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn single_string_types_value_is_handled() {
        let wf = json!({ "on": { "issues": { "types": "opened" } } });
        assert!(findings(wf).is_empty());
        let wf = json!({ "on": { "issues": { "types": "opend" } } });
        assert_eq!(findings(wf).len(), 1);
    }
}
