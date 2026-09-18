//! GitHub Actions `${{ }}` expression engine: lexer, parser, and AST.
//!
//! This is the foundation for the semantic checks that go beyond structural validation
//! (ADR-0006): expression type-checking, context availability, and `if:` analysis all
//! consume the AST produced here.
//!
//! Grammar (from GitHub's expressions docs): literals (`true`/`false`/`null`, numbers,
//! single-quoted strings with `''` escaping), context access (`a.b.c`), index (`a[0]`,
//! `a['k']`), the `*` object filter (`a.*.b`), function calls (`f(x, y)`), unary `!`, the
//! binary operators `< <= > >= == != && ||`, and parentheses.

mod ast;
pub mod availability;
pub mod builtins;
mod check;
mod lexer;
mod parser;
pub mod ty;

pub use ast::{BinaryOp, Expr, UnaryOp};
pub use availability::AvailabilityTable;
pub use check::{check, check_with_availability, CheckError};
pub use lexer::{lex, Token, TokenKind};
pub use parser::{parse, ParseError};
pub use ty::Type;
