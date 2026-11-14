# 0048: Reach, and lookups against a table's own statistics

## Status

Proposed.  The second of two ADRs that prepare the language for leak
detection across a `split` (ROADMAP M6, taken ahead of order); it
relies on `docs/decisions/0047-destructuring-a-split-pair.md` to make a
split's sides usable.  Adds a qualifier to `Qs`
(`docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`)
and extends the account of
`docs/language/08-lineage.md`, whose disjointness fact it complements
rather than changes.  Backed by `formal/Mensura/SelfLookup.lean`,
`formal/Mensura/Completeness/BinaryJoins.lean`, and
`formal/Mensura/Leak.lean`; the obligations still open are listed under
"Formal status".

## Context

Mensura's account of leakage is disjointness.  `split` establishes that
its two sides share no key, Tier A operations carry the fact, `demote`
and `pivot` drop it, and the learning operations of M6 will demand it
(`08-lineage.md`, "Demanding disjointness").  That catches a key that
lands on both sides.  It does not catch the most common leak in
practice, in which no key is shared at all.  Take a store `patients`
keyed by `(cohort, id)`, with `id` an `int`, and an `age`:

```mensura
view cohort_mean {
  patients |> demote id
           |> assume { complete }
           |> map_bags |k, b| (.mean_age = bag.sum b.age / to_real (#b.age))
}

view features {
  let (train, test) = patients |> lookup cohort_mean (|k, r| k.cohort)
                               |> split |k| k.id < 500;
  train
}
```

`train` and `test` are disjoint: no patient is on both sides.  Yet every
training row carries a mean that was computed from test patients' ages.
This is the "join back a whole-table statistic" leak, and mean
imputation is its everyday form.  `formal/Mensura/Leak.lean` proves it
is not split-invariant (`joinMean_not_splitInvariant`,
`imputeMean_not_splitInvariant`), although `lookup` against a fixed
right table is split-safe (`lookup_splitSafe`).  The failure comes from
the right table, which here is not fixed: it is computed from the same
rows the lookup is applied to.

The checker cannot see this, for two reasons.  `op_join` gives its
result the left table's lineage and ignores the right's.  And lineage
records which splits a table's *rows* passed through, not which rows its
*values* were computed from; it is also dropped at `demote`, which is
exactly how a statistic like `cohort_mean` is built.

The same pipeline is not always a leak.  Split by cohort instead
(`split |k| k.cohort == "north"`) and each side's means come only from
its own cohorts: running the pipeline on the whole and then splitting
gives each side exactly what running it on that side alone would give.
`formal/Mensura/SelfLookup.lean` proves this in general
(`lookup_self_split`, `lookup_demote_aggregate_split`): a lookup
against a statistic built from its own input by `demote` to the lookup
key and a per-key reduction is leak-free on every split whose predicate
reads only the lookup key (an *aligned* split, `Aligned` in
`BinaryJoins.lean`), and `joinMean_not_splitInvariant` is the same
composite on a split that is not aligned.  So the rule can be exact
rather than a blanket ban on self-derived lookups.

## Decision

### 1.  A new fact: reach

Each table type carries a **reach**: what its values were computed
from, at the granularity of splits.  It has three parts.

- **Roots**: the stores and registries the table reads, transitively,
  including through the right side of every lookup.
- **Sides touched**: for each split `S` whose input shares a root with
  the table, the set of `S`'s sides (`{left}`, `{right}`, or both) whose
  rows the table's values depend on.  A table is **confined** to a side
  of `S` when that set is that single side.  A split absent from the map
  is one whose rows the table does not depend on at all.
- **Cross-row groups**: one entry per lookup whose right table shares a
  root with its left table.  Each entry records the key fields of the
  left table along which the right table's values were gathered, or is
  *opaque* when the checker cannot establish such fields.

A table usable as a lookup's right side may also carry a **gather
grade**: it was built from its roots by `demote` to a set of remaining
key fields, followed only by operations that keep a gather
(decision 3).

Reach is a qualifier like the others (ADR 0013): it is computed by the
checker, carried across a view boundary by identity (ADR 0042
decision 1), and shown on hover.  It changes no runtime behavior.

**Disjointness and confinement are different facts.**  Disjointness says
two tables share no key.  Confinement says one table's values depend on
no row from the other side of a split.  The leak above is disjoint and
not confined.

### 2.  Where reach is established

- A **store or registry** has itself as its only root, touches no split,
  and has no groups.
- A **`split`** records, under its stage's source position (the same
  identity lineage uses), its input's roots and the key fields its
  predicate reads.  Each side touches only itself at this split,
  provided every cross-row group of the input is aligned with the split
  (its fields include every key field the predicate reads; an opaque
  group is aligned only with a predicate that reads no key field).
  Otherwise both sides touch both sides of the split: the rows are
  separated, but their values are not.  Sides touched at earlier splits
  are carried unchanged.

### 3.  How reach propagates

- **Unary operations** (`flat_map`, `map_bags`, `promote`, `demote`,
  `unpivot`, `pivot`, `window`, `closed`, `dense`, `assume`,
  `completeness_check`) carry the roots, the sides touched, and the
  groups.  A table's values never depend on rows its input did not
  depend on, so unlike lineage, reach survives `demote`; that is what
  makes a statistic fitted on `train` confined to `train`.
- **`union`** unions the roots and the groups, and takes the union of
  the sides touched at each split.  Recombining a split's two sides
  therefore touches both, as it should.
- **`lookup` and `lookup_total`** union the roots, the groups, and the
  sides touched of both inputs, with two adjustments:
  - If the right table shares a root with the left, a cross-row group
    is added.  Its fields are the lookup key's field when the key
    lambda is a single projection `k.f` of a left key field and the
    right table carries a gather grade over that field from those
    roots; otherwise the group is opaque.
  - For each split `S` at which the left table is confined to a side,
    and which the right table does not mention although it shares a
    root with `S`'s input, the right table was computed upstream of `S`
    and so depends on both sides.  The result keeps the left table's
    confinement only if the right table is gathered along `k.f` and
    `S`'s predicate reads only `f`; otherwise the result touches both
    sides of `S`.  This is the "fitted on the whole, applied to one
    side" leak, and the aligned case is the same theorem read from the
    other end.
- **Gather grade.**  `demote` of a table with no groups and no gather
  grade establishes a gather over its remaining key fields from its
  roots.  `flat_map` and `map_bags` keep it (they are strict per-key
  maps, `CommutesAligned.fiberMap`).  Every other operation clears it.

A right table that shares no root with the left adds no group and
touches nothing: it is a fixed right table, and `lookup_splitSafe`
applies as before.

### 4.  Who demands confinement

The demand belongs to the learning operations of M6, whose typing this
ADR does not settle.  It fixes the fact they will consume: fitting on a
table `A` and evaluating on a table `B` requires a split `S` at which
`A` is confined to one side and `B`'s rows lie on the other (the
existing lineage disjointness).  The demand is deliberately asymmetric.
`B` may touch `A`'s side, because applying statistics fitted on the
training side to the evaluation side is exactly what a fitted model
does; it is `A` touching `B`'s side that leaks.

Until those operations exist, the fact is observable on hover and in
tests, and nothing is rejected.  In particular the program of the
Context checks today and will keep checking: building and splitting a
contaminated table is not an error, using its training side to fit
against its evaluation side is.

### 5.  The escape is `assume { confined }`

As with completeness and arrangement, a fact the checker cannot derive
can be claimed.  `assume { confined }` immediately after a `split`
asserts that each side touches only itself at that split.  It is
visible, local, and carried to readers like every other `assume`
(ADR 0042 decision 1).

## Formal status

Proved, with no `sorry` and only the standard axioms:

- `lookup_self_split`, `lookupTotal_self_split`, `selfJoin_split`: the
  general aligned-split theorem for jointly strict binary fibre maps
  (decision 2's confinement at a split).
- `gatherMap_commutesAligned`, `demote_then_aggregate_commutesAligned`,
  `CommutesAligned.fiberMap`: the gather grade (decision 3).
- `lookup_demote_aggregate_split`: the headline instance.
- `joinMean_not_splitInvariant`, `imputeMean_not_splitInvariant`: the
  unaligned case leaks.
- `lookup_alignedDiagonal`, `lookupTotal_alignedDiagonal`, and
  `alignedDiagonal_biKeyLocal_iff_biFiberMap`: binary split-invariance,
  settling the book's open binary-join question.

Open, to be proved before the corresponding rule is implemented
(otherwise the rule stays conservative):

- **A filtered left side.**  The self-join theorem with the left
  argument a strict key-local map of the same table, `lookup key (g T)
  (f T)`, so that rows filtered or extended before the lookup keep the
  aligned case.
- **Row-wise stages before the `demote`.**  The gather grade surviving a
  `flat_map` before the `demote`.  This is a `gatherMap` with a composed
  combiner, so it should follow from `gatherMap_commutesAligned`.
- **The upstream-statistic rule.**  Decision 3's second bullet: a fixed
  right table computed from the whole, looked up into one side of an
  aligned split, equals the same lookup with the statistic computed on
  that side.  A corollary of `lookup_self_split` and the split-safety of
  the fixed-right lookup.
- **Soundness of reach as a whole.**  That a table confined to a side of
  `S` is a function of the rows on that side, for every pipeline the
  operator set can build.  This needs pipelines as syntax in Lean, which
  the development does not have yet; until then each propagation rule
  above is backed per operator.

## Consequences

- The leak is caught where it does harm (at the M6 demand), with a
  diagnostic that can name the lookup and the split responsible, and
  the honest variants (a group-aligned split, a statistic fitted on the
  training side) are accepted without annotation.
- `Qualifiers` gains a `reach` field; `op_split` records its predicate's
  key reads; `op_join` inspects the right table's reach and recognizes
  a single-projection key lambda.  The runtime is unchanged.
- `08-lineage.md` gains a section on confinement beside disjointness
  when this is implemented, and its "Demanding disjointness" section is
  revised to the asymmetric demand of decision 4.

## Alternatives considered

- **Reject the unaligned split itself.**  Simpler, and what the first
  analysis proposed, but wrong: a contaminated table is fine to build,
  and recombining its sides is the identity.  Only fitting on one side
  and evaluating on the other is a leak, so the error belongs at the
  demand.
- **Extend lineage instead of adding a fact.**  Lineage answers "can
  these two tables share a key" and is correctly dropped at `demote`;
  reach answers "can this table's values depend on that side's rows"
  and must survive `demote`.  Merging them would weaken one or make the
  other unsound.
- **Ban self-derived lookups.**  Rejects group-aligned splits and
  per-group features, which are the honest and common case, and which
  the theorems show are safe.

## Open questions

- **Splits that cut an entity's cells.**  `unpivot` turns one row into
  several keys, and a split reading the name axis then sends one
  entity's cells to both sides.  That is a group leak rather than a
  value leak, and neither lineage nor reach catches it; M6's grouped
  validation is the place to settle it.
- **Field identity across key moves.**  Alignment compares key fields by
  name.  A key move between the lookup and the split that removes a
  group's field makes every later split unaligned, which is sound but
  may be stricter than necessary.
- **Statistics with more structure** than a demote and a reduction (a
  window over the whole history, a second lookup inside the statistic)
  get an opaque group today.  Each such shape would need its own
  commuting theorem.
