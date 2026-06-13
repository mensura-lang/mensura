//! Runtime storage backends for Mensura stores.
//!
//! Materializes the resolved [`mensura_types::Schema`] into a database.  The
//! first and only backend is SQLite; see `docs/toolkit/03-storage-backend.md`
//! for the mapping and the rationale.

pub mod backend;
pub mod sqlite;

pub use backend::{EnsureOutcome, StorageBackend, StorageError};
pub use sqlite::{SqliteBackend, create_table_sql};
