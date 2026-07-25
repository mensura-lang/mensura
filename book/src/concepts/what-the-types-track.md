# What the types track

Every data tool tracks a *schema*: the names of the columns and the type of
each value.  pandas, Polars, the tidyverse, and SQL all agree on that much, and
it is enough to catch a misspelled column or a string added to an integer.

Mensura tracks the schema too, but the schema is the least interesting part of
a table's type.  A Mensura table is the indexed table of the design notes: a
triple `(K, H, c)` of key columns `K`, non-key columns `H`, and a
cell function `c`.  Rows are *entities*, identified by their key.  On top of
that structure the type carries facts about the *data itself*, facts a schema
cannot express and that other tools therefore leave to runtime, to convention,
or to the analyst's discipline.

Three of those facts do the heavy lifting for correctness: **cardinality**,
**completeness**, and **disjointness**.  This chapter is about what they mean
mathematically and, more to the point, about the mistakes they make
*impossible to write* rather than merely possible to debug.

> The pipeline algebra of this chapter type-checks and runs today: `flat_map`,
> `map_bags`, `promote`/`demote`, the lookups, `split`/`union`, and
> `pivot`/`unpivot` all compile with `mensura check`, and all but
> `demote` (not yet executable) materialize with `mensura run` (see the
> [Transforming data](../transforming/views.md) chapters).  Two things remain
> design previews and are marked as such below:
> the learning operations `fit`/`evaluate` that *consume* disjointness, and the
> richer completeness witnesses.  The [What's next](../whats-next.md) page tracks
> the frontier.  The full specifications live in
> `docs/language/07-pipelines.md` and `docs/language/08-lineage.md`, each backed
> by a machine-checked proof in `formal/`.

The operations used below (`flat_map`, `map_bags`, `demote`, `split`, `union`,
`pivot`, and the rest) are introduced one line each in
[The kernel operations](the-kernel.md); this chapter assumes you have met them
there.

A useful framing before the details.  Formalizations of data-handling algebras
answer two questions: *can I express this operation?* and *can I run it
efficiently?*  A schema is enough for both.  These three properties answer a
third question those tools never ask: *should this operation be allowed on this
data?*  That is the question a number which "looks fine and means nothing" slips
through, and it is the one Mensura turns into a type error.

## Cardinality: how many rows share a key

A key need not identify a single row.  **Cardinality** is the count of rows that
share a key, classified into the three cases that change what is sound:

```text
card(k) = 0      no row at this key
card(k) = 1      exactly one row
card(k) = many   a bag of rows
```

Carried per column, the distinction that matters is **`card <= 1`** (at most one
value per key) versus **many** (a bag of values).  It is part of the content
type, and it enters at the source: a plain store holds at most one row per key
(a duplicate is rejected as a sign the identity criterion is wrong), while a
store declared with `attr*` is a *bag store*, holding many observations per
entity (see [Recurring observations](../modelling/stores.md#recurring-observations)).
From there every operation transforms it predictably: coarsening a key with
`demote` makes rows that differed only in the dropped component share a key,
so cardinality *grows*; an aggregating `map_bags` reduces a bag back to a
single record, so cardinality drops to `1`.

Cardinality is what makes scalar reasoning legal.  A scalar operator (`a - b`, a
comparison) needs a *single known value*, not a bag; the bag combinators (`sum`,
`mean`, `count`, `any`) are precisely the bridge that brings a many-row bag down
to one value.  And reshaping a long table to a wide one is sound only when each
cell it spreads holds at most one value:

```mensura
{{#include ../examples/grades-pivot.mensura}}
```

**What other systems cannot do.**  Run the same reshape in pandas and a
duplicate `(index, column)` pair throws `ValueError: Index contains duplicate
entries, cannot reshape` at runtime, after the pipeline has already done its
work, and only on the data that happened to contain a duplicate.  Mensura makes
"at most one value per key" a fact the type carries, so a `pivot` whose input is
not known to be `card <= 1` is rejected at compile time, on every input, and the
error points at the missing upstream aggregate that would have guaranteed it.
The duplicate that would have exploded at runtime cannot reach runtime.

## Completeness: is the partition whole

**Completeness** is a unary fact about one table: writing `complete_over(k)`,
every key's bag has *all* of its rows present.  It is not about the schema and
not about any single value; it is about whether a partition is fully
materialized.

Completeness is what licenses *reducing* a bag.  A fold (`sum`, `max`,
`count`) over a bag equals the true value only if none of the bag's rows
are missing, so it is the reducing `map_bags` that *consumes* the fact.
Coarsening the key with `demote` demands nothing by itself: a bag of the
rows present is an honest table, and the operation *propagates* the fact from
the finer key to the coarser one.  (`pivot` also coarsens the key and needs no
license either: an absent row simply becomes a missing cell, and a related
fact, `exhaustive`, decides whether the spread columns come out total.)  So
completeness is *established* upstream, *propagated* by the rekey, and
*consumed* by the reduction:

```mensura,ignore
enrollments
|> completeness_check { assert row_count open_offerings == 0 }  // establish
|> demote course                                    // propagate to the coarser key
|> map_bags |k, b| (.total_credits = sum b.credits) // consume
```

One case discharges for free: a reduction over a default store's own key
needs no fact at all, because at most one row per key means a present bag
is already whole; only a coarsened key, or a bag store, can hold a partial
bag for a fold to be silently wrong on.

A table earns the fact in one of three ways: by **mechanism** (a `registry`
source is complete by construction), by a **check** (the `completeness_check`
stage above), or by an **annotation** (`@complete_over(col)` on a source store).
When none of these apply, `assume { ... }` admits the operation by fiat, locally
and visibly.

**What other systems cannot do.**  `SELECT student, sum(credits) ... GROUP BY
student` will compute a per-student total over whatever rows happen to be loaded.
If half a student's enrollments were filtered upstream, or a join silently
dropped them, the total is wrong, the query succeeds, and the number looks
exactly as plausible as the right one.  No tool in the pandas/SQL lineage can
tell a complete partition from a partial one, because the difference is not in
the schema.  Mensura refuses the rollup unless completeness is in scope, so "I
summed over a partition with holes in it" stops being a silent error and becomes
a rejected program.

## Disjointness: do two tables share entities

The first two properties are facts about one table.  **Disjointness** is a
*relation between two tables*, and it is the property the whole language exists
to protect.  In the formalization,

```text
Disjoint T0 T1  :=  forall k,  T0.rows k = 0  or  T1.rows k = 0
```

at every key, at least one of the two tables is empty: the two tables share no
entity.  A relation looks unlike the unary facts above, but it is tracked the
same way.  Each table carries a **lineage region**, the set of keys at which it
is present (its support), recorded symbolically as a predicate over the key.  Two
tables are disjoint when their regions are provably non-overlapping, a check the
compiler performs where the fact is needed.

Disjointness is the precondition for leak-free validation.  A model trained on
one table and scored on another gives an *honest* metric only if the two tables
share no entity; otherwise the model has effectively seen its own test data.
`split` establishes the fact by construction (it routes each entity wholly to one
side, so the halves cannot overlap), and the learning operations demand it:

```mensura,ignore
let (train, test) = enrollments |> split |k| hash k < 0.8   // Disjoint by construction
let model         = train |> fit logistic_regression
let score         = test  |> evaluate model                 // demands Disjoint train test
```

Here `fit` and `evaluate` are learning operations, not part of the kernel; their
typing is the subject of a later design round, and what fixes them is precisely
this demand that their two tables be disjoint.

The fact survives a whole pipeline because every split-invariant (Tier A)
operation preserves it, and such operations compose: a disjointness fact
established by `split` is carried, intact, through `flat_map`, `filter`, the
lookups, and `map_bags` to the point where `evaluate` consumes it.  Two
operations lose it on purpose: `union` unions two regions (so a merged table is
disjoint from a third only if *both* halves were), and the key-changing
operations `demote` and `pivot` drop it, because a region described over the old
key no longer denotes the same entities.  Past such a point the fact must be
re-established with a check or relaxed with `assume`.

**What other systems cannot do.**  Leakage is the canonical, expensive, and
*silent* failure of applied machine learning.  In scikit-learn with pandas you
split a frame, train, and score, and nothing stops an entity from landing on both
sides: a duplicated id, several readings of the same machine, or overlapping time
windows of one subject.  The metric comes back too good, the code raises no
error, and the gap surfaces only in production.  Mensura makes "train and test
are disjoint" a fact the type system tracks from the split to the evaluation and
*demands* at `evaluate`.  A plain random split that would scatter a grouped or
temporal entity across both halves does not type-check, so the leak is caught
before the model is ever fit.

## The common thread

None of these three is an annotation you remember to add.  Each is **derived
from how data enters** (a `store` or `registry` mechanism fixes it), **transformed
by every operation** (each primitive states how it moves the fact), and
**demanded exactly where unsoundness would otherwise hide** (a reshape, a
rollup, an evaluation).  Where the fact cannot be proved, you do not lose it
quietly: you write `assume`, and the relaxation is local, visible, and
auditable.

That is the difference a schema cannot capture.  A schema tells you a column is
called `credits` and holds a number.  It cannot tell you that summing it over
students is meaningful only if no enrollment is missing, that spreading it into a
wide table is sound only if each cell holds one value, or that a metric computed
from it is honest only if the training and test rows are different entities.
Those are facts about the data, and in Mensura they are facts about its type.
