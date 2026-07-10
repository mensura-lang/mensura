# 0023: Completeness is consumed by the reducer, not by shrink_key

## Status

Proposed.  Under discussion in
[issue #38](https://github.com/mensura-lang/mensura/issues/38) (Track 1).
Amends the consumer placement of
`docs/decisions/0017-completeness-establish-consume.md`, which names
`shrink_key` (and index `pivot`, already dissolved by
`docs/decisions/0020-reshape-as-a-true-inverse-pair.md`) as the completeness
consumer.  Independent of, but reinforced by,
`docs/decisions/0022-observations-as-bags-declared-store-cardinality.md`: 0022
makes the completeness fact natural to establish at the source; this ADR
fixes where it is consumed.  Nothing here changes `pipe_check` or the language
docs until it is accepted and the backing lemma lands in `formal/` (repo rule:
propagation rules are backed by proofs or stay conservative,
`docs/decisions/0021-formal-proof-pipeline.md`).

## Context

`0017` ratified the M1 surface for completeness: `completeness_check { assert
... }` and `assume { complete }` **establish** the fact, and a Tier B operation
**consumes** it.  The only surviving Tier B consumer is `shrink_key` (`pivot`
lost its obligation in `0020`).  The checker therefore demands completeness
*at* `shrink_key`, as in

```
readings                                  // keyed by (machine, ts)
|> assume { complete }
|> shrink_key ts
|> group_map |k, g| (.max_kelvin = max g.kelvin)
```

Pushing on this while writing the book's aggregation example (issue #38)
showed the demand is attached to the wrong operation.  Three distinct
obligations are fused at `shrink_key`, and only one belongs there.

- **Disjointness break (lineage).**  Coarsening the key merges entities: two
  tables disjoint at `(machine, ts)` (row `(m, 1)` in one, `(m, 2)` in the
  other) overlap at `machine`.  This is `project_not_preservesDisjoint`, it
  fires *at* `shrink_key`, and completeness neither causes nor repairs it.
  This is the true reason `shrink_key` is Tier B, and it is correctly placed.
- **Split-safety of the composite.**  `shrink_key` alone distributes over
  `bind` (`shrink(A) ++ shrink(B) = shrink(A ++ B)`), so in isolation it is
  split-invariant.  A **reducing** `group_map` distributes over `bind` only
  when the two sides are key-disjoint, which is exactly why `group_map` is
  Tier A given that `split` routes whole keys.  The composite
  `shrink_key |> group_map` breaks only because the split happened at the old
  key and can cut an entity at the new key; what repairs that is "the split
  respects the retained key," a lineage property, not a no-missing-rows one.
- **Aggregate faithfulness (completeness).**  `max` / `sum` over an entity's
  bag equals the true value only if none of the entity's rows are missing.
  This bites at the **reduction**, not at the coarsening.

Evidence the demand is misplaced:

- `readings |> shrink_key ts` with **no** reducer produces a faithful, possibly
  partial `bag` over `machine`.  Nothing about it is unsound; a bag is an
  honest representation of the rows present.  Demanding completeness to admit
  it (and the diagnostic points at the `shrink_key` line) over-constrains.
- On a `card <= 1` store, completeness over the full key is **vacuous** (a
  singleton group is trivially whole), so the fact `assume { complete }`
  supplies at `shrink_key` is either content-free or is silently a claim about
  a coarser key that the store never established.
- The overview's own account of a tracked fact (it *enters* by mechanism, is
  *transformed* by each operation, and is *demanded* where unsoundness would
  hide) fits **establish-at-source, propagate-through-`shrink_key`,
  consume-at-the-reducer** far better than a fact that materialises at
  `shrink_key` out of a bare `assume`.

## Decision

Move the completeness obligation from the coarsening to the reduction.

- **`shrink_key` no longer demands completeness.**  It keeps only its lineage
  effect: it is Tier B because it breaks disjointness
  (`project_not_preservesDisjoint`) and drops the lineage fact.  A `shrink_key`
  with no downstream reducer is admitted on its own and yields a `bag` over the
  coarser key.
- **`shrink_key` propagates completeness from the fine key to the coarse key.**
  If the input is complete over its key `(a, b)` in the non-trivial sense (no
  row that should exist is absent), then after `shrink_key b` the result is
  complete over `a`: no `a`-group is missing rows, because none of the
  underlying `(a, b)` rows were missing.  `shrink_key` transforms the fact, it
  does not consume or invent it.
- **A reducing `group_map` consumes completeness.**  A `group_map` whose body
  folds a bag to a single record (`sum`, `max`, `count`, `mean`, ...) requires
  the fact "complete over the current key," because a fold over a partial bag
  is silently wrong.  A **windowing** `group_map` (a bag return, one row per
  input row: `rank`, `cumsum`) does **not** reduce a population to a claim and
  carries no completeness obligation.
- **Establishment is unchanged.**  `completeness_check`, `assume { complete }`,
  a source annotation, and (`0022`) a `bag` store's source-level fact all still
  establish completeness.  Only the consumer moves.

Consequence for the running example: the discharge sits before the reducer, and
`shrink_key` is a pure reindex between them.

```
readings
|> shrink_key ts                          // reindex to a bag over machine; propagates completeness
|> assume { complete }                     // establish for the reducer (or annotate the source)
|> group_map |k, g| (.max_kelvin = max g.kelvin)   // reducer consumes it
```

The `assume` may equally sit before `shrink_key` (the fact then propagates
through it); what changes is that the *demand* is the reducer's, so a
`shrink_key` without a reducer needs no discharge at all.

## What this needs from `formal/`

This is a propagation-rule change and does not land until proven:

- **`shrink_key` completeness propagation.**  A lemma: input complete over
  `(a, b)` implies output complete over `a` after `shrink_key b`.
- **Reducer obligation.**  The reducing/​windowing distinction for `group_map`
  made precise (it already exists as "single-record return vs bag return",
  `fiberMap` in `formal/`), with completeness required exactly for the
  reducing case.
- **Non-regression of `shrink_key`'s Tier.**  Confirm `shrink_key` stays Tier B
  purely on the lineage break (`project_not_preservesDisjoint` is unaffected).

## Consequences

Positive:

- The obligation lands where the unsoundness is (a fold over a partial bag),
  so `shrink_key` used purely to reindex is no longer gratuitously rejected.
- Provenance matches the "enters, transforms, is demanded" model: source
  establishes, `shrink_key` propagates, reducer consumes.
- Composes cleanly with `0022`: a `bag` store establishes completeness at the
  source and a reducing `group_map` consumes it with no `shrink_key` in
  between.

Negative:

- Amends a ratified ADR (`0017`) and moves a checker obligation, so
  `docs/language/09-typing-reference.md` (sections 6.3, 6.6, 8) and the
  `pipe_check` rules must be re-read and re-tested together.
- Requires the reducing/​windowing distinction to be a first-class, checkable
  property of a `group_map` body, where today the completeness rule did not
  depend on it.

Neutral:

- No surface change: the same `completeness_check` / `assume` / annotation
  establish the fact; only the operation that *reports the error when it is
  missing* changes.
- `pivot` is unaffected (it already carries no completeness obligation, `0020`).

## Alternatives considered

1. **Keep the demand at `shrink_key` (status quo, `0017`).**  Simplest: one
   Tier B operation carries one obligation.  Rejected because it rejects
   sound reindex-only pipelines, and because on a `card <= 1` store the fact it
   demands is vacuous, so the check tests nothing.

2. **Demand completeness at *every* `group_map`.**  Uniform, no
   reducing/​windowing distinction.  Rejected: it over-demands on windows and
   on bags that are complete by construction (a `map` expansion, a `collect`
   census), forcing noise `assume`s.

3. **Attach completeness to `bind` / the split boundary instead.**  Model the
   fact purely as a lineage/partition property.  Rejected: it conflates
   aggregate faithfulness (a within-table fold concern) with disjointness (a
   between-table concern), the very conflation this ADR is untangling.

## Open questions

- **Exact witness carried by `shrink_key`.**  Whether propagation needs the
  input completeness to name the fine key explicitly, or whether "complete over
  the key" tracked table-wide suffices once `assume` gains a key argument
  (today `assume` takes only the bare `complete`, issue #38).
- **Interaction with a bare `assume { complete }`.**  With the demand at the
  reducer, does `assume { complete }` before a `shrink_key` still read as
  "complete over the pre-shrink key" and propagate, or should the surface let
  the author name the key the reducer will need?
