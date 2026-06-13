//! Syntax layer for the Mensura language: lexer, and (later) parser and AST.
//!
//! At this stage only the lexer exists.  See `ROADMAP.md` (M1) for the
//! planned recursive-descent, LL(1) parser that will consume these tokens.

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod token;

pub use ast::{DomainEntry, Field, Ident, Item, Program, StoreDecl, StrLit, TypeExpr, UnitDecl};
pub use lexer::{LexError, tokenize};
pub use parser::{ParseError, parse};
pub use token::{Span, Token, TokenKind};
