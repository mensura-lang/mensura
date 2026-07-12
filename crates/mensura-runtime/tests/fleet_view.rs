//! End-to-end materialization of the committed fleet example
//! (`docs/toolkit/04-processing-layer.md`, "Validation"): create the stores,
//! seed `machines`, materialize `attention_needed`, and assert exactly the
//! degraded rows come back.  Seeding happens at the SQL level through the
//! backend; that is a test scaffold until M4's typed ingestion exists.

use mensura_runtime::{SqliteBackend, StorageBackend, Value, materialize_views};
use mensura_types::ResolvedProgram;

fn fleet_program() -> ResolvedProgram {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/examples/fleet-monitoring.mensura");
    let src = std::fs::read_to_string(&path).expect("readable example");
    let tokens = mensura_syntax::tokenize(&src).expect("should lex");
    let program = mensura_syntax::parse(&tokens).expect("should parse");
    mensura_types::resolve(&program).expect("should resolve")
}

/// Create the example's stores and seed both: the `machines` singletons
/// store and the `readings` bag store (duplicate keys are the point of the
/// latter, ADR 0022).
fn seeded_db(program: &ResolvedProgram) -> SqliteBackend {
    let mut db = SqliteBackend::open_in_memory().unwrap();
    for schema in &program.schemas {
        db.ensure_store(schema).unwrap();
    }
    db.execute_sql(
        r#"INSERT INTO "machines" VALUES
             ('m1', '2020-01-01', 'operational', NULL),
             ('m2', '2021-06-15', 'degraded', '2025-12-01'),
             ('m3', '2022-03-10', 'failure', NULL);
           INSERT INTO "readings" VALUES
             ('m1', 300.0),
             ('m1', 302.5),
             ('m2', 299.0);"#,
    )
    .unwrap();
    db
}

#[test]
fn attention_needed_materializes_the_degraded_machines() {
    let program = fleet_program();
    let mut db = seeded_db(&program);

    let materialized = materialize_views(&mut db, &program).unwrap();
    assert_eq!(
        materialized,
        vec![
            ("attention_needed".to_string(), 1),
            ("machine_temperature".to_string(), 2),
        ]
    );

    let view = program
        .views
        .iter()
        .find(|v| v.name == "attention_needed")
        .expect("the example declares attention_needed");
    let rows = db.scan(&view.shape()).unwrap();
    // The whole-row `flat_map` body yields the attributes in checker
    // (alphabetical) order: commissioned, last_service, status.
    assert_eq!(
        rows,
        vec![vec![
            Value::String("m2".into()),
            Value::Date("2021-06-15".into()),
            Value::Date("2025-12-01".into()),
            Value::Enum("degraded".into()),
        ]]
    );

    // A re-run over unchanged stores replaces the contents, not appends.
    let again = materialize_views(&mut db, &program).unwrap();
    assert_eq!(again.len(), 2);
    assert_eq!(db.scan(&view.shape()).unwrap().len(), 1);
}

#[test]
fn machine_temperature_reduces_the_bag_store() {
    // The end-to-end bag-store path (ADR 0022 + ADR 0023): the `readings`
    // bag holds several rows per machine, the view assumes completeness and
    // the reducing `map_bags` folds each machine's bag to its maximum.
    let program = fleet_program();
    let mut db = seeded_db(&program);
    materialize_views(&mut db, &program).unwrap();

    let view = program
        .views
        .iter()
        .find(|v| v.name == "machine_temperature")
        .expect("the example declares machine_temperature");
    let rows = db.scan(&view.shape()).unwrap();
    assert_eq!(
        rows,
        vec![
            vec![Value::String("m1".into()), Value::Real(302.5)],
            vec![Value::String("m2".into()), Value::Real(299.0)],
        ]
    );
}
