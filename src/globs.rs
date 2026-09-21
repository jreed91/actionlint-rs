//! Glob-pattern validation for `on.<event>.{branches,tags,paths}` filters (and their
//! `-ignore` variants).
//!
//! GitHub filter patterns are a small glob dialect: `*` (not `/`), `**` (crosses `/`), `?`,
//! `+`, `!` (negate, first char), and `[...]` character ranges. Most syntax errors GitHub
//! rejects are subtle and dialect-specific; flagging them aggressively risks false positives,
//! which this project treats as unacceptable. We therefore validate only the **unambiguous**
//! structural errors:
//!
//! - an **unclosed character class** (`[abc` with no `]`) — always invalid;
//! - an **empty pattern** (`''`) — never matches anything, always a mistake.
//!
//! (An "empty class" `[]` isn't a distinct error: a `]` right after `[` is a *literal member*
//! per glob rules, so `[]` is really an unclosed class and reported as such.)
//!
//! Patterns containing `${{ }}` can't occur here (`on:` filters aren't expression-evaluated).

use serde_json::Value;

use crate::checks::CheckFinding;

/// Filter keys whose values are glob-pattern lists.
const GLOB_KEYS: &[&str] = &[
    "branches",
    "branches-ignore",
    "tags",
    "tags-ignore",
    "paths",
    "paths-ignore",
];

/// Check glob patterns under every event's filter keys.
pub fn check(workflow: &Value) -> Vec<CheckFinding> {
    let mut out = Vec::new();
    let Some(on) = workflow.get("on").and_then(|o| o.as_object()) else {
        return out;
    };
    for (event, config) in on {
        let Some(cfg) = config.as_object() else {
            continue;
        };
        for key in GLOB_KEYS {
            if let Some(list) = cfg.get(*key).and_then(|v| v.as_array()) {
                let base = format!("/on/{}/{}", escape(event), key);
                for (i, item) in list.iter().enumerate() {
                    if let Value::String(pat) = item {
                        if let Some(reason) = glob_error(pat) {
                            out.push(CheckFinding {
                                pointer: format!("{base}/{i}"),
                                message: format!("invalid glob pattern `{pat}`: {reason}"),
                                rule_id: "glob/syntax".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

/// Return an error reason if `pat` is unambiguously an invalid GitHub filter pattern.
fn glob_error(pat: &str) -> Option<String> {
    if pat.is_empty() {
        return Some("empty pattern".to_string());
    }
    let mut chars = pat.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // Backslash escapes the next char (so `\[` is a literal `[`, not a class).
            '\\' => {
                chars.next();
            }
            '[' => {
                // Scan to the closing `]`. A `]` immediately after `[` (or `[!`) is a literal
                // member, per glob rules, so it doesn't close the class.
                let mut members = 0usize;
                // Optional leading negation.
                if matches!(chars.peek(), Some('!') | Some('^')) {
                    chars.next();
                }
                let mut closed = false;
                while let Some(&nc) = chars.peek() {
                    if nc == ']' && members > 0 {
                        chars.next();
                        closed = true;
                        break;
                    }
                    chars.next();
                    members += 1;
                }
                if !closed {
                    return Some("unclosed `[` character class".to_string());
                }
                debug_assert!(members > 0, "a closed class always has >=1 member");
            }
            _ => {}
        }
    }
    None
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

    #[test]
    fn valid_patterns_are_clean() {
        let wf = json!({
            "on": { "push": {
                "branches": ["main", "release/**", "feature/*", "v[0-9]", "!main"],
                "paths": ["src/**", "**.rs", "docs/*.md"],
                "tags": ["v*.*.*"]
            } }
        });
        let f = findings(wf);
        assert!(f.is_empty(), "{f:?}");
    }

    #[test]
    fn unclosed_char_class_is_flagged() {
        let wf = json!({ "on": { "push": { "branches": ["v[0-9"] } } });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule_id, "glob/syntax");
        assert!(f[0].message.contains("unclosed"));
        assert_eq!(f[0].pointer, "/on/push/branches/0");
    }

    #[test]
    fn bracket_then_literal_close_is_unclosed() {
        // `[]x` — the `]` is a literal first member, so there's no closing bracket: unclosed.
        let wf = json!({ "on": { "push": { "tags": ["v[]x"] } } });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("unclosed"));
    }

    #[test]
    fn empty_pattern_is_flagged() {
        let wf = json!({ "on": { "push": { "paths": [""] } } });
        assert_eq!(findings(wf).len(), 1);
    }

    #[test]
    fn escaped_bracket_is_not_a_class() {
        // `\[` is a literal bracket, not an unclosed class.
        let wf = json!({ "on": { "push": { "paths": ["weird\\[name"] } } });
        assert!(findings(wf).is_empty());
    }

    #[test]
    fn closing_bracket_as_first_member_is_literal() {
        // `[]abc]` — the first `]` is a literal member; the class closes at the second `]`.
        assert!(glob_error("[]abc]").is_none());
        // `[!]abc]` — negated, `]` literal first member.
        assert!(glob_error("[!]x]").is_none());
    }

    #[test]
    fn ignore_variants_are_checked() {
        let wf = json!({ "on": { "push": { "paths-ignore": ["v[0-9"], "branches-ignore": ["ok"] } } });
        let f = findings(wf);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].pointer.contains("paths-ignore"));
    }

    #[test]
    fn non_string_and_missing_are_ignored() {
        assert!(findings(json!({ "on": "push" })).is_empty());
        assert!(findings(json!({ "on": { "push": {} } })).is_empty());
    }

    #[test]
    fn event_with_non_object_config_is_skipped() {
        // e.g. `on: { push: null }` or `on: { workflow_dispatch: {} }` — the config isn't an
        // object with filter keys; hits the `continue`.
        assert!(findings(json!({ "on": { "push": null } })).is_empty());
        assert!(findings(json!({ "on": { "workflow_dispatch": "x" } })).is_empty());
    }

    #[test]
    fn non_string_pattern_entry_is_ignored() {
        // A non-string item in the glob list is skipped.
        assert!(findings(json!({ "on": { "push": { "branches": [42] } } })).is_empty());
    }
}
