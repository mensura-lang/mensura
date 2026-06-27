//! Name resolution and the resolved schema model for Mensura.
//!
//! Consumes the AST from `mensura-syntax` and produces [`Schema`]s, the
//! boundary IR the runtime materializes.  See `docs/language/02-stores.md`
//! for what a store is and `docs/toolkit/03-storage-backend.md` for how a
//! `Schema` becomes a table.

pub mod expr_check;
pub mod model;
pub mod pipe_check;
pub mod resolve;
pub mod table;

pub use expr_check::{Context, Optionality, Ty, TypeError, type_expr};
pub use model::{Column, ColumnRole, ColumnType, Schema};
pub use pipe_check::{PipeTy, Sources, type_pipeline};
pub use resolve::{ResolveError, resolve};
pub use table::{
    Branch, Cardinality, Completeness, Content, Lineage, Qualifiers, Side, SplitId, TableType,
    Totality,
};
