# Storage backend

A `store` is a persistent tabulation of observations of a unit
(`docs/language/01-units.md`, `docs/language/02-stores.md`).  This document
specifies how a resolved store schema is materialized in a backing database
and the abstraction the rest of the toolchain uses to do so.  It covers the
first feature, creating a store; reads, writes, and migrations come later.

## Storage layer versus processing layer

Mensura separates two concerns that other tools conflate:

- The **storage layer** holds the current state of each store and is the
  source of truth for changes.  It answers CRUD and, later, REST.
- The **processing layer** computes derived tables (views, pipelines)
  incrementally.  The intended engine for that is DBSP
  (<https://docs.rs/dbsp>), with Polars for batch work.

The store is *not* the compute engine.  DBSP maintains incremental views by
consuming a stream of **deltas** (insert/delete records carrying a weight; an
update is a delete plus an insert) over **typed rows**, and it runs
synchronously on its own batch scheduler.  Keeping storage and processing
separate, and making the storage layer able to emit deltas of typed rows, is
what lets the processing layer plug in later without reshaping the store.

This separation drives the backend choice below.

## The `StorageBackend` abstraction

The toolchain talks to storage through one trait, so the SQL dialect never
leaks into the rest of the compiler and other backends can be added later:

```rust
pub trait StorageBackend {
    /// Ensure the store's table exists, creating it if absent.
    fn ensure_store(&mut self, schema: &Schema) -> Result<EnsureOutcome, StorageError>;

    // Planned, not in the first feature:
    //   fn apply_delta(&mut self, store: &str, delta: Delta<Row>) -> Result<(), StorageError>;
    //   fn scan(&self, store: &str) -> Result<RowStream, StorageError>;
    // Writes are delta-shaped and rows are typed values, so the same
    // records feed DBSP's Z-sets directly.
}

pub enum EnsureOutcome {
    Created,
    AlreadyExists,
}
```

`Schema` is the resolved store model produced by the type checker (store
name, unit, and the ordered list of columns with role and type); it is the
boundary between the front end and the runtime.

## Mapping a store to a table

A store becomes exactly one table.

- **Table name**: the store name, quoted.
- **Columns**: the unit's key fields first (in declaration order), then the
  store's attributes (in declaration order).  Key fields supply identity;
  attributes supply the accompanying data.
- **Primary key**: for a `singletons` store (the default), the key
  columns, as a single composite `PRIMARY KEY`.  This enforces the 0-or-1
  cardinality rule of `docs/language/01-units.md` at the storage level: one
  row per key tuple.
- **Bag stores** (`attr*`,
  `docs/decisions/0022-observations-as-bags-declared-store-cardinality.md`)
  hold many rows per key, so their key columns cannot be a primary key.
  The table is created with no `PRIMARY KEY`; SQLite's implicit rowid is
  the surrogate row identifier, and the backend adds a non-unique covering
  index `"<store>_key"` over the key columns so key lookups stay fast.
  Per-row addressability is lost, by definition.  Scans order by the key
  columns with the rowid as tiebreak, so they stay deterministic even
  though a bag carries no row order.
- **Nullability**: a total attribute (the default) is `NOT NULL`; an optional
  one (declared with a trailing `?`, ADR 0010) is nullable.  Key columns are
  always total, so the primary key is non-null too (this also sidesteps
  SQLite's legacy nullable-primary-key quirk).
- **Creation**: `CREATE TABLE IF NOT EXISTS`.  The backend first checks
  `sqlite_master` so it can report `Created` versus `AlreadyExists`.  It does
  *not* reconcile an existing table whose shape differs from the schema; that
  is migration, and it is out of scope here.

### Type mapping

| Mensura type | SQLite column type | Notes                                  |
|--------------|--------------------|----------------------------------------|
| `string`     | `TEXT`             |                                        |
| `int`        | `INTEGER`          | discrete (ADR 0014)                    |
| `real`       | `REAL`             | continuous measurement                 |
| `bool`       | `INTEGER`          | `0` / `1`                              |
| `date`       | `TEXT`             | ISO 8601                               |
| `enum(...)`  | `TEXT`             | `CHECK (col IN ('a', 'b', ...))`       |
| `D[real]`    | `REAL`             | base-unit magnitude; see below         |

A dimensioned column (`temperature[real]`, `11-physical-units.md`, ADR
0026) persists its magnitude **normalized to the coherent SI base unit**
in an ordinary `REAL` column; the dimension is tracked only at the type
level and stores no companion metadata.  This closes ADR 0026's open
question on persisting a dimensioned magnitude: values are already
base-normalized the moment they are computed (units are scale factors,
conversion happens in expression evaluation), so the storage layer sees
plain reals and round-trips them unchanged.  The recorded consequence: a
database file does not itself say that a column holds kelvin; the source
declaration does.  Precision types remain deferred and have no mapping
yet.

### Example

```mensura
unit Person {
  id: string
}

store Persons {
  unit { Person }
  attr {
    birthdate: date
    last_name: string?
  }
}
```

materializes as:

```sql
CREATE TABLE IF NOT EXISTS "Persons" (
  "id"        TEXT NOT NULL,
  "birthdate" TEXT NOT NULL,
  "last_name" TEXT,
  PRIMARY KEY ("id")
);
```

`last_name` is optional (`string?`), so its column is nullable; the total
columns carry `NOT NULL`.

## Backend choice: rusqlite with bundled SQLite

The first backend is SQLite via `rusqlite` with the `bundled` feature, which
compiles SQLite from source so there is no system dependency.

The choice is made with the processing layer in mind:

- DBSP consumes **deltas of typed rows**, not table scans, so the deciding
  factor is whether the write path can emit row deltas, not the SQL dialect.
  The trait's planned write API is delta-shaped for exactly this reason.
- DBSP elements are **typed Rust values**, which is also how rows are
  represented on the way into SQLite, so one representation serves both.
- DBSP runs **synchronously** on a batch scheduler, which bridges cleanly to
  synchronous `rusqlite`.  An async client (for example `sqlx`) would add a
  runtime and an impedance mismatch for no DBSP benefit.

SQLite remains the queryable current-state store; DBSP, when it arrives, is a
separate engine fed by the store's changes.

## Compound stores

A store of a compound unit maps like any other (ADR 0032): its
unit-reference fields are already flattened to dotted columns in the
schema, so the table carries quoted dotted column names
(`"course.department.code"`) and the composite primary key spans the
flattened key.  Each resolved `domain` entry emits a `FOREIGN KEY` clause
referencing the target store's key columns.  The clauses are
**enforced**: the backend sets `PRAGMA foreign_keys = ON` per connection
when it opens a database, so the ingestion write path checks every
declared reference (ADR 0034, `05-ingestion.md`).  ADR 0032 emitted the
clauses unenforced because nothing wrote yet and left the pragma to the
ingestion slice; that slice turned it on.

## Forward references

- Updates and deletes at the CLI.  The read side and batch view
  materialization are specified in `04-processing-layer.md`; the insert
  side, the delta-shaped write path, and the typed decoder are
  `05-ingestion.md`.  The changelog that feeds an incremental engine
  arrives with M5.
- Schema migration when a store's shape changes between revisions
  (`mensura migrate` in `ROADMAP.md`).
- Additional backends behind the same trait.
