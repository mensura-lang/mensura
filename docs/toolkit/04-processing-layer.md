# Processing layer: batch view materialization

M1 ends with views fully typechecked and nothing that computes them:
`mensura run` creates store tables and stops.  This document specifies the
first slice of the processing layer (`ROADMAP.md` M2): `mensura run`
materializes a Tier A view into a table over the storage backend.

The storage-versus-processing split is `docs/toolkit/00-storage-backend.md`;
the view surface and its typing obligations are
`docs/language/10-views.md`; the operation semantics are
`docs/language/07-pipelines.md` and `docs/language/09-typing-reference.md`.
This document fixes only the execution: how a checked view becomes rows in
a table.

## Scope of the first slice

- **Views over stores.**  Sources in a view body resolve to stores, exactly
  as the resolver presents them today.  View-on-view sources are deferred.
- **No runtime obligations.**  Views host Tier A pipelines plus the Tier B
  stages `pivot` and `demote`.  Both are Tier B only for compile-time
  effects (`pivot` for its lineage effect and the `exhaustive` totality
  upgrade, ADR 0020; `demote` for its lineage drop, its completeness
  clearing, and the reducer's discharge, ADR 0023/0024/0035), so batch
  evaluation is unaffected: the runtime trusts the checker and only
  rekeys.
- **One-shot batch recompute.**  A view is recomputed from its sources'
  current state every `mensura run`.  This stayed a complete semantics
  when ingestion landed (M4, `05-ingestion.md`): a batch applies between
  runs, so the next run sees it whole.  What makes recompute an
  approximation is *incremental refresh*, which is M5's.
- **`flat_map` first.**  The first operation implemented end to end is `flat_map`,
  which subsumes filtering (ADR 0015) and is what the committed
  `attention_needed` view in `docs/examples/fleet-monitoring.mensura`
  needs.  The remaining operations (`promote`, `map_bags`,
  `split`/`union`, the joins, `unpivot`, `pivot`) follow the same lowering
  and land incrementally within M2.

## Engine choice: batch recompute now, DBSP at M5

`00-storage-backend.md` names DBSP (<https://docs.rs/dbsp>) as the intended
incremental engine.  This slice does **not** adopt the `dbsp` crate:

- DBSP earns its keep by consuming a **stream of deltas**, and no delta
  stream exists yet.  Stores gain a write path in M4 (ingestion) and views
  gain `on_change` refresh in M5; until then every computation is a full
  batch over current state, which a plain evaluator does without a circuit
  API or a heavyweight dependency.
- The boundary is kept DBSP-shaped anyway: the evaluator works on **typed
  rows** read as a batch and written as a batch, the same representation
  `00-storage-backend.md` plans for deltas.  Replacing the batch evaluator
  with a DBSP circuit at M5 changes the engine behind the boundary, not the
  boundary.

Decision: a small interpreter in `mensura-runtime` that evaluates a checked
view body over batches of typed rows.  Incrementality is a non-goal of this
slice.

## The runtime value model

The evaluator and the storage boundary share one value representation,
mirroring the boundary IR's `ColumnType` (ADR 0014):

```rust
pub enum Value {
    String(String),
    Int(i64),
    Real(f64),
    Bool(bool),
    Date(String),      // ISO 8601, as stored
    Enum(String),      // the variant literal
    Missing,           // an optional (`?`) value that is not known
}

pub type Row = Vec<Value>;  // ordered per the table's column list
```

A `Row` is positional: its values follow the table's column order (key
columns first, then attributes; ADR 0019).  `Missing` is the runtime image
of ADR 0010's optional values and round-trips with SQL `NULL`; the checker
guarantees a total column never holds it.

The empty collection `()` that a `flat_map` body may return is **not** a
`Value`.  It is a control outcome of body evaluation (drop this row), never
a cell.

## The view boundary IR

`resolve` today returns the store `Schema` list; views are typechecked in
pass 4 and then dropped.  The resolver output grows into a resolved
program: the store schemas plus one **view plan** per view.  A view plan
carries

- the view's name;
- its output columns (name, type, role, optionality), read off the checked
  `TableType` that `type_view` already computes;
- the computed **cardinality** (`singletons` or `bag`), which decides the
  materialized table's key discipline below;
- the checked body (the block AST) and the names of the stores it reads.

The runtime evaluates the checked AST directly.  There is no separate plan
IR while the operation set is this small: the checker has already
established that the body is well-typed, so evaluation cannot fail on
shape, and a second IR would duplicate the algebra for no consumer.  A plan
IR becomes worth its cost when the algebra grows optimizing rewrites or a
DBSP lowering; that is M5's call.

One consequence of evaluating the AST directly: the evaluator's columns
carry their domain (`ColumnType`) where it is statically known, because
`pivot` must recover the name column's declared enum variants at runtime
(absent variants still become columns, so the variant set cannot be read
off the data).  Propagation is deliberately conservative: a store column
keeps its domain, structural operations carry it through, and a computed
column drops to unknown, which only `pivot` consumes (it fails with an
explicit "enum lost upstream" report rather than guessing).  The
principled alternative, threading the checker's per-stage `TableType`
through the view plan, is a natural part of the M5 plan IR.

## Evaluating a pipeline

Evaluation mirrors the checker's structure (`pipe_check`): an environment
maps names to table values, `let` bindings extend it, and the block's
trailing expression is the result.

- A **source name** evaluates to a scan of its store: the current rows,
  decoded to `Row`s.
- **`flat_map |k, r| body`** evaluates its body once per input row with the
  key and value parameters bound.  A record result contributes one output
  row; `()` contributes none.  This is the row-multiset semantics of ADR
  0015 restricted to collection size at most 1, which is all the current
  surface can express.
- **Scalar expressions** inside a body (literals, field access, arithmetic,
  comparisons, boolean operators, `if`/`then`/`else`, `is known`, `??`,
  record literals) follow `docs/language/06-expressions.md` over `Value`s.
  The checker has already enforced domain rules (for example, no `==` on
  `real`), so the evaluator implements each operator only on the variants
  it can meet; a `Value::Missing` operand makes any lifted operator's
  result missing (ADR 0039).

- **`window w p size stride`** replicates each row into every window
  containing its point, splicing the window start into the key block.
  The extents are const expressions the checker has already validated;
  lowering substitutes their const names, so the evaluator folds them in
  an empty scope and converts through the same exact-or-error
  millisecond predicate the checker used.  Grid arithmetic is euclidean,
  so a pre-epoch point does not shift by a stride.
- **`closed`** filters rows whose window can still receive one.  It runs
  before any grouping, since `demote` is a column-role move and fibers
  are not formed until `map_bags`, so it is a cheap per-row predicate.
- **`dense w population bound`** completes the grid after the reduction
  (ADR 0038).  It reads the population's rows from the environment, the
  way a join reads its right table, aligns each row's bound up to the
  first full slot, walks the grid to the last closed slot, and emits a row
  wherever an anti-join against the reduced rows finds none.  The upper
  bound is computed from the same prefetched watermark `closed` filtered
  on, so the two stages agree by construction: `slot + size + lateness <=
  effective`, floored to the grid.  A grain with no watermark at all is
  skipped, since nothing licenses calling any of its slots closed; the
  declared closure floor is what gives a never-reporting entity its silent
  slots (ADR 0041 decision 3).  Fill values come from the combiner each
  column reduced at, read through the same `identity_of` table `prescan`
  uses, and a column with no identity is filled `Missing`, which the
  checker has already typed optional.

Later Tier A operations slot into the same shape: each is a function from
input row batches to an output row batch, with its property effects already
discharged at compile time.

## Watermarks reach the evaluator by prefetch

`closed` is the first operation needing something the row batches do not
carry: the effective watermark of each grain (ADR 0037 decision 4,
ADR 0041).  The evaluator stays a pure function of its inputs, and
`materialize_views` reads the watermarks **once per run** through
`StorageBackend::watermarks`, before evaluating, alongside the source
scans.

That placement is the ADR's requirement rather than a convenience: a
batch run must be deterministic and reproducible, so the watermark is
read once and wall-clock time never enters the semantics.  Two runs over
the same database agree.

Because the qualifier row does not cross the view boundary IR (only
columns, cardinality, and the body do), the evaluator maintains its own
lightweight mirror of the facts it needs while walking the pipeline: which
column is a window column, over which point and at what extent and
stride, which store the rows still unambiguously come from, and which
attribute column a single combiner produced.  A join or a union clears
them, exactly as the checker clears its own facts there.

The third of those is what `dense` fills from, and the mirror is why all
three cross a reducing `map_bags` here as they do in the checker: `dense`
runs *after* the reduction by construction (ADR 0038 decision 1), so the
facts have to survive exactly that stage.  What does not survive is the
point column itself, which is why `closed` after a reduction is a compile
error rather than a runtime surprise.

## Materializing the result

A view materializes into a table by the same mapping stores use
(`00-storage-backend.md`), with two differences:

- **The primary key follows cardinality.**  A `singletons` view gets the
  composite primary key over its key columns, as a store does.  A `bag`
  view is admitted (`10-views.md`) and gets **no** primary key; its key
  columns are still `NOT NULL`, but several rows may share a key.
- **Contents are replaced, not accumulated.**  Each `mensura run`
  recomputes the view inside one transaction: ensure the table exists,
  delete all rows, insert the computed batch, commit.  Readers see either
  the previous materialization or the new one, never a partial state.

As with stores, `CREATE TABLE IF NOT EXISTS` is used and an existing table
whose shape differs from the plan is not reconciled; that is migration and
stays out of scope.

`mensura run` orders the work: ensure every store, then materialize views
in declaration order.  With store-only sources any order works; declaration
order is deterministic and reads naturally once view-on-view arrives.

## StorageBackend extensions

The trait gains the read side and the view write path sketched in
`00-storage-backend.md`:

```rust
pub trait StorageBackend {
    fn ensure_store(&mut self, schema: &Schema) -> Result<EnsureOutcome, StorageError>;

    /// Read a table's current rows, decoded to typed values in column order.
    fn scan(&self, table: &TableShape) -> Result<Vec<Row>, StorageError>;

    /// Ensure the view's table and replace its contents in one transaction.
    fn materialize_view(&mut self, view: &TableShape, rows: &[Row])
        -> Result<(), StorageError>;
}
```

`TableShape` names the table and lists its columns and key discipline; it
is the part of the boundary IR that stores and view plans share (the exact
Rust carrier is an implementation detail).  `scan` returns a batch
(`Vec<Row>`); a streaming or delta-shaped read arrives with ingestion and
the incremental engine, behind the same trait.

## CLI behavior

`mensura run file.mensura [--db <path>]` keeps its current store output and
adds one line per view:

```
created store machines (4 columns)
materialized view attention_needed (2 rows)
```

`mensura check` is unchanged: it typechecks views today and never touches a
database.

## Validation

- The committed `attention_needed` view materializes end to end: a runtime
  test seeds the `machines` table, runs the pipeline, and asserts the
  degraded rows and only those come back.  These tests seed stores at the
  SQL level through the backend, a test scaffold rather than a language
  surface; M4's typed ingestion (`05-ingestion.md`) is the supported way
  to put rows in.
- `docs/examples/fleet-monitoring.mensura` stays the driving example: its
  existing resolve test is joined by a `mensura run` test over an in-memory
  database.
- The must-accept / must-reject corpus is unaffected; this slice changes
  the resolver's output shape, not what it accepts.

## Forward references

- The remaining Tier A operations at runtime, completing M2.
- Typed ingestion and the delta-shaped write path landed in M4
  (`05-ingestion.md`); the delta stream it produces is what the
  incremental engine below consumes.
- Incremental refresh: the DBSP engine replacing the batch evaluator behind
  the same boundary, with `on_change` and windows (M5).
- View-on-view sources.
