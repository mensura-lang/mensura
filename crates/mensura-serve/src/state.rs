//! What an instance holds (ADR 0045 decision 1): the resolved program, the
//! backend behind a lock, the configuration, and nothing that is not also
//! in the database.  Stopping an instance loses nothing.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use mensura_runtime::{GuardOutcome, SqliteBackend, StorageBackend, StorageError};
use mensura_types::{ResolvedProgram, Schema, ViewPlan};
use tokio::sync::Notify;

use crate::auth::{Principals, wire_segment};
use crate::config::{Config, ConfigError, DatabaseTarget, Role};

/// The shared state behind every handler and the materializer.
#[derive(Clone)]
pub struct AppState {
    pub program: Arc<ResolvedProgram>,
    pub backend: Arc<Mutex<SqliteBackend>>,
    pub config: Arc<Config>,
    pub principals: Arc<Principals>,
    /// The program's fingerprint, as the schema guard recorded or verified
    /// it (decision 9).
    pub fingerprint: String,
    /// Signalled after every committed write, so the materializer wakes
    /// without waiting for its poll (decision 5).
    pub wake: Arc<Notify>,
    pub health: Arc<Mutex<Health>>,
}

/// What `health` reports beyond the positions: the views whose last
/// materialization failed, with the error and the position that caused
/// it (decision 5).
#[derive(Clone, Debug, Default)]
pub struct Health {
    pub failed_views: BTreeMap<String, FailedView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedView {
    pub error: String,
    pub position: u64,
}

/// A table a request names: a store or registry, or a view.
#[derive(Clone, Copy, Debug)]
pub enum Target<'a> {
    Table(&'a Schema),
    View(&'a ViewPlan),
}

impl Target<'_> {
    pub fn name(&self) -> &str {
        match self {
            Target::Table(s) => &s.store,
            Target::View(v) => &v.name,
        }
    }

    pub fn shape(&self) -> mensura_types::TableShape {
        match self {
            Target::Table(s) => s.shape(),
            Target::View(v) => v.shape(),
        }
    }
}

/// Why an instance did not start.
#[derive(Debug)]
pub enum StartError {
    Config(ConfigError),
    /// The program has diagnostics, already formatted one per entry.
    Program(Vec<String>),
    Storage(StorageError),
    /// The namespace was created for a different program (decision 9).
    SchemaMismatch {
        tables: Vec<String>,
        stored: String,
        program: String,
    },
    Refused(String),
    Io(std::io::Error),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::Config(e) => write!(f, "{e}"),
            StartError::Program(diagnostics) => write!(f, "{}", diagnostics.join("\n")),
            StartError::Storage(e) => write!(f, "{e}"),
            StartError::SchemaMismatch {
                tables,
                stored,
                program,
            } => write!(
                f,
                "the namespace was created for a different program (fingerprint \
                 {stored}, this program {program}); the stored tables that differ: {}.  \
                 A changed store or registry needs a migration, which this slice \
                 does not generate (ADR 0045 decision 9)",
                tables.join(", ")
            ),
            StartError::Refused(msg) => write!(f, "{msg}"),
            StartError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StartError {}

impl From<ConfigError> for StartError {
    fn from(e: ConfigError) -> Self {
        StartError::Config(e)
    }
}

impl From<StorageError> for StartError {
    fn from(e: StorageError) -> Self {
        StartError::Storage(e)
    }
}

/// Open the configured database, refusing what this slice cannot serve:
/// anything but embedded SQLite, and SQLite under a proper subset of the
/// roles (decision 2), since a file in one process cannot be shared by
/// instances.
pub fn open_backend(config: &Config) -> Result<SqliteBackend, StartError> {
    let base = config.program.parent().unwrap_or_else(|| Path::new("."));
    let DatabaseTarget::Sqlite(path) = config.database_target(base)?;
    if config.roles.len() < 3 {
        return Err(StartError::Refused(format!(
            "embedded SQLite serves only the all-in-one deployment, and this \
             instance is configured with {} of the three roles; separating \
             roles across instances needs a database reached by address \
             (ADR 0045 decision 2)",
            config.roles.len()
        )));
    }
    let backend = if path.as_os_str() == ":memory:" {
        SqliteBackend::open_in_memory()?
    } else {
        SqliteBackend::open(&path)?
    };
    Ok(backend)
}

impl AppState {
    /// Build the state over an opened backend: run the schema guard and
    /// keep the fingerprint it settled on.
    pub fn open(
        config: Config,
        program: ResolvedProgram,
        mut backend: SqliteBackend,
    ) -> Result<AppState, StartError> {
        let fingerprint = match backend.guard_program(&program)? {
            GuardOutcome::Created { fingerprint } | GuardOutcome::Verified { fingerprint } => {
                fingerprint
            }
            GuardOutcome::Mismatch {
                tables,
                stored,
                program,
            } => {
                return Err(StartError::SchemaMismatch {
                    tables,
                    stored,
                    program,
                });
            }
        };
        let principals = Principals::new(&config.principals);
        Ok(AppState {
            program: Arc::new(program),
            backend: Arc::new(Mutex::new(backend)),
            config: Arc::new(config),
            principals: Arc::new(principals),
            fingerprint,
            wake: Arc::new(Notify::new()),
            health: Arc::new(Mutex::new(Health::default())),
        })
    }

    pub fn runs(&self, role: Role) -> bool {
        self.config.roles.contains(&role)
    }

    /// The table a wire segment names, if any.
    pub fn target(&self, segment: &str) -> Option<Target<'_>> {
        if let Some(s) = self
            .program
            .schemas
            .iter()
            .find(|s| wire_segment(&s.store) == segment)
        {
            return Some(Target::Table(s));
        }
        self.program
            .views
            .iter()
            .find(|v| wire_segment(&v.name) == segment)
            .map(Target::View)
    }

    /// The backend, surviving a poisoned lock: a panic in one request
    /// must not take the instance down, and SQLite's transactions keep
    /// the database consistent regardless.
    pub fn backend(&self) -> MutexGuard<'_, SqliteBackend> {
        self.backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn health(&self) -> MutexGuard<'_, Health> {
        self.health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
