# 0042: Views reading views

## Status

Accepted.  Closes the open question "Views reading views" of
`docs/decisions/0012-view-hosting.md`, and lifts the "(later, another
view)" deferral of `docs/language/10-views.md`, "Sources resolve by
name".  Changes the view ordering of `docs/toolkit/04-processing-layer.md`
from declaration order to dependency order.

## Context

A view's pipeline source must be a store or a registry.  Naming another
view reports "unknown source", so every consumer of a derived table has
to repeat the pipeline that derives it.  Two places in
`docs/examples/fleet-monitoring.mensura` already pay for this:
`silence_per_machine` repeats all of `sensor_health` before its own two
stages, and `reading_rate` is one multi-stage view where a small view
over a shared upstream would read better.  The general form is the IIoT
sketch's: one windowed feature view feeding both a training view and a
serving view.  Without view sources the two cannot share it.

The question ADR 0012 left open is how a view's facts reach a reader:
its cardinality, totality, completeness, gradings, window facts, and
lineage, which the checker computed from its pipeline.  There are two
halves to it.

**The checker.**  A view's type is the full `Table<Qs, C>` its body
computes (`10-views.md`, "Properties at the view boundary").  Every stage
is typed compositionally: its output type is a function of its input
types and its own syntactic arguments, never of where the input came
from.  A view body's `let` already relies on this.  It binds a name to a
computed table type, and every later use of the name reads that type
unchanged.

**The runtime.**  The evaluator mirrors three of the checker's facts on
its table values, so that the stages which consume them do not re-derive
them from syntax (`crates/mensura-runtime/src/eval.rs`, `SourceTable`).
`origin` names the store whose intake contract `closed` reads, `windows`
records the grid `closed` and `dense` filter and fill, and `reductions`
records the combiner `dense` fills from.  A materialized view table in
storage holds rows only, so none of the three survive a round trip
through the backend.

## Decision

### 1.  A view is a table source, and its name reads its computed type

A bare name in a table position may name a view as well as a store or a
registry.  It is presented to the reader as the view's computed
`TableType`, **every qualifier intact**.  Reading a view types exactly
as if its body were bound by a `let` at the top of the reader's body.  A
view is a program-scope `let` whose value is materialized.

Nothing is reset at the boundary.  A view name is not an operation, and
the materialized table is the value the body computed, so every fact
the checker established about that value holds of the name.  In
particular:

- **Completeness** and the facts it is re-established from carry over,
  so `silence_per_machine` can read `sensor_health` and demote its
  dense window grid (ADR 0038 decision 4) with no `assume`.
- **Lineage** carries over.  A split tag is identified by the `split`
  stage's source position, which is unique within the program, so two
  readers of one view see one lineage, and two views that each `split`
  see two unrelated tags.  Reading one view twice therefore does not
  make the two reads disjoint, which is the same answer a `let` gives.
- **`assume` claims** carry over too.  A claim is visible where it is
  written, in the upstream body, and its consequences reach every reader
  as they reach every later stage of that body.

This is not a new propagation rule, so it needs no new theorem in
`formal/`.  Each fact is carried by identity, and identity is the one
transport every fact admits.

### 2.  Table references, the dependency graph, and checking order

A view's **table references** are the free names it reads in the three
positions where a pipeline resolves a table: pipeline position (a bare
name, a tuple item, the left of `|>`, a bare application's trailing
argument), the right side of `lookup` and `lookup_total`, and `dense`'s
population.  A name bound by an earlier `let` in the same body is local
and shadows a declaration of the same name.  Column selectors and lambda
bodies are not table positions, so a column that happens to share a
view's name is not a reference.

The view dependency graph has an edge from a view to each view it
references, and it must be acyclic, since a view is computed from its
sources and so none may read itself, directly or through others.  A
cycle is reported once, at the reference that closes it, naming the
path.  The checker types views in dependency order (declaration order
among independent views), so each reader sees its sources' types.  A
view that depends on a view that failed to check, or that sits on a
cycle, is skipped without a further diagnostic: the upstream error is
the one to fix.

A conformance failure (`: Shape`) does not stop readers.  It is a claim
the view fails to meet, not an uncomputed type.

### 3.  The runtime evaluates in dependency order and hands tables over in memory

`mensura run` materializes views in the order the checker fixed.  A
reader receives its source view's **evaluated table value**, with its
`origin`, `windows`, and `reductions` intact, rather than a rescan of the
materialized table.  The rescan would drop exactly the runtime mirror of
the facts decision 1 carries, so a `closed` or a `dense` downstream of a
windowed view would lose the grid the checker vouched for.  Handing over
the value keeps the runtime and the checker in agreement by construction.

The intake contracts and their watermarks are read once per run, before
any view is evaluated, for every store that declares one.  A reader of a
view can then reach the contract of a store it does not name, through
the upstream table's `origin`.  Reading them up front keeps each
evaluation a pure function of its inputs (ADR 0037 decision 4), as
before.

Each view is still evaluated once per run however many readers it has,
and its materialization is still replaced in one transaction.

## Consequences

- `silence_per_machine` reads `sensor_health` instead of repeating it,
  and shared feature views become expressible.
- `ViewPlan::sources` lists the views a body reads as well as its
  stores, and the plans are emitted in dependency order.
- A store and a view already share one table namespace (ADR 0012), so a
  reference is never ambiguous between the two.
- The dependency graph is the structure the M5 refresh slice needs for
  incremental maintenance: a change to a store invalidates the views
  that reach it, in this order.

## Alternatives considered

**Reset the qualifiers at the boundary**, presenting a view as a store
is presented (`TableType::from_store`): untagged, facts cleared,
completeness by cardinality alone.  Conservative, and wrong for the
motivating case.  `silence_per_machine` would lose the rectangularity
of `sensor_health`'s grid and need an `assume { complete }` to restore
a fact a mechanism had already established, replacing proof with fiat
at every view boundary.  It would also make a `let` and a view disagree
about the same table.

**Rescan the materialized table at runtime.**  Simpler orchestration,
but it loses the runtime's mirror of the window and reduction facts
(context, above), so the evaluator could not run a `closed` or `dense`
the checker accepted.

**Inline the upstream body textually.**  Types the same, but it
evaluates a shared view once per reader, and reports an upstream error
once per reader at spans inside a different declaration.

## Open questions

- **Views across modules.**  Imports resolve bundled modules only
  (ADR 0027), which declare no views.  When user modules arrive, a view
  reference crosses a module boundary and a split tag needs a
  program-wide identity rather than a source position.
- **Incremental refresh.**  How the refresh slice of M5 propagates a
  delta along the dependency graph, rather than recomputing every view.
- **Serving.**  Whether reading a view requires the reader to be
  authorized for the view or for its sources (M7, ADR 0005).

## Forward references

- `docs/language/10-views.md` states the surface rule; the evaluation
  order is in `docs/toolkit/04-processing-layer.md`.
- The checker's source environment is `Sources` in
  `crates/mensura-types/src/pipe_check.rs`; the orchestration is
  `materialize_views` in `crates/mensura-runtime/src/eval.rs`.
