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
    /// ISO 8601 (`YYYY-MM-DD`), as stored.
    Date(String),
    /// RFC 3339 in the normalized fixed-width UTC form
    /// `YYYY-MM-DDTHH:MM:SS.sssZ` (ADR 0036 decision 7): one zone and one
    /// width, so lexicographic order is chronological order.
    Instant(String),
    /// The variant literal of a named enum.
    Enum(String),
    Missing,
}

impl Value {
    /// This value as a grain key (ADR 0041 decision 2): the text a
    /// watermark map is keyed by.
    ///
    /// Injective where it is used, because a grain column is a key column,
    /// hence equatable and never `real` (the domain matrix of
    /// `09-typing-reference.md`).  Shared by the intake, which builds the
    /// map, and the evaluator, which reads it, so the two cannot disagree
    /// on what names a grain.
    pub fn grain_key(&self) -> String {
        match self {
            Value::String(s) | Value::Date(s) | Value::Instant(s) | Value::Enum(s) => s.clone(),
            Value::Int(n) => n.to_string(),
            Value::Real(x) => x.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Missing => "(missing)".to_string(),
        }
    }
}

/// One table row: values in the table's column order.
pub type Row = Vec<Value>;
