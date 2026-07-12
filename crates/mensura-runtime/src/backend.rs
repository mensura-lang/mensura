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
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Sqlite(e) => write!(f, "sqlite error: {e}"),
            StorageError::Decode(msg) => write!(f, "decode error: {msg}"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Sqlite(e) => Some(e),
            StorageError::Decode(_) => None,
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::Sqlite(e)
    }
}
