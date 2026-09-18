//! Expression lexer: tokenizes the contents of a `${{ }}` expression.

use std::fmt;

/// A token with its byte offset within the expression source (0-based, relative to the start
/// of the expression string — callers map it onto the workflow via the outer span).
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals / names
    Ident(String),
    Number(f64),
    Str(String),
    // Punctuation
    Dot,
    Star,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    // Operators
    Not,
    Lt,
    LtEq,
    Gt,
    GtEq,
    EqEq,
    NotEq,
    AndAnd,
    OrOr,
    // End of input
    Eof,
}

/// A lexing error with the byte offset where it occurred.
#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub message: String,
    pub offset: usize,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at offset {})", self.message, self.offset)
    }
}

/// Tokenize an expression string. Whitespace is skipped; the stream always ends with `Eof`.
pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut tokens = Vec::new();

    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let kind = match c {
            '.' => {
                i += 1;
                TokenKind::Dot
            }
            '*' => {
                i += 1;
                TokenKind::Star
            }
            '(' => {
                i += 1;
                TokenKind::LParen
            }
            ')' => {
                i += 1;
                TokenKind::RParen
            }
            '[' => {
                i += 1;
                TokenKind::LBracket
            }
            ']' => {
                i += 1;
                TokenKind::RBracket
            }
            ',' => {
                i += 1;
                TokenKind::Comma
            }
            '!' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    i += 2;
                    TokenKind::NotEq
                } else {
                    i += 1;
                    TokenKind::Not
                }
            }
            '<' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    i += 2;
                    TokenKind::LtEq
                } else {
                    i += 1;
                    TokenKind::Lt
                }
            }
            '>' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    i += 2;
                    TokenKind::GtEq
                } else {
                    i += 1;
                    TokenKind::Gt
                }
            }
            '=' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    i += 2;
                    TokenKind::EqEq
                } else {
                    return Err(LexError {
                        message: "expected `==` (single `=` is not valid)".into(),
                        offset: start,
                    });
                }
            }
            '&' => {
                if bytes.get(i + 1) == Some(&b'&') {
                    i += 2;
                    TokenKind::AndAnd
                } else {
                    return Err(LexError {
                        message: "expected `&&`".into(),
                        offset: start,
                    });
                }
            }
            '|' => {
                if bytes.get(i + 1) == Some(&b'|') {
                    i += 2;
                    TokenKind::OrOr
                } else {
                    return Err(LexError {
                        message: "expected `||`".into(),
                        offset: start,
                    });
                }
            }
            '\'' => {
                let (s, next) = lex_string(bytes, i)?;
                i = next;
                TokenKind::Str(s)
            }
            _ if c == '-' || c.is_ascii_digit() => {
                let (n, next) = lex_number(src, bytes, i)?;
                i = next;
                TokenKind::Number(n)
            }
            _ if is_ident_start(c) => {
                let (name, next) = lex_ident(src, bytes, i);
                i = next;
                TokenKind::Ident(name)
            }
            _ => {
                return Err(LexError {
                    message: format!("unexpected character `{c}`"),
                    offset: start,
                })
            }
        };
        tokens.push(Token { kind, offset: start });
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        offset: bytes.len(),
    });
    Ok(tokens)
}

/// Lex a single-quoted string starting at `start` (which points at the opening `'`). A `'`
/// inside the string is written as `''` (doubled). Returns the unescaped value and the index
/// just past the closing quote.
fn lex_string(bytes: &[u8], start: usize) -> Result<(String, usize), LexError> {
    let mut i = start + 1; // skip opening quote
    let mut out = String::new();
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                out.push('\'');
                i += 2;
            } else {
                return Ok((out, i + 1)); // closing quote
            }
        } else {
            out.push(c as char);
            i += 1;
        }
    }
    Err(LexError {
        message: "unterminated string literal".into(),
        offset: start,
    })
}

fn lex_number(src: &str, bytes: &[u8], start: usize) -> Result<(f64, usize), LexError> {
    let mut i = start;
    if bytes[i] == b'-' {
        i += 1;
    }
    // Hex literal (GitHub allows e.g. 0xff).
    if bytes.get(i) == Some(&b'0') && matches!(bytes.get(i + 1), Some(&b'x') | Some(&b'X')) {
        let hex_start = i + 2;
        let mut j = hex_start;
        while j < bytes.len() && (bytes[j] as char).is_ascii_hexdigit() {
            j += 1;
        }
        if j == hex_start {
            return Err(LexError {
                message: "invalid hex number".into(),
                offset: start,
            });
        }
        let val = i64::from_str_radix(&src[hex_start..j], 16).map_err(|_| LexError {
            message: "invalid hex number".into(),
            offset: start,
        })?;
        let signed = if bytes[start] == b'-' { -val } else { val };
        return Ok((signed as f64, j));
    }
    // Decimal / float / exponent.
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-' {
            i += 1;
        } else {
            break;
        }
    }
    let text = &src[start..i];
    let n: f64 = text.parse().map_err(|_| LexError {
        message: format!("invalid number `{text}`"),
        offset: start,
    })?;
    Ok((n, i))
}

fn lex_ident(src: &str, bytes: &[u8], start: usize) -> (String, usize) {
    let mut i = start;
    while i < bytes.len() && is_ident_continue(bytes[i] as char) {
        i += 1;
    }
    (src[start..i].to_string(), i)
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lexes_context_access() {
        assert_eq!(
            kinds("github.event.pull_request"),
            vec![
                TokenKind::Ident("github".into()),
                TokenKind::Dot,
                TokenKind::Ident("event".into()),
                TokenKind::Dot,
                TokenKind::Ident("pull_request".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lexes_operators() {
        assert_eq!(
            kinds("! a == b && c || d != e <= f >= g < h > i"),
            vec![
                TokenKind::Not,
                TokenKind::Ident("a".into()),
                TokenKind::EqEq,
                TokenKind::Ident("b".into()),
                TokenKind::AndAnd,
                TokenKind::Ident("c".into()),
                TokenKind::OrOr,
                TokenKind::Ident("d".into()),
                TokenKind::NotEq,
                TokenKind::Ident("e".into()),
                TokenKind::LtEq,
                TokenKind::Ident("f".into()),
                TokenKind::GtEq,
                TokenKind::Ident("g".into()),
                TokenKind::Lt,
                TokenKind::Ident("h".into()),
                TokenKind::Gt,
                TokenKind::Ident("i".into()),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lexes_string_with_escaped_quote() {
        assert_eq!(
            kinds("'it''s'"),
            vec![TokenKind::Str("it's".into()), TokenKind::Eof]
        );
    }

    #[test]
    fn lexes_numbers() {
        assert_eq!(kinds("42"), vec![TokenKind::Number(42.0), TokenKind::Eof]);
        assert_eq!(kinds("-1.5"), vec![TokenKind::Number(-1.5), TokenKind::Eof]);
        assert_eq!(kinds("0xFF"), vec![TokenKind::Number(255.0), TokenKind::Eof]);
    }

    #[test]
    fn lexes_call_and_index_and_star() {
        assert_eq!(
            kinds("contains(a.*.b, x[0])"),
            vec![
                TokenKind::Ident("contains".into()),
                TokenKind::LParen,
                TokenKind::Ident("a".into()),
                TokenKind::Dot,
                TokenKind::Star,
                TokenKind::Dot,
                TokenKind::Ident("b".into()),
                TokenKind::Comma,
                TokenKind::Ident("x".into()),
                TokenKind::LBracket,
                TokenKind::Number(0.0),
                TokenKind::RBracket,
                TokenKind::RParen,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn errors_on_unterminated_string() {
        let e = lex("'oops").unwrap_err();
        assert!(e.message.contains("unterminated"));
    }

    #[test]
    fn errors_on_single_equals_and_lone_amp() {
        assert!(lex("a = b").is_err());
        assert!(lex("a & b").is_err());
        assert!(lex("a | b").is_err());
    }

    #[test]
    fn errors_on_unexpected_char() {
        let e = lex("a @ b").unwrap_err();
        assert!(e.message.contains("unexpected character"));
        assert_eq!(e.offset, 2);
    }

    #[test]
    fn errors_on_invalid_hex() {
        let e = lex("0xZZ").unwrap_err();
        assert!(e.message.contains("hex"), "got: {}", e.message);
    }

    #[test]
    fn errors_on_hex_overflow() {
        // A hex literal too large for i64 hits the from_str_radix error path.
        let e = lex("0xFFFFFFFFFFFFFFFFF").unwrap_err();
        assert!(e.message.contains("hex"), "got: {}", e.message);
    }

    #[test]
    fn errors_on_invalid_number() {
        // Multiple dots isn't a parseable f64.
        let e = lex("1.2.3").unwrap_err();
        assert!(e.message.contains("invalid number"), "got: {}", e.message);
    }

    #[test]
    fn lex_error_displays_with_offset() {
        let e = lex("a @ b").unwrap_err();
        assert!(e.to_string().contains("offset 2"), "got: {}", e);
    }

    #[test]
    fn negative_hex_is_lexed() {
        assert_eq!(kinds("-0x10"), vec![TokenKind::Number(-16.0), TokenKind::Eof]);
    }
}
