//! The abstract syntax tree for the unit and store subset.
//!
//! The shapes here mirror the grammar in `docs/language/04-grammar.md`.
//! Every node carries a [`Span`] so later passes can point diagnostics at the
//! source.

use crate::expr::Block;
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
    /// A `registry`: the same declaration as a `store`, differing only in
    /// its intake discipline and hence its completeness (ADR 0033).  It
    /// reuses [`StoreDecl`], whose `kind` records which one it is.
    Registry(StoreDecl),
    Shape(ShapeDecl),
    Enum(EnumDecl),
    View(ViewDecl),
    Import(ImportDecl),
    Let(LetDecl),
}

/// `import name`: a qualified, bundled-only module import
/// (`docs/language/12-modules-and-imports.md`, ADR 0027).
#[derive(Clone, Debug, PartialEq)]
pub struct ImportDecl {
    pub name: Ident,
    pub span: Span,
}

/// A top-level `let` binding (`docs/language/12-modules-and-imports.md`,
/// ADR 0027, Decision 1 as revised).  Like every other item body, a `let`
/// body is brace-closed, so item boundaries stay independent of the
/// expression grammar.  The kind is decided by the token after the name:
/// `[` opens the parameter list of a type-level dimension alias (ADR
/// 0026, Decision 8); `:` (an ascription) or `{` continues a const value
/// binding.
#[derive(Clone, Debug, PartialEq)]
pub struct LetDecl {
    pub name: Ident,
    pub kind: LetKind,
    pub span: Span,
}

/// The two kinds a top-level `let` binds.
#[derive(Clone, Debug, PartialEq)]
pub enum LetKind {
    /// `let name [: type] { block }`: an immutable, pure const value.  The
    /// body is the ordinary statement block; the const evaluator bounds
    /// what it may compute.
    Value {
        ty: Option<TypeExpr>,
        value: crate::expr::Block,
    },
    /// `let name[T, ...] { <type-level expr> }`: a dimension alias, whose
    /// braced body is parsed with the type grammar.
    DimAlias { params: Vec<Ident>, body: TypeExpr },
}

/// `view name [: Shape, ...] { ...block... }` (`docs/language/10-views.md`).
/// The body is the ordinary statement block of `06-expressions.md`; it hosts a
/// pipeline whose trailing expression is the materialized table.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewDecl {
    pub name: Ident,
    /// The shapes claimed by the `:` conformance clause, in source order.
    pub conforms: Vec<ShapeRef>,
    pub body: Block,
    pub span: Span,
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

/// `enum Name { "variant" ... }`
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

/// A `name: type` pair: a unit key field, or an `attr` attribute of a
/// store or shape.  The name may be backtick-quoted and, in a shape, may
/// interpolate `string` parameters; a plain identifier is a single literal
/// [`NameSeg`].
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: NameTemplate,
    pub ty: TypeExpr,
    pub span: Span,
}

/// One attribute of a store or shape: the `name: type` field plus its
/// declared cardinality (`attr` versus `attr*`, ADR 0022).
#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub field: Field,
    /// The span of the `*` when the attribute came from an `attr*` block:
    /// the column is bag-valued (many observations per key).  `None` is a
    /// plain `attr` (singleton) column.
    pub many: Option<Span>,
}

/// Which kind of tabulation a [`StoreDecl`] declares (ADR 0033).  The two
/// share a grammar, a resolved model, and a storage mapping; they differ in
/// their intake discipline, and hence in whether the resolved table is
/// complete by mechanism.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StoreKind {
    /// A `store`: written by create, update, and delete, so it holds some
    /// of the observations that exist and its table is incomplete.
    #[default]
    Store,
    /// A `registry`: the declaration is the sole intake for its
    /// observations and the intake only appends, which establishes
    /// completeness at the type level.
    Registry,
}

impl StoreKind {
    /// The keyword that introduces this kind, for diagnostics.
    pub fn keyword(self) -> &'static str {
        match self {
            StoreKind::Store => "store",
            StoreKind::Registry => "registry",
        }
    }
}

/// `store Name [: ShapeRef, ...] { unit { U } (attr|domain block)* }`, and
/// the identical `registry` form (ADR 0033); `kind` says which.
#[derive(Clone, Debug, PartialEq)]
pub struct StoreDecl {
    pub kind: StoreKind,
    pub name: Ident,
    /// The unit named by the `unit { U }` clause.
    pub unit: Ident,
    /// The shapes claimed by the `:` conformance clause, in source order.
    pub conforms: Vec<ShapeRef>,
    /// The attributes of all `attr` and `attr*` blocks, merged in source
    /// order.
    pub attrs: Vec<Attr>,
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

/// `shape Name [[params]] { [unit { U }] (attr block)* }`
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
    /// The attributes of all `attr` and `attr*` blocks, merged in source
    /// order.
    pub attrs: Vec<Attr>,
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

/// A type expression in a field, attribute, ascription, or alias body.
///
/// The common case is a single identifier: a primitive name (`string`,
/// `real`, ...), a unit reference, or a named `enum`; the resolver decides
/// which it is.  The remaining forms are the type-level expression grammar
/// of `docs/language/11-physical-units.md` (ADR 0026): a dimension (or
/// alias) applied to a backing (`temperature[real]`), and `*`/`/`/`^`
/// combinations of those.  A trailing `?` on the whole type marks the
/// value optional (it may be missing in an observed row); see ADR 0010 and
/// `docs/language/02-stores.md`.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeExpr {
    pub kind: TypeKind,
    /// The span of the trailing `?` optional marker, if present.  `None` means
    /// the value is total (known in every observed row, the default).  Only
    /// the outermost type of a field carries it; nested nodes are `None`.
    pub optional: Option<Span>,
    /// The source span of the whole type expression, covering the `?` if any.
    pub span: Span,
}

/// The shape of a type-level expression node
/// (`04-grammar.md`, `tl_expr`).
#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    /// A lone identifier: a primitive, a unit reference, a named `enum`, a
    /// base dimension, a dimension alias, or an alias parameter; the
    /// resolver decides which.
    Named(Ident),
    /// `base[backing]`: type-level application.  The base is a named
    /// dimension or alias, or a parenthesized type-level expression; the
    /// backing is a single identifier (`real`, or an alias parameter).
    Apply { base: Box<TypeExpr>, backing: Ident },
    /// `a * b` at the type level (dimension product).
    Mul(Box<TypeExpr>, Box<TypeExpr>),
    /// `a / b` at the type level (dimension quotient).
    Div(Box<TypeExpr>, Box<TypeExpr>),
    /// `a ^ n` at the type level: an integer-literal exponent, optionally
    /// negated.
    Pow(Box<TypeExpr>, i32),
}

impl TypeExpr {
    /// The source span of the whole type expression.
    pub fn span(&self) -> Span {
        self.span
    }

    /// Whether the value may be missing (`?` was written).
    pub fn is_optional(&self) -> bool {
        self.optional.is_some()
    }

    /// The lone identifier when this type is a plain named type
    /// (`string`, `MyEnum`, ...), else `None`.
    pub fn named(&self) -> Option<&Ident> {
        match &self.kind {
            TypeKind::Named(id) => Some(id),
            _ => None,
        }
    }
}
