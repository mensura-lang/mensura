# 0035: Completeness does not survive a genuine `demote`

## Status

Accepted.  Amends the propagation rule of
`docs/decisions/0023-completeness-consumed-by-the-reducer.md` (`demote`
no longer carries the completeness fact to a coarser key) and the
propagation sentence of
`docs/decisions/0033-registry-declarations.md` decision 2 (a registry's
by-mechanism fact holds at its own declared key only).  The consumer
placement 0023 fixed (the reducing `map_bags`) and the establishment
surface of `docs/decisions/0017-completeness-establish-consume.md`
(`completeness_check`, `assume { complete }`, the registry mechanism)
are unchanged.  The re-derivation rule leans on the ADR 0024 gradings so
`promote`/`demote` remain a true inverse pair.

## Context

The checker tracks completeness as a single two-valued qualifier
(`Completeness::{Complete, Incomplete}`,
`crates/mensura-types/src/table.rs`).  A `registry` source enters
`Complete` by mechanism (ADR 0033), `demote` carried the flag through
unchanged, and the reducing `map_bags` consumes it.  So

```
readings |> demote taken_at
         |> map_bags |k, b| (.max_temperature = bag.max b.temperature)
```

compiled with no establishment step anywhere, on the strength of
`Mensura.demote_completeWrt`.

That lemma is true and it is the wrong justification.  It is
*reference-relative*: if `T` is complete with respect to a reference
`R` at the fine key, then `demote T` is complete with respect to
`demote R` at the coarse key.  The reference coarsens **with** the
table.  The fact a reducing fold needs is different: fiber-level
completeness against a *fixed* intended population
(`Mensura.FiberCompleteWrt`), "every group the fold emits is whole".
The single tracked bit cannot say which reference it is relative to, so
carrying it across a `demote` silently strengthens a claim about the
fine key into a claim about the coarse one.

The gap is concrete.  A `singletons` registry of

```
unit Reading { machine: string  ts: int }
```

is complete in its own key: a key names at most one reading, so every
*present* key carries its whole fiber, trivially
(`fiberCompleteWrt_of_functional`).  What the mechanism does not and
cannot give is key coverage of the world: an unrecorded `(machine, ts)`
is an **absent key**, invisible to the fiber-level fact.  `demote ts`
merges each machine's fibers, and the absence becomes a **gap inside
the machine's bag**.  The formal development already says exactly this
(`formal/Mensura/Completeness/CompleteOver.lean`, doc comment on
`fiberCompleteWrt_of_functional`): "whole keys may still be absent from
`T`, and coarsening converts exactly that absence into a fiber gap".
Sole-intake and append-only guarantee "everything recorded is here",
not "everything that happened was recorded", so `bag.max` over the
demoted bag is the maximum of a sample, and the checker blessed it as
the maximum of the population.  That is precisely the silent wrongness
the reducer's obligation exists to surface.

## Decision

1. **The tracked fact is fiber-level completeness at the current key.**
   The `Complete` qualifier means: against the fixed intended
   population, every fiber of the *current* key that is present is
   whole (`FiberCompleteWrt`, not `CompleteWrt` against a co-coarsening
   reference).  This is exactly the fact the reducing `map_bags`
   consumes, so the bit now means what its consumer needs.

2. **A key move re-derives completeness from the graded cardinality.**
   The ADR 0024 gradings already survive both key moves and re-derive
   the cardinality bound.  Completeness rides the same machinery:

   - result graded **`singletons`**: the table is `Complete`.  A
     present singleton fiber is its whole fiber; this is
     `fiberCompleteWrt_of_functional` applied at the post-move key, the
     same proved base case the reducer's trivial discharge rests on.
   - **`demote`** to a genuine **`bag`** coarsening: the fact is
     **cleared**.  Coarsening merges fibers, and an absent fine key
     becomes a gap in a coarse fiber; the counterexample is recorded in
     `CompleteOver.lean` (see "Formal status" below).
   - **`promote`** that still yields a **`bag`**: the fact is
     **preserved**.  Refining the key partitions each fiber by row
     content, and a whole fiber partitions into whole sub-fibers.

   The source lift applies the same corollary: any table declared
   `singletons` enters `Complete`, store or registry alike, so the
   qualifier never disagrees with what a key move would re-derive one
   stage later (and the reducer's trivial discharge already treated
   every `singletons` input as fiber-complete).  The two declaration
   kinds therefore differ exactly at `bag`, where the registry
   mechanism is the only source-level establishment.

3. **Establishment is unchanged, but its placement now matters.**
   `completeness_check { assert ... }` and `assume { complete }`
   establish the fact at the current key, and the fact does not survive
   a subsequent coarsening.  A reduction over a demoted bag therefore
   discharges its obligation **after** the last `demote`:

   ```
   readings
   |> demote ts                 // coarsen; the fine-key fact is forfeited
   |> assume { complete }        // establish at the key the reducer folds
   |> map_bags |k, g| (.max_kelvin = bag.max g.kelvin)
   ```

   The 0023 remark that "the `assume` may equally sit before
   `shrink_key`" is revoked: before the coarsening it claims the wrong
   key.

4. **A registry is complete at its own declared key, and nowhere
   coarser.**  ADR 0033 decision 2's uniform rule stands (`Complete` at
   either cardinality, one rule per keyword); its propagation sentence
   is revoked.  The surviving contentful payoff is unchanged: an
   `attr*` registry reduced at its own entity key still needs no
   ceremony, because the declaration is where the per-entity reference
   population is pinned.  A `singletons` registry's flag is the trivial
   reading and dies at the first genuine coarsening, by design: the
   mechanism never claimed key coverage of the world.

5. **The key moves remain a true inverse pair.**  Because rule 2 is
   stated in terms of the graded cardinality rather than as an
   unconditional clear, `promote x |> demote x` and
   `demote x |> promote x` re-derive `singletons` from the grading and
   with it `Complete`, so the whole qualifier vector is restored in
   either order.  Nothing about the grading bookkeeping itself changes.

## Formal status

Under the repo rule (`docs/decisions/0021-formal-proof-pipeline.md`:
propagation rules are backed by proofs or stay conservative), the pieces
of decision 2 stand as follows:

- **Clearing at a coarsening `demote` is conservative** and needs no
  lemma; the fiber-gap counterexample that motivates it (population with
  two fine keys, table holding one, whole at its own key, short after
  the merge) is recorded prose-first in `CompleteOver.lean`, and its
  mechanization as `demote_not_fiberCompleteWrt` is this slice's open
  formal item.
- **The `singletons` re-derivation is proof-backed**: it is
  `fiberCompleteWrt_of_functional` applied at the post-move key, the
  lemma the reducer's trivial discharge already rests on.
- **Preservation at a `bag`-result `promote`** is inherited from the
  pre-0035 checker unproven (the partition-of-whole-fibers argument
  above); its lemma joins the open formal item.

## Consequences

Positive:

- The obligation lands where the unsoundness is, again: a fold over a
  demoted bag now demands a discharge at the key it actually folds,
  and the discharge step names the claim being made about the world.
- The bit's meaning, its consumer, and its formal backing agree:
  `FiberCompleteWrt` at the current key, consumed by the reducing
  `map_bags`, established at that key or trivially re-derived at a
  graded `singletons`.
- The inverse-pair property of ADR 0024 extends to the completeness
  qualifier, carried by the same gradings that carry cardinality.

Negative:

- `docs/examples/fleet-monitoring.mensura` regains the
  `assume { complete }` that ADR 0033 removed, now placed after the
  `demote` and stating the worldly claim (no reading a machine produced
  went unrecorded).  The M4 slice's headline shrinks accordingly: the
  ceremony-free case is the `attr*` registry at its own key, not the
  demoted `singletons` one.
- A `singletons` registry's static payoff at coarser keys is gone, and
  so is its payoff at the source: a `singletons` store now enters
  `Complete` on the same trivial corollary, so the keyword's type-level
  content lives entirely at `bag`.  What `registry` still buys at
  `singletons` is the intake discipline (append-only, sole intake).

Neutral:

- `Mensura.demote_completeWrt` remains true and proved; it is simply
  not the fact the checker's bit tracks.  It stays as the
  reference-relative statement ADR 0023 drafted it for.
- Row-wise operations are untouched: they map whole fibers to whole
  fibers, so preservation stays sound under the fiber reading.

## Alternatives considered

1. **Keep the propagation (status quo, ADR 0033).**  Sound only under
   the reading where the registry is its own reference population, but
   under that reading the fact is reflexive (`completeWrt_refl`) and
   the reducer's obligation is vacuous for any self-referenced source:
   the fold is "faithful to whatever was recorded", which no one
   doubted and which is not what the obligation exists to check.
   Rejected.

2. **Clear unconditionally at `demote`.**  Simpler than rule 2, but an
   exact ADR 0024 round trip would lose the fact even though the
   grading proves the result `singletons`, breaking the inverse pair at
   the qualifier level and demanding a bogus `assume` after a content
   identity.  Rejected in favour of re-derivation from the grading.

3. **A key-carrying completeness fact** (`complete_over(k)` tracked
   with its key), letting a fact survive any move to a key at or finer
   than the one it was established over.  The honest general fix, and
   both 0023 and 0033 already defer it.  It needs `assume` to grow a
   key argument and the qualifier to carry column sets, for no consumer
   the current surface cannot serve with placement.  Stays deferred.

## The other qualifiers were audited against the same failure mode

The defect above has a shape: a fact **about the current key**, carried
across an operation that changes what the key means, so a claim about
the fine key is silently read as a claim about the coarse one.  Every
other tracked qualifier was checked against it.  All three are sound,
and for three different reasons, which is worth recording because the
differences say what to watch when a fifth qualifier is added.

| qualifier | about the current key? | at a coarsening `demote` | claimable by fiat |
|---|---|---|---|
| completeness | yes | **was carried** | `assume { complete }` |
| `exhaustive` | yes | cleared | no |
| lineage | yes | dropped | no |
| `arranged` | **no** (a flat-table fact) | carried, soundly | `assume { arranged }` |

- **`exhaustive`** has the vulnerable shape (a set of key columns,
  defined over "every residual key present in the table") and is
  nonetheless safe: every key-changing operation clears it.  The
  `promote` case was caught during ADR 0020's implementation and
  overridden there, against that ADR's own "preserved" sketch, on
  precisely this reasoning: the promoted column refines the residual
  key, which can cut a fiber.  `demote` clears it conservatively
  pending mechanization, `split` destroys it on both sides, and `union`
  intersects.  There is also no claim form, so a stale fact cannot be
  injected by hand.
- **Lineage** drops at `demote` and key `pivot`.  That is not a
  precaution but the definition of Tier B
  (`demote_not_preservesDisjoint`, `pivot_not_splitInvariant`):
  forfeiting disjointness at a coarsening is what the Tier names.
  Worth knowing when reading a pipeline: `union` never *rejects* on a
  missing disjointness fact, it degrades the result to `bag`, so the
  failure surfaces at whatever downstream stage demands `singletons`
  rather than at the join itself.
- **`arranged`** is not key-relative at all; see the next section.

The pattern: completeness was the only qualifier that was key-relative
**and** hand-injectable **and** carried across the coarsening.  Remove
any one of the three and the bug cannot arise.  A new qualifier that has
all three needs the re-derivation treatment of decision 2 from the
start.

## `arranged` was audited and is not the same bug

The obvious follow-up to this ADR is whether `assume { arranged }`
(ADR 0029 Decision 11's tier 3) has the same defect, since it too is a
claim discharged from the ADR 0024 gradings and too survives a `demote`.
It does not, and the reason is worth recording so the next reader need
not re-derive it.

The two facts quantify over different things:

- **Completeness** is about *which rows exist*, relative to an intended
  population, at the **current key**.  A coarsening merges fibers and
  turns an absent fine key into a gap inside a coarse one, so the
  fine-key claim is strictly weaker than the coarse-key claim it was
  being read as.  Hence the clearing rule above.
- **Tie-freedom** is a **functional dependency of the flat table**: no
  two rows share the order key.  It is not indexed by the current key at
  all, so a key move re-reads the same property rather than
  strengthening it.  `formal/Mensura/Arranged.lean` states this
  directly: a grading is "carried unchanged through `demote` because it
  is a fact about the flat table rather than about the current key",
  and `keyInjOn_demote_tag` proves the step the checker relies on (two
  rows of one output fiber agreeing on the tag would have been two rows
  of one *input* key, which `Functional` forbids).

So `assume { arranged }` before a `demote` claims exactly what survives
it, while `assume { complete }` before a `demote` claims something the
coarsening destroys.  Two smaller differences reinforce this.
Tie-freedom is checkable from the rows in hand (a duplicate key is
*present*), where completeness asserts something about rows that were
never recorded and so cannot be seen.  And a tie yields an undetermined
arrangement, which is a real bug but a visible one, not a number
misreported as authoritative.

## Open questions

- The key-carrying fact of alternative 3, if a consumer appears that
  needs a fact established coarser than it is consumed.
- **A `demote` along an `exhaustive` axis preserves completeness.**
  The clearing rule of decision 2 guards against exactly one thing:
  absent fine keys along the demoted axis, which the coarsening turns
  into gaps inside coarse bags.  ADR 0020's `exhaustive(A)` fact rules
  those absences out: every residual key present in the table carries
  its row for every variant of `A`.  So the two facts compose.  If a
  table is `Complete` at `(machine, sensor)` and `exhaustive(sensor)`,
  then `demote sensor` is `FiberCompleteWrt` at `(machine)`: the coarse
  bag at a present `machine` is the union of the fibers at
  `(machine, s)`, exhaustiveness makes every such key present (and the
  population's sensor values range over the same variant set, by
  typing), fiber-completeness makes every present fiber whole, and a
  union of whole fibers covering all the population's fibers is a
  whole bag.  A wholly absent `machine` stays an honest absence, which
  `FiberCompleteWrt` permits.  Multi-column demotes chain: if every
  demoted column is in the exhaustive set, the per-axis facts compose
  into the full rectangle.  The fiber-gap counterexample recorded in
  `CompleteOver.lean` fails the exhaustive hypothesis (one variant's
  key is absent), so the candidate lemma
  `demote_fiberCompleteWrt_of_exhaustive` should be mechanizable.

  Against alternative 3, this is the narrow, cheap fix rather than the
  honest general one: it applies only to enum-domained axes (the
  reference is the variant set, which is what keeps `exhaustive`
  extensional and decidable) and it rides machinery that already
  exists, since the qualifier is already a per-column set and the
  checker's `demote` already inspects the demoted columns.

  Recorded rather than adopted because the establishment surface is
  undecided.  Today only `unpivot` establishes `exhaustive`, there is
  no claim form, and the audit table above counts that absence as a
  safety property: a stale fact cannot be injected by hand.  Adopting
  the rule means choosing how a source or a pipeline stage comes to
  hold the fact honestly.  The candidates stay open on purpose: a
  registry-level axis declaration enforced at ingest (ADR 0020's
  store-level-declaration open question), an operation that fills the
  missing entries and thereby *makes* the axis exhaustive (a
  rectangularizing stage, establishing the fact the way
  `completeness_check` establishes completeness), or other means.
- Whether `union`'s rule (complete iff both inputs are) deserves the
  same scrutiny at overlapping lineages, where merging bags is not
  merging whole fibers.
- The two mechanizations of "Formal status": the
  `demote_not_fiberCompleteWrt` witness and the `bag`-result `promote`
  preservation lemma.
- **Whether the ordered operations need a completeness obligation of
  their own, distinct from tie-freedom.**  The section above settles
  that `arranged` is sound *as tie-freedom*, but tie-freedom is not the
  only thing a window silently assumes.  The `series` vocabulary is
  defined over the rows that are *present* in the fiber, so `lag` means
  "the previous row in this bag", not "the previous time step".  Over an
  order key reading `1, 2, 4, 5` the result is well-typed, deterministic
  (the key is tie-free), and still misleading: `lag` at `4` reports
  `2`'s value while the reader takes it for `3`'s, and a caller
  differencing consecutive readings computes a rate over the wrong
  interval.  `rank` counts present rows rather than positions, and
  `cumsum` totals a sample of the series.

  This is *not* the fact this ADR governs.  Fiber-level completeness
  says no row of a present group is missing, which for a genuinely
  gap-free intended population is what would rule the case out; but the
  window vocabulary makes the stronger *contiguity* reading tempting,
  and nothing in the type system distinguishes "every row that exists is
  here" from "the order key has no holes".  Densely-indexed series (a
  reading per minute) and irregular ones (an event log) are both legal
  and want different answers, which is why the obligation cannot simply
  be added.

  Recorded rather than decided, since a fix needs a notion the language
  does not have: either a *dense*/*regular* marker on an order key, a
  gap-aware reading of the window vocabulary (`lag` over a step rather
  than over a position), or an explicit resampling stage that
  establishes contiguity the way `completeness_check` establishes
  completeness.  M5's streaming work is where windows meet
  window-closedness and is the natural place to settle it.  Until then
  the risk is unflagged by the checker, and `13-registries.md` and
  `07-pipelines.md` should not imply that a discharged `arranged` makes
  a window faithful.

## Forward references

- `docs/language/09-typing-reference.md` sections 3.4, 6.3, 7, 8, 10
  (the re-derivation rule and the amended effect matrix).
- `docs/language/07-pipelines.md`, "Completeness: establish, clear,
  consume".
- `docs/decisions/0024-key-moves-as-a-true-inverse-pair.md` (the
  gradings the re-derivation rides on).
- `formal/Mensura/Completeness/CompleteOver.lean`
  (`FiberCompleteWrt`, `fiberCompleteWrt_of_functional`, the recorded
  fiber-gap counterexample, and the retained `demote_completeWrt`).
