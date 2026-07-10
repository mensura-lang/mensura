//! The Mensura runtime: storage backends and the processing layer.
//!
//! Materializes the resolved [`mensura_types::Schema`] into a database (the
//! storage layer, `docs/toolkit/00-storage-backend.md`) and evaluates checked
//! views into their tables (the processing layer,
//! `docs/toolkit/04-processing-layer.md`).  The first and only backend is
//! SQLite.

pub mod backend;
pub mod eval;
pub mod sqlite;
pub mod value;

pub use backend::{EnsureOutcome, StorageBackend, StorageError};
pub use eval::{EvalError, RunError, SourceTable, eval_view, materialize_views};
pub use sqlite::{SqliteBackend, create_table_sql};
pub use value::{Row, Value};
