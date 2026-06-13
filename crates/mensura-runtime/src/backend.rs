//! The storage abstraction.
//!
//! The toolchain talks to storage only through [`StorageBackend`], so the SQL
//! dialect never leaks into the rest of the compiler and other backends can
//! be added later.  See `docs/toolkit/03-storage-backend.md`.

use std::fmt;

use mensura_types::Schema;

/// A persistent place where stores are materialized.
pub trait StorageBackend {
    /// Ensure the store's table exists, creating it if absent.
    fn ensure_store(&mut self, schema: &Schema) -> Result<EnsureOutcome, StorageError>;
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
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Sqlite(e) => write!(f, "sqlite error: {e}"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Sqlite(e) => Some(e),
        }
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::Sqlite(e)
    }
}
