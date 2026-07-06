//! A SQLite-backed [`StorageBackend`] using rusqlite (bundled SQLite).

use std::path::Path;

use mensura_types::{ColumnRole, ColumnType, Schema, TableShape};
use rusqlite::Connection;
use rusqlite::types::ValueRef;

use crate::backend::{EnsureOutcome, StorageBackend, StorageError};
use crate::value::{Row, Value};

/// A store backend that materializes schemas as SQLite tables.
pub struct SqliteBackend {
    conn: Connection,
}

impl SqliteBackend {
    /// Open (or create) a database at `path`.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(SqliteBackend {
            conn: Connection::open(path)?,
        })
    }

    /// Open a transient in-memory database (used in tests).
    pub fn open_in_memory() -> Result<Self, StorageError> {
        Ok(SqliteBackend {
            conn: Connection::open_in_memory()?,
        })
    }

    /// Execute raw SQL against the backing database.  A test scaffold: until
    /// M4's typed ingestion exists, tests seed store rows at the SQL level
    /// (`docs/toolkit/04-processing-layer.md`, "Validation").  Not a language
    /// surface.
    pub fn execute_sql(&self, sql: &str) -> Result<(), StorageError> {
        self.conn.execute_batch(sql)?;
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
        self.conn
            .execute_batch(&create_table_sql(&schema.shape()))?;
        Ok(if existed {
            EnsureOutcome::AlreadyExists
        } else {
            EnsureOutcome::Created
        })
    }

    fn scan(&self, table: &TableShape) -> Result<Vec<Row>, StorageError> {
        let cols: Vec<String> = table.columns.iter().map(|c| quote_ident(&c.name)).collect();
        let index: Vec<String> = table
            .columns
            .iter()
            .filter(|c| c.role == ColumnRole::Index)
            .map(|c| quote_ident(&c.name))
            .collect();
        let order = if index.is_empty() {
            String::new()
        } else {
            format!(" ORDER BY {}", index.join(", "))
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
}

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
        ColumnType::Enum { .. } => cell.as_str().map(|s| Value::Enum(s.to_string())),
        ColumnType::Int => cell.as_i64().map(Value::Int),
        ColumnType::Real => cell.as_f64().map(Value::Real),
        ColumnType::Bool => cell.as_i64().map(|i| Value::Bool(i != 0)),
    }
    .map_err(|_| mismatch())
}

/// Encode one typed [`Value`] as a SQL parameter.  [`Value::Missing`] is `NULL`.
fn encode(value: &Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sql;
    match value {
        Value::String(s) | Value::Date(s) | Value::Enum(s) => Sql::Text(s.clone()),
        Value::Int(i) => Sql::Integer(*i),
        Value::Real(r) => Sql::Real(*r),
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Missing => Sql::Null,
    }
}

/// Build the `CREATE TABLE IF NOT EXISTS` statement for a table shape.  A
/// keyed shape (a store, or a `singletons` view) gets the composite primary
/// key over its index columns; a `bag` view gets none
/// (`docs/toolkit/04-processing-layer.md`).
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

    let index: Vec<String> = shape
        .columns
        .iter()
        .filter(|c| c.role == ColumnRole::Index)
        .map(|c| quote_ident(&c.name))
        .collect();
    if shape.keyed && !index.is_empty() {
        lines.push(format!("  PRIMARY KEY ({})", index.join(", ")));
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n);",
        quote_ident(&shape.name),
        lines.join(",\n")
    )
}

fn column_type_sql(ty: &ColumnType, col: &str) -> String {
    match ty {
        ColumnType::String => "TEXT".to_string(),
        ColumnType::Int => "INTEGER".to_string(),
        ColumnType::Real => "REAL".to_string(),
        ColumnType::Bool => "INTEGER".to_string(),
        ColumnType::Date => "TEXT".to_string(),
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
        // (`docs/toolkit/04-processing-layer.md`); its index columns stay
        // `NOT NULL`.
        let mut shape = schema(PERSONS, "persons").shape();
        shape.keyed = false;
        let sql = create_table_sql(&shape);
        assert!(!sql.contains("PRIMARY KEY"));
        assert!(sql.contains("\"id\" TEXT NOT NULL"));
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
    fn scan_decodes_typed_rows_in_index_order() {
        let src = r#"
            unit Machine { id: string }
            enum Status { "ok", "bad" }
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
}
