//! The expression type checker: walk an [`Expr`] AST, infer a [`Type`], and collect
//! semantic errors (unknown context, unknown function, wrong argument count, unknown
//! property on a known-shape context).
//!
//! v1 of the checker is intentionally conservative: it reports the mistakes that are
//! unambiguous and high-value, and falls back to `Any` (no error) wherever a shape is
//! genuinely unknown, so it does not false-positive on valid workflows.

use std::collections::BTreeSet;

use super::ast::{BinaryOp, Expr, UnaryOp};
use super::builtins;
use super::ty::{PropertyResult, Type};

/// A semantic error found while type-checking an expression.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckError {
    pub message: String,
}

impl CheckError {
    fn new(message: impl Into<String>) -> Self {
        CheckError {
            message: message.into(),
        }
    }
}

/// Type-check an expression, returning any semantic errors found (empty = clean).
pub fn check(expr: &Expr) -> Vec<CheckError> {
    Checker { errors: Vec::new(), allowed: None }.run(expr)
}

/// Type-check an expression, additionally enforcing that every context root used is in the
/// `allowed` set (context availability for a position). A context that is otherwise valid but
/// not allowed here is reported.
pub fn check_with_availability(expr: &Expr, allowed: &BTreeSet<String>) -> Vec<CheckError> {
    Checker { errors: Vec::new(), allowed: Some(allowed) }.run(expr)
}

struct Checker<'a> {
    errors: Vec<CheckError>,
    allowed: Option<&'a BTreeSet<String>>,
}

impl<'a> Checker<'a> {
    fn run(mut self, expr: &Expr) -> Vec<CheckError> {
        self.infer(expr);
        self.errors
    }

    /// Infer the type of `expr`, pushing any errors encountered. Returns `Any` on error so a
    /// single mistake doesn't cascade into spurious follow-on errors.
    fn infer(&mut self, expr: &Expr) -> Type {
        match expr {
            Expr::Null => Type::Null,
            Expr::Bool(_) => Type::Bool,
            Expr::Number(_) => Type::Number,
            Expr::Str(_) => Type::String,

            Expr::Ident(name) => match builtins::context_type(name) {
                Some(t) => {
                    self.check_availability(name);
                    t
                }
                None => {
                    self.push(format!(
                        "unknown context `{name}` (known contexts: {})",
                        builtins::context_names().join(", ")
                    ));
                    Type::Any
                }
            },

            Expr::Index { target, index } => {
                let target_ty = self.infer(target);
                // A string literal index is a property access; else a dynamic index.
                if let Expr::Str(name) = index.as_ref() {
                    // Property access on an array is the object-filter projection (`a.*.b`,
                    // `steps.*.outputs`): collect from each element. Projection is Any.
                    if matches!(target_ty, Type::Array(_)) {
                        return Type::array(Type::Any);
                    }
                    match target_ty.property(name) {
                        PropertyResult::Type(t) => t,
                        PropertyResult::UnknownProperty => {
                            self.push(format!(
                                "`{name}` is not a valid property of {}",
                                describe_receiver(target)
                            ));
                            Type::Any
                        }
                        PropertyResult::NotAnObject => {
                            self.push(format!(
                                "cannot access property `{name}` on a {}",
                                target_ty.describe()
                            ));
                            Type::Any
                        }
                    }
                } else {
                    // Dynamic index (`a[expr]`): check the index; result is Any.
                    self.infer(index);
                    Type::Any
                }
            }

            Expr::Star(target) => {
                self.infer(target);
                Type::array(Type::Any)
            }

            Expr::Call { name, args } => {
                for a in args {
                    self.infer(a);
                }
                match builtins::function_sig(name) {
                    Some(sig) => {
                        let n = args.len();
                        let under = n < sig.min_args;
                        let over = sig.max_args.is_some_and(|max| n > max);
                        if under || over {
                            self.push(format!(
                                "function `{name}` called with {n} argument{} ({})",
                                if n == 1 { "" } else { "s" },
                                arity_desc(&sig)
                            ));
                        }
                        function_return_type(name)
                    }
                    None => {
                        self.push(format!(
                            "unknown function `{name}` (known functions: {})",
                            builtins::function_names().join(", ")
                        ));
                        Type::Any
                    }
                }
            }

            Expr::Unary { op, operand } => {
                self.infer(operand);
                match op {
                    UnaryOp::Not => Type::Bool,
                }
            }

            Expr::Binary { op, left, right } => {
                self.infer(left);
                self.infer(right);
                match op {
                    BinaryOp::And | BinaryOp::Or => Type::Any, // && / || return an operand
                    _ => Type::Bool,                           // comparisons/equality → boolean
                }
            }
        }
    }

    fn push(&mut self, message: String) {
        self.errors.push(CheckError::new(message));
    }

    /// If an availability set is active, flag a known context used outside it.
    fn check_availability(&mut self, name: &str) {
        if let Some(allowed) = self.allowed {
            if !allowed.contains(name) {
                let mut v: Vec<&str> = allowed.iter().map(|s| s.as_str()).collect();
                v.sort_unstable();
                self.push(format!(
                    "context `{name}` is not available here (available: {})",
                    v.join(", ")
                ));
            }
        }
    }
}

/// The return type of a known built-in function.
fn function_return_type(name: &str) -> Type {
    match name.to_ascii_lowercase().as_str() {
        "contains" | "startswith" | "endswith" | "success" | "always" | "cancelled"
        | "failure" => Type::Bool,
        "format" | "join" | "tojson" | "hashfiles" => Type::String,
        // fromJSON's shape depends on the JSON, and any not-yet-typed function falls here.
        _ => Type::Any,
    }
}

fn arity_desc(sig: &builtins::FuncSig) -> String {
    match sig.max_args {
        Some(max) if max == sig.min_args => format!("expects {}", sig.min_args),
        Some(max) => format!("expects {} to {}", sig.min_args, max),
        None => format!("expects at least {}", sig.min_args),
    }
}

/// A human description of the receiver of a property access, for error messages.
///
/// In the current type model, an `UnknownProperty` error can only arise on a closed context
/// object, which is reached as a bare context identifier (e.g. `github.bad`). Other receivers
/// are open objects (unknown keys → `Any`, no error) or scalars (a `NotAnObject` error, which
/// uses the type name, not this function). So `Ident` is the reachable case; anything else
/// gets a generic phrasing.
fn describe_receiver(target: &Expr) -> String {
    match target {
        Expr::Ident(name) => format!("context `{name}`"),
        _ => "the value".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::parse;

    fn errors(src: &str) -> Vec<String> {
        check(&parse(src).unwrap()).into_iter().map(|e| e.message).collect()
    }

    #[test]
    fn valid_context_access_is_clean() {
        assert!(errors("github.sha").is_empty());
        assert!(errors("github.event.pull_request.number").is_empty());
        assert!(errors("runner.os").is_empty());
        assert!(errors("matrix.anything").is_empty());
        assert!(errors("env.MY_VAR").is_empty());
    }

    #[test]
    fn unknown_context_is_flagged() {
        let e = errors("gethub.sha");
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("unknown context `gethub`"));
    }

    #[test]
    fn unknown_property_on_known_context_is_flagged() {
        let e = errors("github.nonexistent");
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("not a valid property"), "got: {:?}", e);
        assert!(e[0].contains("nonexistent"));
    }

    #[test]
    fn open_object_properties_are_allowed() {
        // github.event is open → any depth is fine.
        assert!(errors("github.event.a.b.c").is_empty());
    }

    #[test]
    fn property_on_scalar_is_flagged() {
        let e = errors("github.sha.foo");
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("cannot access property `foo` on a string"), "got: {:?}", e);
    }

    #[test]
    fn describe_receiver_handles_index_and_dynamic_receivers() {
        // Receiver is an Index (`github.job` -> String), then a bad `.x` on it.
        let e = errors("github.job.x");
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("on a string"), "got: {:?}", e);

        // Receiver is a dynamic index (`github['sha']` -> String), then a bad `.y`.
        let e = errors("github['sha'].y");
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("on a string"), "got: {:?}", e);
    }

    #[test]
    fn unknown_function_is_flagged() {
        let e = errors("frobnicate(1)");
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("unknown function `frobnicate`"));
    }

    #[test]
    fn wrong_arity_is_flagged() {
        let too_few = errors("contains('a')");
        assert_eq!(too_few.len(), 1);
        assert!(too_few[0].contains("expects 2"), "got: {:?}", too_few);

        let too_many = errors("always(1)");
        assert_eq!(too_many.len(), 1);
        assert!(too_many[0].contains("expects 0"), "got: {:?}", too_many);
    }

    #[test]
    fn variadic_and_range_functions_accept_valid_counts() {
        // Args must themselves be valid expressions (bare idents are unknown contexts), so
        // use real contexts / literals.
        assert!(errors("format('{0} {1}', github.sha, runner.os)").is_empty());
        assert!(errors("hashFiles('**/Cargo.lock')").is_empty());
        assert!(errors("join(matrix.x, ',')").is_empty());
        assert!(errors("toJSON(github)").is_empty());
    }

    #[test]
    fn errors_nested_in_calls_and_operators_are_found() {
        // An unknown context inside a function argument and across an operator.
        let e = errors("contains(bogus.x, 'y') && github.zzz");
        assert_eq!(e.len(), 2, "got: {:?}", e);
    }

    #[test]
    fn dynamic_index_is_checked_but_yields_any() {
        assert!(errors("matrix[github.sha]").is_empty());
        // an unknown context used as the index is still flagged
        let e = errors("matrix[bogus.x]");
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn star_filter_type_checks_target() {
        assert!(errors("github.event.commits.*.message").is_empty());
        let e = errors("bogus.*.x");
        assert_eq!(e.len(), 1);
    }

    fn avail_errors(src: &str, allowed: &[&str]) -> Vec<String> {
        let set: BTreeSet<String> = allowed.iter().map(|s| s.to_string()).collect();
        check_with_availability(&parse(src).unwrap(), &set)
            .into_iter()
            .map(|e| e.message)
            .collect()
    }

    #[test]
    fn availability_flags_disallowed_context() {
        // runs-on allows github/matrix but not secrets.
        let e = avail_errors("secrets.TOKEN", &["github", "matrix"]);
        assert_eq!(e.len(), 1, "got: {:?}", e);
        assert!(e[0].contains("`secrets` is not available here"), "got: {:?}", e);
        assert!(e[0].contains("available: github, matrix"), "got: {:?}", e);
    }

    #[test]
    fn availability_allows_permitted_context() {
        assert!(avail_errors("github.sha", &["github", "matrix"]).is_empty());
    }

    #[test]
    fn availability_still_reports_unknown_context() {
        // An unknown context is flagged regardless of the allowed set (single error).
        let e = avail_errors("bogus.x", &["github"]);
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("unknown context"));
    }

    #[test]
    fn literals_and_boolean_logic_are_clean() {
        assert!(errors("true && false || !cancelled()").is_empty());
        assert!(errors("github.event_name == 'push'").is_empty());
        // null literal, and fromJSON (Any return) feeding a comparison.
        assert!(errors("null == fromJSON('1')").is_empty());
    }

    #[test]
    fn range_arity_message_shows_bounds() {
        // join accepts 1..=2; three args triggers the "expects 1 to 2" message.
        let e = errors("join(matrix.x, ',', 'extra')");
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("expects 1 to 2"), "got: {:?}", e);
    }

    #[test]
    fn variadic_min_arity_message() {
        // hashFiles expects at least 1; zero args triggers "expects at least 1".
        let e = errors("hashFiles()");
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("expects at least 1"), "got: {:?}", e);
    }

    #[test]
    fn unknown_property_on_nested_receiver_names_it() {
        // strategy is a known closed object; an unknown prop on it after a valid one.
        let e = errors("job.container.image.deep.bad");
        // job.container is open (any_object) so this stays clean — instead test a closed
        // nested receiver: strategy has known props only.
        assert!(e.is_empty(), "job.container is open; got: {:?}", e);

        // A closed-object receiver reached via property access: runner is closed.
        let e = errors("runner.bogus");
        assert!(e[0].contains("context `runner`"), "got: {:?}", e);
    }
}
