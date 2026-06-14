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
    Shape(ShapeDecl),
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

/// `store Name [: ShapeRef, ...] { unit { U } (const|var|domain block)* }`
#[derive(Clone, Debug, PartialEq)]
pub struct StoreDecl {
    pub name: Ident,
    /// The unit named by the `unit { U }` clause.
    pub unit: Ident,
    /// The shapes claimed by the `:` conformance clause, in source order.
    pub conforms: Vec<ShapeRef>,
    pub consts: Vec<Field>,
    pub vars: Vec<Field>,
    pub domain: Vec<DomainEntry>,
    pub span: Span,
}

/// One entry in a `:` conformance clause: a shape name with positional
/// arguments, e.g. `Tabular(Person)` or the parameter-free `PersonRecord`.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeRef {
    pub name: Ident,
    /// Positional arguments; bare unit names for `Unit` parameters.
    pub args: Vec<Ident>,
    pub span: Span,
}

/// `shape Name [(params)] { [unit { U }] (const|var block)* }`
///
/// A structural contract: an optional unit plus the attributes a conforming
/// store must carry.  Shapes hold no `domain` block, no policy, and no
/// storage; see `docs/language/03-shapes.md`.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeDecl {
    pub name: Ident,
    /// Parameters in source order; their kind is the annotation `Ident`.
    pub params: Vec<ShapeParam>,
    /// The unit named by the `unit { U }` clause, if any.  `None` is a
    /// unit-agnostic shape.
    pub unit: Option<Ident>,
    pub consts: Vec<Field>,
    pub vars: Vec<Field>,
    pub span: Span,
}

/// A shape parameter `name: Kind`.  The parser leaves `kind` as the raw
/// annotation identifier (`Unit`, `string`, ...); the resolver gives it
/// meaning.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeParam {
    pub name: Ident,
    pub kind: Ident,
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
