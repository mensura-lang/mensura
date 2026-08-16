//! The M4 loop end to end: ingest a batch, then materialize the views over
//! it (`docs/toolkit/05-ingestion.md`, ADRs 0033 and 0034).
//!
//! This exercises the whole slice against the committed fleet example: the
//! typed decoder, the delta-shaped write path, enforced foreign keys, and
//! the completeness-by-mechanism that lets `machine_temperature` reduce with
//! no `assume { complete }` anywhere in the program.

use mensura_runtime::{
    Delta, SqliteBackend, StorageBackend, Value, decode_jsonl, materialize_views,
};
use mensura_syntax::{parse, tokenize};
use mensura_types::{ResolvedProgram, Schema};

const FLEET: &str = include_str!("../../../docs/examples/fleet-monitoring.mensura");

fn resolve(src: &str) -> ResolvedProgram {
    let tokens = tokenize(src).expect("should lex");
    let program = parse(&tokens).expect("should parse");
    mensura_types::resolve(&program).expect("should resolve")
}

fn table<'a>(program: &'a ResolvedProgram, name: &str) -> &'a Schema {
    program
        .schemas
        .iter()
        .find(|s| s.store == name)
        .unwrap_or_else(|| panic!("no table named {name}"))
}

/// Open a database with every table of `program` created.
fn seeded(program: &ResolvedProgram) -> SqliteBackend {
    let mut db = SqliteBackend::open_in_memory().expect("in-memory database");
    for schema in &program.schemas {
        db.ensure_store(schema).expect("create table");
    }
    db
}

#[test]
fn ingesting_the_fleet_registry_feeds_its_views() {
    let program = resolve(FLEET);
    let mut db = seeded(&program);

    // `readings` has no `domain`, so it needs no machines to reference; the
    // views below nonetheless read both tables.
    let machines = table(&program, "machines");
    let rows = decode_jsonl(
        machines,
        r#"{"machine_id":"m-01","commissioned":"2026-01-05","activated":"2026-01-09T08:00:00Z","status":"operational","last_service":null}
{"machine_id":"m-02","commissioned":"2026-02-11","activated":"2026-02-15T09:30:00Z","status":"degraded","last_service":"2026-06-01"}
"#,
    )
    .expect("machines decode");
    assert_eq!(
        db.apply(&machines.shape(), &Delta::appending(rows))
            .expect("append machines")
            .inserted,
        2
    );

    // The producer emits a local offset; the decoder stores normalized UTC
    // (ADR 0036 decision 7), and the views below order by the stored form.
    let readings = table(&program, "readings");
    let rows = decode_jsonl(
        readings,
        r#"{"machine_id":"m-01","taken_at":"2026-07-30T12:00:00+02:00","temperature":300.0}
{"machine_id":"m-01","taken_at":"2026-07-31T12:00:00+02:00","temperature":312.5}
{"machine_id":"m-02","taken_at":"2026-07-31T12:00:00+02:00","temperature":355.0}
"#,
    )
    .expect("readings decode");
    assert_eq!(
        db.apply(&readings.shape(), &Delta::appending(rows))
            .expect("append readings")
            .inserted,
        3
    );

    let materialized = materialize_views(&mut db, &program).expect("materialize");
    let count = |name: &str| {
        materialized
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("no view named {name}"))
            .1
    };

    // The payoff: a reducing fold over the registry, with no completeness
    // discharge anywhere in the program (ADR 0033).  One row per machine,
    // carrying that machine's maximum.
    assert_eq!(count("machine_temperature"), 2);
    let peaks = db
        .scan(
            &program
                .views
                .iter()
                .find(|v| v.name == "machine_temperature")
                .expect("the view")
                .shape(),
        )
        .expect("scan");
    assert_eq!(
        peaks,
        vec![
            vec![Value::String("m-01".into()), Value::Real(312.5)],
            vec![Value::String("m-02".into()), Value::Real(355.0)],
        ]
    );

    // The windowed view emits one row per input row, and `overheating`
    // filters against the 350 K threshold, so only m-02's reading survives.
    assert_eq!(count("reading_trend"), 3);
    assert_eq!(count("overheating"), 1);
    // `attention_needed` reads `machines`, where m-02 is degraded.
    assert_eq!(count("attention_needed"), 1);
}

#[test]
fn a_bad_record_stops_the_batch_before_anything_is_written() {
    // The decode runs over the whole batch first, so a bad record at the end
    // means no write is attempted at all.
    let program = resolve(FLEET);
    let db = seeded(&program);
    let readings = table(&program, "readings");

    let err = decode_jsonl(
        readings,
        r#"{"machine_id":"m-01","taken_at":"2026-07-30T12:00:00Z","temperature":300.0}
{"machine_id":"m-02","taken_at":"2026-07-31T12:00:00Z","temperature":"warm"}
"#,
    )
    .expect_err("the second record's temperature is not a number");
    assert_eq!(err.record, 2);
    assert!(err.message.contains("temperature"), "{err}");
    assert!(db.scan(&readings.shape()).expect("scan").is_empty());
}

#[test]
fn a_batch_that_fails_at_the_write_rolls_back() {
    // The other half of all-or-nothing (ADR 0034 decision 4): these records
    // all decode, so the failure lands mid-transaction, and the good rows
    // ahead of the bad one must not survive it.
    let program = resolve(FLEET);
    let mut db = seeded(&program);
    let machines = table(&program, "machines");

    let rows = decode_jsonl(
        machines,
        r#"{"machine_id":"m-01","commissioned":"2026-01-05","activated":"2026-01-09T08:00:00Z","status":"operational","last_service":null}
{"machine_id":"m-01","commissioned":"2026-03-09","activated":"2026-03-13T08:00:00Z","status":"failure","last_service":null}
"#,
    )
    .expect("both records decode; the clash is a key one");
    db.apply(&machines.shape(), &Delta::appending(rows))
        .expect_err("a singletons store holds one row per key");
    assert!(
        db.scan(&machines.shape()).expect("scan").is_empty(),
        "the first row must not survive the batch that failed"
    );
}

#[test]
fn a_lateness_contract_rejects_late_batches_and_a_store_never_does() {
    // ADR 0037 decision 4: the `lateness` contract belongs to the registry,
    // because the sole append-only intake is what makes it enforceable.  The
    // contrast with a store is double: *declaring* `lateness` on a store is
    // a compile error (see `resolve`'s tests and the corpus), and a store's
    // intake carries no watermark, so it accepts arbitrarily late rows
    // silently.  A store accumulates observations with gaps and revisions;
    // only the registry can promise finality.
    let src = r#"
        import si
        unit Reading { machine_id: string  taken_at: instant }
        registry readings {
          unit { Reading }
          attr { temperature: real }
          lateness { taken_at: 10.0 * si.minute }
        }
        store observations {
          unit { Reading }
          attr { temperature: real }
        }
    "#;
    let program = resolve(src);
    let mut db = seeded(&program);
    let readings = table(&program, "readings");
    let observations = table(&program, "observations");

    let fresh = r#"{"machine_id":"m-01","taken_at":"2026-08-10T10:31:12Z","temperature":300.0}
"#;
    let late = r#"{"machine_id":"m-01","taken_at":"2026-08-10T10:20:45Z","temperature":301.0}
"#;

    // The registry: m-01's first batch sets *its own* watermark to
    // 10:31:12, so a later 10:20:45 record for m-01 is older than
    // `watermark - lateness` (10:21:12) and its batch is rejected whole.
    let rows = decode_jsonl(readings, fresh).expect("decodes");
    db.apply(&readings.shape(), &Delta::appending(rows))
        .expect("the first batch of a grain is unconstrained");
    let rows = decode_jsonl(readings, late).expect("decodes: lateness is an intake concern");
    let err = db
        .apply(&readings.shape(), &Delta::appending(rows))
        .expect_err("m-01's gateway broke its ten-minute bound");
    let shown = err.to_string();
    assert!(shown.contains("arrived too late"), "{shown}");
    assert!(shown.contains("2026-08-10T10:21:12.000Z"), "{shown}");
    // The diagnostic names the grain, so it says which producer broke its
    // contract rather than merely that one did (ADR 0041 decision 2).
    assert!(shown.contains("`machine_id` = m-01"), "{shown}");
    assert_eq!(db.scan(&readings.shape()).expect("scan").len(), 1);

    // The same two records into the store, in the same order: both land,
    // because no contract exists and none can be declared.
    for payload in [fresh, late] {
        let rows = decode_jsonl(observations, payload).expect("decodes");
        db.apply(&observations.shape(), &Delta::appending(rows))
            .expect("a store accepts late rows");
    }
    assert_eq!(db.scan(&observations.shape()).expect("scan").len(), 2);
}

#[test]
fn the_watermark_is_grained_so_one_fast_machine_cannot_refuse_another() {
    // ADR 0041 decision 2, and the reason for the whole regraining: a
    // watermark is per grain (the declared key minus the contracted
    // column, here `{machine_id}`), so the fleet's fastest reporter
    // cannot refuse a slower machine's honest traffic.  Under ADR 0037's
    // global watermark every assertion below except the first would fail.
    let src = r#"
        import si
        unit Reading { machine_id: string  taken_at: instant }
        registry readings {
          unit { Reading }
          attr { temperature: real }
          lateness { taken_at: 10.0 * si.minute }
        }
    "#;
    let program = resolve(src);
    let mut db = seeded(&program);
    let readings = table(&program, "readings");
    let append = |db: &mut SqliteBackend, payload: &str| {
        let rows = decode_jsonl(readings, payload).expect("decodes");
        db.apply(&readings.shape(), &Delta::appending(rows))
    };

    // m-07 races ahead to 10:31:12.
    append(
        &mut db,
        r#"{"machine_id":"m-07","taken_at":"2026-08-10T10:31:12Z","temperature":300.0}
"#,
    )
    .expect("first batch");

    // m-19's gateway was partitioned and flushes readings from 10:06.
    // Globally these are 25 minutes behind the watermark and would be
    // refused; per grain m-19 is measured against itself and has no
    // watermark yet, so its buffered data lands.
    append(
        &mut db,
        r#"{"machine_id":"m-19","taken_at":"2026-08-10T10:06:00Z","temperature":301.0}
{"machine_id":"m-19","taken_at":"2026-08-10T10:20:00Z","temperature":302.0}
"#,
    )
    .expect("a slow machine is measured against itself");

    // Onboarding m-42 with a month of history: no observed watermark for
    // that grain, so the backfill is admitted.
    append(
        &mut db,
        r#"{"machine_id":"m-42","taken_at":"2026-07-10T09:00:00Z","temperature":295.0}
"#,
    )
    .expect("a new machine's history is not late");

    // m-19's own contract still binds: 10:06 is now below its own
    // watermark (10:20) minus ten minutes.
    let err = append(
        &mut db,
        r#"{"machine_id":"m-19","taken_at":"2026-08-10T10:06:00Z","temperature":303.0}
"#,
    )
    .expect_err("m-19 is late against m-19");
    assert!(err.to_string().contains("`machine_id` = m-19"), "{err}");
    assert_eq!(db.scan(&readings.shape()).expect("scan").len(), 4);
}

#[test]
fn a_reference_to_a_missing_row_is_rejected_by_name() {
    // Foreign keys are enforced (ADR 0034 decision 5), and the diagnostic
    // names the `domain` entry rather than a SQLite code.
    let src = r#"
        unit Machine { serial: string }
        unit Event { machine: Machine  at: date }
        store machines { unit { Machine } attr { commissioned: date } }
        registry events {
          unit { Event }
          domain { machine: machines }
          attr { note: string }
        }
    "#;
    let program = resolve(src);
    let mut db = seeded(&program);
    let events = table(&program, "events");

    let rows = decode_jsonl(
        events,
        r#"{"machine.serial":"ghost","at":"2026-07-31","note":"swap"}
"#,
    )
    .expect("decodes: the reference is a storage concern, not a decode one");
    let err = db
        .apply(&events.shape(), &Delta::appending(rows))
        .expect_err("no such machine");
    let shown = err.to_string();
    assert!(shown.contains("`machines`"), "{shown}");
    assert!(shown.contains("`machine`"), "{shown}");
}
