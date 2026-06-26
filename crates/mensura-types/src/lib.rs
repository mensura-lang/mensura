//! Name resolution and the resolved schema model for Mensura.
//!
//! Consumes the AST from `mensura-syntax` and produces [`Schema`]s, the
//! boundary IR the runtime materializes.  See `docs/language/02-stores.md`
//! for what a store is and `docs/toolkit/03-storage-backend.md` for how a
//! `Schema` becomes a table.

pub mod model;
pub mod resolve;
pub mod table;

pub use model::{Column, ColumnRole, ColumnType, Schema};
pub use resolve::{ResolveError, resolve};
pub use table::{
    Branch, Cardinality, Completeness, Content, Lineage, Qualifiers, Side, SplitId, TableType,
    Totality,
};
