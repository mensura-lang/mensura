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
    Enum(EnumDecl),
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

/// `enum Name { "variant", ... }`
///
/// A named enumerated type: a fixed set of string-literal variants, referenced
/// by name in a field's type position.  Its name is a type (PascalCase); its
/// variants are unconstrained string literals.
#[derive(Clone, Debug, PartialEq)]
pub struct EnumDecl {
    pub name: Ident,
    pub variants: Vec<StrLit>,
    pub span: Span,
}

/// A `name: type` pair: a unit index field, or a `const`/`var` attribute of a
/// store or shape.  The name may be backtick-quoted and, in a shape, may
/// interpolate `string` parameters; a plain identifier is a single literal
/// [`NameSeg`].
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: NameTemplate,
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
/// arguments, e.g. `Tabular[Person]`, `Ageable["birthdate"]`, or the
/// parameter-free `PersonRecord`.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeRef {
    pub name: Ident,
    /// Positional arguments, matched to parameters by position.
    pub args: Vec<ShapeArg>,
    pub span: Span,
}

/// One positional argument in a conformance reference.  Its form picks the
/// parameter kind it can fill: a bare identifier for a `Unit` parameter, a
/// string literal for a `string` parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeArg {
    Unit(Ident),
    Str(StrLit),
}

impl ShapeArg {
    pub fn span(&self) -> Span {
        match self {
            ShapeArg::Unit(id) => id.span,
            ShapeArg::Str(s) => s.span,
        }
    }
}

/// `shape Name [[params]] { [unit { U }] (const|var block)* }`
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

/// An attribute name as literal text with optional `{param}` holes.  A plain
/// identifier is a single [`NameSeg::Lit`] segment.
#[derive(Clone, Debug, PartialEq)]
pub struct NameTemplate {
    pub segments: Vec<NameSeg>,
    pub span: Span,
}

impl NameTemplate {
    /// The name as a plain string when it has no interpolation, else `None`.
    pub fn as_literal(&self) -> Option<&str> {
        match self.segments.as_slice() {
            [NameSeg::Lit(s)] => Some(s),
            _ => None,
        }
    }
}

/// One piece of a [`NameTemplate`]: fixed text or an interpolated parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum NameSeg {
    Lit(String),
    Param(Ident),
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
///
/// A primitive name (`string`, `number`, ...), a unit reference, or a named
/// `enum` type.  All three are a single identifier in type position; the
/// resolver decides which it is.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeExpr {
    Named(Ident),
}

impl TypeExpr {
    /// The source span of the whole type expression.
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named(id) => id.span,
        }
    }
}
