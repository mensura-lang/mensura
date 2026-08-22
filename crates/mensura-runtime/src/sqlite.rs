//! A SQLite-backed [`StorageBackend`] using rusqlite (bundled SQLite).

use std::collections::BTreeMap;
use std::path::Path;

use mensura_types::{ColumnRole, ColumnType, Lateness, Schema, TableShape};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};

use crate::backend::{Applied, Delta, EnsureOutcome, StorageBackend, StorageError, Watermarks};
use crate::temporal::{instant_to_ms, ms_to_instant};
use crate::value::{Row, Value};

/// A store backend that materializes schemas as SQLite tables.
pub struct SqliteBackend {
    conn: Connection,
}

impl SqliteBackend {
    /// Open (or create) a database at `path`.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Self::configure(Connection::open(path)?)
    }

    /// Open a transient in-memory database (used in tests).
    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::configure(Connection::open_in_memory()?)
    }

    /// Settings every connection needs.  SQLite scopes `foreign_keys` to the
    /// connection rather than the database, so it is set at open and both
    /// the read and write paths see it (ADR 0034 decision 5): the
    /// `FOREIGN KEY` clauses `CREATE TABLE` emits for each resolved `domain`
    /// entry (ADR 0032) are enforced now that there is a write path.
    fn configure(conn: Connection) -> Result<Self, StorageError> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(SqliteBackend { conn })
    }

    /// Execute raw SQL against the backing database.  A test scaffold for
    /// seeding rows (`docs/toolkit/04-processing-layer.md`, "Validation"),
    /// kept because it bypasses decoding; the supported intake is
    /// [`StorageBackend::apply`] (`docs/toolkit/05-ingestion.md`).
    pub fn execute_sql(&self, sql: &str) -> Result<(), StorageError> {
        self.conn.execute_batch(sql)?;
        Ok(())
    }

    /// Advance a contract's declared closure floor (ADR 0041 decision 4):
    /// the point through which the deployment asserts the world is closed.
    ///
    /// The floor is the stored half of the watermark, because nothing
    /// derives an assertion.  Storing it rather than reading a clock is
    /// what keeps `mensura run` a function of the database, and storing it
    /// rather than writing it in the program is what keeps advancing time
    /// from being a recompile.
    ///
    /// **It only ever advances.**  Lowering a floor reopens windows
    /// already reported final, which retracts published results; that is
    /// the same deliberate override as relaxing a `lateness` bound, and
    /// neither is designed yet, so this refuses rather than guessing.
    ///
    /// `point` is the column's own text (a normalized instant, or an
    /// integer count for an `int` point).
    pub fn advance_floor(
        &mut self,
        store: &str,
        column: &str,
        point: &str,
    ) -> Result<(), StorageError> {
        let grain = match instant_to_ms(point) {
            Ok(ms) => ms,
            Err(_) => point.parse::<i64>().map_err(|_| {
                StorageError::Decode(format!(
                    "`{point}` is neither a normalized instant \
                     (`YYYY-MM-DDTHH:MM:SS.sssZ`) nor an integer count"
                ))
            })?,
        };
        let tx = self.conn.transaction()?;
        tx.execute_batch(&create_floor_table_sql())?;
        let current: Option<i64> = tx
            .query_row(
                &format!(
                    "SELECT \"floor\" FROM {} WHERE \"store\" = ?1 AND \"column\" = ?2",
                    quote_ident(FLOOR_TABLE)
                ),
                rusqlite::params![store, column],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(current) = current
            && grain < current
        {
            return Err(StorageError::Refused(format!(
                "a closure floor only advances: `{store}.{column}` already \
                 stands at {}, and lowering it to {point} would reopen \
                 windows already reported final (ADR 0041)",
                render_point(&ColumnType::Instant, current)
            )));
        }
        tx.execute(
            &format!(
                "INSERT INTO {} (\"store\", \"column\", \"floor\") VALUES (?1, ?2, ?3) \
                 ON CONFLICT (\"store\", \"column\") DO UPDATE SET \"floor\" = ?3",
                quote_ident(FLOOR_TABLE)
            ),
            rusqlite::params![store, column, grain],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn table_exists(&self, name: &str) -> Result<bool, StorageError> {
        let found: i64 = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [name],
            |row| row.get(0),
        )?;
        Ok(found != 0)
    }
}

impl StorageBackend for SqliteBackend {
    fn ensure_store(&mut self, schema: &Schema) -> Result<EnsureOutcome, StorageError> {
        let existed = self.table_exists(&schema.store)?;
        let shape = schema.shape();
        self.conn.execute_batch(&create_table_sql(&shape))?;
        // A `bag` store has no primary key (per-row addressability is lost by
        // definition, ADR 0022); its key columns get an ordinary non-unique
        // covering index instead, and SQLite's implicit rowid is the
        // surrogate row identifier.
        if !shape.keyed
            && let Some(sql) = create_key_index_sql(&shape)
        {
            self.conn.execute_batch(&sql)?;
        }
        Ok(if existed {
            EnsureOutcome::AlreadyExists
        } else {
            EnsureOutcome::Created
        })
    }

    fn scan(&self, table: &TableShape) -> Result<Vec<Row>, StorageError> {
        let cols: Vec<String> = table.columns.iter().map(|c| quote_ident(&c.name)).collect();
        let mut key: Vec<String> = table
            .columns
            .iter()
            .filter(|c| c.role == ColumnRole::Key)
            .map(|c| quote_ident(&c.name))
            .collect();
        // An unkeyed table (a `bag` store or view, ADR 0022) can hold several
        // rows per key; the surrogate rowid breaks the tie so a scan stays
        // deterministic even though a bag carries no row order.
        if !table.keyed {
            key.push("_rowid_".to_string());
        }
        let order = if key.is_empty() {
            String::new()
        } else {
            format!(" ORDER BY {}", key.join(", "))
        };
        let sql = format!(
            "SELECT {} FROM {}{}",
            cols.join(", "),
            quote_ident(&table.name),
            order
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let mut row = Vec::with_capacity(table.columns.len());
            for (i, col) in table.columns.iter().enumerate() {
                row.push(decode(r.get_ref(i)?, &col.ty, &table.name, &col.name)?);
            }
            out.push(row);
        }
        Ok(out)
    }

    fn materialize_view(&mut self, view: &TableShape, rows: &[Row]) -> Result<(), StorageError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(&create_table_sql(view))?;
        tx.execute(&format!("DELETE FROM {}", quote_ident(&view.name)), [])?;
        let cols: Vec<String> = view.columns.iter().map(|c| quote_ident(&c.name)).collect();
        let holes: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_ident(&view.name),
            cols.join(", "),
            holes.join(", ")
        );
        {
            let mut stmt = tx.prepare(&sql)?;
            for row in rows {
                stmt.execute(rusqlite::params_from_iter(row.iter().map(encode)))?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn watermarks(
        &self,
        table: &TableShape,
        contract: &Lateness,
    ) -> Result<Watermarks, StorageError> {
        // The observed half, derived rather than stored (ADR 0041
        // decision 3): one maximum per grain, which is the same projection
        // the intake computes per touched grain, taken here over the whole
        // table at once.
        let grain_cols: Vec<String> = contract.grain.iter().map(|g| quote_ident(g)).collect();
        let selected = grain_cols
            .iter()
            .map(|c| format!("{c}, "))
            .collect::<String>();
        let group = if grain_cols.is_empty() {
            String::new()
        } else {
            format!(" GROUP BY {}", grain_cols.join(", "))
        };
        let sql = format!(
            "SELECT {selected}MAX({}) FROM {}{group}",
            quote_ident(&contract.column),
            quote_ident(&table.name),
        );
        let ty = table
            .columns
            .iter()
            .find(|c| c.name == contract.column)
            .map(|c| c.ty.clone())
            .ok_or_else(|| {
                StorageError::Decode(format!(
                    "`{}` has no column `{}` for its lateness contract",
                    table.name, contract.column
                ))
            })?;

        let mut observed = BTreeMap::new();
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        let n = contract.grain.len();
        while let Some(r) = rows.next()? {
            let key: Vec<String> = (0..n)
                .map(|i| {
                    Ok(decode(
                        r.get_ref(i)?,
                        &ty_of_grain(table, &contract.grain[i]),
                        &table.name,
                        &contract.grain[i],
                    )?
                    .grain_key())
                })
                .collect::<Result<_, StorageError>>()?;
            let point = match r.get_ref(n)? {
                ValueRef::Text(t) => {
                    let text =
                        std::str::from_utf8(t).map_err(|e| StorageError::Decode(e.to_string()))?;
                    Some(instant_to_ms(text).map_err(StorageError::Decode)?)
                }
                ValueRef::Integer(v) => Some(v),
                // A grain with no rows cannot appear in a `GROUP BY` over
                // the table, so a NULL maximum means an empty table.
                _ => None,
            };
            if let Some(point) = point {
                observed.insert(key, point);
            }
        }
        let _ = ty;
        Ok(Watermarks::new(
            observed,
            read_floor(&self.conn, &table.name, &contract.column)?,
        ))
    }

    fn apply(&mut self, table: &TableShape, delta: &Delta) -> Result<Applied, StorageError> {
        // One transaction for the batch: every change lands or none does
        // (ADR 0034 decision 4).  The `lateness` check and the watermark
        // advance ride the same transaction, so a rejected batch leaves the
        // watermark untouched and an accepted one can never land without
        // advancing it.
        let tx = self.conn.transaction()?;
        enforce_lateness(&tx, table, delta)?;
        let cols: Vec<String> = table.columns.iter().map(|c| quote_ident(&c.name)).collect();

        let mut applied = Applied::default();
        if !delta.inserts.is_empty() {
            let holes: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "INSERT INTO {} ({}) VALUES ({})",
                quote_ident(&table.name),
                cols.join(", "),
                holes.join(", ")
            );
            let mut stmt = tx.prepare(&sql)?;
            for row in &delta.inserts {
                stmt.execute(rusqlite::params_from_iter(row.iter().map(encode)))
                    .map_err(|e| write_error(e, table))?;
                applied.inserted += 1;
            }
        }

        if !delta.deletes.is_empty() {
            // Delete by whole-row match: the row is the identity on an
            // unkeyed (bag) table, and matches the key on a keyed one.
            let matches: Vec<String> = cols
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{c} IS ?{}", i + 1))
                .collect();
            let sql = format!(
                "DELETE FROM {} WHERE {}",
                quote_ident(&table.name),
                matches.join(" AND ")
            );
            let mut stmt = tx.prepare(&sql)?;
            for row in &delta.deletes {
                let n = stmt
                    .execute(rusqlite::params_from_iter(row.iter().map(encode)))
                    .map_err(|e| write_error(e, table))?;
                applied.deleted += n;
            }
        }

        tx.commit()?;
        Ok(applied)
    }
}

/// The reserved table holding each contract's declared closure floor
/// (ADR 0041 decision 4).  The floor is the only stored half of the
/// watermark: it is an operator assertion, so nothing derives it, and it
/// carries no grain, so it needs no encoding of a heterogeneous key.
pub(crate) const FLOOR_TABLE: &str = "mensura_lateness_floors";

pub(crate) fn create_floor_table_sql() -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {} (\n  \
           \"store\" TEXT NOT NULL,\n  \
           \"column\" TEXT NOT NULL,\n  \
           \"floor\" INTEGER NOT NULL,\n  \
           PRIMARY KEY (\"store\", \"column\")\n);",
        quote_ident(FLOOR_TABLE)
    )
}

/// Read a contract's declared floor, in the column's storage grain.
pub(crate) fn read_floor(
    conn: &Connection,
    store: &str,
    column: &str,
) -> Result<Option<i64>, StorageError> {
    conn.execute_batch(&create_floor_table_sql())?;
    Ok(conn
        .query_row(
            &format!(
                "SELECT \"floor\" FROM {} WHERE \"store\" = ?1 AND \"column\" = ?2",
                quote_ident(FLOOR_TABLE)
            ),
            rusqlite::params![store, column],
            |r| r.get(0),
        )
        .optional()?)
}

/// Enforce the table's `lateness` contracts (ADR 0037 decision 4, grained
/// by ADR 0041), inside the caller's transaction.
///
/// The **effective watermark** of a grain is `max(observed, floor)`:
///
/// - *observed* is the maximum point already accepted in that grain, and
///   it is **derived, never stored** (ADR 0041 decision 3).  Under an
///   append-only intake it is exactly `max(point) where grain = ...`, so
///   storing it would only let it drift, and its key would be a
///   heterogeneous per-registry tuple needing an encoding.  Derived, it
///   compares the registry's own columns with their own types, travels
///   with a backup, and costs a keyed maximum that the primary key
///   already indexes for a key-borne point.
/// - *floor* is the declared point the deployment asserts the world is
///   closed through.  It is the irreducible half, and the half with no
///   grain.
///
/// The **grain** is the declared key minus the contracted column
/// (ADR 0041 decision 2), so a slow producer is measured against itself
/// rather than against the fleet's fastest.  An empty grain is the
/// degenerate single-watermark case that recovers ADR 0037's global rule.
///
/// A batch containing a row *older than* `effective - bound` is rejected
/// whole; the boundary point is still admissible, and the strict upper
/// bound of the window interval is what keeps that safe for `closed`
/// (`Mensura.closedWindow_stable`).  Rows are compared against the value
/// as of intake, never against each other, so a batch stays an unordered
/// bag.  Nothing is written here: an accepted append advances the
/// observed maximum by landing, which is the point of deriving it.
fn enforce_lateness(
    tx: &rusqlite::Transaction<'_>,
    table: &TableShape,
    delta: &Delta,
) -> Result<(), StorageError> {
    if table.lateness.is_empty() || delta.inserts.is_empty() {
        return Ok(());
    }
    for contract in &table.lateness {
        let Some(idx) = table.columns.iter().position(|c| c.name == contract.column) else {
            return Err(StorageError::Decode(format!(
                "`{}` has no column `{}` for its lateness contract",
                table.name, contract.column
            )));
        };
        let ty = &table.columns[idx].ty;
        // The point of one row, in the column's storage grain.
        let point_of = |row: &Row| -> Result<i64, StorageError> {
            match &row[idx] {
                Value::Instant(s) => instant_to_ms(s).map_err(StorageError::Decode),
                Value::Int(n) => Ok(*n),
                other => Err(StorageError::Decode(format!(
                    "`{}.{}` carries {other:?} where its lateness contract \
                     expects a point",
                    table.name, contract.column
                ))),
            }
        };
        let render = |point: i64| -> String { render_point(ty, point) };

        // The grain columns' positions, resolved once per contract.
        let grain: Vec<usize> = contract
            .grain
            .iter()
            .map(|name| {
                table
                    .columns
                    .iter()
                    .position(|c| &c.name == name)
                    .ok_or_else(|| {
                        StorageError::Decode(format!(
                            "`{}` has no column `{name}` for its lateness grain",
                            table.name
                        ))
                    })
            })
            .collect::<Result<_, _>>()?;

        // The floor has no grain, so it is read once per contract.
        let floor = read_floor(tx, &table.name, &contract.column)?;

        // Group the batch by grain and keep each group's oldest row: only
        // the oldest can violate, and one violation rejects the batch.
        // Grouped on the rendered grain, which is injective here because a
        // grain column is a key column, hence equatable and never `real`
        // (the domain matrix of `09-typing-reference.md`); the values
        // themselves ride along for the query.
        let mut oldest_by_grain: BTreeMap<Vec<String>, (Vec<Value>, i64)> = BTreeMap::new();
        for row in &delta.inserts {
            let values: Vec<Value> = grain.iter().map(|&i| row[i].clone()).collect();
            let key: Vec<String> = values.iter().map(describe_value).collect();
            let p = point_of(row)?;
            oldest_by_grain
                .entry(key)
                .and_modify(|(_, old)| *old = (*old).min(p))
                .or_insert((values, p));
        }

        let where_grain = contract
            .grain
            .iter()
            .enumerate()
            .map(|(i, name)| format!("{} IS ?{}", quote_ident(name), i + 1))
            .collect::<Vec<_>>()
            .join(" AND ");
        let sql = format!(
            "SELECT MAX({}) FROM {}{}",
            quote_ident(&contract.column),
            quote_ident(&table.name),
            if where_grain.is_empty() {
                String::new()
            } else {
                format!(" WHERE {where_grain}")
            }
        );
        let mut stmt = tx.prepare(&sql)?;

        for (_, (key, oldest)) in oldest_by_grain {
            // The observed watermark of this grain, derived from the rows
            // already accepted.  Absent (no rows yet) leaves the floor.
            // `MAX` comes back in the column's own storage type: TEXT for a
            // normalized instant (whose fixed width makes lexicographic
            // order chronological, ADR 0036 decision 7), INTEGER for a
            // count.  A grain with no rows yet yields SQL NULL.
            let stored: Option<rusqlite::types::Value> = stmt
                .query_row(rusqlite::params_from_iter(key.iter().map(encode)), |r| {
                    r.get(0)
                })
                .optional()?;
            let observed = match stored {
                Some(rusqlite::types::Value::Text(text)) => {
                    Some(instant_to_ms(&text).map_err(StorageError::Decode)?)
                }
                Some(rusqlite::types::Value::Integer(n)) => Some(n),
                _ => None,
            };
            let effective = match (observed, floor) {
                (Some(o), Some(f)) => Some(o.max(f)),
                (Some(o), None) => Some(o),
                (None, f) => f,
            };
            let Some(effective) = effective else {
                // No rows in this grain and no floor: the first batch of a
                // grain is unconstrained, which is what makes onboarding a
                // device with history work (ADR 0041's worked example).
                continue;
            };
            let limit = effective - contract.bound;
            if oldest < limit {
                return Err(StorageError::Lateness {
                    table: table.name.clone(),
                    column: contract.column.clone(),
                    point: render(oldest),
                    limit: render(limit),
                    grain: contract
                        .grain
                        .iter()
                        .cloned()
                        .zip(key.iter().map(describe_value))
                        .collect(),
                });
            }
        }
    }
    Ok(())
}

/// The declared type of one grain column, for decoding a grouped read.
fn ty_of_grain(table: &TableShape, column: &str) -> ColumnType {
    table
        .columns
        .iter()
        .find(|c| c.name == column)
        .map(|c| c.ty.clone())
        // A grain column is always a declared key column, so this is
        // unreachable; `String` keeps the decode total rather than panicking
        // on a corrupt shape.
        .unwrap_or(ColumnType::String)
}

/// Render a point in the column's own form, for a diagnostic.
fn render_point(ty: &ColumnType, point: i64) -> String {
    match ty {
        ColumnType::Instant => {
            ms_to_instant(point).unwrap_or_else(|_| format!("{point} ms from the epoch"))
        }
        _ => point.to_string(),
    }
}

/// A grain value as a diagnostic shows it.
fn describe_value(v: &Value) -> String {
    match v {
        Value::String(s) | Value::Date(s) | Value::Instant(s) | Value::Enum(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        Value::Real(x) => x.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Missing => "(missing)".to_string(),
    }
}

/// Translate a constraint failure into a diagnostic that names what the
/// program declared, rather than a raw SQLite code (ADR 0034 decision 5).
fn write_error(e: rusqlite::Error, table: &TableShape) -> StorageError {
    use rusqlite::ErrorCode;
    let rusqlite::Error::SqliteFailure(err, _) = &e else {
        return StorageError::Sqlite(e);
    };
    match err.code {
        ErrorCode::ConstraintViolation if err.extended_code == FOREIGN_KEY_VIOLATION => {
            StorageError::ForeignKey {
                table: table.name.clone(),
                references: table
                    .foreign_keys
                    .iter()
                    .map(|fk| (fk.field.clone(), fk.store.clone()))
                    .collect(),
            }
        }
        ErrorCode::ConstraintViolation
            if matches!(err.extended_code, PRIMARY_KEY_VIOLATION | UNIQUE_VIOLATION) =>
        {
            StorageError::DuplicateKey {
                table: table.name.clone(),
            }
        }
        _ => StorageError::Sqlite(e),
    }
}

/// `SQLITE_CONSTRAINT_FOREIGNKEY`.
const FOREIGN_KEY_VIOLATION: i32 = 787;
/// `SQLITE_CONSTRAINT_PRIMARYKEY`.
const PRIMARY_KEY_VIOLATION: i32 = 1555;
/// `SQLITE_CONSTRAINT_UNIQUE`.
const UNIQUE_VIOLATION: i32 = 2067;

/// Decode one stored cell into a typed [`Value`].  `NULL` is [`Value::Missing`]
/// (an optional value, ADR 0010); anything else must match the column's
/// declared type.
fn decode(
    cell: ValueRef<'_>,
    ty: &ColumnType,
    table: &str,
    col: &str,
) -> Result<Value, StorageError> {
    let mismatch = || {
        StorageError::Decode(format!(
            "table `{table}`, column `{col}`: stored value does not match its declared type"
        ))
    };
    if matches!(cell, ValueRef::Null) {
        return Ok(Value::Missing);
    }
    match ty {
        ColumnType::String => cell.as_str().map(|s| Value::String(s.to_string())),
        ColumnType::Date => cell.as_str().map(|s| Value::Date(s.to_string())),
        ColumnType::Instant => cell.as_str().map(|s| Value::Instant(s.to_string())),
        ColumnType::Enum { .. } => cell.as_str().map(|s| Value::Enum(s.to_string())),
        ColumnType::Int => cell.as_i64().map(Value::Int),
        // A dimensioned column persists its base-unit magnitude as a plain
        // real; the dimension is compile-time only (ADR 0026,
        // `docs/toolkit/00-storage-backend.md`).
        ColumnType::Real | ColumnType::Quantity(_) => cell.as_f64().map(Value::Real),
        ColumnType::Bool => cell.as_i64().map(|i| Value::Bool(i != 0)),
    }
    .map_err(|_| mismatch())
}

/// Encode one typed [`Value`] as a SQL parameter.  [`Value::Missing`] is `NULL`.
fn encode(value: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sql;
    match value {
        Value::String(s) | Value::Date(s) | Value::Instant(s) | Value::Enum(s) => {
            Sql::Text(s.clone())
        }
        Value::Int(i) => Sql::Integer(*i),
        Value::Real(r) => Sql::Real(*r),
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Missing => Sql::Null,
    }
}

/// Build the `CREATE TABLE IF NOT EXISTS` statement for a table shape.  A
/// keyed shape (a `singletons` store or view) gets the composite primary
/// key over its key columns; an unkeyed one (a `bag` store or view,
/// ADR 0022, `docs/toolkit/04-processing-layer.md`) gets none.
pub fn create_table_sql(shape: &TableShape) -> String {
    let mut lines: Vec<String> = shape
        .columns
        .iter()
        .map(|c| {
            // A total column is `NOT NULL`; an optional one (`?`) is nullable
            // (ADR 0010, `docs/toolkit/00-storage-backend.md`).  Index columns
            // are always total, so the primary key is non-null too.
            let null = if c.optional { "" } else { " NOT NULL" };
            format!(
                "  {} {}{}",
                quote_ident(&c.name),
                column_type_sql(&c.ty, &c.name),
                null
            )
        })
        .collect();

    let key: Vec<String> = shape
        .columns
        .iter()
        .filter(|c| c.role == ColumnRole::Key)
        .map(|c| quote_ident(&c.name))
        .collect();
    if shape.keyed && !key.is_empty() {
        lines.push(format!("  PRIMARY KEY ({})", key.join(", ")));
    }

    // One clause per resolved `domain` entry (ADR 0032).  SQLite accepts
    // the clauses regardless of table-creation order, and enforces them
    // because `SqliteBackend::configure` sets `PRAGMA foreign_keys = ON`
    // (ADR 0034 decision 5, the question ADR 0032 left to this slice).
    for fk in &shape.foreign_keys {
        let children: Vec<String> = fk
            .columns
            .iter()
            .map(|(child, _)| quote_ident(child))
            .collect();
        let parents: Vec<String> = fk
            .columns
            .iter()
            .map(|(_, parent)| quote_ident(parent))
            .collect();
        lines.push(format!(
            "  FOREIGN KEY ({}) REFERENCES {} ({})",
            children.join(", "),
            quote_ident(&fk.store),
            parents.join(", ")
        ));
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n);",
        quote_ident(&shape.name),
        lines.join(",\n")
    )
}

/// Build the non-unique covering index over an unkeyed store's key columns
/// (ADR 0022): a `bag` store keeps its key lookup fast without a PRIMARY KEY.
/// `None` when the shape has no key columns to cover.
pub fn create_key_index_sql(shape: &TableShape) -> Option<String> {
    let key: Vec<String> = shape
        .columns
        .iter()
        .filter(|c| c.role == ColumnRole::Key)
        .map(|c| quote_ident(&c.name))
        .collect();
    if key.is_empty() {
        return None;
    }
    Some(format!(
        "CREATE INDEX IF NOT EXISTS {} ON {} ({});",
        quote_ident(&format!("{}_key", shape.name)),
        quote_ident(&shape.name),
        key.join(", ")
    ))
}

fn column_type_sql(ty: &ColumnType, col: &str) -> String {
    match ty {
        ColumnType::String => "TEXT".to_string(),
        ColumnType::Int => "INTEGER".to_string(),
        // A dimensioned column stores its base-unit magnitude (ADR 0026).
        ColumnType::Real | ColumnType::Quantity(_) => "REAL".to_string(),
        ColumnType::Bool => "INTEGER".to_string(),
        ColumnType::Date => "TEXT".to_string(),
        // The normalized fixed-width UTC form (ADR 0036 decision 7): plain
        // `TEXT`, ordered chronologically by the string comparison above.
        ColumnType::Instant => "TEXT".to_string(),
        ColumnType::Enum { variants, .. } => {
            let list = variants
                .iter()
                .map(|v| quote_str(v))
                .collect::<Vec<_>>()
                .join(", ");
            format!("TEXT CHECK ({} IN ({}))", quote_ident(col), list)
        }
    }
}

/// Quote a SQL identifier with double quotes, doubling any embedded quote.
fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Quote a SQL string literal with single quotes, doubling any embedded quote.
fn quote_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::EnsureOutcome;

    fn schema(src: &str, store: &str) -> Schema {
        let tokens = mensura_syntax::tokenize(src).expect("should lex");
        let program = mensura_syntax::parse(&tokens).expect("should parse");
        mensura_types::resolve(&program)
            .expect("should resolve")
            .schemas
            .into_iter()
            .find(|s| s.store == store)
            .unwrap_or_else(|| panic!("no store named {store}"))
    }

    const PERSONS: &str = r#"
        unit Person { id: string }
        store persons {
          unit { Person }
          attr { birthdate: date }
          attr { last_name: string }
        }
    "#;

    #[test]
    fn create_table_sql_for_persons() {
        let sql = create_table_sql(&schema(PERSONS, "persons").shape());
        assert_eq!(
            sql,
            "CREATE TABLE IF NOT EXISTS \"persons\" (\n  \"id\" TEXT NOT NULL,\n  \"birthdate\" TEXT NOT NULL,\n  \"last_name\" TEXT NOT NULL,\n  PRIMARY KEY (\"id\")\n);"
        );
    }

    #[test]
    fn optional_column_is_nullable() {
        // A `?` attribute is nullable; total columns keep `NOT NULL` (ADR 0010).
        let src = r#"
            unit Person { id: string }
            store persons {
              unit { Person }
              attr { last_name: string? }
              attr { status: string }
            }
        "#;
        let sql = create_table_sql(&schema(src, "persons").shape());
        assert!(sql.contains("\"last_name\" TEXT,"));
        assert!(sql.contains("\"status\" TEXT NOT NULL"));
        assert!(sql.contains("\"id\" TEXT NOT NULL"));
    }

    #[test]
    fn unkeyed_shape_has_no_primary_key() {
        // A `bag` view materializes without a primary key
        // (`docs/toolkit/04-processing-layer.md`); its key columns stay
        // `NOT NULL`.
        let mut shape = schema(PERSONS, "persons").shape();
        shape.keyed = false;
        let sql = create_table_sql(&shape);
        assert!(!sql.contains("PRIMARY KEY"));
        assert!(sql.contains("\"id\" TEXT NOT NULL"));
    }

    const BAG_READINGS: &str = r#"
        unit Machine { id: string }
        store readings {
          unit { Machine }
          attr* { kelvin: real }
        }
    "#;

    #[test]
    fn bag_store_has_no_primary_key_and_a_covering_index() {
        // ADR 0022: a `bag` store maps to a rowid table (no PRIMARY KEY, the
        // implicit rowid is the surrogate row identifier) plus a non-unique
        // covering index over the key columns.
        let s = schema(BAG_READINGS, "readings");
        let sql = create_table_sql(&s.shape());
        assert!(!sql.contains("PRIMARY KEY"), "{sql}");
        assert!(sql.contains("\"id\" TEXT NOT NULL"));
        assert_eq!(
            create_key_index_sql(&s.shape()).as_deref(),
            Some("CREATE INDEX IF NOT EXISTS \"readings_key\" ON \"readings\" (\"id\");")
        );

        let mut db = SqliteBackend::open_in_memory().unwrap();
        assert_eq!(db.ensure_store(&s).unwrap(), EnsureOutcome::Created);
        // Duplicate keys insert cleanly: the store holds many rows per entity.
        db.execute_sql(
            "INSERT INTO \"readings\" VALUES ('m1', 300.0), ('m1', 301.5), ('m2', 299.0);",
        )
        .expect("duplicate keys are admitted in a bag store");
        // The covering index exists.
        let indexed: i64 = db
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'readings_key')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(indexed, 1);
    }

    #[test]
    fn bag_store_scan_is_deterministic_within_a_key() {
        // Rows come back key-ordered with the rowid tiebreak, so a scan of a
        // bag store is stable across runs.
        let s = schema(BAG_READINGS, "readings");
        let mut db = SqliteBackend::open_in_memory().unwrap();
        db.ensure_store(&s).unwrap();
        db.execute_sql(
            "INSERT INTO \"readings\" VALUES ('m2', 299.0), ('m1', 300.0), ('m1', 301.5);",
        )
        .unwrap();
        let rows = db.scan(&s.shape()).unwrap();
        assert_eq!(
            rows,
            vec![
                vec![Value::String("m1".into()), Value::Real(300.0)],
                vec![Value::String("m1".into()), Value::Real(301.5)],
                vec![Value::String("m2".into()), Value::Real(299.0)],
            ]
        );
    }

    #[test]
    fn create_table_sql_for_enum_has_check() {
        let src = r#"
            unit U { id: string }
            enum Status { "active" "inactive" }
            store s { unit { U } attr { status: Status } }
        "#;
        let sql = create_table_sql(&schema(src, "s").shape());
        assert!(sql.contains("\"status\" TEXT CHECK (\"status\" IN ('active', 'inactive'))"));
    }

    const COMPOUND: &str = r#"
        unit Person { id: string }
        unit Department { code: string }
        unit Course {
          department: Department
          name: string
          year: int
        }
        unit Enrollment {
          student: Person
          course: Course
        }
        store students { unit { Person } }
        store departments { unit { Department } }
        store courses {
          unit { Course }
          domain { department: departments }
        }
        store student_grades {
          unit { Enrollment }
          domain {
            student: students
            course:  courses
          }
          attr { grade: real }
        }
    "#;

    #[test]
    fn compound_store_maps_to_dotted_columns_and_foreign_keys() {
        // ADR 0032: dotted quoted columns, a composite primary key over the
        // flattened key, and one FOREIGN KEY clause per resolved `domain`
        // entry (enforced since ADR 0034 turned the pragma on).
        let sql = create_table_sql(&schema(COMPOUND, "student_grades").shape());
        assert_eq!(
            sql,
            "CREATE TABLE IF NOT EXISTS \"student_grades\" (\n  \
               \"student.id\" TEXT NOT NULL,\n  \
               \"course.department.code\" TEXT NOT NULL,\n  \
               \"course.name\" TEXT NOT NULL,\n  \
               \"course.year\" INTEGER NOT NULL,\n  \
               \"grade\" REAL NOT NULL,\n  \
               PRIMARY KEY (\"student.id\", \"course.department.code\", \"course.name\", \"course.year\"),\n  \
               FOREIGN KEY (\"student.id\") REFERENCES \"students\" (\"id\"),\n  \
               FOREIGN KEY (\"course.department.code\", \"course.name\", \"course.year\") REFERENCES \"courses\" (\"department.code\", \"name\", \"year\")\n\
             );"
        );

        // The DDL is accepted by SQLite regardless of table-creation order,
        // which is what lets `ensure_store` run in declaration order even
        // though the targets do not exist yet.
        let mut db = SqliteBackend::open_in_memory().unwrap();
        assert_eq!(
            db.ensure_store(&schema(COMPOUND, "student_grades"))
                .unwrap(),
            EnsureOutcome::Created
        );
    }

    #[test]
    fn ensure_store_creates_then_reports_existing() {
        let mut db = SqliteBackend::open_in_memory().unwrap();
        let s = schema(PERSONS, "persons");
        assert_eq!(db.ensure_store(&s).unwrap(), EnsureOutcome::Created);
        assert_eq!(db.ensure_store(&s).unwrap(), EnsureOutcome::AlreadyExists);

        // Columns, types, and the primary key are as declared.
        let cols: Vec<(String, String, i64)> = db
            .conn
            .prepare("PRAGMA table_info(\"persons\")")
            .unwrap()
            .query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(5)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            cols,
            vec![
                ("id".into(), "TEXT".into(), 1),
                ("birthdate".into(), "TEXT".into(), 0),
                ("last_name".into(), "TEXT".into(), 0),
            ]
        );
    }

    #[test]
    fn enum_check_constraint_is_enforced() {
        let src = r#"
            unit U { id: string }
            enum Status { "active" "inactive" }
            store s { unit { U } attr { status: Status } }
        "#;
        let mut db = SqliteBackend::open_in_memory().unwrap();
        db.ensure_store(&schema(src, "s")).unwrap();

        db.conn
            .execute(
                "INSERT INTO \"s\" (\"id\", \"status\") VALUES ('a', 'active')",
                [],
            )
            .expect("valid enum value should insert");
        let bad = db.conn.execute(
            "INSERT INTO \"s\" (\"id\", \"status\") VALUES ('b', 'bogus')",
            [],
        );
        assert!(bad.is_err(), "value outside the enum must be rejected");
    }

    #[test]
    fn scan_decodes_typed_rows_in_key_order() {
        let src = r#"
            unit Machine { id: string }
            enum Status { "ok" "bad" }
            store readings {
              unit { Machine }
              attr {
                size: int
                temperature: real
                flag: bool
                at: date
                status: Status
                note: string?
              }
            }
        "#;
        let s = schema(src, "readings");
        let mut db = SqliteBackend::open_in_memory().unwrap();
        db.ensure_store(&s).unwrap();
        db.execute_sql(
            "INSERT INTO \"readings\" VALUES
               ('m2', 2, 21.5, 0, '2026-07-02', 'bad', NULL),
               ('m1', 1, 20.5, 1, '2026-07-01', 'ok', 'fine');",
        )
        .unwrap();

        let rows = db.scan(&s.shape()).unwrap();
        assert_eq!(
            rows,
            vec![
                vec![
                    Value::String("m1".into()),
                    Value::Int(1),
                    Value::Real(20.5),
                    Value::Bool(true),
                    Value::Date("2026-07-01".into()),
                    Value::Enum("ok".into()),
                    Value::String("fine".into()),
                ],
                vec![
                    Value::String("m2".into()),
                    Value::Int(2),
                    Value::Real(21.5),
                    Value::Bool(false),
                    Value::Date("2026-07-02".into()),
                    Value::Enum("bad".into()),
                    Value::Missing,
                ],
            ]
        );
    }

    #[test]
    fn materialize_view_replaces_contents_and_round_trips() {
        let shape = TableShape {
            name: "v".into(),
            columns: schema(PERSONS, "persons").columns,
            keyed: true,
            foreign_keys: Vec::new(),
            lateness: Vec::new(),
        };
        let mut db = SqliteBackend::open_in_memory().unwrap();

        let first = vec![vec![
            Value::String("a".into()),
            Value::Date("2000-01-01".into()),
            Value::String("x".into()),
        ]];
        db.materialize_view(&shape, &first).unwrap();
        assert_eq!(db.scan(&shape).unwrap(), first);

        // A second materialization replaces, not accumulates.
        let second = vec![
            vec![
                Value::String("b".into()),
                Value::Date("2001-01-01".into()),
                Value::String("y".into()),
            ],
            vec![
                Value::String("c".into()),
                Value::Date("2002-01-01".into()),
                Value::String("z".into()),
            ],
        ];
        db.materialize_view(&shape, &second).unwrap();
        assert_eq!(db.scan(&shape).unwrap(), second);
    }

    // --- the write path (`05-ingestion.md`, ADR 0034) ---------------------

    const FLEET: &str = r#"
        unit Machine { serial: string }
        unit Reading { machine: Machine  at: date }
        store machines {
          unit { Machine }
          attr { commissioned: date }
        }
        registry readings {
          unit { Reading }
          domain { machine: machines }
          attr { kelvin: real }
        }
    "#;

    fn opened(src: &str, tables: &[&str]) -> (SqliteBackend, Vec<TableShape>) {
        let tokens = mensura_syntax::tokenize(src).expect("should lex");
        let program = mensura_syntax::parse(&tokens).expect("should parse");
        let resolved = mensura_types::resolve(&program).expect("should resolve");
        let mut db = SqliteBackend::open_in_memory().unwrap();
        for schema in &resolved.schemas {
            db.ensure_store(schema).unwrap();
        }
        let shapes = tables
            .iter()
            .map(|t| {
                resolved
                    .schemas
                    .iter()
                    .find(|s| &s.store == t)
                    .unwrap_or_else(|| panic!("no table named {t}"))
                    .shape()
            })
            .collect();
        (db, shapes)
    }

    #[test]
    fn foreign_keys_are_enforced() {
        // The pragma is on (ADR 0034 decision 5), so the clauses ADR 0032
        // emits now bite.  The reading references a machine that was never
        // created.
        let (mut db, shapes) = opened(FLEET, &["readings"]);
        let err = db
            .apply(
                &shapes[0],
                &Delta::appending(vec![vec![
                    Value::String("m-01".into()),
                    Value::Date("2026-07-31".into()),
                    Value::Real(300.0),
                ]]),
            )
            .expect_err("the machine does not exist");
        let StorageError::ForeignKey { table, references } = &err else {
            panic!("expected a foreign-key error, got {err:?}");
        };
        assert_eq!(table, "readings");
        assert_eq!(
            references,
            &[("machine".to_string(), "machines".to_string())]
        );
        // The message names the declaration, not a SQLite code.
        let shown = err.to_string();
        assert!(shown.contains("`machines`"), "{shown}");
        assert!(shown.contains("`machine`"), "{shown}");
    }

    #[test]
    fn appending_lands_rows_and_a_satisfied_reference_passes() {
        let (mut db, shapes) = opened(FLEET, &["machines", "readings"]);
        let (machines, readings) = (&shapes[0], &shapes[1]);

        let applied = db
            .apply(
                machines,
                &Delta::appending(vec![vec![
                    Value::String("m-01".into()),
                    Value::Date("2026-01-05".into()),
                ]]),
            )
            .unwrap();
        assert_eq!(applied.inserted, 1);

        let rows = vec![
            vec![
                Value::String("m-01".into()),
                Value::Date("2026-07-30".into()),
                Value::Real(300.0),
            ],
            vec![
                Value::String("m-01".into()),
                Value::Date("2026-07-31".into()),
                Value::Real(312.5),
            ],
        ];
        assert_eq!(
            db.apply(readings, &Delta::appending(rows.clone()))
                .unwrap()
                .inserted,
            2
        );
        assert_eq!(db.scan(readings).unwrap(), rows);
    }

    #[test]
    fn a_singletons_target_rejects_a_repeated_key() {
        let (mut db, shapes) = opened(FLEET, &["machines"]);
        let row = vec![
            Value::String("m-01".into()),
            Value::Date("2026-01-05".into()),
        ];
        db.apply(&shapes[0], &Delta::appending(vec![row.clone()]))
            .unwrap();
        let err = db
            .apply(&shapes[0], &Delta::appending(vec![row]))
            .expect_err("a singletons tabulation holds one row per key");
        assert!(
            matches!(err, StorageError::DuplicateKey { .. }),
            "expected a duplicate-key error, got {err:?}"
        );
    }

    #[test]
    fn a_bag_target_accepts_a_repeated_key() {
        // ADR 0022: a bag holds many observations per entity, so the same
        // key twice is the point rather than an error.
        let src = r#"
            unit Machine { serial: string }
            registry pings { unit { Machine } attr* { latency_ms: int } }
        "#;
        let (mut db, shapes) = opened(src, &["pings"]);
        let rows = vec![
            vec![Value::String("m-01".into()), Value::Int(12)],
            vec![Value::String("m-01".into()), Value::Int(15)],
        ];
        assert_eq!(
            db.apply(&shapes[0], &Delta::appending(rows.clone()))
                .unwrap()
                .inserted,
            2
        );
        assert_eq!(db.scan(&shapes[0]).unwrap().len(), 2);
    }

    #[test]
    fn a_batch_is_all_or_nothing() {
        // One transaction (ADR 0034 decision 4): the good row ahead of the
        // bad one must not survive.
        let (mut db, shapes) = opened(FLEET, &["machines", "readings"]);
        db.apply(
            &shapes[0],
            &Delta::appending(vec![vec![
                Value::String("m-01".into()),
                Value::Date("2026-01-05".into()),
            ]]),
        )
        .unwrap();
        db.apply(
            &shapes[1],
            &Delta::appending(vec![
                vec![
                    Value::String("m-01".into()),
                    Value::Date("2026-07-30".into()),
                    Value::Real(300.0),
                ],
                vec![
                    Value::String("ghost".into()),
                    Value::Date("2026-07-31".into()),
                    Value::Real(312.5),
                ],
            ]),
        )
        .expect_err("the second row has no machine");
        assert!(db.scan(&shapes[1]).unwrap().is_empty());
    }

    #[test]
    fn deletes_remove_matching_rows() {
        let (mut db, shapes) = opened(FLEET, &["machines"]);
        let row = vec![
            Value::String("m-01".into()),
            Value::Date("2026-01-05".into()),
        ];
        db.apply(&shapes[0], &Delta::appending(vec![row.clone()]))
            .unwrap();
        let applied = db
            .apply(
                &shapes[0],
                &Delta {
                    inserts: Vec::new(),
                    deletes: vec![row],
                },
            )
            .unwrap();
        assert_eq!(applied.deleted, 1);
        assert!(db.scan(&shapes[0]).unwrap().is_empty());
    }

    #[test]
    fn the_foreign_key_pragma_is_on() {
        let db = SqliteBackend::open_in_memory().unwrap();
        let on: i64 = db
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(on, 1);
    }

    /// The `lateness` contract at the intake (ADR 0037 decision 4): the
    /// watermark is the maximum accepted point, a batch containing a row
    /// older than `watermark - bound` is rejected whole in the same
    /// transaction, the boundary point is still admissible, and an empty
    /// registry has no watermark, so its first batch is unconstrained.
    #[test]
    fn lateness_rejects_a_late_batch_whole() {
        let src = r#"
            import si
            unit Reading { machine_id: string  taken_at: instant }
            registry readings {
              unit { Reading }
              attr { temperature: real }
              lateness { taken_at: 10.0 * si.minute }
            }
        "#;
        let schema = schema(src, "readings");
        let shape = schema.shape();
        let mut db = SqliteBackend::open_in_memory().expect("open");
        db.ensure_store(&schema).expect("create");
        let row = |m: &str, at: &str| -> Row {
            vec![
                Value::String(m.into()),
                Value::Instant(at.into()),
                Value::Real(300.0),
            ]
        };
        // No watermark yet: the first batch of a grain is unconstrained,
        // and internal skew is legal either way, because rows are checked
        // against the watermark as of intake, never against each other.
        db.apply(
            &shape,
            &Delta::appending(vec![
                row("M-07", "2026-08-10T10:31:12.000Z"),
                row("M-07", "2026-08-10T09:00:00.000Z"),
            ]),
        )
        .expect("the first batch is unconstrained");
        // M-07's watermark is 10:31:12 and the bound is ten minutes, so its
        // oldest admissible point is 10:21:12, and one late row rejects the
        // whole batch (the fresh 10:32:00 row must not land either,
        // ADR 0034 decision 4).
        let err = db
            .apply(
                &shape,
                &Delta::appending(vec![
                    row("M-07", "2026-08-10T10:32:00.000Z"),
                    row("M-07", "2026-08-10T10:20:45.000Z"),
                ]),
            )
            .expect_err("one late row rejects the whole batch");
        assert!(matches!(err, StorageError::Lateness { .. }));
        let msg = err.to_string();
        assert!(msg.contains("2026-08-10T10:20:45.000Z"), "{msg}");
        assert!(msg.contains("2026-08-10T10:21:12.000Z"), "{msg}");
        assert!(msg.contains("`machine_id` = M-07"), "{msg}");
        assert_eq!(db.scan(&shape).expect("scan").len(), 2);
        // The rejected batch landed nothing, so M-07's observed maximum is
        // unmoved and the boundary point is still admissible, exactly
        // `watermark - bound`.
        db.apply(
            &shape,
            &Delta::appending(vec![row("M-07", "2026-08-10T10:21:12.000Z")]),
        )
        .expect("the boundary point is admissible");
        assert_eq!(db.scan(&shape).expect("scan").len(), 3);
        // A different machine is a different grain (ADR 0041 decision 2):
        // the same instant that M-07 refuses is fine for M-08, which has
        // no watermark of its own yet.
        db.apply(
            &shape,
            &Delta::appending(vec![row("M-08", "2026-08-10T10:20:45.000Z")]),
        )
        .expect("a grain is measured against itself");
        assert_eq!(db.scan(&shape).expect("scan").len(), 4);
    }

    /// The declared floor (ADR 0041 decisions 3 and 4): stored state that
    /// raises a grain's effective watermark, so an entity that has stopped
    /// reporting (or never started) is still governed.  It is the half
    /// that cannot be derived, and the half with no grain.
    #[test]
    fn a_declared_floor_raises_every_grain() {
        let src = r#"
            import si
            unit Reading { machine_id: string  taken_at: instant }
            registry readings {
              unit { Reading }
              attr { temperature: real }
              lateness { taken_at: 10.0 * si.minute }
            }
        "#;
        let schema = schema(src, "readings");
        let shape = schema.shape();
        let mut db = SqliteBackend::open_in_memory().expect("open");
        db.ensure_store(&schema).expect("create");
        let row = |m: &str, at: &str| -> Row {
            vec![
                Value::String(m.into()),
                Value::Instant(at.into()),
                Value::Real(300.0),
            ]
        };
        // With no floor, a brand-new grain accepts anything.
        db.apply(
            &shape,
            &Delta::appending(vec![row("M-01", "2026-08-10T09:00:00.000Z")]),
        )
        .expect("no floor, no watermark");
        // Declare the world closed through 10:40.  The floor applies to
        // *every* grain, including one that has never reported, which is
        // what lets a silent machine's windows close at all.
        db.advance_floor("readings", "taken_at", "2026-08-10T10:40:00.000Z")
            .expect("advance");
        let err = db
            .apply(
                &shape,
                &Delta::appending(vec![row("M-99", "2026-08-10T10:29:00.000Z")]),
            )
            .expect_err("below the floor minus the bound");
        assert!(
            err.to_string().contains("2026-08-10T10:30:00.000Z"),
            "{err}"
        );
        // The boundary under the floor is admissible, as under any
        // watermark.
        db.apply(
            &shape,
            &Delta::appending(vec![row("M-99", "2026-08-10T10:30:00.000Z")]),
        )
        .expect("the boundary point is admissible");
        // A floor never moves backwards: lowering it would reopen windows
        // already reported final, which is a deliberate override this
        // slice does not offer (ADR 0041 open questions).
        let err = db
            .advance_floor("readings", "taken_at", "2026-08-10T10:00:00.000Z")
            .expect_err("a floor only advances");
        assert!(err.to_string().contains("only advances"), "{err}");
    }

    /// `diff(int) = int` (ADR 0036 decision 4): a count-based contract runs
    /// through the same watermark with no unit machinery.
    #[test]
    fn lateness_counts_apply_to_int_points() {
        let src = r#"
            unit Tick { seq: int }
            registry ticks {
              unit { Tick }
              attr { level: real }
              lateness { seq: 5 }
            }
        "#;
        let schema = schema(src, "ticks");
        let shape = schema.shape();
        let mut db = SqliteBackend::open_in_memory().expect("open");
        db.ensure_store(&schema).expect("create");
        let row = |seq: i64| -> Row { vec![Value::Int(seq), Value::Real(1.0)] };
        db.apply(&shape, &Delta::appending(vec![row(10)]))
            .expect("first batch");
        let err = db
            .apply(&shape, &Delta::appending(vec![row(4)]))
            .expect_err("4 < 10 - 5");
        let msg = err.to_string();
        assert!(msg.contains("`seq` is 4"), "{msg}");
        db.apply(&shape, &Delta::appending(vec![row(5)]))
            .expect("the boundary count is admissible");
    }
}
