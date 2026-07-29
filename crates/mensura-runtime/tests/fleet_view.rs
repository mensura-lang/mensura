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
/// latter, ADR 0022).  The dimensioned `temperature` column stores plain
/// base-unit (kelvin) magnitudes (`docs/toolkit/00-storage-backend.md`).
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
           -- `m1`'s two readings are inserted out of order under `taken_at`,
           -- so a window that ignores the key would be indistinguishable from
           -- one that honours it (ADR 0031, Decision 7).
           INSERT INTO "readings" VALUES
             ('m1', 302.5, '2025-01-02'),
             ('m1', 300.0, '2025-01-01'),
             ('m2', 299.0, '2025-01-01'),
             ('m3', 371.5, '2025-01-01');"#,
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
fn machine_temperature_reduces_the_bag_store() {
    // The end-to-end bag-store path (ADR 0022 + ADR 0023): the `readings`
    // bag holds several rows per machine, the view assumes completeness and
    // the reducing `map_bags` folds each machine's bag to its maximum.  The
    // column is dimensioned (`temperature[real]`, ADR 0026); at runtime it
    // is a plain base-unit magnitude.
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
    // Stage 2).  `m1`'s two readings are *seeded out of order* under
    // `taken_at`, so this distinguishes a scan that honours the key from one
    // that walks the bag as stored.
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

    // Columns, in checker order: id, running_peak, previous, rank_hottest.
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
    // And the descending ones-scan ranks the hotter reading first, which is
    // the *later* one here: direction is per-component, on the key value.
    assert_eq!(later[3], Value::Int(1));
    assert_eq!(earlier[3], Value::Int(2));

    // A single-reading machine: its only row is also its first, so `previous`
    // is missing and the running peak is its own value.
    let m3 = rows
        .iter()
        .find(|r| r[0] == Value::String("m3".into()))
        .expect("m3 has a reading");
    assert_eq!(m3[2], Value::Missing);
    assert_eq!(m3[1], Value::Real(371.5));
    // The ones-scan over a descending key ranks the hottest reading first.
    assert_eq!(m3[3], Value::Int(1));
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
