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
    ShapeParam, ShapeRef, StoreDecl, StrLit, TypeExpr, UnitDecl,
};
pub use expr::{BinOp, Block, Expr, ExprKind, Presence, RecordField, Stmt, UnOp};
pub use lexer::{LexError, is_identifier, tokenize};
pub use parser::{ParseError, parse, parse_expr};
pub use token::{Span, Token, TokenKind};
