//! The abstract syntax tree for the expression sublanguage.
//!
//! Mensura has one expression language, used at every site that evaluates an
//! expression (`when:`, `where:`, `@auto(...)`, and the pipeline operations);
//! see `docs/language/06-expressions.md` and its grammar in
//! `docs/language/04-grammar.md`.  This module is the parsed shape only; the
//! meaning (context, result type, cardinality) is a later, per-site concern.
//!
//! Every node carries a [`Span`] so later passes can locate diagnostics.

use crate::ast::{Ident, TypeExpr};
use crate::token::Span;

/// An expression: a [`ExprKind`] tagged with its source span.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

/// The shape of an expression node.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// An integer literal: `42`.
    Int(i64),
    /// A real literal: `3.14`.
    Float(f64),
    /// A string literal, already unescaped: `"text"`.
    Str(String),
    /// A boolean literal: `true` / `false`.
    Bool(bool),
    /// A bare name resolved against the site's context: `machine`.
    Name(String),
    /// A backtick-quoted combiner: `` `+` ``, `` `<<` ``, `` `:>` `` (ADR 0031,
    /// Decision 6).  It carries the raw inner text; resolution against the
    /// closed combiner table (and the table's per-operation admissibility
    /// rules) is the checker's job, so the parser stays purely syntactic and
    /// an unknown combiner gets a *typing* diagnostic naming the table rather
    /// than a parse error.
    Combiner(String),
    /// Member access `a.b`, the tightest-binding postfix.
    Member(Box<Expr>, Ident),
    /// Function application by juxtaposition: `f x`, left-associative.
    App(Box<Expr>, Box<Expr>),
    /// A prefix operator: `not e`, unary `-e`.
    Unary(UnOp, Box<Expr>),
    /// An infix operator, including the `|>` pipe and the comparisons.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// A presence test on a value: `e is known` / `e is missing`.
    Presence(Box<Expr>, Presence),
    /// An anonymous function: `|x| e`, `|x, y| e`, `|x| : T e`.
    Lambda {
        params: Vec<Ident>,
        ret: Option<TypeExpr>,
        body: Box<Expr>,
    },
    /// A positional tuple `(a, b, ...)`.  The empty tuple is `()`; a single
    /// parenthesized expression `(e)` is grouping and is *not* represented as
    /// a one-element tuple (it reduces to `e`).
    Tuple(Vec<Expr>),
    /// A labeled record `(.a = x, .b : T = y)`.
    Record(Vec<RecordField>),
    /// A statement block `{ let ...; assert ...; result }`.
    Block(Block),
    /// A conditional `if c then a else b` (ADR 0015).
    If {
        cond: Box<Expr>,
        then: Box<Expr>,
        els: Box<Expr>,
    },
}

/// A prefix operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// `not`
    Not,
    /// unary `-`
    Neg,
    /// `#`, cardinality (ADR 0031, Decision 9).  Its operand is a member
    /// access, so `#b.x` is `#(b.x)`; it binds tighter than the comparisons,
    /// so `#b > 3` reads as expected.
    Card,
}

/// An infix operator.  Listed loosest-binding first; precedence is fixed by
/// the parser, not by this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `|>`
    Pipe,
    /// `or`
    Or,
    /// `and`
    And,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `in`, bag membership
    In,
    /// `<<`, binary minimum (ADR 0031, Decision 6).  Looser than `+ -`,
    /// tighter than the comparisons.
    Min,
    /// `>>`, binary maximum.
    Max,
    /// `<:`, keep-left: `a <: b` is `a`.  Trivial as a scalar operator by
    /// design; its habitat is the backticked combiner slot, where it and
    /// `:>` are what make the ordered reductions scan-derivable.
    KeepLeft,
    /// `:>`, keep-right: `a :> b` is `b`.
    KeepRight,
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `^`, right-associative
    Pow,
}

/// The two presence tests: `is known` and `is missing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Known,
    Missing,
}

/// One labeled field of a record: `.name [: Type] = value`.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordField {
    pub name: Ident,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

/// A statement block: zero or more statements separated by `;`, where a final
/// bare expression is the block's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

/// One statement inside a [`Block`].
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    /// `let name [: Type] = value`.
    Let {
        name: Ident,
        ty: Option<TypeExpr>,
        value: Expr,
    },
    /// `assert expr`.
    Assert(Expr),
    /// A bare expression; as the last statement it is the block's result.
    Expr(Expr),
}
