# 0020: Reshape as a true inverse pair

## Status

Proposed.  Amends `docs/decisions/0016-reshape-surface.md`: the surface
spelling of `unpivot` and `pivot` survives, but what `unpivot` emits and
which `pivot` form exists change.  Refines the reading of
`docs/language/09-typing-reference.md` section 3.4, which names coarser-key
completeness only operationally, into tracked (graded) qualifier facts.  To
be realized in `09-typing-reference.md` (sections 3.4, 6.3, 6.6, 8, 10),
`docs/language/07-pipelines.md`, the Lean development under `formal/`, and
the `mensura-types` / `mensura-runtime` implementation; until then the
implemented behavior is ADR 0016's.

## Context

The driving requirement, fixed during the design discussion this ADR
records: **`pivot` and `unpivot` must form a truly inverse pair**.  Feature
coverage (finer keys, sparse data, duplicated measurements, computed
reshapes) comes from composing the pair with the other primitives, not from
widening the pair itself.

The running example.  A wide store of students and a long store of
enrollments:

```mensura
unit Person { name: string }
enum Subject { "portuguese", "math", "science" }

store students {
  unit { Person }
  attr {
    portuguese: real
    math: real
    science: real
  }
}
```

Three problems with the ratified surface (ADR 0016) surfaced together:

1. **The round trip passes through Tier B.**  ADR 0016's `unpivot` extends
   the key (the long form is keyed by an Enrollment-like compound), so the
   inverse direction is the index form of `pivot`, which demands a
   completeness discharge.  The identity view

   ```mensura
   view identity {
     students
       |> unpivot subject score (portuguese, math, science)
       |> pivot subject score
   }
   ```

   is rejected without an `assume { complete }`, even though the discharged
   fact holds by construction: `unpivot` saturates the subject axis at every
   student, so "the previous key has not been split".

2. **A provenance artifact.**  Reaching the bag long form (key unchanged,
   `subject` an attribute, several rows per student) from stored long data
   goes through `shrink_key`, a Tier B operation with a completeness
   obligation.  Yet the same data is reachable from the wide side with no
   obligation at all.  The obligation on that path is an artifact of losing
   the provenance fact, not of the data.

3. **Spread-cell totality.**  Pivoting a genuinely sparse population (a
   student enrolled in two of three subjects) must yield optional spread
   columns, while pivoting an `unpivot` result must restore the original
   total columns.  Nothing tracked distinguishes the two.

The formal inventory (all on `main`) already contains the answer's parts:

- `unpivot : Table K N -> Table (K x N)` is split-safe
  (`unpivot_splitSafe`), and the index `pivot` is its inverse on
  functional wide tables: `Functional T := forall k, card <= 1` and
  `pivot_unpivot : Functional T -> pivot (unpivot T) = T`
  (`formal/Mensura/Table.lean`).
- The index `pivot` is **not** split-invariant
  (`pivot_not_splitInvariant`), and `project` (index-long to bag-long) does
  not preserve disjointness (`project_not_preservesDisjoint`).
- The **bag-form** pivot `pivotAttr` (residual key `K`, the (value, name)
  pairs carried in the bag at each key) is split-safe
  (`pivotAttr_splitSafe`, `formal/Mensura/Completeness.lean`): it reads
  only one key's bag, so a split cannot cut a fiber.
- A cell reads as an `Option` (`cellOf`, `formal/Mensura/Table.lean`): one
  row gives the value, zero give missing, and two or more are deliberately
  meaningless.
- Wide to bag-long is a per-row `map` (each wide row becomes one long row
  per variant at the same key), a `BindHom`, so that direction is
  split-safe for free.

The split-safe round trip therefore exists only through the **bag long
form**, and the pair can be made inverse there by theorem.

## Decision

### 1.  The bag pair, inverse on a characterized domain

**`unpivot name value (cols)`** keeps ADR 0016's spelling, but the key is
unchanged: `name` lands as a **non-index enum attribute** whose synthesized
variants are the folded column names, `value` carries the folded cells, and
the cardinality becomes `bag`.  Formally the operation is a per-row `map`
(one output row per variant at the same key) plus the enum typing of the
tag.  On a `singletons` input it establishes `functional(name)` and
`saturated(name)` by mechanism (section 2).  A missing folded cell still
yields its row, with a missing value: row presence and value presence are
distinct axes (ADR 0010).

**`pivot name value`** has one form: the attribute (`pivotAttr`) form.

- **Gate**: `functional(name)`; a `singletons` input implies it.
- **Spread cells**: total iff `saturated(name)` holds and the value column
  is total; optional otherwise.
- `name` in index position is rejected with a hint to `shrink_key` first;
  an index spelling may return later as sugar for that composition.

**Definition by desugaring.**  `unpivot` adds no computational power: its
rows are exactly those of a per-row `map` whose body is the tuple of
tagged records, one per folded column.  The specification takes that as
its definition: `unpivot name value (a, b, ...)` desugars to that `map`,
plus the enum typing of the tag.  What the name adds is the typing
theorem: a general `map` body cannot be granted `functional` and
`saturated` (the checker cannot see that the tags are distinct constants
covering every variant), while the desugared form guarantees them by
construction.  The runtime may evaluate `unpivot` through the desugaring,
and the Lean laws for the pair can reuse the `map` lemmas.  `pivot`, by
contrast, is not expressible as `map`: it reads the whole bag at a key,
so it sits in the other `fiberMap` generator, the whole-bag
(aggregate-shaped) one (`formal/Mensura/Completeness.lean`).  The inverse
pair thus spans the two principal generators of the safe fiber
operations: per-row one way, whole-bag the other.

**The inverse contract**, the design's centerpiece, to be mechanized for
the bag pair:

- `pivot (unpivot W) = W` for every functional (singletons) wide table
  `W`.  The values round-trip exactly, including missing cells.  The type
  round-trips exactly when the folded columns share one totality; a mixed
  fold coarsens every spread column to the join of the folded totalities.
- `unpivot (pivot L) = L` exactly for the long tables `L` that are
  `functional(name)` and `saturated(name)`.
- Outside that domain the two operations act as **normalizers along the
  row-versus-value axis**: `pivot` sends an absent (key, variant) row to a
  missing cell, and `unpivot` reifies a missing cell as a present row with
  a missing value.  This is why saturation is the domain condition, and
  why it cannot be dropped: the pair is inverse precisely where the two
  axes carry the same information.

**Coverage by composition**, the second half of the requirement:

- The Enrollment-style finer key is one Tier A step away:
  `|> extend_key subject`.  `extend_key` **consumes** `functional(subject)`
  to yield a `singletons` result, so the long form loses nothing except
  the spelling of its identity.
- Stored long data is unaffected: a store must key its axis (ADR 0001), so
  a `grades` store is keyed by (name, subject).  Widening it composes
  `shrink_key subject |> pivot subject score`, where `shrink_key` carries
  its honest completeness discharge and establishes `functional(subject)`
  by provenance (the dropped column was part of the identity).
- Duplicated measurements reduce by `group_map` aggregation before the
  pivot; a `pivot_table`-style aggregating pivot is future sugar over
  `group_map |> pivot`, never a primitive.
- Computed reshapes are `map`'s job; the bag `unpivot` is itself only a
  `map` plus enum typing.

### 2.  The tracked facts: graded qualifiers, not new axes

The qualifier row stays the closed four of ADR 0013 (cardinality,
totality, completeness, lineage).  Two of its entries become **graded by a
key extension** instead of being single points:

- **`functional(c)`** is cardinality read at the extended key: at most one
  row per (key, c-value), i.e. `singletons` over key + {c}.  Today's
  `singletons` / `bag` is the degenerate grade (extension by nothing).
- **`saturated(c)`** and **`complete_over S`** are completeness read at an
  extended, respectively coarser, key.  `saturated(c)` (for a
  finite-enumerable attribute `c`) says every present key's bag covers all
  of `c`'s variants; equivalently, `extend_key c` of the table is complete
  over the original key.  `complete_over S` says every present S-fiber
  holds all its real rows; it is monotone in `S`, and the current global
  bit is the current-key grade.  This promotes the operational reading of
  `09-typing-reference.md` section 3.4 into a carried fact.

`unpivot` establishes `functional(name)` and `saturated(name)` with one
mechanism because it is the operation that materializes the key extension.
The M0 freeze's shape is preserved, and the grading stays inside ADR 0004's
propagation-combinator framework.

Establishment, consumption, and conservative propagation:

- `functional(c)`: established by `unpivot` (its name column) and by
  `shrink_key` (each dropped former key column); consumed by `pivot`'s
  gate and by `extend_key c`'s upgrade to `singletons`.  Preserved by
  `split`, by the joins (the right side is keyed, so row counts cannot
  grow), and by a non-expanding `map` that copies `c` verbatim; destroyed
  by expanding maps and by `bind`.
- `saturated(c)`: established by `unpivot` by mechanism; consumed by
  `pivot`'s totality upgrade.  Preserved by `split` (the whole bag rides
  with its residual key, which is the content of `pivotAttr_splitSafe`),
  by `left_join`, and by non-dropping maps that copy `c`; destroyed by
  dropping maps and by `inner_join`.
- `complete_over S`: established by assertion (`completeness_check`,
  `assume`, the reserved `@complete_over`) or by mechanism (`collect`,
  globally); consumed by `shrink_key`.

At the storage boundary the graded cardinality has a cheap physical
witness: a view carrying `functional(c)` materializes with a `UNIQUE`
constraint over its index columns plus `c`, generalizing the composite
primary key a `singletons` table gets
(`docs/toolkit/00-storage-backend.md`).  The facts remain proven at
compile time and trusted at runtime; the constraint is defense in depth,
turning a frontend/runtime disagreement into a loud transaction failure
instead of a silently persisted violation.

**Assertions establish `complete_over` only, never `saturated`.**  A
faithful observation of a sparse population cannot promise rectangularity:
recording every enrollment does not create a science enrollment for a
student who has none.  Sparse data therefore keeps a sound path,

```mensura
view back_to_students {
  grades                       // stored long, keyed (name, subject)
    |> assume { complete }     // faithful: all enrollments recorded
    |> shrink_key subject      // functional(subject) by provenance
    |> pivot subject score     // admissible; spread cells OPTIONAL
}
```

with `real?` spread columns, while the mechanism path (the identity view)
restores total columns.  Totality is exactly as reversible as the facts
warrant.

### 3.  No multiplicity in cells

A cell is 0-or-1, always (ADR 0010).  There is no `T*` cell type, and this
ADR records that as a position, not an omission: multiplicity lives in the
table-scoped cardinality qualifier (ADR 0013), now refined by
`functional`, and in the transient expression-layer bags of
`09-typing-reference.md` section 5.4, which only combinators may consume.
Formally, `cellOf` reads the head of a multiset: zero rows give missing,
one gives the value, and two or more are deliberately meaningless; the
`functional` gate keeps that case unreachable.  A `real*` column would
collapse the cardinality axis into the content, break the scalar storage
mapping, and re-create the list-column problem the two-axis design avoids.
When several values share a (key, name) cell, the language's answer is
`group_map` aggregation, chosen explicitly.

## Consequences

Positive:

- `pivot` and `unpivot` are a truly inverse pair on a stated domain, with
  the mechanization target named (`pivot_unpivot` is the index-pair
  precedent; the bag-pair identities are new `formal/` work).
- The round trip is Tier A end to end: no `assume`, no completeness
  discharge, lineage preserved, and usable after a `split`, since the bag
  rides whole with its residual key.
- `pivot` has one form; the index spelling reduces to a composition.
- Sparse widening is honest (`real?` cells) and mechanism-established
  totality is recovered exactly where it is warranted.
- ADR 0016's surface spelling survives unchanged.

Negative:

- Amends the freshly ratified ADR 0016 and the frozen effects of
  `09-typing-reference.md` section 6.6.  Accepted: the project is pre-1.0
  and the M0 freeze anticipated reconciliation rounds.
- The long form no longer carries the finer identity in its key;
  `extend_key` recovers it, consuming `functional`.
- The qualifier machinery grows two graded facts with their propagation
  rules; the grading must stay within ADR 0004's framework.
- The M2 runtime's reshape operations and the index-form totality rule
  need rework when this ADR is realized.

Neutral:

- Stores of long data still key their axis (ADR 0001), so the completeness
  discharge survives exactly at the stored-widening boundary, where
  sparseness is a real question.

## Alternatives considered

1. **The index pair as the surface** (status quo plus a by-mechanism
   discharge).  Matches `pivot_unpivot` as already proven, and the long
   form carries the Enrollment identity directly.  Rejected: the inverse
   direction stays Tier B, so the round trip is rejected after a `split`
   and drops lineage; the pair is inverse but not freely composable, which
   fails the driving requirement.
2. **Bag primitive plus index sugar now.**  One inverse pair underneath,
   both ergonomics on top.  Deferred rather than rejected: the sugar
   (`pivot` on a key column elaborating to `shrink_key |> pivot`) can
   return once the primitive pair has landed.
3. **Rectangular `complete_over`** (bundling saturation into the asserted
   fact).  Simpler, and runtime-checkable for enum axes, but a sparse
   population then has no sound path to optional spread columns except a
   false assertion.  Rejected for the two-level reading.
4. **`T*` list cells** for multiple values per (key, name).  Rejected;
   section 3.
5. **Per-variant totality on the value column**, to make mixed-totality
   folds exactly type-reversible.  Rejected: dependent-totality machinery
   for a corner case; fold columns of equal totality instead, in two
   `unpivot`s if needed.

## Open questions

- **The tag's typing surface.**  The synthesized enum (current mechanism)
  versus referencing a declared enum (`unpivot subject:Subject ...`): the
  declared form round-trips nominally with stores that declare the enum,
  but folds become sensitive to variant renames, and the synthesized enum
  does not compare equal to a declared one today.  Its own ADR when
  reshape meets nominal enums.
- **Softening `shrink_key`.**  Whether an undischarged `shrink_key` should
  yield an `Incomplete`-marked result instead of a rejection, moving the
  demand to the eventual consumers (the M6 learning operations).
- **Representation of the graded qualifiers.**  A single witness set per
  axis versus an antichain of key extensions; decided at realization.
- **Recognizing the desugared form.**  Whether the checker should detect
  the literal-tagged tuple pattern in a bare `map` and grant `functional`
  and `saturated` there too, making `unpivot` pure sugar with no
  privileged status.  Doable but brittle (an `if` in the body breaks the
  pattern); recorded as a question, not a commitment.
- **Formal work items.**  Mechanize the bag `unpivot` (map plus tag), the
  two bag-pair round-trip identities and their domain conditions, and the
  propagation lemmas for `functional` and `saturated`.
