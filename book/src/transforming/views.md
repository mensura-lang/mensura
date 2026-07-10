# Views

A **store** holds observations that arrive from outside.  A **view** holds rows
that are *computed* from other tables by a pipeline.  It is the first
declaration that hosts the transforming algebra: where a store enumerates its
columns in `attr` blocks, a view names a pipeline and lets the algebra decide
what comes out.

```mensura
{{#include ../examples/view-summary.mensura}}
```

`readings` is a source, referred to by name.  The `|>` pipe threads it through
two stages: `extend_key machine` moves `machine` into the key, and `group_map`
reduces each group to a single record.  The view's schema, its index, and its
tracked properties are whatever that pipeline produces; a view has no `attr`
block because it declares nothing, it computes.

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

The `map` stage rewrites each reading into a record with a `celsius` column, so
the output carries exactly what the `Celsius` shape requires.  A shape claim
checks structure, not cardinality: it does not force a view to `singletons`.

## Creating a view

`mensura run` materializes a view the same way it creates a store: it scans the
sources, evaluates the pipeline, and writes the result to a table that can be
queried like any other.  A view is recomputed from its sources on each run, so
re-running an unchanged program leaves the same rows.

The chapters that follow look at the two reshaping operations most worth seeing
on their own: [`map`](map.md), which rewrites, drops, and expands rows, and
[`pivot`/`unpivot`](pivot.md), which move between long and wide form.
