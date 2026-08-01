# Typed ingestion

Through M3 the toolchain only ever read: `mensura run` creates store
tables and materializes views, and runtime tests seed rows at the SQL
level as a scaffold.  This document specifies the write path
(`ROADMAP.md` M4): how external records become typed rows in a store or
a registry.

The storage-versus-processing split is
`docs/toolkit/00-storage-backend.md`; view materialization is
`04-processing-layer.md`; the registry declaration whose intake this is
is `docs/language/13-registries.md`.  This document fixes only the
intake: how a record becomes a row.

Decided in `docs/decisions/0034-typed-ingestion.md`.

## Scope of the slice

- **No language surface.**  Ingestion is a typed API and a CLI
  subcommand.  There is no `insert` statement, no `update`, no `set`;
  the expression sublanguage is untouched
  (`docs/decisions/0007-single-expression-sublanguage.md`).
- **Append for registries, insert for stores.**  A registry's intake
  only ever adds, which is what licenses its completeness guarantee
  (`docs/decisions/0033-registry-declarations.md`).  Update and delete
  on a store are supported by the trait and not yet exposed by the CLI.
- **Batch, not stream.**  A batch of records applied in one
  transaction.  Streaming intake, backpressure, and window closedness
  are M5's.
- **Local, not wired.**  The CLI reads a local file or standard input.
  Transports (MQTT, REST, gRPC) are M7's and become callers of the same
  decoder (`docs/decisions/0006-transport-agnostic-surface.md`).
- **Views recompute as before.**  A view is still recomputed in full by
  `mensura run`, so rows ingested since the last run are picked up by
  the next one.  Incremental refresh is M5's.

## The decoder

The substantive part of the slice is a decoder from a **name-keyed
record** to a typed row, checked against the resolved `Schema`.  A
record is a map from field name to scalar value; the decoder knows
nothing about how it arrived.

The rules, all derived from the schema:

- **Names, not positions.**  Each field is matched to a column by name.
  A flattened compound component is addressed by its dotted path
  (`course.department.code`), the same name the resolved schema and the
  table use
  (`docs/decisions/0032-compound-keys-flatten-to-dotted-columns.md`).
- **Types.**  Each value is decoded per the column's `ColumnType`:
  `string`, `int`, `real`, `bool`, `date`, an `enum`, or a dimensioned
  `D[real]`.  A value of the wrong shape is an error, not a coercion.
  In particular **`int` does not widen to `real`**: a payload `300` for a
  `temperature[real]` column is rejected and must be written `300.0`.
  This is stricter than most JSON tooling, and deliberately so: ADR 0014
  keeps the two domains apart in expressions, and a boundary that
  silently widened would be the one place the distinction did not hold.
  A JSON number counts as an `int` only when written with no fraction and
  no exponent.
- **Enums.**  The value must be one of the type's declared variants.
- **Optionality.**  A value may be absent only where the column is
  declared optional (`?`).  A missing required field is an error.  An
  absent optional field and an explicitly null one both yield a missing
  value.
- **Closed column set.**  An unknown field name is an error.  The
  schema's columns are closed, and a typo'd field would otherwise be
  silently dropped, which is exactly the class of mistake the language
  exists to catch.
- **Dimensioned columns carry base-unit magnitudes.**  A
  `temperature[real]` column is ingested in kelvin.  Values normalize to
  the SI base unit throughout the system, and the dimension is tracked
  only at the type level
  (`docs/language/11-physical-units.md`).  Affine input units (Celsius)
  are converted by the producer; a per-column input-unit declaration
  belongs to the endpoint payload contract and lands with M7
  (`docs/decisions/0026-dimensional-physical-units.md` Decision 7).

A failure names the record and the field, so a bad batch says which row
and which column rather than which SQL statement.

Because the decoder is format-free, the same code serves a JSON Lines
file today and a GraphQL mutation, a REST body, or an MQTT payload when
M7 wires them up.

## `StorageBackend` extensions

The trait gains the write path sketched in `00-storage-backend.md`:

```rust
pub trait StorageBackend {
    // ... ensure_store, scan, materialize_view as before ...

    /// Apply a batch of row changes to a table in one transaction.
    fn apply(&mut self, table: &TableShape, delta: &Delta)
        -> Result<Applied, StorageError>;
}

/// A batch of row changes.  An update is a delete plus an insert.
pub struct Delta {
    pub inserts: Vec<Row>,
    pub deletes: Vec<Row>,
}

pub struct Applied {
    pub inserted: usize,
    pub deleted: usize,
}
```

`apply` keys off `TableShape`, the same boundary-IR projection `scan`
and `materialize_view` take, so the write path names tables the way the
read path already does.

Insert and delete lists rather than DBSP's `(row, weight)` Z-set
encoding: weights earn their keep inside a circuit, and no circuit
exists until M5.  The two convert in a few lines (an insert is weight
`+1`, a delete `-1`), so adopting DBSP widens this representation
instead of replacing the interface, which is the discipline
`04-processing-layer.md` already committed to.

## Foreign keys are enforced

`PRAGMA foreign_keys = ON` is set per connection when the backend opens
a database.  The `FOREIGN KEY` clauses that `CREATE TABLE` already emits
for each resolved `domain` entry
(`docs/decisions/0032-compound-keys-flatten-to-dotted-columns.md`)
therefore mean something once there is a write path to check.

A violated reference is reported as a typed error naming the `domain`
entry, the offending child values, and the target table, not as a raw
SQLite constraint code.

Two notes.  SQLite requires the pragma per connection rather than per
database, so it is set at open and both the read and write paths see the
same setting.  And turning it on enforces constraints on databases
created before this slice, which is intended but is a behaviour change
for an existing file.

## CLI behavior

```
mensura ingest <file.mensura> <target> --data <rows.jsonl> [--db <path>]
```

The program is typechecked first, exactly as `check` and `run` do, so
ingestion never writes against an unresolved schema.  `<target>` names a
store or a registry in the program; naming a view is an error.  `--data
-` reads standard input.  `--db` defaults to an in-memory database, as
elsewhere, which makes a dry run cheap.

Output is one line:

```
appended 1440 rows to registry readings
```

The batch is **one transaction**: every record decodes and applies, or
none does.  A single malformed record rejects the file and reports its
position.  Best-effort loading (`--continue-on-error`) is deferred.

`mensura run` and `mensura check` are unchanged.

## The interchange format

`mensura ingest` reads **JSON Lines**: one JSON object per line, each a
name-keyed record.

```jsonl
{"machine_id": "m-01", "taken_at": "2026-07-31", "temperature": 312.5}
{"machine_id": "m-02", "taken_at": "2026-07-31", "temperature": 297.0}
```

Line-oriented because a large batch then streams and a malformed record
is localized to its line; no schema negotiation because the resolved
schema is already the contract.  This is an encoding for a local file,
in the same sense that `--db path.db` names a local database, and by the
decoder's format-independence it is not a load-bearing choice.

## Validation

- Round trip: ingest a batch into the fleet registry over an in-memory
  database, then `mensura run`, and assert the materialized view
  reflects the ingested rows.
- Decoder coverage: one case per `ColumnType`, plus a missing required
  field, an unknown field, a bad enum variant, a dotted compound
  column, and an optional column absent versus explicitly null.
- Constraint coverage: a duplicate key against a `singletons` target, a
  repeated key accepted by a `bag` target, and a violated `domain`
  reference producing the typed foreign-key error.

## Forward references

- Update and delete on a store through the CLI, once the change-control
  family (`@audited`, `@versioned`) is designed.
- Best-effort batches and their report (`--continue-on-error`).
- Streaming intake and incremental refresh: the delta stream this write
  path produces is what M5's engine consumes
  (`04-processing-layer.md`).
- The wire: transports as callers of the decoder, and the payload
  contract that would carry a per-column input unit
  (`docs/decisions/0006-transport-agnostic-surface.md`, M7).
