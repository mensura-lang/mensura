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

/// Create the example's stores and seed both singletons stores: `machines`
/// and the `readings` history keyed by `(machine_id, taken_at)` that the
/// views `demote` (ADR 0024).  The dimensioned `temperature` column stores
/// plain base-unit (kelvin) magnitudes (`docs/toolkit/00-storage-backend.md`).
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
           -- `m1`'s two readings are inserted out of order under `taken_at`.
           -- The store scan orders by the full key, so they arrive sorted
           -- anyway; the scan-order discrimination for the window operators
           -- lives in the evaluator's unit tests.
           INSERT INTO "readings" VALUES
             ('m1', '2025-01-02', 302.5),
             ('m1', '2025-01-01', 300.0),
             ('m2', '2025-01-01', 299.0),
             ('m3', '2025-01-01', 371.5);"#,
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
            // The window view emits one row per reading, not one per machine:
            // four readings in, four rows out (ADR 0031, Decision 7).
            ("reading_trend".to_string(), 4),
            ("machine_temperature".to_string(), 3),
            ("overheating".to_string(), 1),
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
    assert_eq!(again.len(), 4);
    assert_eq!(db.scan(&view.shape()).unwrap().len(), 1);
}

#[test]
fn machine_temperature_reduces_the_demoted_history() {
    // The end-to-end key-coarsening path (ADR 0024 + ADR 0023 + ADR 0035):
    // `demote taken_at` drops the time out of the key, leaving a bag of
    // readings per machine, and the reducing `map_bags` folds each machine's
    // bag to its maximum.  The coarsening forfeits the registry's own-key
    // completeness, so the view claims the fact after the `demote` with an
    // `assume { complete }`, a runtime identity (eval.rs): evaluation is
    // unchanged.  The column is dimensioned (`temperature[real]`, ADR 0026);
    // at runtime it is a plain base-unit magnitude.
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
            vec![Value::String("m3".into()), Value::Real(371.5)],
        ]
    );
}

#[test]
fn reading_trend_scans_each_machine_in_key_order() {
    // The window path end to end (ADR 0031 Decision 7, gated by ADR 0029's
    // Stage 2), over a demoted history: no `assume { arranged }`, because
    // the `(machine_id, taken_at)` grading survives the `demote` and
    // discharges the scan's tie-freedom (ADR 0024).
    let program = fleet_program();
    let mut db = seeded_db(&program);
    materialize_views(&mut db, &program).unwrap();

    let view = program
        .views
        .iter()
        .find(|v| v.name == "reading_trend")
        .expect("the example declares reading_trend");
    let rows = db.scan(&view.shape()).unwrap();
    assert_eq!(rows.len(), 4, "one output row per input reading");

    // Columns, in the record's declaration order: machine_id, running_peak,
    // previous, next.
    // `m1` at 2025-01-01 is the earlier reading (300.0) even though it was
    // inserted second, so it is the one whose `previous` is missing and whose
    // running peak is its own value.
    let m1: Vec<&Vec<Value>> = rows
        .iter()
        .filter(|r| r[0] == Value::String("m1".into()))
        .collect();
    assert_eq!(m1.len(), 2);
    let earlier = m1
        .iter()
        .find(|r| r[1] == Value::Real(300.0))
        .expect("the earlier reading's running peak is its own value");
    assert_eq!(
        earlier[2],
        Value::Missing,
        "the first row under the order has no predecessor: `lag` is a \
         `prescan` at keep-right, which has no identity"
    );
    let later = m1
        .iter()
        .find(|r| r[1] == Value::Real(302.5))
        .expect("the later reading's running peak is the group max so far");
    assert_eq!(
        later[2],
        Value::Real(300.0),
        "the later row's predecessor is the earlier one *under the key*, not \
         under the insertion order"
    );
    // `lead` is the mirror: it is `lag` at the dual key, so the *last* row
    // under the order is the missing one, not the first.  That symmetry is not
    // coded anywhere; it falls out of `lead` reusing `prescan` at `desc`.
    assert_eq!(earlier[3], Value::Real(302.5));
    assert_eq!(later[3], Value::Missing);

    // A single-reading machine: its only row is also its first, so `previous`
    // is missing and the running peak is its own value.
    let m3 = rows
        .iter()
        .find(|r| r[0] == Value::String("m3".into()))
        .expect("m3 has a reading");
    assert_eq!(m3[1], Value::Real(371.5));
    // Its only row is both the first and the last under the order, so both
    // neighbours are absent.
    assert_eq!(m3[2], Value::Missing);
    assert_eq!(m3[3], Value::Missing);
}

#[test]
fn overheating_compares_against_the_lowered_const() {
    // The units path end to end (ADR 0026/0027): `overheat` is a top-level
    // const (`350.0 * kelvin`) folded into the view body at resolve time,
    // so the runtime compares plain magnitudes and only the reading above
    // 350 kelvin survives the filter.
    let program = fleet_program();
    let mut db = seeded_db(&program);
    materialize_views(&mut db, &program).unwrap();

    let view = program
        .views
        .iter()
        .find(|v| v.name == "overheating")
        .expect("the example declares overheating");
    let rows = db.scan(&view.shape()).unwrap();
    assert_eq!(
        rows,
        vec![vec![
            Value::String("m3".into()),
            Value::Date("2025-01-01".into()),
            Value::Real(371.5),
        ]]
    );
}
