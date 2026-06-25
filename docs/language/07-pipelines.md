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
operation is read by what it does to the table's **tracked properties**, and
the type checker rejects a pipeline that would violate one of them.  A pipeline
threads four kinds of property:

- **Content** (`C`): the schema, namely the index (key) columns and the
  non-index columns with their domains.  Reindexing moves columns between the
  key and the non-key part.
- **Cardinality**: how many values a cell holds, and (per key) how many rows an
  entity has.  Cardinality is part of the content type (`06-expressions.md`);
  operations transform it predictably, and some operations *demand* a
  particular cardinality (scalar operators want card 1, `pivot` wants
  `card <= 1`).
- **Completeness**: whether a partition is fully present, that is, whether
  every group over some key has all of its rows.  Completeness is what makes a
  key-shrinking operation sound.  It is established (by a check, a source
  annotation, or a `collect` mechanism) and consumed (by a Tier B operation).
- **Qualifiers** (`Qs`): sampling, dependency, and lineage, defined in `std`
  per ADR 0004.  Each primitive carries propagation rules for them; this
  document does not re-specify those rules, but it notes where an operation
  imposes a qualifier-level precondition.

The two properties this document makes first-class, beyond plain content, are
**cardinality** and **completeness**: every operation below states how it
affects each.

## Composition

Three forms thread operations together, all from the expression sublanguage:

- **`|>`**, the pipe: `data |> op` applies `op` to `data`.  An op is an
  ordinary curried function, so a partially applied stage such as
  `left_join machines (|r| r.machine)` is the `Table -> Table` value the pipe
  feeds.
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
on **cardinality**, and on **completeness**, the Tier, and the backing theorem.
Throughout, `|r| ...` is a lambda over a single row, `|g| ...` a lambda over a
group (a row whose cells are bags), and a bare column name (`machine`) is a
reference to a column of the current schema.

### `map` - per-row transform

```
data |> map |r| (.bmi = r.mass / r.height ^ 2)
```

The lambda receives one row and returns a row (a record).  Content: the output
columns are those of the returned record.  Cardinality: 1:1, so per-key
cardinality is preserved.  Completeness: preserved.  Tier A
(`map_splitSafe`).

`map` is the per-row case of the general key-preserving operation; dropping a
row (a filter) or emitting several rows (an expansion) is the same primitive
returning zero or many rows, which needs the expression-level conditionals and
collection literals that are not yet specified (see forward references).

### `group_map` - per-key whole-group transform

```
data |> group_map |g| (.total = sum g.credits)
```

The lambda receives the whole group at a key, presented as a row whose cells
are bags (so `g.credits` is the bag of `credits` across the group, a
cardinality-many cell reduced here by `sum`).  Empty groups are skipped, so the
lambda always sees a non-empty group.  Content: the output columns are those of
the return.  Cardinality: **inferred from the return** - returning a single
record yields card 1 per key (this is the `aggregate` shape, and it is what
later lets `pivot` satisfy its `card <= 1` precondition); returning a bag yields
card many (the window shape: one output row per input row).  Completeness:
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
**grows** (the result is card many over the coarser key unless a following
`group_map` reduces it).  Completeness: **demanded** - shrinking is split-safe
only over a partition that is complete over the retained key, so `shrink_key`
*consumes* a completeness fact.  Tier B (`project_not_preservesDisjoint`).

### `left_join` / `inner_join` - join a fixed table

```
readings |> left_join machines (|r| r.machine)
```

Joins the current table against a fixed right table; the lambda maps a left row
to the right table's key.  Content: the right table's columns are added.
Cardinality: preserved when the right table is functional (`card <= 1` per
key); a right table with several rows per key multiplies them in.  Completeness:
preserved on the left; `left_join` keeps unmatched left rows (their right
columns missing), `inner_join` drops them.  Tier A (`leftJoin_splitSafe`,
`innerJoin_splitSafe`).

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
`card <= 1`; binding **overlapping** inputs may push an entity above one row, so
the result is card many.  That lost guarantee is the only thing disjointness
buys; it is not required for the operation to be defined or safe.  Completeness:
the union is complete over a key iff both inputs are.  Tier A.

Disjointness itself (the precondition for *not* leaking across a split) is a
lineage-qualifier matter, tracked in `Qs`, not an algebra precondition on
`bind`.  How that fact is established, propagated, demanded, and assumed is
specified in `08-lineage.md`.

### `unpivot` / `pivot` - reshape long and wide

```
wide |> unpivot reading_a reading_b      // long form, keyed by (..., name)
long |> pivot name value                 // wide form
```

**`unpivot cols`** turns the named value columns into rows, spreading the column
*name* into the key.  Content: the names move into the index, the values into a
single column.  Cardinality: preserved.  Completeness: preserved.  Tier A
(`unpivot_splitSafe`).

**`pivot name value`** is the inverse: it gathers, for each key, the values
indexed by the `name` column into one wide row.  It has two forms with
different status:

- **Attribute form** (the `name` is a non-index column): split-safe, and
  admissible exactly when each (key, name) cell is **`card <= 1`** - which is
  the cardinality guarantee an upstream `group_map`/aggregate provides.  Tier A
  (`pivotAttr_splitSafe`; reversible against `unpivot` via `pivotAttr_reversible`).
- **Index form** (the `name` is part of the key): not split-invariant, because
  a split can cut across the spread names.  Tier B (`pivot_not_splitInvariant`).

So `pivot` is where cardinality tracking pays off directly: the attribute form
type-checks only when the cell it spreads is known to hold at most one value.

## Tier B and completeness

Two operations are Tier B: **`shrink_key`** and the **index form of `pivot`**.
Each is sound only over a complete partition, so each *consumes* a completeness
fact about its input.  Completeness is established in one of three ways:

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
  |> group_map |g| (.total_credits = sum g.credits)
  ```

- **`@complete_over(col)`** on a source store, establishing the fact globally so
  no per-use check is needed.  This is an annotation; its surface lands with
  the annotation family (`@audited`, `@versioned`, ...), so this document names
  it but does not fix its grammar.
- **mechanism**: a `collect` source is complete by construction (overview
  pillar 7), so a Tier B operation over it needs no further discharge.

`assume { ... }` remains the escape hatch: it admits a Tier B operation by
fiat, locally and visibly, when the obligation cannot be discharged.

## Cardinality and the type

Cardinality is carried in the content type and threaded by every operation
above.  The rules that consume it are stated in `06-expressions.md`: a scalar
operator requires its operands at card 1, and the bag combinators (`sum`,
`mean`, `count`, `any`, `all`, `in`) are the explicit way to bring a
cardinality-many cell down to one.  At the pipeline level the same currency
pays for `pivot`: its attribute form is admitted only at `card <= 1`.  How a
cardinality is written in a type (card 1 versus 0-or-1 versus many) is the
content/types document's job; this document specifies how each operation
*changes* cardinality and where one is *demanded*, and leans on inference for
the rest.

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
|> group_map |g| (.temp_mean = mean g.temperature, .temp_max = max g.temperature)
```

`extend_key` adds `machine` to the key (content: index grows; cardinality and
completeness preserved); `group_map` reduces each group to one record, so the
result is **card 1** per `(…, machine)` key.  All Tier A, so it composes safely;
it type-checks.

**Coarsen the key (Tier B, with the completeness fact established first).**

```
enrollments
|> completeness_check { assert row_count open_offerings == 0 }
|> shrink_key course
|> group_map |g| (.total_credits = sum g.credits)
```

The check **establishes** "complete over student"; `shrink_key course`
**consumes** it (dropping `course` makes the table card many over `student`);
`group_map` brings it back to **card 1** per student.  It type-checks because
the obligation was discharged.  Remove the check (and `@complete_over`, and
`assume`) and `shrink_key` is rejected.

**Train/test split and re-merge (cardinality under `bind`).**

```
let (train, test) = data |> split |k| hash k < threshold
let full          = (train, test) |> bind
```

`split` yields a disjoint pair, each complete over the keys it keeps; binding
the disjoint pair preserves `card <= 1` and reconstructs `data` (`bind_split`).
Binding two *overlapping* tables would instead yield card many, the documented
cost of dropping disjointness.  It type-checks.

## Forward references and open questions

- **Named sugar.**  `filter`, `mutate`, `select`, `aggregate`,
  `group`/`ungroup`/`project`, window functions (`rank`, `cumsum`), and
  `tagged_bind`/`tagged_split` are sugar over the primitives above and get
  their own round.
- **Expression features the fuller surfaces need.**  Row-dropping and
  row-expanding `map`, and bag-returning `group_map` (windows), need
  expression-level conditionals, collection literals, and (for windows)
  ordering; those are specified before the sugar that uses them.
- **The cardinality-type notation.**  How card 1 / 0-or-1 / many is written in
  a `Type` is the content/types document.
- **`@complete_over` and other annotations.**  The annotation surface
  (`@audited`, `@versioned`, `@auto`, `@complete_over`) is its own document.
- **Hosting and streaming.**  `transform`/`view` declarations that host
  pipelines, and the streaming operations (`sliding_window`, `latest`,
  reactive `on` blocks), extend this grammar and get their own sections.
