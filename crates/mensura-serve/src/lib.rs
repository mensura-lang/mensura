//! `mensura serve`: a Mensura program running as a service
//! (`docs/decisions/0045-serving-a-program.md`, `docs/toolkit/06-serving.md`).
//!
//! An instance runs some of three roles over one database: `intake`
//! (positioned writes), `read` (reads that report their position), and
//! `materializer` (views recomputed at a log position).  This slice serves
//! embedded SQLite in one process, which is the all-in-one deployment; the
//! Postgres backend, which makes the roles separable across instances, is
//! the next slice.
//!
//! The server is a thin projection: every operation is one of the four
//! wire-agnostic operations of ADR 0045 decision 3, implemented by the
//! runtime, and HTTP only names them (decision 7).

pub mod auth;
pub mod config;
pub mod frontend;
pub mod server;
pub mod state;

use std::path::Path;

pub use config::{Config, ConfigError, Role};
pub use server::{materialize_once, router};
pub use state::{AppState, StartError};

/// Run a service from its configuration file until `SIGTERM` or `SIGINT`.
///
/// Startup refuses, in this order, a configuration that does not load, a
/// program with any diagnostic (exactly as `mensura check` would), a
/// database the slice cannot serve, and a namespace created for a
/// different program (ADR 0045 decision 9).
pub fn run(config_path: &Path) -> Result<(), StartError> {
    let config = Config::load(config_path)?;
    let program = frontend::load_program(&config.program).map_err(StartError::Program)?;
    let backend = state::open_backend(&config)?;
    let state = AppState::open(config, program, backend)?;
    let runtime = tokio::runtime::Runtime::new().map_err(StartError::Io)?;
    runtime.block_on(server::serve(state))
}
