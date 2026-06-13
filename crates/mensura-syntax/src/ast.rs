//! The abstract syntax tree for the unit and store subset.
//!
//! The shapes here mirror the grammar in `docs/language/04-grammar.md`.
//! Every node carries a [`Span`] so later passes can point diagnostics at the
//! source.

use crate::token::Span;

/// A whole parsed source file: a sequence of top-level items.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub items: Vec<Item>,
}

/// A top-level declaration.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Unit(UnitDecl),
    Store(StoreDecl),
}

/// An identifier together with where it appeared.
#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// A string literal: its already-unescaped value and where it appeared.
#[derive(Clone, Debug, PartialEq)]
pub struct StrLit {
    pub value: String,
    pub span: Span,
}

/// `unit Name { field* }`
#[derive(Clone, Debug, PartialEq)]
pub struct UnitDecl {
    pub name: Ident,
    pub fields: Vec<Field>,
    pub span: Span,
}

/// A `name: type` pair, used for both unit index fields and store attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `store Name { unit { U } (const|var|domain block)* }`
#[derive(Clone, Debug, PartialEq)]
pub struct StoreDecl {
    pub name: Ident,
    /// The unit named by the `unit { U }` clause.
    pub unit: Ident,
    pub consts: Vec<Field>,
    pub vars: Vec<Field>,
    pub domain: Vec<DomainEntry>,
    pub span: Span,
}

/// One `field: Store` line inside a `domain { ... }` block.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainEntry {
    /// The unit-reference field being resolved.
    pub field: Ident,
    /// The store it resolves into.
    pub store: Ident,
    pub span: Span,
}

/// A type expression in a field or attribute.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeExpr {
    /// A primitive name (`string`, `number`, ...) or a unit reference.
    Named(Ident),
    /// `enum("a", "b", ...)` with one or more string-literal variants.
    Enum { variants: Vec<StrLit>, span: Span },
}

impl TypeExpr {
    /// The source span of the whole type expression.
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named(id) => id.span,
            TypeExpr::Enum { span, .. } => *span,
        }
    }
}
