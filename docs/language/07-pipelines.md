# Pipelines

A pipeline transforms one table into another.  In Mensura a pipeline is not a
separate construct: it is an ordinary expression of table type, built from the
one expression sublanguage of `06-expressions.md`.  Stages are function
applications composed left to right with the `|>` pipe, intermediate tables are
named with `let`, and several tables are combined by tupling them into a merge.
There is no special pipeline grammar; there is a set of table-valued
operations, catalogued here.

This document specifies those operations.  Each one is a pure function over
`Table<Qs, C>` (a table's qualifiers and content, see
`docs/decisions/0004-qualifier-mechanism.md`), and each is backed by a theorem
in the Lean formalization (`formal/Mensura/`), cited inline.  This round
specifies the **primitives** only; the familiar named forms (`filter`,
`mutate`, `select`, `aggregate`, `group`/`ungroup`/`project`, window functions,
`tagged_bind`/`tagged_split`) are sugar over these and are deferred to a
follow-up, as are the streaming operations and the hosting of pipelines in
`transform`/`view` declarations.

The syntax shown is preliminary, like the rest of the language docs at this
stage; the design content is not.

## What a pipeline tracks

The point of typing pipelines is that a table carries more than its rows.  An
operation is read by what it does to the table's **content** and its
**qualifiers** (the two parts of `Table<Qs, C>`, ADR 0013), and the type
checker rejects a pipeline that would violate one of them:

- **Content** (`C`): the structure, namely the index (key) columns and the
  non-index columns with their domains.  Reindexing moves columns between the
  key and the non-key part.
- **Cardinality** (table-scoped qualifier): how many nested rows share a key,
  **singletons** (`card <= 1`) or **bag** (`card 0..*`).  Operations
  transform it predictably, and some operations *demand* a particular
  cardinality (`pivot` wants `singletons`, at most one row per key).
- **Totality** (column-scoped qualifier): whether each non-index value is
  known or may be missing (`Cell = Option`).  A value is total unless its
  type is marked `?` (ADR 0010); `left_join` makes its right columns
  optional, and a default, an aggregate, or an `is known` narrowing makes a
  column total again.
- **Completeness** (table-scoped qualifier): whether a partition is fully
  present, that is, whether every group over some key has all of its rows.
  Completeness is what makes a key-shrinking operation sound.  It is
  established (by a check, a source annotation, or a `collect` mechanism)
  and consumed (by a Tier B operation).
- **Lineage** (table-scoped qualifier): the split ancestry that carries
  disjointness, specified in `08-lineage.md`.  Sampling and dependency, the
  two `std` qualifiers of ADR 0004 with no rules yet written, are deferred;
  this document notes where an operation imposes a qualifier-level
  precondition but does not re-specify propagation rules.

The qualifiers this document makes first-class are **cardinality**,
**totality**, and **completeness**: every operation below states how it
affects each one it changes.

## Composition

Three forms thread operations together, all from the expression sublanguage:

- **`|>`**, the pipe: `data |> op` applies `op` to `data`.  An op is an
  ordinary curried function, so a partially applied stage such as
  `left_join machines (|k, l| l.machine)` is the `Table -> Table` value the pipe
  feeds.  The pipe is reversed application, `x |> g` means `g x`, so a stage
  may always be written either way; the equivalence and its two-class
  consequence are spelled out in `06-expressions.md` and recorded in
  `docs/decisions/0018-application-piping-equivalence.md`.  How the checker
  realizes that equivalence is `docs/toolkit/01-application-checking.md`.
- **`let`**, to name an intermediate table and reuse it (forking a pipeline is
  binding a table once and using it twice).
- **tuples**, to bring several tables together for a merge:
  `(train, test) |> bind`.

The central guarantee is **split-safety**.  Every Tier A operation is
`SplitSafe` (`PreservesDisjoint` and `SplitInvariant`), and split-safe
operations are closed under composition (`SplitSafe.comp`, `Table.lean`).  So a
pipeline built only from Tier A operations commutes with a split: running it on
the whole table and running it on each side of a split and re-binding give the
same result.  That is the formal content of "no leakage between train and
test."  A Tier B operation breaks this and must discharge a completeness
obligation to be admitted.

## The primitives

Each entry gives the surface form, the parameters, the effect on **content**,
on **cardinality**, on **completeness** (and on **totality** where the
operation changes it), the Tier, and the backing theorem.
Throughout, lambdas are **key-first** (ADR 0015): `|k, r| ...` binds the key
`k` and a single value row `r`, `|k, g| ...` binds the key and the group `g`
(a row whose cells are bags), `split`'s `|k|` binds the key alone, and
`|_, r|` ignores the key.  A bare column name (`machine`) is a reference to a
column of the current schema.

### `map` - per-row transform

```
data |> map |k, r| (.bmi = r.mass / r.height ^ 2.0)
```

The key-first lambda receives the key and one value row and returns a
**collection of value rows** (ADR 0015): a bare row or record keeps one,
`()` drops the row, and `(a, b, ...)` expands to several.  Content: the
output columns are those of the returned rows; the index is preserved.
Cardinality: the maximum collection size, so a body that returns at most one
row preserves per-key cardinality and a body that may return two or more
yields `bag`.  Completeness: preserved.  Tier A (`map_splitSafe`).

Because the body is a collection, dropping a row (a filter) and emitting
several rows (an expansion) are the same primitive: a filter is
`map |k, r| if c then r else ()`, using the conditionals and collection
literals of `06-expressions.md`.  There is no `filter` primitive; a named
`filter` may later be sugar for this form (ADR 0015).

### `group_map` - per-key whole-group transform

```
data |> group_map |k, g| (.total = sum g.credits)
```

The key-first lambda receives the key and the whole group at it, presented
as a row whose cells are bags (so `g.credits` is the bag of `credits` across
the group, a cardinality-many cell reduced here by `sum`).  Empty groups are
skipped, so the lambda always sees a non-empty group.  Content: the output columns are those of
the return.  Cardinality: **inferred from the return** - returning a single
record yields `singletons` (one row per key, the `aggregate` shape, and it is
what later lets `pivot` satisfy its `singletons` precondition); returning a
bag yields `bag` (the window shape: one output row per input row).  Completeness:
preserved.  Tier A (`fiberMap_splitSafe`).

Window-style returns (a bag, one row per input row, such as a running total or
a rank) additionally require an **ordering** within the group, which is a
dependency-qualifier concern, not a property of the algebra: split-safety holds
regardless, but `rank`/`cumsum` are well-defined only on an ordered group.

### `extend_key` / `shrink_key` - reindexing

Reindexing is one idea with two directions: move a column into the key, or move
one out.  The direction fixes the Tier.

```
data |> extend_key machine      // move the `machine` column into the key
data |> shrink_key course       // move `course` out of the key
```

**`extend_key cols`** promotes non-index column(s) into the key.  Content: the
named columns join the index.  Cardinality: an entity's rows are redistributed
across the finer key; per-key cardinality does not grow.  Completeness:
preserved.  Tier A (`ungroup_splitSafe`).

**`shrink_key cols`** drops index component(s) into the non-index part.
Content: the named key columns become ordinary columns.  Cardinality: rows that
differed only in the dropped component now share a key, so per-key cardinality
**grows** (the result is `bag` over the coarser key unless a following
`group_map` reduces it).  Completeness: **demanded** - shrinking is split-safe
only over a partition that is complete over the retained key, so `shrink_key`
*consumes* a completeness fact.  Tier B (`project_not_preservesDisjoint`).

### `left_join` / `inner_join` - join a fixed table

```
readings |> left_join machines (|k, l| l.machine)
```

Joins the current table against a fixed right table; the key-first lambda
maps a left row (key `k`, value `l`) to the right table's key.  Content: the
right table's columns are added.
Cardinality: preserved when the right table is functional (`singletons`); a
right table with several rows per key multiplies them in, raising the bound
to `bag`.  Totality:
`left_join` makes the added right columns **optional**, since an unmatched left
row is kept with them missing; `inner_join` drops unmatched rows, so it adds no
optionality.  Completeness: preserved on the left.  Tier A
(`leftJoin_splitSafe`, `innerJoin_splitSafe`).

### `split` / `bind` - partition and merge

```
let (train, test) = data |> split |k| hash k < threshold
let full          = (train, test) |> bind
```

**`split |k| pred`** routes each *entity* (each key) wholly to one side of a
pair according to a predicate over the key, never cutting a key's rows apart.
The two halves are disjoint by construction.  Content: unchanged on both sides.
Cardinality: unchanged.  Completeness: each side is complete over the keys it
keeps.  Tier A (`split_disjoint`; `bind_split` shows `bind` undoes it).

**`(a, b) |> bind`** is the multiset union of two tables of the same schema at
each key.  It is **total**: it has no disjointness precondition, and it is
always split-safe and associative/commutative (`bind_comm`, `bind_assoc`).
Content: unchanged.  Cardinality: binding **disjoint** inputs preserves
`singletons`; binding **overlapping** inputs may push an entity above one row,
raising the bound to `bag`.  That lost guarantee is the only thing disjointness
buys; it is not required for the operation to be defined or safe.  Completeness:
the union is complete over a key iff both inputs are.  Tier A.

Disjointness itself (the precondition for *not* leaking across a split) is a
lineage-qualifier matter, tracked in `Qs`, not an algebra precondition on
`bind`.  How that fact is established, propagated, demanded, and assumed is
specified in `08-lineage.md`.

### `unpivot` / `pivot` - reshape long and wide

```
wide |> unpivot metric reading (temperature, humidity)  // long, keyed by (..., metric)
long |> pivot metric reading                            // wide form
```

The ratified surface is in `docs/decisions/0016-reshape-surface.md`:
`unpivot name value (col, ...)` names the new key and value columns
explicitly, and `pivot name value` selects the attribute or index form by
where `name` sits.

**`unpivot name value (cols)`** turns the named value columns into rows,
spreading the column *name* into the key.  Content: the names move into a new
`enum` index column, the values into a single column.  Cardinality: preserved.
Completeness: preserved.  Tier A (`unpivot_splitSafe`).

**`pivot name value`** is the inverse: it gathers, for each key, the values
indexed by the `name` column into one wide row.  It has two forms with
different status:

- **Attribute form** (the `name` is a non-index column): split-safe, and
  admissible exactly when each (key, name) cell is **`singletons`** - which is
  the cardinality guarantee an upstream `group_map`/aggregate provides.  Tier A
  (`pivotAttr_splitSafe`; reversible against `unpivot` via `pivotAttr_reversible`).
- **Index form** (the `name` is part of the key): not split-invariant, because
  a split can cut across the spread names.  Tier B (`pivot_not_splitInvariant`).

So `pivot` is where cardinality tracking pays off directly: the attribute form
type-checks only when the cell it spreads is known to hold at most one value.

## Tier B and completeness

Two operations are Tier B: **`shrink_key`** and the **index form of `pivot`**.
Each is sound only over a complete partition, so each *consumes* a completeness
fact about its input.  The M1 surface for establishing and consuming the fact
is ratified in `docs/decisions/0017-completeness-establish-consume.md`: M1
ships the `completeness_check` and `assume { complete }` stages (with
key-context asserts), and defers `collect`-by-mechanism completeness and the
`@complete_over` annotation.  Completeness is established in one of three ways:

- **`completeness_check { assert ... }`**, a pipe stage that *establishes* the
  fact locally.  It is an ordinary stage (`completeness_check` applied to a
  block of `assert` statements); conceptually it is an operation that
  guarantees completeness, and a later round may let a combination of asserting
  operations stand in for it.  Each `assert` is a boolean expression; together
  they witness that the partition is complete over the relevant key.  The fact
  must hold where the Tier B operation runs, so the check is placed on the
  pipeline ahead of it.

  ```
  enrollments
  |> completeness_check { assert row_count open_offerings == 0 }
  |> shrink_key course
  |> group_map |k, g| (.total_credits = sum g.credits)
  ```

- **`@complete_over(col)`** on a source store, establishing the fact globally so
  no per-use check is needed.  This is an annotation; its surface lands with
  the annotation family (`@audited`, `@versioned`, ...), so this document names
  it but does not fix its grammar.
- **mechanism**: a `collect` source is complete by construction (overview
  pillar 7), so a Tier B operation over it needs no further discharge.

`assume { ... }` remains the escape hatch: it admits a Tier B operation by
fiat, locally and visibly, when the obligation cannot be discharged.

## Cardinality, totality, and the type

Two orthogonal qualifiers are threaded by every operation above:
**cardinality** (table-scoped: how many rows share a key, `singletons` or
`bag`) and **totality** (column-scoped: whether a value is known or may be
missing, `Cell = Option`).
The rules that consume them are stated in `06-expressions.md`: a scalar
operator requires a **single known value** (`card 1` and not missing), the bag
combinators (`sum`, `min`, `max`, `count`, `any`, `all`, `in`; `mean` is
derived, `sum(x) / to_real(count(x))`, ADR 0014) bring a many-row bag
down to one value, and a missing value is made known by a default, an
aggregate, or an `is known` narrowing.  At the pipeline level `pivot`'s
attribute form is admitted only at `singletons` (at most one row per key).
ADR 0010 settles the total/optional axis and its `?` marker; how
`singletons` / `bag` (and the derived `exhaustive`) are written in a type
stays the content/types document's job.  This document specifies how each operation
*changes* these properties and where one is *demanded*, and leans on inference
for the rest.

## Qualifiers and purity

Sampling, dependency, and lineage propagate through every operation by the rule
combinators of ADR 0004; this document does not re-state those rules per
operation.  Two qualifier-level preconditions are worth flagging because they
sit next to operations here: window-shaped `group_map` returns need an ordering
from the dependency qualifier, and leak-free use of `bind` is governed by the
lineage qualifier (disjointness, specified in `08-lineage.md`), not by the
algebra.

Every operation is pure and lazy, as everything in the expression sublanguage
is.  A pipeline is a description of a table; the hosting site
(`view`/`collect`/`store`/endpoint) decides when it runs.

## Worked examples

**Summarize by an attribute (Tier A throughout).**

```
readings
|> extend_key machine
|> group_map |k, g| (.temp_mean = sum g.temperature / to_real (count g.temperature), .temp_max = max g.temperature)
```

`extend_key` adds `machine` to the key (content: index grows; cardinality and
completeness preserved); `group_map` reduces each group to one record, so the
result is **singletons** per `(…, machine)` key.  All Tier A, so it composes
safely; it type-checks.

**Coarsen the key (Tier B, with the completeness fact established first).**

```
enrollments
|> completeness_check { assert row_count open_offerings == 0 }
|> shrink_key course
|> group_map |k, g| (.total_credits = sum g.credits)
```

The check **establishes** "complete over student"; `shrink_key course`
**consumes** it (dropping `course` makes the table `bag` over `student`);
`group_map` brings it back to **singletons** per student.  It type-checks
because the obligation was discharged.  Remove the check (and `@complete_over`, and
`assume`) and `shrink_key` is rejected.

**Train/test split and re-merge (cardinality under `bind`).**

```
let (train, test) = data |> split |k| hash k < threshold
let full          = (train, test) |> bind
```

`split` yields a disjoint pair, each complete over the keys it keeps; binding
the disjoint pair preserves `singletons` and reconstructs `data` (`bind_split`).
Binding two *overlapping* tables would instead yield `bag`, the documented
cost of dropping disjointness.  It type-checks.

## Forward references and open questions

- **Consolidated rules.**  The per-primitive rules above are collected, with the
  expression and completeness rules, in `09-typing-reference.md` (the M0
  freeze), which makes the four tracked properties (cardinality, totality,
  completeness, and disjointness via a lineage hierarchy) explicit in
  `Table<Qs, C>`.
- **Named sugar.**  `filter`, `mutate`, `select`, `aggregate`,
  `group`/`ungroup`/`project`, window functions (`rank`, `cumsum`), and
  `tagged_bind`/`tagged_split` are sugar over the primitives above and get
  their own round.
- **Expression features the fuller surfaces need.**  Row-dropping and
  row-expanding `map` are covered by the conditionals and collection literals
  of `06-expressions.md` (ADR 0015); bag-returning `group_map` (windows)
  still needs an ordering from the dependency qualifier.
- **The cardinality-type notation.**  How `singletons` / `bag` (and the
  derived `exhaustive`) are written in a `Type` is the content/types
  document.  (The orthogonal total/optional axis and its `?` marker are
  settled in ADR 0010.)
- **`@complete_over` and other annotations.**  The annotation surface
  (`@audited`, `@versioned`, `@auto`, `@complete_over`) is its own document.
- **Hosting and streaming.**  `view` declarations that host pipelines are
  specified in `10-views.md`.  The other hosting sites (`transform`, `collect`,
  `device`) and the streaming operations (`sliding_window`, `latest`, reactive
  `on` blocks) extend this grammar and get their own sections.
