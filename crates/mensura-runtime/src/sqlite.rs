//! A SQLite-backed [`StorageBackend`] using rusqlite (bundled SQLite).

use std::path::Path;

use mensura_types::{ColumnRole, ColumnType, Schema};
use rusqlite::Connection;

use crate::backend::{EnsureOutcome, StorageBackend, StorageError};

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
        self.conn.execute_batch(&create_table_sql(schema))?;
        Ok(if existed {
            EnsureOutcome::AlreadyExists
        } else {
            EnsureOutcome::Created
        })
    }
}

/// Build the `CREATE TABLE IF NOT EXISTS` statement for a store.
pub fn create_table_sql(schema: &Schema) -> String {
    let mut lines: Vec<String> = schema
        .columns
        .iter()
        .map(|c| {
            format!(
                "  {} {}",
                quote_ident(&c.name),
                column_type_sql(&c.ty, &c.name)
            )
        })
        .collect();

    let index: Vec<String> = schema
        .columns
        .iter()
        .filter(|c| c.role == ColumnRole::Index)
        .map(|c| quote_ident(&c.name))
        .collect();
    if !index.is_empty() {
        lines.push(format!("  PRIMARY KEY ({})", index.join(", ")));
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {} (\n{}\n);",
        quote_ident(&schema.store),
        lines.join(",\n")
    )
}

fn column_type_sql(ty: &ColumnType, col: &str) -> String {
    match ty {
        ColumnType::String => "TEXT".to_string(),
        ColumnType::Number => "NUMERIC".to_string(),
        ColumnType::Bool => "INTEGER".to_string(),
        ColumnType::Date => "TEXT".to_string(),
        ColumnType::Enum(variants) => {
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
            .into_iter()
            .find(|s| s.store == store)
            .unwrap_or_else(|| panic!("no store named {store}"))
    }

    const PERSONS: &str = r#"
        unit Person { id: string }
        store Persons {
          unit { Person }
          const { birthdate: date }
          var   { last_name: string }
        }
    "#;

    #[test]
    fn create_table_sql_for_persons() {
        let sql = create_table_sql(&schema(PERSONS, "Persons"));
        assert_eq!(
            sql,
            "CREATE TABLE IF NOT EXISTS \"Persons\" (\n  \"id\" TEXT,\n  \"birthdate\" TEXT,\n  \"last_name\" TEXT,\n  PRIMARY KEY (\"id\")\n);"
        );
    }

    #[test]
    fn create_table_sql_for_enum_has_check() {
        let src = r#"
            unit U { id: string }
            store S { unit { U } var { status: enum("active", "inactive") } }
        "#;
        let sql = create_table_sql(&schema(src, "S"));
        assert!(sql.contains("\"status\" TEXT CHECK (\"status\" IN ('active', 'inactive'))"));
    }

    #[test]
    fn ensure_store_creates_then_reports_existing() {
        let mut db = SqliteBackend::open_in_memory().unwrap();
        let s = schema(PERSONS, "Persons");
        assert_eq!(db.ensure_store(&s).unwrap(), EnsureOutcome::Created);
        assert_eq!(db.ensure_store(&s).unwrap(), EnsureOutcome::AlreadyExists);

        // Columns, types, and the primary key are as declared.
        let cols: Vec<(String, String, i64)> = db
            .conn
            .prepare("PRAGMA table_info(\"Persons\")")
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
            store S { unit { U } var { status: enum("active", "inactive") } }
        "#;
        let mut db = SqliteBackend::open_in_memory().unwrap();
        db.ensure_store(&schema(src, "S")).unwrap();

        db.conn
            .execute(
                "INSERT INTO \"S\" (\"id\", \"status\") VALUES ('a', 'active')",
                [],
            )
            .expect("valid enum value should insert");
        let bad = db.conn.execute(
            "INSERT INTO \"S\" (\"id\", \"status\") VALUES ('b', 'bogus')",
            [],
        );
        assert!(bad.is_err(), "value outside the enum must be rejected");
    }
}
