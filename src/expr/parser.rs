//! Recursive-descent / precedence-climbing parser for GitHub Actions expressions.

use std::fmt;

use super::ast::{BinaryOp, Expr, UnaryOp};
use super::lexer::{lex, Token, TokenKind};

/// A parse (or lex) error with a byte offset within the expression source.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub offset: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at offset {})", self.message, self.offset)
    }
}

/// Parse an expression string (the contents between `${{` and `}}`) into an [`Expr`].
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let tokens = lex(src).map_err(|e| ParseError {
        message: e.message,
        offset: e.offset,
    })?;
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.parse_or()?;
    p.expect(TokenKind::Eof)?;
    Ok(expr)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn offset(&self) -> usize {
        self.tokens[self.pos].offset
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, kind: TokenKind) -> Result<(), ParseError> {
        if *self.peek() == kind {
            self.bump();
            Ok(())
        } else {
            Err(self.err(format!("expected {kind:?}, found {:?}", self.peek())))
        }
    }

    fn err(&self, message: String) -> ParseError {
        ParseError {
            message,
            offset: self.offset(),
        }
    }

    // `||`
    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while *self.peek() == TokenKind::OrOr {
            self.bump();
            let right = self.parse_and()?;
            left = binary(BinaryOp::Or, left, right);
        }
        Ok(left)
    }

    // `&&`
    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_equality()?;
        while *self.peek() == TokenKind::AndAnd {
            self.bump();
            let right = self.parse_equality()?;
            left = binary(BinaryOp::And, left, right);
        }
        Ok(left)
    }

    // `==` `!=`
    fn parse_equality(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                TokenKind::EqEq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::NotEq,
                _ => break,
            };
            self.bump();
            let right = self.parse_comparison()?;
            left = binary(op, left, right);
        }
        Ok(left)
    }

    // `<` `<=` `>` `>=`
    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::LtEq => BinaryOp::LtEq,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::GtEq => BinaryOp::GtEq,
                _ => break,
            };
            self.bump();
            let right = self.parse_unary()?;
            left = binary(op, left, right);
        }
        Ok(left)
    }

    // prefix `!`
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if *self.peek() == TokenKind::Not {
            self.bump();
            let operand = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                operand: Box::new(operand),
            });
        }
        self.parse_postfix()
    }

    // postfix: `.name`, `.*`, `[expr]`
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        loop {
            match self.peek() {
                TokenKind::Dot => {
                    self.bump();
                    match self.peek().clone() {
                        TokenKind::Ident(name) => {
                            self.bump();
                            expr = Expr::Index {
                                target: Box::new(expr),
                                index: Box::new(Expr::Str(name)),
                            };
                        }
                        TokenKind::Star => {
                            self.bump();
                            expr = Expr::Star(Box::new(expr));
                        }
                        other => {
                            return Err(self.err(format!(
                                "expected a property name or `*` after `.`, found {other:?}"
                            )))
                        }
                    }
                }
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.parse_or()?;
                    self.expect(TokenKind::RBracket)?;
                    expr = Expr::Index {
                        target: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            TokenKind::Number(n) => {
                self.bump();
                Ok(Expr::Number(n))
            }
            TokenKind::Str(s) => {
                self.bump();
                Ok(Expr::Str(s))
            }
            TokenKind::LParen => {
                self.bump();
                let inner = self.parse_or()?;
                self.expect(TokenKind::RParen)?;
                Ok(inner)
            }
            TokenKind::Ident(name) => {
                self.bump();
                // A `(` immediately after an identifier makes it a function call.
                if *self.peek() == TokenKind::LParen {
                    self.bump();
                    let args = self.parse_args()?;
                    self.expect(TokenKind::RParen)?;
                    Ok(Expr::Call { name, args })
                } else {
                    Ok(match name.as_str() {
                        "true" => Expr::Bool(true),
                        "false" => Expr::Bool(false),
                        "null" => Expr::Null,
                        _ => Expr::Ident(name),
                    })
                }
            }
            other => Err(self.err(format!("unexpected token {other:?}"))),
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut args = Vec::new();
        if *self.peek() == TokenKind::RParen {
            return Ok(args); // no args
        }
        loop {
            args.push(self.parse_or()?);
            match self.peek() {
                TokenKind::Comma => {
                    self.bump();
                }
                _ => break,
            }
        }
        Ok(args)
    }
}

fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_literals_and_keywords() {
        assert_eq!(parse("true").unwrap(), Expr::Bool(true));
        assert_eq!(parse("false").unwrap(), Expr::Bool(false));
        assert_eq!(parse("null").unwrap(), Expr::Null);
        assert_eq!(parse("42").unwrap(), Expr::Number(42.0));
        assert_eq!(parse("'hi'").unwrap(), Expr::Str("hi".into()));
    }

    #[test]
    fn parses_context_access() {
        let e = parse("github.event.number").unwrap();
        // github.event.number => Index(Index(Ident(github), "event"), "number")
        assert_eq!(
            e,
            Expr::Index {
                target: Box::new(Expr::Index {
                    target: Box::new(Expr::Ident("github".into())),
                    index: Box::new(Expr::Str("event".into())),
                }),
                index: Box::new(Expr::Str("number".into())),
            }
        );
    }

    #[test]
    fn parses_star_filter() {
        let e = parse("github.event.commits.*.message").unwrap();
        // The `.*` wraps the target, then `.message` indexes the star.
        assert!(matches!(
            e,
            Expr::Index { target, index }
                if *index == Expr::Str("message".into()) && matches!(*target, Expr::Star(_))
        ));
    }

    #[test]
    fn parses_index_expr() {
        let e = parse("matrix['os']").unwrap();
        assert_eq!(
            e,
            Expr::Index {
                target: Box::new(Expr::Ident("matrix".into())),
                index: Box::new(Expr::Str("os".into())),
            }
        );
    }

    #[test]
    fn parses_function_call() {
        let e = parse("contains(a, 'b')").unwrap();
        assert_eq!(
            e,
            Expr::Call {
                name: "contains".into(),
                args: vec![Expr::Ident("a".into()), Expr::Str("b".into())],
            }
        );
        assert_eq!(
            parse("always()").unwrap(),
            Expr::Call { name: "always".into(), args: vec![] }
        );
    }

    #[test]
    fn respects_operator_precedence() {
        // a || b && c  =>  a || (b && c)
        assert!(matches!(
            parse("a || b && c").unwrap(),
            Expr::Binary {
                op: BinaryOp::Or,
                right,
                ..
            } if matches!(*right, Expr::Binary { op: BinaryOp::And, .. })
        ));
        // a == b || c  =>  (a == b) || c
        assert!(matches!(
            parse("a == b || c").unwrap(),
            Expr::Binary { op: BinaryOp::Or, .. }
        ));
    }

    #[test]
    fn parens_override_precedence() {
        // (a || b) && c => And at top
        assert!(matches!(
            parse("(a || b) && c").unwrap(),
            Expr::Binary { op: BinaryOp::And, .. }
        ));
    }

    #[test]
    fn unary_not_binds_tighter_than_comparison() {
        // !a == b  =>  (!a) == b
        assert!(matches!(
            parse("!a == b").unwrap(),
            Expr::Binary {
                op: BinaryOp::Eq,
                left,
                ..
            } if matches!(*left, Expr::Unary { op: UnaryOp::Not, .. })
        ));
    }

    #[test]
    fn errors_on_trailing_tokens() {
        assert!(parse("a b").is_err());
    }

    #[test]
    fn errors_on_incomplete_expression() {
        assert!(parse("a &&").is_err());
        assert!(parse("(a").is_err());
        assert!(parse("a.").is_err());
    }

    #[test]
    fn errors_carry_offset() {
        let e = parse("a @ b").unwrap_err();
        assert_eq!(e.offset, 2);
    }

    #[test]
    fn parses_every_comparison_and_equality_operator() {
        for (src, op) in [
            ("a < b", BinaryOp::Lt),
            ("a <= b", BinaryOp::LtEq),
            ("a > b", BinaryOp::Gt),
            ("a >= b", BinaryOp::GtEq),
            ("a == b", BinaryOp::Eq),
            ("a != b", BinaryOp::NotEq),
        ] {
            match parse(src).unwrap() {
                Expr::Binary { op: got, .. } => assert_eq!(got, op, "for `{src}`"),
                other => panic!("expected Binary for `{src}`, got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_error_displays_with_offset() {
        let e = parse("a @ b").unwrap_err();
        let s = e.to_string();
        assert!(s.contains("offset 2"), "got: {s}");
    }
}
