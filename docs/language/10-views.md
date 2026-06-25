# Views

A view is a named, materialized table defined by a pipeline.  Where a `store`
holds observations that arrive from outside, a view holds rows that are
*computed* from other tables by the algebra of `07-pipelines.md`.  It is the
first declaration site that **hosts a pipeline**: the open question that
`07-pipelines.md` and `docs/decisions/0009-pipeline-surface.md` left under
"hosting".

This document defines the view declaration and the typing obligations at its
boundary.  The pipeline operations themselves are specified in
`07-pipelines.md`, the expression sublanguage they are built from in
`06-expressions.md`, and the consolidated typing rules in
`09-typing-reference.md`; this document adds only the surface that names a
pipeline and gives it a place to run.

The syntax shown is preliminary, like the rest of the language docs at this
stage; the design content is not.

## What a view is

A view is a derived table with a name.  It reads from one or more sources
(stores, and later other views), applies a pipeline, and is materialized so the
result can be queried like a store.  Three things follow from "derived by a
pipeline":

- **Its content is computed, not declared.**  A store enumerates its columns in
  `const`/`var` blocks; a view does not.  The view's schema, its index columns,
  and its non-index columns are whatever the pipeline produces.  A view
  therefore has no `const`/`var` blocks.
- **Its properties are derived from mechanism.**  This is overview pillar 7
  applied to a derived table: just as a `store` fixes its sampling and lineage
  by the act of declaring it, a view fixes its tracked properties by the
  pipeline that defines it.  The programmer does not choose them; the algebra
  computes them.
- **It is a resource, not a type.**  A view is materialized and queried over a
  wire, exactly as a store is (`docs/decisions/0006-transport-agnostic-surface.md`),
  so its name follows the term convention, **snake_case**, like `store` and
  `collect` (`05-naming-and-casing.md`).

## Surface form

A view declaration is the word `view`, a snake_case name, an optional shape
conformance clause, and a **block** that hosts the pipeline:

```mensura
view feature_window : Tabular[Machine] {
  let base = readings |> extend_key machine;
  base |> group_map |g| (.temp_mean = mean g.temperature, .temp_max = max g.temperature)
}
```

- **The body is a block.**  It is the ordinary statement block of
  `06-expressions.md`: zero or more `let` bindings (to name and reuse
  intermediate tables) and `assert` statements, followed by a trailing
  expression of table type.  That trailing expression is the materialized
  result.  Forking a pipeline is binding a table with `let` and using it twice;
  joining several tables is tupling them for a `bind`.  No pipeline-specific
  grammar is introduced: a view body is exactly a block (`04-grammar.md`).
- **Sources resolve by name.**  A bare name in the pipeline (`readings`) refers
  to a store (later, another view) in scope, presented to the pipeline as a
  table value.  This is the context model of `06-expressions.md`: the site
  supplies the named tables, the grammar stays the same.
- **The conformance clause is optional.**  When present, the `:` clause claims
  one or more shapes the view's *output* must satisfy, with the same meaning and
  the same check as a store's `:` clause (`03-shapes.md`, "Conformance").

## Properties at the view boundary

A view computes a full `Table<Qs, C>` for its result (`09-typing-reference.md`,
section 1) and that is the view's type.  All four tracked properties are
threaded through the hosted pipeline and surface on the view:

- **Content** (`C`): the index and non-index columns the pipeline yields, with
  their domains.
- **Cardinality** (in `C`): `singletons` or `bag`, as the pipeline leaves it.  A
  summarizing view that ends in a single-record `group_map` is `singletons`; a
  view that ends in a `bag`-shaped stage is `bag`.
- **Totality** (per column, in `C`): a value is total unless an operation made
  it optional (a `left_join` leaves its added columns optional until a default
  or an `is known` narrowing restores them; ADR 0010).
- **Completeness** and **lineage** (in `Qs`): carried as the Tier A operations
  carry them (`09-typing-reference.md`, sections 8 and 9).

A view is **not** forced to any particular cardinality.  It is a general
materialized table: `bag` results are admitted.  This is the deliberate point
where a view differs from the unit boundary of
`docs/decisions/0001-unit-as-identity-discipline.md`: that 0-or-1 rule binds
anything *promising a tabulation of a unit*, and a bare view promises no such
thing.  A view opts into the unit discipline only by claiming a unit-fixing
shape (next section); see `docs/decisions/0012-view-hosting.md`.

## Constraining a view with a shape

The optional `: Shape` clause is the one structural constraint a view may carry,
and it is how a view is pinned to a unit when that is wanted.

- **A unit-fixing shape** such as `Tabular[Machine]` requires the view's output
  to be a tabulation of `Machine`: its index columns must be `Machine`'s index
  (`03-shapes.md`, "The unit clause").  Claiming it makes the view a unit
  boundary, and at that boundary the cardinality is expected to be `singletons`,
  recovering the discipline of ADR 0001 for views that want it.
- **A content shape** such as `Named` requires the output to carry the named
  columns, regardless of unit.
- **No clause** leaves the view a free table of whatever shape the pipeline
  produces; nothing beyond the algebra constrains it.

The check is the existing store conformance check, run against the computed
output schema rather than a declared one.  This is the sense in which
"conformance machinery becomes the carrier of table properties"
(`03-shapes.md`, forward references): a shape claim on a view is a check on the
pipeline's result.

## Worked examples

**Summarize by an attribute (Tier A throughout).**

```mensura
view machine_temperature : Tabular[Machine] {
  readings
  |> extend_key machine
  |> group_map |g| (.temp_mean = mean g.temperature, .temp_max = max g.temperature)
}
```

`extend_key` adds `machine` to the key (content: index grows; cardinality and
completeness preserved); `group_map` reduces each group to one record, so the
result is `singletons` per `(..., machine)` key.  All Tier A, so it composes
safely.  The view claims `Tabular[Machine]`, so the boundary check confirms the
output is a tabulation of `Machine`.

**Split and re-merge, with a `let` fork.**

```mensura
view full_dataset {
  let parts = data |> split |k| hash k < threshold;
  parts |> bind
}
```

`split` yields a disjoint pair, each side carrying a sibling lineage tag; binding
the disjoint pair preserves `singletons` and reconstructs `data` (`bind_split`,
`09-typing-reference.md`, section 11).  The view claims no shape, so it is a free
table; its lineage and cardinality are whatever the pipeline computes.

## Scope of this round

This round hosts **Tier A** pipelines in a view, the split-safe kernel that
composes freely and carries the four properties end to end
(`09-typing-reference.md`, section 7).  The following are deferred to their own
rounds, each ahead of the milestone that needs it (`ROADMAP.md`, "specs first"),
and are noted here only so the scope is unambiguous:

- **Tier B inside a view.**  Hosting `shrink_key` and the index form of `pivot`
  in a view body, and discharging their completeness obligation
  (`completeness_check`, `@complete_over`, a `collect` source, or `assume`) at
  the hosting site (`09-typing-reference.md`, section 8).
- **Lineage-demanding sites.**  The learning operations (`fit`/`evaluate`) that
  *consume* disjointness when fed two views (`09-typing-reference.md`, section
  9, deferred ledger).
- **Streaming and refresh.**  `sliding_window`, `latest`, window-closedness, and
  `on_change` reactive refresh of a view (M5).
- **Serving.**  The query and subscription surface a view exposes over a wire,
  and its authorization (M7, `docs/decisions/0006-transport-agnostic-surface.md`,
  `docs/decisions/0005-identity-and-authorization.md`).
- **Materialization at runtime.**  How `mensura run` computes a view over the
  storage backend (the DBSP-style processing layer, M2,
  `docs/toolkit/00-storage-backend.md`); this document fixes the surface and the
  typing, not the execution.

## Forward references

- The hosted operations and their per-operation property rules are in
  `07-pipelines.md`; the consolidated rules and the `Table<Qs, C>` type are in
  `09-typing-reference.md`.
- The grammar production for `view` is in `04-grammar.md`; the decision and its
  alternatives are in `docs/decisions/0012-view-hosting.md`.
- `collect` (the process variant of a store) and `device` are sibling
  declaration sites that also host or feed pipelines; they get their own
  documents.
