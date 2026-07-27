//! The resolved schema model.
//!
//! This is the boundary IR between the front end (lexer, parser, resolver)
//! and the runtime.  A [`Schema`] is one store flattened into an ordered list
//! of typed columns; a [`ViewPlan`] is one view's computed output plus its
//! checked body, ready for the processing layer to evaluate
//! (`docs/toolkit/04-processing-layer.md`).

use mensura_syntax::{Block, Span};

use crate::table::Cardinality;

/// A resolved store: its name, the unit it tabulates, its columns in
/// storage order (key fields, then attributes in declaration order), and
/// its declared cardinality.
#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub store: String,
    pub unit: String,
    pub columns: Vec<Column>,
    /// The declared store cardinality (ADR 0022): `Singletons` (`attr`
    /// blocks, the ADR 0001 default, `card <= 1` over the key) or `Bag`
    /// (`attr*` blocks, many observations per entity, `card >= 0`).
    pub cardinality: Cardinality,
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
    /// trailing `?`).  Orthogonal to cardinality; key columns are never
    /// optional.
    pub optional: bool,
    pub span: Span,
}

/// Where a column comes from, which fixes its storage semantics: key
/// columns form the primary key, attribute columns carry data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnRole {
    Key,
    Attr,
}

/// The resolved type (scalar domain) of a column (ADR 0014).  `number` is split
/// into `int` (discrete, exact, equality-stable) and `real` (a continuous
/// measurement); a dimensioned measurement is `real` refined by a physical
/// dimension (ADR 0026, `docs/language/11-physical-units.md`).
#[derive(Clone, Debug, PartialEq)]
pub enum ColumnType {
    String,
    /// A discrete whole number: a count, a year, an integer identifier.
    /// Never dimensioned (ADR 0014).
    Int,
    /// A dimensionless continuous measurement.  Not equatable and not
    /// key-eligible.
    Real,
    /// A dimensioned continuous measurement: a `real` backing refined by a
    /// physical dimension (`temperature[real]`).  The invariant that the
    /// dimension is never the group identity (which collapses to [`Real`])
    /// is kept by [`crate::units::Dimension::applied`].
    Quantity(crate::units::Dimension),
    Bool,
    Date,
    /// A named enumerated type: its declared name and its string variants.
    Enum {
        name: String,
        variants: Vec<String>,
    },
}

impl ColumnType {
    /// Supports `== !=` (ADR 0014).  Every domain except the `real`-backed
    /// ones: exact equality on a continuous measurement is unsound.
    pub fn is_equatable(&self) -> bool {
        !matches!(self, ColumnType::Real | ColumnType::Quantity(_))
    }

    /// Has a total order, supporting `< <= > >=` and `min`/`max`: `int`, the
    /// `real`-backed domains, `date`.
    pub fn is_orderable(&self) -> bool {
        matches!(
            self,
            ColumnType::Int | ColumnType::Real | ColumnType::Quantity(_) | ColumnType::Date
        )
    }

    /// Supports arithmetic and `sum`: `int` and the `real`-backed domains.
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            ColumnType::Int | ColumnType::Real | ColumnType::Quantity(_)
        )
    }

    /// The physical dimension of a `real`-backed domain: the group identity
    /// for bare `real`, the carried dimension for a quantity, and `None`
    /// for every other domain (ADR 0026).
    pub fn dimension(&self) -> Option<crate::units::Dimension> {
        match self {
            ColumnType::Real => Some(crate::units::Dimension::DIMENSIONLESS),
            ColumnType::Quantity(d) => Some(*d),
            _ => None,
        }
    }

    /// Listable values, so it can be spread across column names (key `pivot`,
    /// `unpivot`): `enum` only.
    pub fn is_enumerable(&self) -> bool {
        matches!(self, ColumnType::Enum { .. })
    }

    /// May form a key.  A key is identified by equality, so
    /// key-eligibility is exactly equatability (ADR 0014); `real` is excluded.
    pub fn is_key_eligible(&self) -> bool {
        self.is_equatable()
    }
}

/// A whole resolved program: the boundary between the front end and the
/// runtime.  Stores become tables ([`Schema`]); views become batch
/// materializations ([`ViewPlan`], `docs/toolkit/04-processing-layer.md`).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedProgram {
    pub schemas: Vec<Schema>,
    pub views: Vec<ViewPlan>,
}

/// A resolved view, ready for the processing layer: its computed output
/// shape (read off the checked table type) and the checked body the runtime
/// evaluates.  The body is the view's block AST; the checker has already
/// established it well-typed, so evaluation cannot fail on shape.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewPlan {
    pub name: String,
    /// Output columns in storage order: the key columns, then the computed
    /// attribute columns in the order the checker produced them.
    pub columns: Vec<Column>,
    /// The computed cardinality.  `Singletons` gets the composite primary
    /// key over the key columns; a `Bag` view gets none.
    pub cardinality: Cardinality,
    /// The checked view body (`10-views.md`: `let` bindings, then a trailing
    /// table expression).
    pub body: Block,
    /// The stores the body reads, by name.
    pub sources: Vec<String>,
    pub span: Span,
}

/// The table-level shape stores and views share at the storage boundary:
/// what `scan` reads and `materialize_view` writes
/// (`docs/toolkit/04-processing-layer.md`).
#[derive(Clone, Debug, PartialEq)]
pub struct TableShape {
    pub name: String,
    /// Columns in storage order (key first, then attributes).
    pub columns: Vec<Column>,
    /// Whether rows are 0-or-1 per key tuple: `true` maps to a composite
    /// primary key over the key columns, `false` (a `bag` view) to none.
    pub keyed: bool,
}

impl Schema {
    /// This store's storage shape.  A `singletons` store is a 0-or-1 unit
    /// tabulation (ADR 0001), so it is keyed; a `bag` store (ADR 0022) holds
    /// many rows per key, so it maps to an unkeyed table (the backend adds a
    /// non-unique covering index over the key columns instead).
    pub fn shape(&self) -> TableShape {
        TableShape {
            name: self.store.clone(),
            columns: self.columns.clone(),
            keyed: self.cardinality == Cardinality::Singletons,
        }
    }
}

impl ViewPlan {
    /// This view's storage shape; the primary key follows the computed
    /// cardinality.
    pub fn shape(&self) -> TableShape {
        TableShape {
            name: self.name.clone(),
            columns: self.columns.clone(),
            keyed: self.cardinality == Cardinality::Singletons,
        }
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
