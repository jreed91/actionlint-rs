//! Expression AST.

/// A parsed GitHub Actions expression (the contents between `${{` and `}}`).
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `null`.
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// A numeric literal. GitHub numbers are f64 (may be written as int or float/hex).
    Number(f64),
    /// A single-quoted string literal (with `''` already unescaped to `'`).
    Str(String),
    /// An identifier: a context name (`github`) or a bare name used as a property root.
    Ident(String),
    /// Property access `obj.name` (e.g. `github.event`).
    Index {
        target: Box<Expr>,
        /// The property/index. For `a.b`, a `Str("b")`; for `a[expr]`, the inner expr.
        index: Box<Expr>,
    },
    /// The object-filter star: `a.*` — projects a property across an array/object.
    Star(Box<Expr>),
    /// A function call `name(args...)`.
    Call { name: String, args: Vec<Expr> },
    /// A unary operation.
    Unary { op: UnaryOp, operand: Box<Expr> },
    /// A binary operation.
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `!`
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Lt,
    LtEq,
    Gt,
    GtEq,
    Eq,
    NotEq,
    And,
    Or,
}
