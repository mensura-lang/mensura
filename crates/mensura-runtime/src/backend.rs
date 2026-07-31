//! The storage abstraction.
//!
//! The toolchain talks to storage only through [`StorageBackend`], so the SQL
//! dialect never leaks into the rest of the compiler and other backends can
//! be added later.  See `docs/toolkit/03-storage-backend.md`.

use std::fmt;

use mensura_types::{Schema, TableShape};

use crate::value::Row;

/// A persistent place where stores and views are materialized.
pub trait StorageBackend {
    /// Ensure the store's table exists, creating it if absent.
    fn ensure_store(&mut self, schema: &Schema) -> Result<EnsureOutcome, StorageError>;

    /// Read a table's current rows, decoded to typed values in column order
    /// (`docs/toolkit/04-processing-layer.md`).  Rows come back ordered by
    /// the key columns (with a storage-level tiebreak within a key on an
    /// unkeyed table), so a scan is deterministic.
    fn scan(&self, table: &TableShape) -> Result<Vec<Row>, StorageError>;

    /// Ensure the view's table exists and replace its contents with `rows`
    /// in one transaction: readers see the previous materialization or the
    /// new one, never a partial state.
    fn materialize_view(&mut self, view: &TableShape, rows: &[Row]) -> Result<(), StorageError>;

    /// Apply a batch of row changes to a table in one transaction: every
    /// change lands or none does (`docs/toolkit/05-ingestion.md`, ADR 0034).
    fn apply(&mut self, table: &TableShape, delta: &Delta) -> Result<Applied, StorageError>;
}

/// A batch of row changes against one table (ADR 0034 decision 2).
///
/// Insert and delete lists rather than DBSP's `(row, weight)` Z-set
/// encoding: weights earn their keep inside a circuit, and no circuit exists
/// until M5.  The two convert in a few lines (an insert is weight `+1`, a
/// delete `-1`), so adopting DBSP widens this representation rather than
/// replacing the interface (`docs/toolkit/04-processing-layer.md`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delta {
    pub inserts: Vec<Row>,
    pub deletes: Vec<Row>,
}

impl Delta {
    /// The append-only delta a registry's intake produces (ADR 0033).
    pub fn appending(rows: Vec<Row>) -> Delta {
        Delta {
            inserts: rows,
            deletes: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inserts.is_empty() && self.deletes.is_empty()
    }
}

/// What [`StorageBackend::apply`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub inserted: usize,
    pub deleted: usize,
}

/// What [`StorageBackend::ensure_store`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnsureOutcome {
    /// The table did not exist and was created.
    Created,
    /// The table already existed; nothing was changed.
    AlreadyExists,
}

/// A storage-layer failure.
#[derive(Debug)]
pub enum StorageError {
    Sqlite(rusqlite::Error),
    /// A stored value did not decode to its column's declared type.
    Decode(String),
    /// A write violated a declared `domain` reference (ADR 0034 decision 5).
    /// Reported against the declaration rather than as a raw SQLite
    /// constraint code, so the message names what the program said.
    ForeignKey {
        table: String,
        /// The `domain` entries the table declares, as
        /// `field -> target` pairs, since SQLite does not say which clause
        /// failed.
        references: Vec<(String, String)>,
    },
    /// A write violated the key discipline: a `singletons` tabulation holds
    /// at most one row per key (ADR 0001).
    DuplicateKey { table: String },
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Sqlite(e) => write!(f, "sqlite error: {e}"),
            StorageError::Decode(msg) => write!(f, "decode error: {msg}"),
            StorageError::ForeignKey { table, references } => {
                write!(
                    f,
                    "a row of `{table}` references a value that does not exist in "
                )?;
                match references.as_slice() {
                    [] => write!(f, "its `domain` target"),
                    [(field, target)] => write!(f, "`{target}` (the `domain` entry `{field}`)"),
                    many => {
                        let list: Vec<String> = many
                            .iter()
                            .map(|(field, target)| format!("`{target}` (via `{field}`)"))
                            .collect();
                        write!(f, "one of {}", list.join(", "))
                    }
                }
            }
            StorageError::DuplicateKey { table } => write!(
                f,
                "`{table}` holds at most one row per key, and this batch \
                 repeats one"
            ),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Sqlite(e) => Some(e),
            StorageError::Decode(_)
            | StorageError::ForeignKey { .. }
            | StorageError::DuplicateKey { .. } => None,
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::Sqlite(e)
    }
}
