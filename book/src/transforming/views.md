# Views

A **store** holds observations that arrive from outside.  A **view** holds rows
that are *computed* from other tables by a pipeline.  It is the first
declaration that hosts the transforming algebra: where a store enumerates its
columns in attribute blocks, a view names a pipeline and lets the algebra
decide what comes out.

```mensura
{{#include ../examples/view-celsius.mensura}}
```

`readings` is the bag store of
[Recurring observations](../modelling/stores.md#recurring-observations): each
machine carries a bag of kelvin readings, keyed by the machine itself.  The
`|>` pipe feeds it to `map`, which rewrites each reading into one carrying a
`celsius` column.  The view's schema, its index, and its tracked properties are
whatever that pipeline produces; a view has no attribute block because it
declares nothing, it computes.

## The body is a block

A view body is an ordinary block: zero or more `let` bindings, then a trailing
expression of table type that is the materialized result.  Naming an
intermediate table with `let` is how you fork a pipeline, binding a table once
and using it more than once.

```mensura
{{#include ../examples/view-fork.mensura}}
```

Here `split` routes each row wholly to one side of a pair by a predicate over
the key, and `bind` merges the pair back into one table.  Because the key is
the machine, the split routes whole machines: every reading of `m-01` lands on
the same side.  That is the point of keeping the entity as the key (see
[What the types track](../concepts/what-the-types-track.md)): the boundary a
split draws coincides with the entities that must not leak across a
train/test divide.  There is no special pipeline grammar: stages compose left
to right, `let` names a table, and a tuple brings several tables together for
a merge like `bind`.

## Aggregating over a group

In a bag store the groups are already there: the machine is the key, and its
readings are the bag.  `group_map` folds each machine's bag to a single
record.

```mensura
{{#include ../examples/view-aggregate.mensura}}
```

The stage with an obligation is the reducing `group_map`: a fold like `max`
over a bag with rows missing is silently wrong, so the reducer *consumes* a
[completeness](../concepts/what-the-types-track.md) fact, "every group is
whole," which must be established upstream.  A raw `store` does not carry it:
unlike a `collect`, which is a complete census by construction, a store
accumulates observations that can have gaps (a machine offline for a stretch
leaves holes in its bag), so completeness is a claim you make, not a given.
`assume { complete }` makes that claim by fiat, locally and visibly; a
`completeness_check { ... }` stage would prove it instead.  (A `group_map`
over a default store's own key needs no such discharge: with at most one row
per key, a present group is already whole.)

## Coarsening a composite key

Time sometimes belongs in the index: keying a history by `(machine, ts)` is
how a program declares that validation happens at the timestamp level.  Such a
store is a default store again, one row per `(machine, ts)`, and grouping per
machine now means *coarsening* the key first: `shrink_key ts` drops `ts` out
of the key, so rows that differed only in their timestamp share the key
`machine` and form one group.

```mensura
{{#include ../examples/view-coarsen.mensura}}
```

`shrink_key` itself only reindexes: its result is an honest bag of whatever
rows are present, so it demands nothing, and it *propagates* a completeness
fact from the finer key to the coarser one, so the `assume` may equally sit
before it.  The obligation is the reducer's, exactly as above.  This view
type-checks today; executing a key-coarsening stage is the part of the
runtime still being built (`docs/toolkit/04-processing-layer.md`).

## What a view tracks

A view carries the same content and qualifiers as any table (see [What the
types track](../concepts/what-the-types-track.md)), each derived from the
pipeline, not declared:

- **Content**: the index and non-index columns the pipeline yields.
- **Cardinality**: `singletons` (at most one row per key) or `bag`.  A view that
  ends in a summarizing `group_map` is `singletons`; one that ends in a
  bag-shaped stage is `bag`.  A view declares no cardinality the way a store
  does: the pipeline decides, and a derived table may genuinely be a bag.
- **Totality**: whether each value is known or may be missing.
- **Completeness** and **lineage**: carried as the pipeline carries them.

## Constraining a view with a shape

A view may claim a shape with the same `: Shape` clause a store uses.  The claim
constrains the view's *output*, and the check is the store conformance check
run against the computed schema rather than a declared one.

```mensura
{{#include ../examples/view-shape.mensura}}
```

This is the `celsius` view from the top of the page with a `: Celsius` shape
claim added.  Its `map` stage rewrites each reading row for row, so the output
is still a bag of `celsius` values per machine: the shape says so with an
`attr*` block, and the conformance check confirms both the column and the
cardinality.  A shape written with plain `attr` blocks demands `singletons`
instead; `hottest` above could claim one, since its reducing `group_map`
leaves one row per machine.

## Creating a view

`mensura run` materializes a view the same way it creates a store: it scans the
sources, evaluates the pipeline, and writes the result to a table that can be
queried like any other.  A view is recomputed from its sources on each run, so
re-running an unchanged program leaves the same rows.

The chapters that follow look at the two reshaping operations most worth seeing
on their own: [`map`](map.md), which rewrites, drops, and expands rows, and
[`pivot`/`unpivot`](pivot.md), which move between long and wide form.
