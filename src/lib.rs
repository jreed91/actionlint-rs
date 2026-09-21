//! actionlint-rs: a schema-driven GitHub Actions workflow linter.
//!
//! v1 validates workflow **structure** against the vendored SchemaStore schema and reports
//! precise `file:line:col` diagnostics. See CONTEXT.md and docs/ for the design.

pub mod diagnostic;
pub mod expr;
pub mod expr_lint;
pub mod filter;
pub mod graph;
pub mod humanize;
pub mod lint;
pub mod reconcile;
pub mod sarif;
pub mod schema;
pub mod transpile;
pub mod span;
pub mod uses;
pub mod yaml;
