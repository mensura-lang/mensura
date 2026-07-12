//! The runtime value model (`docs/toolkit/04-processing-layer.md`).
//!
//! The evaluator and the storage boundary share one representation of typed
//! rows, mirroring the boundary IR's `ColumnType` (ADR 0014).  A [`Row`] is
//! positional: its values follow the table's column order (key columns
//! first, then attributes).

/// A single runtime value.  `Missing` is the runtime image of ADR 0010's
/// optional (`?`) values and round-trips with SQL `NULL`; the checker
/// guarantees a total column never holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    String(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    /// ISO 8601, as stored.
    Date(String),
    /// The variant literal of a named enum.
    Enum(String),
    Missing,
}

/// One table row: values in the table's column order.
pub type Row = Vec<Value>;
