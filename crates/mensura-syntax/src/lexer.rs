//! A hand-written, dependency-free lexer for Mensura.
//!
//! It scans a `&str` into a `Vec<Token>` terminated by [`TokenKind::Eof`].
//! Whitespace and `//` line comments are skipped.  On the first malformed
//! token it returns a [`LexError`] with the offending [`Span`]; error
//! recovery (reporting many errors at once) is a later concern.

use crate::token::{Span, Token, TokenKind};
use unicode_xid::UnicodeXID;

/// A lexing failure, located by a source span.
#[derive(Clone, Debug, PartialEq)]
pub struct LexError {
    pub message: String,
    pub span: Span,
}

impl LexError {
    fn new(message: impl Into<String>, span: Span) -> Self {
        LexError {
            message: message.into(),
            span,
        }
    }
}

/// Tokenize `src` into a vector of tokens ending in [`TokenKind::Eof`].
pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    Lexer::new(src).run()
}

/// True if `s` is a valid Mensura identifier: a UAX#31 identifier, augmented
/// with a leading `_` (the same profile the lexer accepts).  Shared so the
/// resolver can validate names produced by template interpolation.
pub fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_xid_start() => {}
        _ => return false,
    }
    chars.all(|c| c.is_xid_continue())
}

struct Lexer<'a> {
    src: &'a str,
    /// Current byte offset into `src`.
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Lexer { src, pos: 0 }
    }

    /// The current character without consuming it.
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    /// The character after the current one without consuming anything.
    fn peek2(&self) -> Option<char> {
        let mut it = self.src[self.pos..].chars();
        it.next();
        it.next()
    }

    /// Consume and return the current character, advancing `pos` by its
    /// UTF-8 length.
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn run(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_trivia();
            let start = self.pos;
            let Some(c) = self.peek() else {
                tokens.push(Token::new(TokenKind::Eof, Span::new(start, start)));
                return Ok(tokens);
            };
            // Identifiers follow UAX#31, augmented with a leading `_` (the
            // common language profile, as in Rust).  XID_Continue excludes
            // the `No` category, so superscripts like `²` are not identifier
            // characters and never glue onto `time` in `time^2`.
            let kind = if c == '_' || c.is_xid_start() {
                self.lex_ident()
            } else if c.is_ascii_digit() {
                self.lex_number()?
            } else if c == '"' {
                self.lex_string()?
            } else if c == '`' {
                self.lex_template()?
            } else {
                self.lex_symbol()?
            };
            tokens.push(Token::new(kind, Span::new(start, self.pos)));
        }
    }

    /// Skip whitespace and `//` line comments.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('/') if self.peek2() == Some('/') => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                _ => return,
            }
        }
    }

    fn lex_ident(&mut self) -> TokenKind {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_xid_continue() {
                self.bump();
            } else {
                break;
            }
        }
        TokenKind::Ident(self.src[start..self.pos].to_string())
    }

    fn lex_number(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
        // A fractional part makes it a float, but only if a digit follows the
        // dot.  `1.0` is a float; the dot in `input.id` stays a separate
        // `Dot` token.
        let is_float = self.peek() == Some('.') && self.peek2().is_some_and(|c| c.is_ascii_digit());
        if is_float {
            self.bump(); // '.'
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        let text = &self.src[start..self.pos];
        let span = Span::new(start, self.pos);
        if is_float {
            text.parse::<f64>()
                .map(TokenKind::Float)
                .map_err(|e| LexError::new(format!("invalid float literal: {e}"), span))
        } else {
            text.parse::<i64>()
                .map(TokenKind::Int)
                .map_err(|e| LexError::new(format!("invalid integer literal: {e}"), span))
        }
    }

    fn lex_string(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        self.bump(); // opening quote
        let mut value = String::new();
        loop {
            match self.bump() {
                None => {
                    return Err(LexError::new(
                        "unterminated string literal",
                        Span::new(start, self.pos),
                    ));
                }
                Some('"') => return Ok(TokenKind::Str(value)),
                Some('\\') => {
                    let esc_start = self.pos - 1;
                    match self.bump() {
                        Some('"') => value.push('"'),
                        Some('\\') => value.push('\\'),
                        Some('n') => value.push('\n'),
                        Some('t') => value.push('\t'),
                        Some('r') => value.push('\r'),
                        Some(other) => {
                            return Err(LexError::new(
                                format!("unknown escape sequence: \\{other}"),
                                Span::new(esc_start, self.pos),
                            ));
                        }
                        None => {
                            return Err(LexError::new(
                                "unterminated string literal",
                                Span::new(start, self.pos),
                            ));
                        }
                    }
                }
                Some(c) => value.push(c),
            }
        }
    }

    /// Lex a backtick template name, returning its raw inner text (without
    /// the backticks).  The content is left unparsed; the parser splits it
    /// into literal and `{param}` segments.
    fn lex_template(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        self.bump(); // opening backtick
        let content_start = self.pos;
        loop {
            match self.peek() {
                None | Some('\n') => {
                    return Err(LexError::new(
                        "unterminated template name (missing closing backtick)",
                        Span::new(start, self.pos),
                    ));
                }
                Some('`') => {
                    let content = self.src[content_start..self.pos].to_string();
                    self.bump(); // closing backtick
                    return Ok(TokenKind::Template(content));
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    fn lex_symbol(&mut self) -> Result<TokenKind, LexError> {
        let start = self.pos;
        let c = self.bump().expect("lex_symbol called at EOF");
        let kind = match c {
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ':' => TokenKind::Colon,
            ',' => TokenKind::Comma,
            ';' => TokenKind::Semi,
            '.' => TokenKind::Dot,
            '?' => TokenKind::Question,
            '@' => TokenKind::At,
            '|' => {
                if self.peek() == Some('>') {
                    self.bump();
                    TokenKind::PipeArrow
                } else {
                    TokenKind::Pipe
                }
            }
            '+' => TokenKind::Plus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '^' => TokenKind::Caret,
            '=' => match self.peek() {
                Some('=') => {
                    self.bump();
                    TokenKind::EqEq
                }
                Some('>') => {
                    self.bump();
                    TokenKind::FatArrow
                }
                _ => TokenKind::Eq,
            },
            '-' => {
                if self.peek() == Some('>') {
                    self.bump();
                    TokenKind::Arrow
                } else {
                    TokenKind::Minus
                }
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.bump();
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.peek() == Some('=') {
                    self.bump();
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.bump();
                    TokenKind::BangEq
                } else {
                    return Err(LexError::new(
                        "unexpected character '!' (did you mean '!='?)",
                        Span::new(start, self.pos),
                    ));
                }
            }
            other => {
                return Err(LexError::new(
                    format!("unexpected character {other:?}"),
                    Span::new(start, self.pos),
                ));
            }
        };
        Ok(kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect just the token kinds, dropping the trailing Eof and all spans.
    fn kinds(src: &str) -> Vec<TokenKind> {
        let mut toks = tokenize(src).expect("should lex");
        assert_eq!(toks.pop().map(|t| t.kind), Some(TokenKind::Eof));
        toks.into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn empty_input_is_just_eof() {
        let toks = tokenize("").unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Eof);
    }

    #[test]
    fn whitespace_and_comments_are_skipped() {
        let toks = tokenize("  // a comment\n\t  // another\n").unwrap();
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].kind, TokenKind::Eof);
    }

    #[test]
    fn device_declaration_from_iiot() {
        // device VibrationSensor { vibration: length / time^2 }
        let src = "device VibrationSensor {\n  vibration: length / time^2\n}";
        assert_eq!(
            kinds(src),
            vec![
                TokenKind::Ident("device".into()),
                TokenKind::Ident("VibrationSensor".into()),
                TokenKind::LBrace,
                TokenKind::Ident("vibration".into()),
                TokenKind::Colon,
                TokenKind::Ident("length".into()),
                TokenKind::Slash,
                TokenKind::Ident("time".into()),
                TokenKind::Caret,
                TokenKind::Int(2),
                TokenKind::RBrace,
            ]
        );
    }

    #[test]
    fn float_versus_member_access() {
        // `75.0` is a float; `input.id` keeps the dot as its own token.
        assert_eq!(kinds("75.0"), vec![TokenKind::Float(75.0)]);
        assert_eq!(
            kinds("input.id"),
            vec![
                TokenKind::Ident("input".into()),
                TokenKind::Dot,
                TokenKind::Ident("id".into()),
            ]
        );
    }

    #[test]
    fn compound_operators() {
        assert_eq!(
            kinds("== = => -> >= <= != < >"),
            vec![
                TokenKind::EqEq,
                TokenKind::Eq,
                TokenKind::FatArrow,
                TokenKind::Arrow,
                TokenKind::GtEq,
                TokenKind::LtEq,
                TokenKind::BangEq,
                TokenKind::Lt,
                TokenKind::Gt,
            ]
        );
    }

    #[test]
    fn pipe_arrow_munches_maximally() {
        // `|>` is one token; a lone `|` stays a Pipe (a lambda bar).
        assert_eq!(kinds("a |> b"), {
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::PipeArrow,
                TokenKind::Ident("b".into()),
            ]
        });
        // The closing-bar caveat: `|x|>0` glues the second bar to `>`.
        assert_eq!(
            kinds("|x|>0"),
            vec![
                TokenKind::Pipe,
                TokenKind::Ident("x".into()),
                TokenKind::PipeArrow,
                TokenKind::Int(0),
            ]
        );
        // A space splits them back into a closing bar and a `>`.
        assert_eq!(
            kinds("|x| > 0"),
            vec![
                TokenKind::Pipe,
                TokenKind::Ident("x".into()),
                TokenKind::Pipe,
                TokenKind::Gt,
                TokenKind::Int(0),
            ]
        );
    }

    #[test]
    fn string_literal_with_escapes() {
        assert_eq!(
            kinds(r#""machine_1" "a\"b\n""#),
            vec![
                TokenKind::Str("machine_1".into()),
                TokenKind::Str("a\"b\n".into()),
            ]
        );
    }

    #[test]
    fn spans_cover_the_lexeme() {
        let toks = tokenize("unit Machine").unwrap();
        assert_eq!(toks[0].span.slice("unit Machine"), "unit");
        assert_eq!(toks[1].span.slice("unit Machine"), "Machine");
    }

    #[test]
    fn unicode_identifiers_uax31() {
        // Non-ASCII XID identifiers are accepted as single Ident tokens.
        assert_eq!(kinds("máquina"), vec![TokenKind::Ident("máquina".into())]);
        assert_eq!(kinds("温度"), vec![TokenKind::Ident("温度".into())]);
        assert_eq!(
            kinds("_private leading_underscore"),
            vec![
                TokenKind::Ident("_private".into()),
                TokenKind::Ident("leading_underscore".into()),
            ]
        );
    }

    #[test]
    fn superscript_does_not_glue_onto_identifiers() {
        // `²` (category No) is not XID_Continue, so `time²` is not one ident;
        // the stray superscript can't start a token either, so it errors.
        let err = tokenize("time²").unwrap_err();
        assert!(err.message.contains("unexpected character"));
        // And the canonical unit syntax stays three tokens.
        assert_eq!(
            kinds("time^2"),
            vec![
                TokenKind::Ident("time".into()),
                TokenKind::Caret,
                TokenKind::Int(2),
            ]
        );
    }

    #[test]
    fn unterminated_string_is_an_error() {
        let err = tokenize("\"oops").unwrap_err();
        assert!(err.message.contains("unterminated"));
    }

    #[test]
    fn template_name_is_one_token() {
        assert_eq!(
            kinds("`{col}_z`"),
            vec![TokenKind::Template("{col}_z".into())]
        );
        // Whitespace inside is preserved verbatim (the parser validates it).
        assert_eq!(kinds("`a b`"), vec![TokenKind::Template("a b".into())]);
    }

    #[test]
    fn unterminated_template_is_an_error() {
        let err = tokenize("`{col}").unwrap_err();
        assert!(err.message.contains("unterminated template"));
        let err = tokenize("`oops\n`").unwrap_err();
        assert!(err.message.contains("unterminated template"));
    }

    #[test]
    fn unexpected_character_is_an_error() {
        let err = tokenize("a # b").unwrap_err();
        assert!(err.message.contains("unexpected character"));
    }
}
