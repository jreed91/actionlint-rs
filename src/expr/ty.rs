//! Types for the expression type system.
//!
//! A deliberately small lattice: enough to catch real mistakes (unknown property on a
//! known-shape context, wrong function arity) without pretending to full soundness. `Any` is
//! the escape hatch — property access / indexing on `Any` yields `Any`, so unknown shapes
//! never produce false positives.

use std::collections::BTreeMap;

/// A value type in the `${{ }}` expression language.
#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// Unknown / untyped — accepts anything, and any access off it is also `Any`.
    Any,
    Null,
    Bool,
    Number,
    String,
    /// A homogeneous array of `elem`.
    Array(Box<Type>),
    /// An object. `props` are the known keys; `open` means unknown keys are allowed and
    /// resolve to `Any` (e.g. `github.event`, whose shape depends on the trigger).
    Object {
        props: BTreeMap<String, Type>,
        open: bool,
    },
}

impl Type {
    /// A closed object from (name, type) pairs.
    pub fn object<I: IntoIterator<Item = (&'static str, Type)>>(pairs: I) -> Type {
        Type::Object {
            props: pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            open: false,
        }
    }

    /// An open object (unknown keys allowed → `Any`).
    pub fn open_object<I: IntoIterator<Item = (&'static str, Type)>>(pairs: I) -> Type {
        Type::Object {
            props: pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            open: true,
        }
    }

    /// A fully-open object of all-`Any` values (a map whose keys we don't enumerate).
    pub fn any_object() -> Type {
        Type::Object {
            props: BTreeMap::new(),
            open: true,
        }
    }

    pub fn array(elem: Type) -> Type {
        Type::Array(Box::new(elem))
    }

    /// Resolve property access `.<name>` on this type.
    ///
    /// - `Any` → `Any`.
    /// - object with a known prop → that prop's type.
    /// - open object with an unknown prop → `Any` (allowed).
    /// - closed object with an unknown prop → `None` (an error the caller reports).
    /// - anything else → `None` (property access on a scalar is invalid).
    pub fn property(&self, name: &str) -> PropertyResult {
        match self {
            Type::Any => PropertyResult::Type(Type::Any),
            Type::Object { props, open } => match props.get(name) {
                Some(t) => PropertyResult::Type(t.clone()),
                None if *open => PropertyResult::Type(Type::Any),
                None => PropertyResult::UnknownProperty,
            },
            _ => PropertyResult::NotAnObject,
        }
    }

    /// A short human name for diagnostics.
    pub fn describe(&self) -> String {
        match self {
            Type::Any => "any".into(),
            Type::Null => "null".into(),
            Type::Bool => "boolean".into(),
            Type::Number => "number".into(),
            Type::String => "string".into(),
            Type::Array(_) => "array".into(),
            Type::Object { .. } => "object".into(),
        }
    }
}

/// Outcome of resolving a property on a type.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyResult {
    /// The property resolved to this type.
    Type(Type),
    /// The receiver is an object but has no such (known) property.
    UnknownProperty,
    /// The receiver isn't an object, so property access is invalid.
    NotAnObject,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_property_is_any() {
        assert_eq!(Type::Any.property("x"), PropertyResult::Type(Type::Any));
    }

    #[test]
    fn known_property_resolves() {
        let t = Type::object([("a", Type::String)]);
        assert_eq!(t.property("a"), PropertyResult::Type(Type::String));
    }

    #[test]
    fn unknown_property_on_closed_object_errors() {
        let t = Type::object([("a", Type::String)]);
        assert_eq!(t.property("b"), PropertyResult::UnknownProperty);
    }

    #[test]
    fn unknown_property_on_open_object_is_any() {
        let t = Type::open_object([("a", Type::String)]);
        assert_eq!(t.property("b"), PropertyResult::Type(Type::Any));
    }

    #[test]
    fn property_on_scalar_is_not_an_object() {
        assert_eq!(Type::String.property("x"), PropertyResult::NotAnObject);
        assert_eq!(Type::Number.property("x"), PropertyResult::NotAnObject);
    }

    #[test]
    fn describe_names_types() {
        assert_eq!(Type::Any.describe(), "any");
        assert_eq!(Type::Null.describe(), "null");
        assert_eq!(Type::Bool.describe(), "boolean");
        assert_eq!(Type::Number.describe(), "number");
        assert_eq!(Type::String.describe(), "string");
        assert_eq!(Type::array(Type::Number).describe(), "array");
        assert_eq!(Type::any_object().describe(), "object");
    }
}
