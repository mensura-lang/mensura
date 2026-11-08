//! The Mensura runtime: storage backends and the processing layer.
//!
//! Materializes the resolved [`mensura_types::Schema`] into a database (the
//! storage layer, `docs/toolkit/00-storage-backend.md`) and evaluates checked
//! views into their tables (the processing layer,
//! `docs/toolkit/04-processing-layer.md`).  The first and only backend is
//! SQLite.

pub mod backend;
pub mod eval;
pub mod fingerprint;
pub mod ingest;
pub mod sqlite;
pub mod temporal;
pub mod value;

pub use backend::{
    Applied, Delta, EnsureOutcome, Floored, GuardOutcome, LOCAL_PRINCIPAL, StorageBackend,
    StorageError,
};
pub use eval::{EvalError, Materialized, RunError, SourceTable, eval_view, materialize_views};
pub use ingest::{
    IngestError, Record, Scalar, decode_jsonl, decode_record, decode_records, encode_record,
    encode_rows, read_jsonl,
};
pub use sqlite::{SqliteBackend, create_key_index_sql, create_table_sql};
pub use value::{Row, Value};
