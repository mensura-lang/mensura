//! The resolved schema model.
//!
//! This is the boundary IR between the front end (lexer, parser, resolver)
//! and the runtime.  A [`Schema`] is one store flattened into an ordered list
//! of typed columns; it carries no AST and no syntax.

use mensura_syntax::Span;

/// A resolved store: its name, the unit it tabulates, and its columns in
/// storage order (index fields, then `const`, then `var`).
#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub store: String,
    pub unit: String,
    pub columns: Vec<Column>,
    pub span: Span,
}

/// One column of a [`Schema`].
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub name: String,
    pub ty: ColumnType,
    pub role: ColumnRole,
    /// Value totality (ADR 0010): `false` is total (every value known, the
    /// default), `true` is optional (the value may be missing, written with a
    /// trailing `?`).  Orthogonal to cardinality; index columns are never
    /// optional.
    pub optional: bool,
    pub span: Span,
}

/// Where a column comes from, which fixes its storage semantics: index
/// columns form the primary key, `const`/`var` carry data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnRole {
    Index,
    Const,
    Var,
}

/// The resolved type of a column in this subset of the language.
#[derive(Clone, Debug, PartialEq)]
pub enum ColumnType {
    String,
    Number,
    Bool,
    Date,
    /// A named enumerated type: its declared name and its string variants.
    Enum {
        name: String,
        variants: Vec<String>,
    },
}
