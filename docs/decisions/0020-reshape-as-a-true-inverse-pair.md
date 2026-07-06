# 0020: Reshape as a true inverse pair

## Status

Proposed.  Amends `docs/decisions/0016-reshape-surface.md`: `unpivot`
loses its column list (the fold is total over the attributes) and changes
its missing-cell semantics (dropped rows, not reified missing values), and
`pivot` keeps only the index form, with no completeness obligation.
Refines the reading of `docs/language/09-typing-reference.md`
section 3.4 in two directions, kept deliberately distinct: its
"exhaustive" corollary becomes a tracked fact on enum index axes, and its
coarser-key completeness (`complete_over`, ADR 0017's obligation) stays
population-relative.  To be realized in
`09-typing-reference.md` (sections 3.4, 6.6, 8, 10),
`docs/language/07-pipelines.md`, the Lean development under `formal/`, and
the `mensura-types` / `mensura-runtime` implementation; until then the
implemented behavior is ADR 0016's.

## Context

The driving requirement, fixed during the design discussion this ADR
records: **`pivot` and `unpivot` must form a truly inverse pair**.
Feature coverage (partial folds, extra columns, sparse data, duplicated
measurements, computed reshapes) comes from composing the pair with the
other primitives, not from widening the pair itself.

The running example.  A wide store of students, and a long store of
enrollments keyed by the finer Enrollment-like compound:

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

Problems this ADR resolves:

1. **The round trip demanded a discharge that held by construction.**
   Under ADR 0016 plus the completeness machinery of ADR 0017, the
   identity view `students |> unpivot ... |> pivot subject score` was
   rejected without an `assume { complete }`, even though `unpivot`
   saturates the subject axis at every student by mechanism.
2. **Spread-cell totality was untracked.**  Pivoting a genuinely sparse
   population (a student enrolled in two of three subjects) must yield
   optional spread columns, while pivoting an `unpivot` result must
   restore the original totalities exactly.  Nothing tracked
   distinguished the two.
3. **Reify semantics blocked exact reversibility.**  ADR 0016's `unpivot`
   turns a missing wide cell into a long row carrying a missing value.
   Then `pivot` sends an *absent row* to a *missing cell*, and the round
   trip long-to-wide-to-long fabricates rows a sparse table never had,
   which is where a saturation side condition kept creeping in.
4. **No recorded answer on multiplicities in cells** (`real*`?).

Formal anchors (`formal/Mensura/Table.lean`,
`formal/Mensura/Completeness.lean`):

- `Functional T := forall k, card <= 1`, and the round trip is already
  proven one way: `pivot_unpivot : Functional T -> pivot (unpivot T) = T`.
- The index `pivot` is **not** split-invariant
  (`pivot_not_splitInvariant`): a split whose predicate reads the spread
  axis can cut a fiber across sides.  This is a real property of the
  operation, not an artifact.
- A cell reads as an `Option` (`cellOf`): zero rows give missing, one
  gives the value, two or more are deliberately meaningless.
- The formal `unpivot` currently reifies missing cells; the drop variant
  decided here needs its own definition and law statements.

The decisive discussion insight: with **drop semantics**, value-missing in
the wide table and row-absent in the long table become the *same
information*, a bijective transposition.  Reify semantics breaks that
correspondence, and every saturation side condition in earlier drafts was
the price of breaking it.

## Decision

### 1.  The pair: index form, total fold, drop semantics

**`unpivot name value`** takes exactly two identifiers and folds **all**
attribute columns, which must share one scalar domain.

- The key extends by `name`, a new enum index column whose variants are
  the folded column names.  `value` carries the folded cells.
- **A missing cell yields no row.**  The long form's value column is
  therefore always total: missing values never enter the long table.
- Establishes **`exhaustive(name)`** by mechanism when every folded
  column is total: each old row then yields one long row per variant, so
  every residual fiber covers the whole axis.  When some folded column is
  optional, the dropped cells leave holes and no fact is established
  (section 2).  Cardinality is preserved.
- Excluding a column, or unpivoting a heterogeneous wide table, is
  upstream composition: project first (a `map` today; a `select`/`drop`
  sugar may come later).  ADR 0016's rationale for explicit *identifiers*
  is untouched; only its column *list* is removed, in favor of a total
  fold with a canonical domain.

**`pivot name value`** is the inverse and the only form.

- `name` must be an enum-domained **index** column; `name` in attribute
  position is rejected with a hint to `extend_key` first.
- The input must be `singletons` and its attributes must be exactly
  `value` ("drop or aggregate other columns first").  The key discipline
  itself guarantees at most one row per (residual key, variant), which is
  what makes each spread cell well-defined; no separate uniqueness fact
  is needed.
- **No completeness obligation.**  An absent (key, variant) row simply
  becomes a missing cell.  ADR 0017's pivot obligation dissolves into a
  totality upgrade: the spread columns are total iff the input is
  `exhaustive(name)` and the value column is total; optional otherwise.
- **Lineage is dropped** (`pivot_not_splitInvariant`).  This is the one
  cost the index form pays, and it is honest: pivoting after a split that
  discriminated the spread axis genuinely differs from pivoting the
  whole.

**The inverse contract**, to be mechanized with the drop-variant
definitions:

- `pivot (unpivot W) = W` for every `singletons` wide table `W` with at
  least one total folded column.  Values round-trip exactly, including
  missing cells.  Types round-trip exactly when the folded columns share
  one totality: all total gives `exhaustive(name)` back, hence total
  spread columns; a mixed fold coarsens every spread column to optional
  (a per-variant refinement is an open question).  (The side condition
  exists because a row whose folded cells are all missing drops with its
  key; one total column guarantees every key survives.)
- `unpivot (pivot L) = L` for every `singletons` long table `L` whose
  only attribute is a total value column.  **No saturation or
  completeness condition**: sparse tables round-trip as they are, because
  drop semantics inverts the missing-cell / absent-row transposition.
- Outside these domains the operations are still defined; they normalize
  toward the domain (pivot canonicalizes absence as missing cells,
  unpivot canonicalizes it as absent rows).

**Definition by desugaring.**  `unpivot` adds no computational power: it
is a per-row `map` (one tagged record per *known* folded cell, so the
drop is a conditional collection) followed by `extend_key name`.  The
specification takes that as its definition.  What the name adds is the
typing theorem: the desugared form types as a `bag` with a plain string
tag, while `unpivot` is entitled to the enum typing of `name`, the
`singletons` cardinality over the extended key, and (for a fold of total
columns) `exhaustive(name)`, all by construction.  `pivot`, by contrast,
is not expressible as `map`: it reads the whole fiber at a residual key,
sitting with the whole-bag (aggregate-shaped) operations, and it changes
the key.  The pair thus
spans the two shapes of fiber operation: row-wise and key-extending one
way, fiber-collapsing the other.

**Coverage by composition:**

- Partial or heterogeneous folds: project upstream, then `unpivot`.
- The bag long form (name as an attribute): `|> shrink_key name`, with
  its own honest completeness discharge (ADR 0017, unchanged).
- A long table with extra attributes: drop or aggregate them before
  `pivot`; widening several value columns is several pivots joined.
- Duplicated measurements (several rows per (key, variant)): the input is
  not `singletons`, so `pivot` rejects it; reduce with `group_map`
  aggregation first.  A `pivot_table`-style aggregating pivot is future
  sugar over `group_map |> pivot`, never a primitive.
- Sparse stored data pivots **directly**: `grades |> pivot subject score`
  is admissible as-is and yields `real?` spread columns, honestly.

### 2.  `exhaustive`: the rectangle fact, distinct from `complete_over`

Two different "all rows present" facts meet at the reshape pair, and this
ADR keeps them apart deliberately (an intermediate draft merged them; the
merge is unsound, see Alternatives):

- **`exhaustive(A)`**, for an enum-domained index column `A`: every
  residual key present in the table carries its `(k, v)` row for **every
  variant** `v`.  The reference is `A`'s *type domain* (the variant set),
  so the fact is extensional and decidable per fiber.  The name is
  section 3.4's "exhaustive" corollary, localized to one axis.
- **`complete_over S`** (ADR 0017, unchanged): every present S-fiber
  holds all its rows *of the population*, the sampling-frame reading of
  section 3.4.  This is `shrink_key`'s obligation, and it stays
  population-relative.

Neither implies the other.  A faithfully recorded but sparse enrollment
store is `complete_over name` without being exhaustive: a never-enrolled
subject has no real row to record.  Conversely, a table padded with
fabricated variant rows is exhaustive without being complete over
anything.  `pivot`'s totality upgrade needs the rectangle, not
faithfulness: granting total spread columns to faithful-but-sparse data
would write missing values into total columns.  So the upgrade consumes
`exhaustive`, never `complete_over`.

This is still **not a new qualifier axis**.  The row stays the closed
four of ADR 0013: both facts are grades of the completeness entry, read
against two different references (the axis' finite type domain; the
population).  Uniqueness needs nothing, because the key already carries
it; that is why this formulation needs one new fact where the bag draft
needed two (see Alternatives).

- **Establish**: `unpivot`, by mechanism, exactly when every folded
  column is total.  By witness: `completeness_check` (ADR 0017), which
  can decide the rectangle for an enum axis.  By fiat: `assume`, locally
  and auditably owned by the author.  A store-level declaration is
  deferred (Open questions).
- **Consume**: `pivot`'s totality upgrade over the residual key.

**Propagation is the design's remaining work.**  `exhaustive` and
`complete_over` are both row-presence facts, so one conservative table
serves both:

- **Preserved** by `extend_key` and `shrink_key` (they re-slot columns;
  the rows, hence the fibers, are unchanged); by `left_join` (adds
  columns, never drops rows); by `group_map` (one output row per present
  key in the aggregate shape, one per input row in the window shape); by
  a provably non-dropping `map` (the checker's collection-size analysis
  already distinguishes a body with no `( )` branch); and by `bind` when
  both sides carry the fact (a union of full fibers is full).
- **Destroyed** by a `map` whose body can drop, by `inner_join`, and by
  `split` (a key predicate can discriminate within a fiber; recognizing
  predicates that provably ignore the dropped axes is the axis-aware
  refinement in Open questions).
- `unpivot` and `pivot` translate row-presence facts across the key
  change: a pre-existing fact survives `unpivot` when the fold drops
  nothing (all folded columns total), and any fact over a sub-key of the
  residual key survives `pivot` (the output has one row per present
  residual key).

The physical witness comes free: the long form is keyed, so the composite
primary key of the materialized table
(`docs/toolkit/00-storage-backend.md`) already enforces at most one row
per (residual key, variant); a frontend/runtime disagreement fails the
insert loudly.

### 3.  No multiplicity in cells

A cell is 0-or-1, always (ADR 0010).  There is no `T*` cell type, and
this ADR records that as a position, not an omission: multiplicity lives
in the table-scoped cardinality qualifier (ADR 0013) and in the transient
expression-layer bags of `09-typing-reference.md` section 5.4, which only
combinators may consume.  Formally, `cellOf` reads the head of a
multiset; the two-or-more case is deliberately meaningless, and `pivot`'s
`singletons` gate keeps it unreachable.  A `real*` column would collapse
the cardinality axis into the content, break the scalar storage mapping,
and re-create the list-column problem the two-axis design avoids.  When
several values share a (key, variant) cell, the language's answer is
`group_map` aggregation, chosen explicitly.

## Consequences

Positive:

- `pivot` and `unpivot` are mutually inverse on cleanly characterized
  domains, in values always and in types for uniform-totality folds, with
  no `assume`, no completeness discharge, and no saturation side
  condition anywhere in the pair.
- One `pivot` form and **one new fact**, `exhaustive`, index-scoped and
  decidable; `functional` and `saturated` from the bag draft disappear
  (the key carries uniqueness), and `complete_over` stays exactly what
  ADR 0017 already consumes.  What remains is the propagation table,
  shared by both row-presence facts.
- The long form is the honest finer unit (the Enrollment key), its value
  column is always total, and sparse stored data pivots directly with
  honest `real?` columns.
- The surface shrinks: `unpivot name value`, two identifiers, no list.
- The proven `pivot_unpivot` is the exact template for one direction; the
  other direction gains an unconditional statement (modulo the total
  value column) that reify semantics could not have.

Negative:

- `pivot` drops lineage; reshape round trips are not usable inside split
  pipelines without losing disjointness facts.  Real and proven
  (`pivot_not_splitInvariant`); possibly softened later by axis-aware
  lineage (Open questions).
- Amends the freshly ratified ADR 0016 twice over (list-free fold, drop
  semantics) and the frozen effects of `09-typing-reference.md` section
  6.6.  Accepted: pre-1.0, and the M0 freeze anticipated reconciliation.
- Heterogeneous wide tables must project before folding; the fold is
  all-or-nothing by design.
- The M2 runtime's reshape operations (which implement ADR 0016: list
  form, reify semantics, an attribute pivot) and the index-form totality
  rule need rework at realization; the formal `unpivot` needs its drop
  variant and restated laws.

Neutral:

- Stores of long data are unaffected: they already key their axis (ADR
  0001).  `shrink_key` and ADR 0017 are untouched.

## Alternatives considered

1. **The bag pair** (the previous draft of this ADR): `unpivot` keeps the
   key and emits name/value as attributes; `pivot` is the split-safe
   `pivotAttr` form, so the round trip preserves lineage and composes
   under `split`.  Rejected after discussion: it needs *two*
   attribute-parameterized facts (`functional`, `saturated`) where the
   index form needs *one* index-scoped fact, because the key discipline
   already provides uniqueness; and with drop semantics the index pair's
   inverse domain is at least as large.  The split-safety loss is priced
   as lineage-drop, and the bag form remains reachable by composition
   (`shrink_key`).  A fused attribute-position pivot recovering
   `pivotAttr_splitSafe` may return later.
2. **Reify semantics** (ADR 0016 and the current formal `unpivot`):
   missing cells become rows with missing values.  Rejected: it breaks
   `unpivot (pivot L) = L` on sparse tables (fabricated rows force a
   saturation side condition) and lets missing values into the long
   form's value column.
3. **The explicit column list** (`unpivot name value (cols)`, ADR 0016).
   Rejected in favor of the total fold: the pair's domain becomes
   canonical (wide tables over one homogeneous value domain), the laws
   lose a parameter, and exclusion is ordinary upstream projection.  The
   explicit name/value identifiers, which ADR 0016's alternative 1
   rightly demanded, are kept.
4. **A single `complete_over` serving both readings** (an intermediate
   draft of this ADR, in two flavors: defining the asserted fact as the
   rectangle, or letting the population fact feed the totality upgrade).
   Rejected as unsound in the second flavor and mislabeled in the first:
   population-relative completeness holds for faithfully recorded sparse
   data, and consuming it for `pivot`'s totality upgrade would grant
   total spread columns whose cells are missing, while redefining it as
   the rectangle silently changes what `shrink_key`'s obligation means.
   The type-domain and sampling-frame references are different facts and
   stay named apart.
5. **`T*` list cells** for multiple values per (key, variant).  Rejected;
   section 3.
6. **A set-valued `exhaustive`** (recording per variant which folded
   columns were total) would make mixed-totality folds round-trip their
   types exactly.  Not adopted now: the all-or-nothing fact plus the
   propagation table is the whole design, and the refinement can be added
   compatibly if a use case demands it (Open questions).

## Open questions

- **The tag's typing surface.**  The synthesized enum versus referencing
  a declared enum (`unpivot subject:Subject score`): the declared form
  round-trips nominally with stores that declare the enum, but folds
  become sensitive to variant renames.  Its own ADR when reshape meets
  nominal enums.
- **A per-variant refinement of `exhaustive`.**  A set-valued form would
  make mixed-totality folds type-exact through the round trip; deferred
  until a use case demands it.
- **Store-level declaration.**  When a unit's index field is an enum, a
  store could declare the axis exhaustive (rectangular by policy),
  letting stored long data pivot to total columns.
- **Axis-aware lineage.**  `pivot` could preserve lineage when every
  upstream split predicate provably ignores the spread axis; whether the
  lineage hierarchy should carry per-axis information is a later round.
- **All-optional folds.**  `pivot (unpivot W) = W` needs one total folded
  column; whether `unpivot` should warn or reject when every attribute is
  optional, rather than silently shrinking the key set.
- **Recognizing the desugared form.**  Whether the checker should detect
  the map-plus-`extend_key` idiom and grant the `unpivot` facts to it,
  making the primitive pure sugar with no privileged status.  Doable but
  brittle; recorded as a question, not a commitment.
- **Formal work items.**  The drop-variant `unpivot`, both round-trip
  laws with the domains stated above (mechanized as `unpivotDrop`,
  `pivot_unpivotDrop`, `unpivotDrop_pivot` in `formal/Mensura/Table.lean`;
  the side condition is the chapter's `Minimal`), and the propagation
  lemmas for the row-presence facts (one per row of the table in
  section 2).
