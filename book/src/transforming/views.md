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
`|>` pipe feeds it to `flat_map`, which rewrites each reading into one carrying
a `celsius` column.  The view's schema, its key columns, and its tracked
properties are whatever that pipeline produces; a view has no attribute block
because it declares nothing, it computes.

## The body is a block

A view body is an ordinary block: zero or more `let` bindings, then a trailing
expression of table type that is the materialized result.  Naming an
intermediate table with `let` is how you fork a pipeline, binding a table once
and using it more than once.

```mensura
{{#include ../examples/view-fork.mensura}}
```

Here `split` routes each row wholly to one side of a pair by a predicate over
the key, and `union` merges the pair back into one table.  Because the key is
the machine, the split routes whole machines: every reading of `m-01` lands on
the same side.  That is the point of keeping the entity as the key (see
[What the types track](../concepts/what-the-types-track.md)): the boundary a
split draws coincides with the entities that must not leak across a
train/test divide.  There is no special pipeline grammar: stages compose left
to right, `let` names a table, and a tuple brings several tables together for
a merge like `union`.

## Aggregating over a bag

In a bag store the bags are already there: the machine is the key, and its
readings are the bag.  `map_bags` folds each machine's bag to a single
record.

```mensura
{{#include ../examples/view-aggregate.mensura}}
```

The stage with an obligation is the reducing `map_bags`: a fold like `max`
over a bag with rows missing is silently wrong, so the reducer *consumes* a
[completeness](../concepts/what-the-types-track.md) fact, "every bag is
whole," which must be established upstream.  A raw `store` does not carry it:
unlike a `registry`, which is a complete census by construction, a store
accumulates observations that can have gaps (a machine offline for a stretch
leaves holes in its bag), so completeness is a claim you make, not a given.
`assume { complete }` makes that claim by fiat, locally and visibly; a
`completeness_check { ... }` stage would prove it instead.  (A `map_bags`
over a default store's own key needs no such discharge: with at most one row
per key, a present bag is already whole.)

Declaring the source a `registry` instead removes the discharge entirely:
the same pipeline with no `assume` compiles, because the declaration is the
sole intake for its observations and so its bags are whole by construction.
That is the difference the keyword buys, at the registry's own key (a
coarsened key is another matter, below), and it is the only difference: a
registry's body, its storage, and its reads are a store's exactly.

## Windows: the other shape of `map_bags`

A reduction collapses each bag to one row.  The other thing you can do with a
bag is walk it *in order*, emitting one row per input row: a running maximum,
the previous reading, a rank.  Those are windows, and they are the same
`map_bags` with a different return.

```mensura
{{#include ../examples/series-windows.mensura}}
```

Three things are worth reading off that.

**The order is named at the operator, not carried by the store.**  A bag store
has no row order, so a window says which order it means, here `|r| r.taken_at`.
Both lambdas take a row: one pulls out the value to accumulate, the other the
key to sort by.  You need both from the same row, which is why the trailing
argument is the bag of rows `b` rather than a single projected column.

**Completeness follows the combiner, not the shape.**  `running_peak` needs
the same fact the reduction above consumed, and for the same reason: a
running maximum's last row *is* the maximum, and every one of its rows folds
the readings so far, so a gap early in the bag corrupts every later output
exactly as it corrupts a fold.  The `assume { complete }` in the example is
that demand made visible; it would be incoherent for `max` to carry the
obligation while a running maximum computed the same number without it.
`previous` is different: "the reading before this one" is a claim about the
readings you *have*, honest whether or not some were lost, so `lag` (and
`lead`, and `first_value`) demand nothing.  The line is per combiner: a scan
that contains its reduction carries the reduction's obligation, and one that
only relates neighbouring present rows does not.

**But a window demands something a fold does not: the order must be
unambiguous.**  If two readings share a `taken_at`, there is no single right
way to arrange them, and a running total would depend on which one the storage
happened to hand over first.  So a scan asks the same kind of question the
reducer asks about completeness, and gets an answer the same two ways.

The example above answers it by *construction*.  `taken_at` is part of the
identity, so at most one reading exists per `(machine, taken_at)`; `demote`
moves the time out of the key while keeping that fact, and within each
machine's group the time is unique.  Nothing is claimed, because nothing needs
to be.  This is the shape worth reaching for, and it is also just honest
modelling: a reading really is identified by when it was taken.

When the order genuinely can tie, say so:

```mensura,ignore
readings
  |> assume { arranged }
  |> assume { complete }
  |> map_bags |k, b| (.hottest = series.rank (|r| desc r.temperature) b)
```

Ranking by temperature is ambiguous on purpose here, so the claim is the
honest thing to write.  An order the compiler can neither prove nor see
claimed is an error, not a guess.  The second claim is the completeness rule
from above wearing a different hat: a rank is a running count, so it is a
scan that contains its reduction, and a rank over a bag whose wholeness
nobody vouched for is a rank among the rows that happened to arrive.  Both
assumptions are visible, which is the point.

**`previous` is optional and the others are not.**  Nobody wrote a rule for
that.  `series.lag` is an *exclusive* scan: each row sees the fold of the rows
strictly before it, and the earliest row in each group has nothing before it.
The value it would report has no answer, so the column carries a missing value
and the type says so.  Its inclusive sibling `running_max` has no such hole,
because every row sees at least itself.  `series.lead` is the mirror image: it
is `lag` under the reversed order, so there the *last* row is the missing one.

Descending order is marked on the key value (`desc r.temperature`), not on the
operator, so each part of a key can go its own way.

The window vocabulary is a library, not language: `cumsum`, `rank`, `lag`,
`lead`, `first_value`, `running_min`, and `running_max` are ordinary definitions
in the bundled `series` module, each one a `scan` or a `prescan` at a chosen
combiner.  You can read them, and the reason `lag`'s first row is missing is
visible in its definition rather than buried in the compiler.

## Coarsening a composite key

Time sometimes belongs in the key: keying a history by `(machine, ts)` is
how a program declares that validation happens at the timestamp level.  Such a
store is a default store again, one row per `(machine, ts)`, and folding per
machine now means *coarsening* the key first: `demote ts` drops `ts` out
of the key, so rows that differed only in their timestamp share the key
`machine` and form one bag.

```mensura
{{#include ../examples/view-coarsen.mensura}}
```

`demote` itself only rekeys: its result is an honest bag of whatever
rows are present, so it demands nothing.  But it does not carry a
completeness fact across the coarsening either.  Completeness is about
the current key, and a `(machine, ts)` row that was never recorded has
no bag to be partial at the fine key; merge the keys and that absence
becomes a hole *inside* the machine's bag.  So the claim is made where
the fold happens: the `assume` sits after the `demote`, at the key the
reducer folds.  The obligation is the reducer's, exactly as above.

## What a view tracks

A view carries the same content and qualifiers as any table (see [What the
types track](../concepts/what-the-types-track.md)), each derived from the
pipeline, not declared:

- **Content**: the key and non-key columns the pipeline yields.
- **Cardinality**: `singletons` (at most one row per key) or `bag`.  A view that
  ends in a summarizing `map_bags` is `singletons`; one that ends in a
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
claim added.  Its `flat_map` stage rewrites each reading row for row, so the
output is still a bag of `celsius` values per machine: the shape says so with an
`attr*` block, and the conformance check confirms both the column and the
cardinality.  A shape written with plain `attr` blocks demands `singletons`
instead; `hottest` above could claim one, since its reducing `map_bags`
leaves one row per machine.

## Creating a view

`mensura run` materializes a view the same way it creates a store: it scans the
sources, evaluates the pipeline, and writes the result to a table that can be
queried like any other.  A view is recomputed from its sources on each run, so
re-running an unchanged program leaves the same rows.

The chapters that follow look at the two reshaping operations most worth seeing
on their own: [`flat_map`](flat-map.md), which rewrites, drops, and expands
rows, and [`pivot`/`unpivot`](pivot.md), which move between long and wide form.
