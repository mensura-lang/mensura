# Views

A **store** holds observations that arrive from outside.  A **view** holds rows
that are *computed* from other tables by a pipeline.  It is the first
declaration that hosts the transforming algebra: where a store enumerates its
columns in `attr` blocks, a view names a pipeline and lets the algebra decide
what comes out.

```mensura
{{#include ../examples/view-celsius.mensura}}
```

`readings` is a source, referred to by name.  The `|>` pipe feeds it to `map`,
which rewrites each row into one carrying a `celsius` column.  The view's
schema, its index, and its tracked properties are whatever that pipeline
produces; a view has no `attr` block because it declares nothing, it computes.

## The body is a block

A view body is an ordinary block: zero or more `let` bindings, then a trailing
expression of table type that is the materialized result.  Naming an
intermediate table with `let` is how you fork a pipeline, binding a table once
and using it more than once.

```mensura
{{#include ../examples/view-fork.mensura}}
```

Here `split` routes each row wholly to one side of a pair by a predicate over
the key, and `bind` merges the pair back into one table.  There is no special
pipeline grammar: stages compose left to right, `let` names a table, and a tuple
brings several tables together for a merge like `bind`.

## Aggregating over a group

To summarize a machine's readings, you *coarsen* the key.  `readings` is keyed
by `(machine, ts)`, so `shrink_key ts` drops `ts` out of the key: the rows that
differed only in their timestamp now share the key `machine` and form one group,
which `group_map` reduces to a single record per machine.  Grouping is coarsening
the key, not refining it.

```mensura
{{#include ../examples/view-aggregate.mensura}}
```

Coarsening a key is sound only when the groups it folds are whole, so
`shrink_key` *consumes* a [completeness](../concepts/what-the-types-track.md)
fact, and that fact has to be established on the pipeline *before* the stage
that needs it, never after.  A raw `store` does not carry it: unlike a
`collect`, which is a complete census by construction, a store accumulates
observations that can have gaps (a machine offline for a stretch leaves holes
in its readings), so completeness over `machine` is a claim you make, not a
given.  `assume { complete }` makes that claim by fiat, locally and visibly,
and is read as completeness over the retained key; a `completeness_check { ... }`
stage would prove it instead.  This view type-checks today; executing a
key-coarsening stage is the part of the runtime still being built
(`docs/toolkit/04-processing-layer.md`).

## What a view tracks

A view carries the same content and qualifiers as any table (see [What the
types track](../concepts/what-the-types-track.md)), each derived from the
pipeline, not declared:

- **Content**: the index and non-index columns the pipeline yields.
- **Cardinality**: `singletons` (at most one row per key) or `bag`.  A view that
  ends in a summarizing `group_map` is `singletons`; one that ends in a
  bag-shaped stage is `bag`.  A view is *not* held to the 0-or-1 discipline a
  store is: a derived table may genuinely be a bag.
- **Totality**: whether each value is known or may be missing.
- **Completeness** and **lineage**: carried as the pipeline carries them.

## Constraining a view with a shape

A view may claim a shape with the same `: Shape` clause a store uses.  The claim
constrains the view's *output content*, and the check is the store conformance
check run against the computed schema rather than a declared one.

```mensura
{{#include ../examples/view-shape.mensura}}
```

This is the `celsius` view from the top of the page with a `: Celsius` shape
claim added.  Its `map` stage yields a record with a `celsius` column, so the
output carries exactly what the shape requires.  A shape claim checks structure,
not cardinality: it does not force a view to `singletons`.

## Creating a view

`mensura run` materializes a view the same way it creates a store: it scans the
sources, evaluates the pipeline, and writes the result to a table that can be
queried like any other.  A view is recomputed from its sources on each run, so
re-running an unchanged program leaves the same rows.

The chapters that follow look at the two reshaping operations most worth seeing
on their own: [`map`](map.md), which rewrites, drops, and expands rows, and
[`pivot`/`unpivot`](pivot.md), which move between long and wide form.
