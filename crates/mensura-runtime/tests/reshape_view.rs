//! End-to-end materialization of the reshape round trip (ADR 0020): the
//! totality the checker derives must match what the evaluator writes, or
//! `materialize_view` would insert NULL into a NOT NULL spread column.
//! Both directions are exercised: a total fold whose spread columns come
//! back NOT NULL, and a sparse fold whose absent long row round-trips to a
//! NULL cell in a nullable column.

use mensura_runtime::{SqliteBackend, StorageBackend, Value, materialize_views};
use mensura_types::ResolvedProgram;

fn program(src: &str) -> ResolvedProgram {
    let tokens = mensura_syntax::tokenize(src).expect("should lex");
    let program = mensura_syntax::parse(&tokens).expect("should parse");
    mensura_types::resolve(&program).expect("should resolve")
}

fn materialize(src: &str, seed: &str) -> (ResolvedProgram, SqliteBackend, Vec<(String, usize)>) {
    let program = program(src);
    let mut db = SqliteBackend::open_in_memory().unwrap();
    for schema in &program.schemas {
        db.ensure_store(schema).unwrap();
    }
    db.execute_sql(seed).unwrap();
    let materialized = materialize_views(&mut db, &program).unwrap();
    (program, db, materialized)
}

#[test]
fn a_total_fold_round_trips_into_not_null_spread_columns() {
    let (program, db, materialized) = materialize(
        r#"
        unit Reading { ts: int }
        store readings { unit { Reading } attr { temperature: real } attr { humidity: real } }
        view wide {
          readings |> unpivot metric reading |> pivot metric reading
        }
        "#,
        r#"INSERT INTO "readings" VALUES (1, 20.0, 30.0), (2, 21.0, 31.0);"#,
    );
    assert_eq!(materialized, vec![("wide".to_string(), 2)]);

    // The rectangle held by mechanism, so the spread columns are total and
    // the values round-trip exactly (`pivot_unpivotDrop`).
    let view = &program.views[0];
    assert_eq!(
        db.scan(&view.shape()).unwrap(),
        vec![
            vec![Value::Int(1), Value::Real(20.0), Value::Real(30.0)],
            vec![Value::Int(2), Value::Real(21.0), Value::Real(31.0)],
        ]
    );
}

#[test]
fn a_sparse_fold_round_trips_missing_cells_through_nullable_columns() {
    let (program, db, materialized) = materialize(
        r#"
        unit Reading { ts: int }
        store readings { unit { Reading } attr { temperature: real } attr { humidity: real? } }
        view wide {
          readings |> unpivot metric reading |> pivot metric reading
        }
        "#,
        r#"INSERT INTO "readings" VALUES (1, 20.0, 30.0), (2, 21.0, NULL);"#,
    );
    assert_eq!(materialized, vec![("wide".to_string(), 2)]);

    // The missing humidity cell dropped its long row and came back as a
    // missing cell; the checker made the spread columns optional, so the
    // insert accepts it.
    let view = &program.views[0];
    assert_eq!(
        db.scan(&view.shape()).unwrap(),
        vec![
            vec![Value::Int(1), Value::Real(20.0), Value::Real(30.0)],
            vec![Value::Int(2), Value::Real(21.0), Value::Missing],
        ]
    );
}
