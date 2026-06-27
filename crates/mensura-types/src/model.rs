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

/// The resolved type (scalar domain) of a column (ADR 0014).  `number` is split
/// into `int` (discrete, exact, equality-stable) and `real` (a continuous
/// measurement).
#[derive(Clone, Debug, PartialEq)]
pub enum ColumnType {
    String,
    /// A discrete whole number: a count, a year, an integer identifier.
    Int,
    /// A continuous measurement.  Not equatable and not key-eligible.
    Real,
    Bool,
    Date,
    /// A named enumerated type: its declared name and its string variants.
    Enum {
        name: String,
        variants: Vec<String>,
    },
}

impl ColumnType {
    /// Supports `== !=` (ADR 0014).  Every domain except `real`: exact equality
    /// on a continuous measurement is unsound.
    pub fn is_equatable(&self) -> bool {
        !matches!(self, ColumnType::Real)
    }

    /// Has a total order, supporting `< <= > >=` and `min`/`max`: `int`, `real`,
    /// `date`.
    pub fn is_orderable(&self) -> bool {
        matches!(self, ColumnType::Int | ColumnType::Real | ColumnType::Date)
    }

    /// Supports arithmetic and `sum`: `int`, `real`.
    pub fn is_numeric(&self) -> bool {
        matches!(self, ColumnType::Int | ColumnType::Real)
    }

    /// Listable values, so it can be spread across column names (index `pivot`,
    /// `unpivot`): `enum` only.
    pub fn is_enumerable(&self) -> bool {
        matches!(self, ColumnType::Enum { .. })
    }

    /// May form an index/key.  A key is identified by equality, so
    /// key-eligibility is exactly equatability (ADR 0014); `real` is excluded.
    pub fn is_key_eligible(&self) -> bool {
        self.is_equatable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_properties_follow_the_matrix() {
        let enum_t = ColumnType::Enum {
            name: "S".into(),
            variants: vec!["a".into()],
        };
        // real: orderable + numeric, but not equatable, so not key-eligible.
        assert!(ColumnType::Real.is_orderable() && ColumnType::Real.is_numeric());
        assert!(!ColumnType::Real.is_equatable() && !ColumnType::Real.is_key_eligible());
        // int: numeric, orderable, equatable, key-eligible.
        assert!(ColumnType::Int.is_numeric() && ColumnType::Int.is_orderable());
        assert!(ColumnType::Int.is_key_eligible());
        // date: orderable, not numeric, key-eligible.
        assert!(ColumnType::Date.is_orderable() && !ColumnType::Date.is_numeric());
        assert!(ColumnType::Date.is_key_eligible());
        // string: equatable, not orderable.
        assert!(ColumnType::String.is_equatable() && !ColumnType::String.is_orderable());
        // enum: the only finite-enumerable domain; key-eligible.
        assert!(enum_t.is_enumerable() && enum_t.is_key_eligible());
        // bool: key-eligible but not spreadable.
        assert!(ColumnType::Bool.is_key_eligible() && !ColumnType::Bool.is_enumerable());
    }
}
