//! Built-in contexts and functions for the expression type system.
//!
//! Sources: the context list is from GitHub's first-party DSL `context` annotations
//! (`env, github, inputs, job, jobs, matrix, needs, runner, secrets, steps, strategy, vars`);
//! function names and arities are from GitHub's expressions documentation. Where a context's
//! shape is trigger-dependent or user-defined (e.g. `github.event`, `env`, `matrix`), it is
//! modeled as an OPEN object so unknown keys resolve to `Any` rather than false-positiving.

use std::collections::BTreeMap;

use super::ty::Type;

/// The type of a top-level context by name, or `None` if the name is not a known context.
pub fn context_type(name: &str) -> Option<Type> {
    Some(match name {
        "github" => github_context(),
        "runner" => runner_context(),
        "job" => job_context(),
        // User-/trigger-defined maps: open objects of Any.
        "env" | "vars" | "secrets" | "inputs" | "matrix" => Type::any_object(),
        // Collections keyed by user ids; each entry's shape is partly known but the keys are
        // user-defined, so model as open objects.
        "steps" | "needs" | "jobs" => Type::any_object(),
        "strategy" => strategy_context(),
        _ => return None,
    })
}

/// Whether `name` is a known top-level context.
pub fn is_context(name: &str) -> bool {
    context_type(name).is_some()
}

/// The list of known context names (for "did you mean" style messaging).
pub fn context_names() -> &'static [&'static str] {
    &[
        "github", "env", "vars", "job", "jobs", "steps", "runner", "secrets", "strategy",
        "matrix", "needs", "inputs",
    ]
}

fn github_context() -> Type {
    Type::object([
        ("action", Type::String),
        ("action_path", Type::String),
        ("action_ref", Type::String),
        ("action_repository", Type::String),
        ("actor", Type::String),
        ("actor_id", Type::String),
        ("api_url", Type::String),
        ("base_ref", Type::String),
        ("env", Type::String),
        ("event", Type::any_object()), // shape depends on the trigger
        ("event_name", Type::String),
        ("event_path", Type::String),
        ("graphql_url", Type::String),
        ("head_ref", Type::String),
        ("job", Type::String),
        ("path", Type::String),
        ("ref", Type::String),
        ("ref_name", Type::String),
        ("ref_protected", Type::Bool),
        ("ref_type", Type::String),
        ("repository", Type::String),
        ("repository_id", Type::String),
        ("repository_owner", Type::String),
        ("repository_owner_id", Type::String),
        ("repositoryUrl", Type::String),
        ("retention_days", Type::Number),
        ("run_id", Type::String),
        ("run_number", Type::String),
        ("run_attempt", Type::String),
        ("secret_source", Type::String),
        ("server_url", Type::String),
        ("sha", Type::String),
        ("token", Type::String),
        ("triggering_actor", Type::String),
        ("workflow", Type::String),
        ("workflow_ref", Type::String),
        ("workflow_sha", Type::String),
        ("workspace", Type::String),
    ])
}

fn runner_context() -> Type {
    Type::object([
        ("name", Type::String),
        ("os", Type::String),
        ("arch", Type::String),
        ("temp", Type::String),
        ("tool_cache", Type::String),
        ("debug", Type::String),
        ("environment", Type::String),
    ])
}

fn job_context() -> Type {
    Type::object([
        ("container", Type::any_object()),
        ("services", Type::any_object()),
        ("status", Type::String),
    ])
}

fn strategy_context() -> Type {
    Type::object([
        ("fail-fast", Type::Bool),
        ("job-index", Type::Number),
        ("job-total", Type::Number),
        ("max-parallel", Type::Number),
    ])
}

/// A built-in function signature: min/max argument counts (`max = None` means variadic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuncSig {
    pub min_args: usize,
    pub max_args: Option<usize>,
}

/// The signature of a built-in function by name, or `None` if unknown.
pub fn function_sig(name: &str) -> Option<FuncSig> {
    // Case-insensitive: GitHub treats function names case-insensitively.
    let lname = name.to_ascii_lowercase();
    let sig = |min, max| FuncSig {
        min_args: min,
        max_args: max,
    };
    Some(match lname.as_str() {
        "contains" => sig(2, Some(2)),
        "startswith" => sig(2, Some(2)),
        "endswith" => sig(2, Some(2)),
        "format" => sig(1, None), // format string + N args
        "join" => sig(1, Some(2)),
        "tojson" => sig(1, Some(1)),
        "fromjson" => sig(1, Some(1)),
        "hashfiles" => sig(1, None), // one or more glob patterns
        // Status-check functions take no arguments.
        "success" | "always" | "cancelled" | "failure" => sig(0, Some(0)),
        _ => return None,
    })
}

/// Known function names (canonical spellings), for messaging.
pub fn function_names() -> &'static [&'static str] {
    &[
        "contains",
        "startsWith",
        "endsWith",
        "format",
        "join",
        "toJSON",
        "fromJSON",
        "hashFiles",
        "success",
        "always",
        "cancelled",
        "failure",
    ]
}

/// A snapshot of all context types, for callers that want the whole environment.
pub fn all_contexts() -> BTreeMap<String, Type> {
    context_names()
        .iter()
        .filter_map(|n| context_type(n).map(|t| (n.to_string(), t)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::ty::PropertyResult;

    #[test]
    fn known_contexts_resolve() {
        for c in context_names() {
            assert!(is_context(c), "{c} should be a context");
        }
        assert!(!is_context("nope"));
    }

    #[test]
    fn github_has_typed_and_open_fields() {
        let g = context_type("github").unwrap();
        assert_eq!(g.property("sha"), PropertyResult::Type(Type::String));
        assert_eq!(g.property("ref_protected"), PropertyResult::Type(Type::Bool));
        // event is open — any sub-property is Any, not an error.
        assert_eq!(g.property("event"), PropertyResult::Type(Type::any_object()));
        // an unknown top-level github property is an error (closed object).
        assert_eq!(g.property("nonexistent"), PropertyResult::UnknownProperty);
    }

    #[test]
    fn user_maps_are_open() {
        // env/secrets/matrix keys are user-defined → open, unknown keys are Any.
        for c in ["env", "secrets", "matrix", "vars", "inputs"] {
            let t = context_type(c).unwrap();
            assert_eq!(t.property("anything"), PropertyResult::Type(Type::Any));
        }
    }

    #[test]
    fn function_signatures() {
        assert_eq!(function_sig("contains"), Some(FuncSig { min_args: 2, max_args: Some(2) }));
        assert_eq!(function_sig("format"), Some(FuncSig { min_args: 1, max_args: None }));
        assert_eq!(function_sig("always"), Some(FuncSig { min_args: 0, max_args: Some(0) }));
        assert!(function_sig("nope").is_none());
    }

    #[test]
    fn function_names_are_case_insensitive() {
        assert!(function_sig("Contains").is_some());
        assert!(function_sig("FROMJSON").is_some());
        assert!(function_sig("toJson").is_some());
    }

    #[test]
    fn all_contexts_snapshot_covers_names() {
        assert_eq!(all_contexts().len(), context_names().len());
    }
}
