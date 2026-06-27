//! Syntax layer for the Mensura language: lexer, AST, and the hand-written
//! recursive-descent, LL(1) parser (`docs/language/04-grammar.md`).
//!
//! The parser covers the declaration subset (`unit`, `store`, `shape`, `enum`)
//! through [`parse`], and the expression sublanguage through [`parse_expr`].

pub mod ast;
pub mod expr;
pub mod lexer;
pub mod parser;
pub mod token;

pub use ast::{
    DomainEntry, EnumDecl, Field, Ident, Item, NameSeg, NameTemplate, Program, ShapeArg, ShapeDecl,
    ShapeParam, ShapeRef, StoreDecl, StrLit, TypeExpr, UnitDecl, ViewDecl,
};
pub use expr::{BinOp, Block, Expr, ExprKind, FieldRole, Presence, RecordField, Stmt, UnOp};
pub use lexer::{LexError, Lexed, is_identifier, lex, tokenize};
pub use parser::{ParseError, Parsed, parse, parse_expr, parse_with_meta};
pub use token::{Span, Token, TokenKind, Trivia, TriviaKind};
