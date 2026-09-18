//! actionlint-rs: a schema-driven GitHub Actions workflow linter.
//!
//! v1 validates workflow **structure** against the vendored SchemaStore schema and reports
//! precise `file:line:col` diagnostics. See CONTEXT.md and docs/ for the design.

pub mod diagnostic;
pub mod lint;
pub mod schema;
pub mod span;
pub mod yaml;
