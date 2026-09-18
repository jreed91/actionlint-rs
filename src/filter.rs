//! `--ignore` diagnostic filtering.
//!
//! Mirrors actionlint's `-ignore` semantics: each pattern is a regular expression matched
//! against a diagnostic's **message**; a diagnostic is suppressed if *any* pattern matches
//! (find, not full-match). Repeatable. This lets users adopt the linter on a repo with known
//! or accepted findings without failing CI on them.

use anyhow::{Context, Result};
use regex::Regex;

use crate::diagnostic::Diagnostic;

/// A compiled set of ignore patterns.
#[derive(Debug)]
pub struct IgnoreFilter {
    patterns: Vec<Regex>,
}

impl IgnoreFilter {
    /// Compile the given patterns. Returns an error naming the offending pattern if any is
    /// not a valid regex (so the user gets a clear message rather than a silent no-op).
    pub fn new<I, S>(patterns: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let compiled = patterns
            .into_iter()
            .map(|p| {
                let p = p.as_ref();
                Regex::new(p).with_context(|| format!("invalid --ignore pattern: {p:?}"))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(IgnoreFilter { patterns: compiled })
    }

    /// Whether this filter would suppress `d` (any pattern matches its message).
    pub fn suppresses(&self, d: &Diagnostic) -> bool {
        self.patterns.iter().any(|re| re.is_match(&d.message))
    }

    /// True when no patterns were configured (so filtering is a no-op).
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Drop every diagnostic whose message matches any ignore pattern.
    pub fn apply(&self, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
        if self.is_empty() {
            return diagnostics;
        }
        diagnostics
            .into_iter()
            .filter(|d| !self.suppresses(d))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Position;

    fn diag(msg: &str) -> Diagnostic {
        Diagnostic::new("wf.yml", Position::new(1, 1), "/x", msg)
    }

    #[test]
    fn empty_filter_keeps_everything() {
        let f = IgnoreFilter::new(Vec::<String>::new()).unwrap();
        assert!(f.is_empty());
        let out = f.apply(vec![diag("a"), diag("b")]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn suppresses_matching_message() {
        let f = IgnoreFilter::new(["missing required key `runs-on`"]).unwrap();
        assert!(f.suppresses(&diag("`build` is missing required key `runs-on`")));
        assert!(!f.suppresses(&diag("`on` must be a string")));
    }

    #[test]
    fn apply_drops_only_matching() {
        let f = IgnoreFilter::new(["must be a"]).unwrap();
        let out = f.apply(vec![
            diag("`on` must be a string"),
            diag("`build` is missing required key `runs-on`"),
            diag("`timeout-minutes` must be a number"),
        ]);
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("runs-on"));
    }

    #[test]
    fn any_of_multiple_patterns_suppresses() {
        let f = IgnoreFilter::new(["runs-on", "must be a number"]).unwrap();
        let out = f.apply(vec![
            diag("`build` is missing required key `runs-on`"),
            diag("`timeout-minutes` must be a number"),
            diag("`on` must be a string"),
        ]);
        assert_eq!(out.len(), 1, "only the unmatched diagnostic remains");
        assert!(out[0].message.contains("must be a string"));
    }

    #[test]
    fn patterns_are_regex_not_literal() {
        let f = IgnoreFilter::new([r"^`\w+` must be"]).unwrap();
        assert!(f.suppresses(&diag("`on` must be a string")));
        // Anchored: a message that doesn't start with the pattern is not suppressed.
        assert!(!f.suppresses(&diag("something `x` must be y")));
    }

    #[test]
    fn invalid_pattern_is_a_clear_error() {
        let err = IgnoreFilter::new(["("]).unwrap_err();
        assert!(err.to_string().contains("invalid --ignore pattern"));
    }
}
