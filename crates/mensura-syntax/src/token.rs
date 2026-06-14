//! Tokens and source spans.
//!
//! The lexer deliberately knows no keywords.  Words like `unit`, `device`,
//! `from`, `by`, `is`, and `and` are all lexed as [`TokenKind::Ident`]; which
//! of them are keywords is a decision left to the parser.  This keeps the
//! keyword set an open, parser-level concern rather than baking it into the
//! token stream before the grammar is settled.

use std::fmt;

/// A half-open byte range `[start, end)` into the source string.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    /// The slice of `src` this span covers.
    pub fn slice<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// The lexical category of a token, carrying literal payloads inline.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    /// An identifier or a (contextual) keyword: `unit`, `Machine`, `from`.
    Ident(String),
    /// An integer literal: `750`, `64`.
    Int(i64),
    /// A floating-point literal: `75.0`.
    Float(f64),
    /// A string literal, already unescaped: `"machine_1"`.
    Str(String),

    // Brackets.
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,

    // Punctuation.
    Colon,
    Comma,
    Semi,
    Dot,
    Question,
    At,
    Pipe,

    // Operators.
    Eq,
    EqEq,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    /// `->`
    Arrow,
    /// `=>`
    FatArrow,

    /// End of input.
    Eof,
}

/// A token: a [`TokenKind`] tagged with its [`Span`] in the source.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Token { kind, span }
    }
}

/// The lexical category of a piece of trivia.
///
/// Trivia is text that is meaningful to a reader and to tooling but not to the
/// grammar: comments today, and only line comments so far.  It rides a
/// separate channel from the token stream (see [`crate::lexer::Lexed`]) so the
/// parser never has to step over it.  See ADR 0005.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriviaKind {
    /// A `//` line comment, up to but not including the newline.
    LineComment,
    // DocComment (`///`, `//!`) is reserved for a later feature.
}

/// A piece of trivia: a [`TriviaKind`] tagged with its [`Span`] in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trivia {
    pub kind: TriviaKind,
    pub span: Span,
}

impl Trivia {
    pub fn new(kind: TriviaKind, span: Span) -> Self {
        Trivia { kind, span }
    }
}
