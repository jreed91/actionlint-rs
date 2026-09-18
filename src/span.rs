//! Resolve a JSON Pointer to a source position in the YAML.
//!
//! This is the core of the MVP (ADR-0002): the schema validator locates each error by a
//! JSON Pointer (e.g. `/jobs/build/runs-on`); we walk the `MarkedYaml` span tree by that
//! same pointer and return the node's start `Position`.
//!
//! Known v1 behavior (see ADR-0002 "open detail"):
//! - saphyr spans point to the node's START; we report `span.start` (1-indexed line/col,
//!   which matches our `Position` convention).
//! - For a missing/required property, the error's pointer points at the *parent* object;
//!   we anchor the diagnostic at the parent, which is the best available position.
//! - When resolving a mapping key, we point at the *value* node by default. Pointing at
//!   the key instead is a future refinement.

use saphyr::{MarkedYaml, Scalar, YamlData};

use crate::diagnostic::Position;

/// Look up the source position of the node addressed by `pointer` within `root`.
///
/// `pointer` is an RFC-6901 JSON Pointer as produced by the validator (e.g. `""` for the
/// root, `/jobs/build/runs-on` for a nested value). Returns the node's start position, or
/// `None` if the pointer does not resolve (in which case the caller should fall back to
/// the document root).
pub fn position_for_pointer(root: &MarkedYaml<'_>, pointer: &str) -> Option<Position> {
    let node = resolve(root, pointer)?;
    Some(start_position(node))
}

/// Like [`position_for_pointer`], but returns the node's full `(start, end)` range. The end
/// is `None` when it would not sit strictly after the start (some nodes report a zero-width
/// span); callers should then fall back to the start alone.
pub fn range_for_pointer(root: &MarkedYaml<'_>, pointer: &str) -> Option<(Position, Position)> {
    let node = resolve(root, pointer)?;
    let start = start_position(node);
    let end = end_position(node);
    // Only report an end that is actually after the start (line, then column).
    if (end.line, end.col) > (start.line, start.col) {
        Some((start, end))
    } else {
        Some((start, start))
    }
}

/// Walk the tree following the pointer's tokens.
fn resolve<'a, 'input>(
    root: &'a MarkedYaml<'input>,
    pointer: &str,
) -> Option<&'a MarkedYaml<'input>> {
    if pointer.is_empty() {
        return Some(root);
    }
    // RFC 6901: pointer is "/"-separated; each token unescapes ~1 -> "/" and ~0 -> "~".
    let mut current = root;
    for raw_token in pointer.split('/').skip(1) {
        let token = unescape_token(raw_token);
        current = step(current, &token)?;
    }
    Some(current)
}

/// Take one step into a node by a pointer token (an object key or array index).
fn step<'a, 'input>(
    node: &'a MarkedYaml<'input>,
    token: &str,
) -> Option<&'a MarkedYaml<'input>> {
    match &node.data {
        YamlData::Mapping(map) => {
            // Match the token against string-valued keys.
            map.iter().find_map(|(k, v)| {
                if key_matches(k, token) {
                    Some(v)
                } else {
                    None
                }
            })
        }
        YamlData::Sequence(items) => {
            let idx: usize = token.parse().ok()?;
            items.get(idx)
        }
        _ => None,
    }
}

fn key_matches(key: &MarkedYaml<'_>, token: &str) -> bool {
    match &key.data {
        YamlData::Value(Scalar::String(s)) => s == token,
        YamlData::Value(Scalar::Boolean(b)) => b.to_string() == token,
        YamlData::Value(Scalar::Integer(i)) => i.to_string() == token,
        YamlData::Value(Scalar::Null) => token == "null",
        // FloatingPoint keys and non-scalar keys never match a pointer token in practice
        // (workflow keys are strings); defensive fallback.
        _ => false,
    }
}

/// The start of a node's span, as a 1-indexed [`Position`].
///
/// saphyr's markers are 1-indexed, but the root/document node can report column 0 (it has
/// no leading indentation). Clamp line and column to a minimum of 1 so diagnostics never
/// emit a `:0` position, which editors and humans read as invalid.
fn start_position(node: &MarkedYaml<'_>) -> Position {
    let start = node.span.start;
    Position::new(start.line().max(1), start.col().max(1))
}

/// The end of a node's span, as a 1-indexed [`Position`] (clamped like the start).
fn end_position(node: &MarkedYaml<'_>) -> Position {
    let end = node.span.end;
    Position::new(end.line().max(1), end.col().max(1))
}

/// Unescape a single JSON Pointer reference token (RFC 6901 §4).
fn unescape_token(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml;

    #[test]
    fn range_returns_start_and_end() {
        let src = "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n";
        let parsed = yaml::parse(src).unwrap();
        let (start, end) = range_for_pointer(&parsed.marked, "/jobs/build/runs-on").unwrap();
        assert_eq!(start.line, 4);
        // End is at or after the start (a real range, or clamped to the start).
        assert!((end.line, end.col) >= (start.line, start.col));
    }

    #[test]
    fn range_of_scalar_value_may_be_zero_width() {
        // A short scalar can report an end equal to its start; range_for_pointer then
        // returns (start, start) rather than an end before the start.
        let src = "on: push\n";
        let parsed = yaml::parse(src).unwrap();
        let (start, end) = range_for_pointer(&parsed.marked, "/on").unwrap();
        assert!((end.line, end.col) >= (start.line, start.col));
    }

    #[test]
    fn range_of_unresolvable_pointer_is_none() {
        let src = "on: push\n";
        let parsed = yaml::parse(src).unwrap();
        assert!(range_for_pointer(&parsed.marked, "/nope").is_none());
    }

    #[test]
    fn resolves_nested_pointer_to_position() {
        // runs-on is on line 4 (1-indexed).
        let src = "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n";
        let parsed = yaml::parse(src).unwrap();
        let pos = position_for_pointer(&parsed.marked, "/jobs/build/runs-on").unwrap();
        assert_eq!(pos.line, 4);
    }

    #[test]
    fn empty_pointer_is_root() {
        let src = "on: push\n";
        let parsed = yaml::parse(src).unwrap();
        let pos = position_for_pointer(&parsed.marked, "").unwrap();
        assert_eq!(pos.line, 1);
        // Root nodes can report column 0 in saphyr; we clamp to 1 (never emit `:0`).
        assert!(pos.col >= 1, "column must be 1-indexed, got {}", pos.col);
    }

    #[test]
    fn unresolvable_pointer_returns_none() {
        let src = "on: push\n";
        let parsed = yaml::parse(src).unwrap();
        assert!(position_for_pointer(&parsed.marked, "/jobs/nope").is_none());
    }

    #[test]
    fn resolves_array_index() {
        let src = "on:\n  - push\n  - pull_request\n";
        let parsed = yaml::parse(src).unwrap();
        let pos = position_for_pointer(&parsed.marked, "/on/1").unwrap();
        assert_eq!(pos.line, 3);
    }

    #[test]
    fn out_of_range_array_index_is_none() {
        let src = "on:\n  - push\n";
        let parsed = yaml::parse(src).unwrap();
        assert!(position_for_pointer(&parsed.marked, "/on/9").is_none());
    }

    #[test]
    fn non_numeric_index_into_sequence_is_none() {
        let src = "on:\n  - push\n";
        let parsed = yaml::parse(src).unwrap();
        assert!(position_for_pointer(&parsed.marked, "/on/notnum").is_none());
    }

    #[test]
    fn stepping_into_scalar_is_none() {
        let src = "on: push\n";
        let parsed = yaml::parse(src).unwrap();
        // `on` is a scalar; stepping further can't resolve.
        assert!(position_for_pointer(&parsed.marked, "/on/deeper").is_none());
    }

    #[test]
    fn matches_non_string_keys() {
        // Integer, boolean, and null keys are addressable via their stringified token.
        let src = "1: one\ntrue: t\nnull: n\n";
        let parsed = yaml::parse(src).unwrap();
        assert!(position_for_pointer(&parsed.marked, "/1").is_some());
        assert!(position_for_pointer(&parsed.marked, "/true").is_some());
        assert!(position_for_pointer(&parsed.marked, "/null").is_some());
    }

    #[test]
    fn unescapes_pointer_tokens() {
        // A key literally containing a slash is escaped as ~1 in a JSON Pointer.
        let src = "\"a/b\": value\n\"c~d\": other\n";
        let parsed = yaml::parse(src).unwrap();
        assert!(position_for_pointer(&parsed.marked, "/a~1b").is_some());
        assert!(position_for_pointer(&parsed.marked, "/c~0d").is_some());
    }
}
