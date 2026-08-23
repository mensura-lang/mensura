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
    // Temporal columns hold what the decoder would have produced: `date`
    // stays `YYYY-MM-DD`, and the `instant` columns (ADR 0036) hold the
    // normalized fixed-width UTC form.
    // The `activated` instants are a fleet-wide sensor retrofit rather than
    // each machine's own commissioning: `dense` completes the grid from this
    // bound to the closed bound, so a bound years before the readings would
    // ask for a row per interval per machine over those years, which is the
    // row-count consequence ADR 0038 records.  The paperwork dates stay where
    // they were, which is also the ADR 0036 point: the two are different
    // facts, and neither converts to the other without a zone.
    db.execute_sql(
        r#"INSERT INTO "machines" VALUES
             ('m1', '2020-01-01', '2024-12-31T00:00:00.000Z', 'operational', NULL),
             ('m2', '2021-06-15', '2024-12-31T00:00:00.000Z', 'degraded', '2025-12-01'),
             ('m3', '2022-03-10', '2024-12-31T00:00:00.000Z', 'failure', NULL);
           -- `m1`'s two readings are inserted out of order under `taken_at`.
           -- The store scan orders by the full key, so they arrive sorted
           -- anyway; the scan-order discrimination for the window operators
           -- lives in the evaluator's unit tests.
           INSERT INTO "readings" VALUES
             ('m1', '2025-01-02T10:00:00.000Z', 302.5),
             ('m1', '2025-01-01T10:00:00.000Z', 300.0),
             ('m2', '2025-01-01T10:00:00.000Z', 299.0),
             ('m3', '2025-01-01T10:00:00.000Z', 371.5);
           -- The wide sensor table the reshape views fold and spread
           -- (ADR 0020).  Both columns are total, which is what makes
           -- `unpivot` establish `exhaustive(sensor)`.
           INSERT INTO "paired_readings" VALUES
             ('m1', '2025-01-01T10:00:00.000Z', 300.0, 291.0),
             ('m2', '2025-01-01T10:00:00.000Z', 299.0, 288.5);
           -- The entity-keyed bag registry (ADR 0022): many samples per
           -- machine, and the bag is whole by mechanism, which is what lets
           -- `machine_vibration` reduce with no `assume { complete }`.
           -- An unkeyed table, so the key columns repeat.
           INSERT INTO "vibrations" ("machine_id", "sampled_at", "amplitude")
             VALUES
             ('m1', '2025-01-01T10:00:00.000Z', 0.4),
             ('m1', '2025-01-02T10:00:00.000Z', 0.9),
             ('m2', '2025-01-01T10:00:00.000Z', 0.2);"#,
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
            // One closed window across the whole fleet, and *which* one is
            // the point of graining the watermark (ADR 0041).  Each of the
            // four readings falls in its own quarter-hour window, and a
            // window closes when `w + 15 min + 10 min` has passed its own
            // machine's watermark.  Only `m1` has a later reading, so only
            // `m1`'s 01-01 window is final; `m2` and `m3` have nothing past
            // their latest reading, so their windows stay open and are
            // absent rather than wrong.  Under one global watermark `m1`'s
            // 01-02 reading would have closed `m2`'s and `m3`'s windows
            // too, and this count would read 3.
            ("machine_peaks".to_string(), 1),
            // The grid completed (ADR 0038).  Daily windows, so `closed`
            // keeps `m1`'s 01-01 window (its 01-02 reading pushed its
            // watermark past `w + 1 day + 10 min`) and nothing else: `m2`
            // and `m3` have no reading past their own, so their 01-01
            // windows are still open.  `dense` then completes each machine's
            // grid from its `activated` bound (12-31) to its own closed
            // bound: `m1` gains 12-31 beside the row it reduced, `m2` and
            // `m3` gain 12-31 alone.  Four rows, three of them for days on
            // which nothing was reported, which is exactly what no view
            // above this one can say.
            ("sensor_health".to_string(), 4),
            // And the query the fill exists for: one row per machine, with
            // no `assume` anywhere, because `demote w` re-derives the
            // completeness `dense` established (ADR 0038 decision 4).
            ("silence_per_machine".to_string(), 3),
            // The window view emits one row per reading, not one per machine:
            // four readings in, four rows out (ADR 0031, Decision 7).
            ("reading_trend".to_string(), 4),
            // The rate view keeps every row too; the rate column is absent
            // on each machine's first reading (ADR 0039).
            ("reading_rate".to_string(), 4),
            // The `?? false` policy keeps first readings out: only `m1`'s
            // second reading is warmer than its predecessor.
            ("warming".to_string(), 1),
            ("machine_temperature".to_string(), 3),
            // The bag registry reduces at its own key with no `assume`
            // (ADR 0033): three samples across two machines, two rows out.
            ("machine_vibration".to_string(), 2),
            // `latest` is a reduction, so one row per fiber: two machines
            // with vibration samples, three with readings (ADR 0037
            // decision 7).
            ("newest_vibration".to_string(), 2),
            ("newest_reading".to_string(), 3),
            ("overheating".to_string(), 1),
            // `unpivot` makes `sensor` an exhaustive axis, so demoting it is
            // a rectangular coarsening and the fold needs no `assume`
            // (ADR 0035's adopted rule): two slots in, two averages out.
            ("sensor_avg".to_string(), 2),
        ]
    );

    let view = program
        .views
        .iter()
        .find(|v| v.name == "attention_needed")
        .expect("the example declares attention_needed");
    let rows = db.scan(&view.shape()).unwrap();
    // The whole-row `flat_map` body yields the attributes in checker
    // (alphabetical) order: activated, commissioned, last_service, status.
    assert_eq!(
        rows,
        vec![vec![
            Value::String("m2".into()),
            Value::Instant("2024-12-31T00:00:00.000Z".into()),
            Value::Date("2021-06-15".into()),
            Value::Date("2025-12-01".into()),
            Value::Enum("degraded".into()),
        ]]
    );

    // A re-run over unchanged stores replaces the contents, not appends.
    let again = materialize_views(&mut db, &program).unwrap();
    assert_eq!(again, materialized);
    assert_eq!(db.scan(&view.shape()).unwrap().len(), 1);
}

/// **Closed windows are final** (ADR 0037 decision 4, the invariant
/// `Mensura.closedWindow_stable` proves): rerunning after further
/// ingestion adds newly closed windows and never changes one already
/// emitted.  This is what the refresh slice will lean on, and what lets
/// alerting treat each row as settled.
/// The case ADR 0038 exists for and ADR 0041 had to make possible: a machine
/// whose sensor never reported at all still gets its silent slots.
///
/// Its own watermark is absent (nothing was ever accepted in its grain), so
/// nothing licenses closing any of its windows on the data alone, and `dense`
/// fills nothing.  The declared closure floor is what supplies the bound
/// (ADR 0041 decision 3), and it is the half of the watermark no data
/// derives: an operator assertion that the world is closed through a point,
/// stored beside the data rather than read from the clock, so `mensura run`
/// stays reproducible.
#[test]
fn a_machine_that_never_reported_still_gets_its_silent_slots() {
    let program = fleet_program();
    let mut db = seeded_db(&program);
    db.execute_sql(
        r#"INSERT INTO "machines" VALUES
             ('m4', '2024-11-02', '2024-12-31T00:00:00.000Z', 'operational', NULL);"#,
    )
    .unwrap();
    let view = program
        .views
        .iter()
        .find(|v| v.name == "sensor_health")
        .expect("the example declares sensor_health");

    // Without a floor, `m4` has no watermark, so none of its windows is
    // closed and its silence is invisible rather than reported.
    materialize_views(&mut db, &program).unwrap();
    let rows = db.scan(&view.shape()).unwrap();
    assert!(
        !rows.iter().any(|r| r[0] == Value::String("m4".into())),
        "an unobserved grain has no closed windows: {rows:?}"
    );

    // Declare the world closed through 01-02, and the silence becomes rows:
    // 12-31 and 01-01, the daily slots from `m4`'s activation whose whole
    // extent plus the lateness bound lies below the floor.
    db.advance_floor("readings", "taken_at", "2025-01-02T00:10:00.000Z")
        .unwrap();
    materialize_views(&mut db, &program).unwrap();
    let rows = db.scan(&view.shape()).unwrap();
    let silent: Vec<&Vec<Value>> = rows
        .iter()
        .filter(|r| r[0] == Value::String("m4".into()))
        .collect();
    assert_eq!(
        silent,
        vec![
            // `n` fills from `+`'s identity, and it is true: zero readings
            // were reduced.  `peak` has no identity to fill from, so it is
            // absent, which is what says "no readings" rather than "a low
            // peak" (ADR 0038 decision 2).
            &vec![
                Value::String("m4".into()),
                Value::Instant("2024-12-31T00:00:00.000Z".into()),
                Value::Int(0),
                Value::Missing,
            ],
            &vec![
                Value::String("m4".into()),
                Value::Instant("2025-01-01T00:00:00.000Z".into()),
                Value::Int(0),
                Value::Missing,
            ],
        ]
    );

    // And the query the fill exists for reports it, with no `assume`.
    let silence = program
        .views
        .iter()
        .find(|v| v.name == "silence_per_machine")
        .expect("the example declares silence_per_machine");
    let rows = db.scan(&silence.shape()).unwrap();
    assert!(
        rows.contains(&vec![Value::String("m4".into()), Value::Int(2)]),
        "the silence count should hold `m4`'s two silent days: {rows:?}"
    );
}

#[test]
fn closed_windows_are_final_under_further_ingestion() {
    let program = fleet_program();
    let mut db = seeded_db(&program);
    let view = program
        .views
        .iter()
        .find(|v| v.name == "machine_peaks")
        .expect("the example declares machine_peaks");

    materialize_views(&mut db, &program).unwrap();
    let before = db.scan(&view.shape()).unwrap();
    // `m1`'s 01-01 window, closed by `m1`'s own later reading.
    assert_eq!(
        before,
        vec![vec![
            Value::String("m1".into()),
            Value::Instant("2025-01-01T10:00:00.000Z".into()),
            Value::Real(300.0),
        ]]
    );

    // More readings arrive: one inside the already-closed window (which the
    // intake would have refused in practice, and which is seeded here
    // precisely to show the emitted row does not move), and one far later
    // that closes further windows for `m2`.
    db.execute_sql(
        r#"INSERT INTO "readings" VALUES
             ('m2', '2025-01-03T10:00:00.000Z', 305.0),
             ('m3', '2025-01-03T10:00:00.000Z', 372.0);"#,
    )
    .unwrap();
    materialize_views(&mut db, &program).unwrap();
    let after = db.scan(&view.shape()).unwrap();

    // The previously emitted row is byte-identical, and the newly closed
    // windows are additions.
    assert!(
        after.contains(&before[0]),
        "a closed window changed under further ingestion: {after:?}"
    );
    assert!(
        after.len() > before.len(),
        "later readings should close further windows: {after:?}"
    );
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
fn reading_rate_is_absent_on_first_readings_and_exact_after() {
    // The taught idiom end to end (ADR 0036 + ADR 0039): the lifted torsor
    // difference recovers the true elapsed seconds, the lifted division
    // yields the dimensioned rate, and each machine's first reading carries
    // an honest NULL instead of a fabricated zero.
    let program = fleet_program();
    let mut db = seeded_db(&program);
    materialize_views(&mut db, &program).unwrap();

    let view = program
        .views
        .iter()
        .find(|v| v.name == "reading_rate")
        .expect("the example declares reading_rate");
    let rows = db.scan(&view.shape()).unwrap();
    assert_eq!(rows.len(), 4, "one output row per input reading");

    // `m1` warmed from 300.0 K to 302.5 K over exactly one day.
    let m1: Vec<&Vec<Value>> = rows
        .iter()
        .filter(|r| r[0] == Value::String("m1".into()))
        .collect();
    let first = m1
        .iter()
        .find(|r| r[1] == Value::Instant("2025-01-01T10:00:00.000Z".into()))
        .expect("the earlier reading");
    assert_eq!(first[2], Value::Missing, "no predecessor, no rate");
    let later = m1
        .iter()
        .find(|r| r[1] == Value::Instant("2025-01-02T10:00:00.000Z".into()))
        .expect("the later reading");
    assert_eq!(later[2], Value::Real(2.5 / 86400.0));

    // Single-reading machines have no rate anywhere.
    let m3 = rows
        .iter()
        .find(|r| r[0] == Value::String("m3".into()))
        .expect("m3 has a reading");
    assert_eq!(m3[2], Value::Missing);
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
            Value::Instant("2025-01-01T10:00:00.000Z".into()),
            Value::Real(371.5),
        ]]
    );
}
