# 0022: Observations as bags, declared store cardinality

## Status

Proposed.  Under discussion in
[issue #38](https://github.com/mensura-lang/mensura/issues/38).  Revisits the
`card <= 1` boundary rule of
`docs/decisions/0001-unit-as-identity-discipline.md` (specifically its
rejected alternative 3) and extends the store surface of
`docs/decisions/0002-stores-tabulate-units.md`.  Nothing here is ratified: no
grammar, checker, or storage change follows until this ADR is accepted.  It is
paired with, but independent of, the completeness-placement erratum tracked in
the same issue (that one fixes a propagation rule; this one changes what may
enter the system).

## Context

`0001` fixed cardinality at the unit boundary: for any unit and any tuple of
index values a tabulation has **at most one** observation, and cardinality
greater than 1 is ill-formed at any unit boundary (`store`, `collect`, a
signature promising a tabulation of a unit).  Multiplicity is modelled by
choosing a finer index, not by a per-unit knob; `0001` considered and rejected
per-unit cardinality declarations (alternative 3) precisely because a
relaxable 0-or-1 rule "would undermine every downstream invariant."

That discipline is right for **entities**: a `Person`, a `Course`, an
`Enrollment` is observed once or not at all, and a duplicate is a signal the
identity criterion is wrong.  It is a poor fit for **observations of an
entity that recur**: sensor readings from a machine, transactions of an
account, events in a session.  The current answer is "add the disambiguating
column to the index" (`Reading { machine, ts }`, keyed by the pair).  This
ADR argues that answer has two costs that were not fully weighed in `0001`,
and that the fix is a declared cardinality at the **store** boundary, which is
a `0002` concern and leaves `0001`'s unit-as-pure-identity intact.

Two costs of forcing recurring observations into a finer key.

**1. Completeness collapses onto identity and goes vacuous.**  A store keyed
by `(machine, ts)` is `card <= 1`, so completeness over that full key is
trivially satisfied: a singleton group is "whole" by construction.  The fact a
downstream rollup actually needs (every machine has all of its readings) is
about *whole rows absent*, which lives on a coarsening axis that the full key
does not expose.  So there is no meaningful place at the source to establish
the fact, and it has to be conjured mid-pipeline (see the completeness-placement
track in issue #38).

**2. Tracked disjointness drifts from the leakage boundary.**  This is the
serious one, because leak-free validation is the language's reason to exist.
With `(machine, ts)`,

```
let (train, test) = readings |> split |k| hash k < 0.8
```

hashes on the *pair*, so the same machine at different timestamps lands on both
sides.  The type system is satisfied: `train` and `test` are genuinely disjoint
**at `(machine, ts)`**.  But the split is scientifically leaky (a model trained
on machine `m` at `t1` is scored on machine `m` at `t2`), and the leak is
invisible because disjointness is tracked at the pair, not at the entity.  That
is a hole in exactly the guarantee Mensura advertises, and it is reachable with
the most natural split one can write.

The algebra already carries `card = many` (the Chapter 5 indexed table tracks
cardinality with a `many` case, and a `view` is `bag` or `singletons`,
`docs/decisions/0012-view-hosting.md`).  So `0001`'s `card <= 1` is a boundary
discipline layered on a model that admits bags everywhere else, not a
consequence of the model.

## Decision

Allow a store to declare its cardinality, defaulting to the current discipline.

- **Default `singletons`.**  A store with no cardinality declaration keeps the
  `0001` rule exactly: `card <= 1` over its key, accidental duplicates
  rejected.  Existing programs are unchanged.
- **Opt-in `bag`.**  A store may declare that it holds many observations per
  key.  Its key is then the **entity** the observations are about (the unit),
  and the store is a `card >= 1` tabulation: the identity criterion still says
  what a row is *about*, but no longer that a row is *unique*.
- **Cardinality is a store concern, not a unit concern.**  The unit stays pure
  identity (`0001` unchanged): it declares index fields and nothing else.  The
  *tabulation* declares how many observations it holds per identity, alongside
  the attribute and change-control concerns `0002` already places on the store.
  This is the distinction `0001` alternative 3 missed: the rejected knob was
  per-*unit* (which would make identity itself ambiguous); this knob is
  per-*store* (a tabulation choice), which is where `0002` already lives.

The exact surface (a keyword, an annotation, a `card` clause) is deferred to
the store language document and is not fixed here; this ADR fixes only that the
property exists, defaults to `singletons`, and lives on the store.

Consequences for the rest of the system, stated as obligations this ADR
imposes on later work rather than as settled mechanism.

- **Storage mapping.**  A `bag` store cannot use its index columns as a
  PRIMARY KEY (`docs/toolkit/00-storage-backend.md`).  It maps to a table with
  a surrogate row identifier and the index columns as an ordinary (non-unique)
  covering index.  Per-row addressability is lost for `bag` stores, by
  definition.
- **Boundary checks.**  The 0-or-1 boundary check of `0001` becomes
  conditional on the declared cardinality.  `collect` inherits the same
  declaration (it is the process-variant of a store,
  `docs/decisions/0006-transport-agnostic-surface.md`).
- **Completeness.**  On a `bag` store, "complete over the key" is contentful
  and establishable at the source (an annotation, or a `collect` mechanism),
  and is consumed by a reducing `group_map` without an intervening
  `shrink_key`.  This is the clean provenance the completeness-placement track
  wants; the two tracks reinforce each other but neither depends on the other.
- **Disjointness / splitting.**  An entity-keyed `bag` store makes `split`
  route whole entities, so tracked disjointness coincides with the leakage
  boundary for entity-level cross-validation.

## Consequences

Positive:

- Completeness becomes meaningful at the point data enters, matching the
  "facts derived from how data enters" pillar rather than being conjured
  mid-pipeline.
- Entity-keyed bags align tracked disjointness with the scientific leakage
  boundary, closing the "disjoint at the pair, leaky at the entity" hole for
  entity-level CV.
- Recurring observations (sensor streams, transactions, events), the driving
  IIoT application's core shape, are modelled directly instead of through an
  artificial composite key and a `shrink_key` round-trip.
- No change to `0001`'s unit-as-identity: units stay pure, and the knob lands
  where `0002` already puts tabulation concerns.

Negative:

- Revisits a foundational boundary rule; every downstream invariant that
  assumed `card <= 1` at a store must be re-read as "at a `singletons` store."
- `bag` stores forgo the clean "index columns are the primary key" storage
  story and per-row addressability.
- A relaxable rule is a rule people can misuse: an author who declares `bag`
  where a finer index was the honest model loses the accidental-duplicate
  check.  The default (`singletons`) and the opt-in keep this explicit, but the
  forcing function is weaker than a universal law.

Neutral:

- Does not remove `shrink_key`.  Genuine multi-level keys still coarsen, and
  temporal cross-validation wants time *in* the index, which reintroduces an
  `(entity, time)` key.  Entity-in-index and time-in-index modellings coexist,
  and the index is how the author declares the validation granularity.
- The completeness-placement erratum (issue #38, Track 1) is orthogonal: it
  must be fixed whether or not this ADR is accepted, because `shrink_key`
  survives.

## Alternatives considered

1. **Keep `0001` unchanged; model multiplicity only with a finer index.**  The
   status quo.  Rejected as the sole option because it induces the vacuous
   completeness and the pair-vs-entity leakage hole above; kept as the
   **default**, since it is right for entities.

2. **Per-unit cardinality declaration.**  `0001` alternative 3.  Still
   rejected: putting the knob on the unit makes identity itself ambiguous and
   undermines cross-store agreement about "what a Person is."  This ADR puts
   the knob on the store instead, which does not touch identity.

3. **A distinct `event` / `observation` declaration** separate from `store`.
   A first-class construct for identity-less recurring observations.  Deferred,
   not rejected: it may be the better long-run surface, but it is a larger
   language-surface commitment than a cardinality property on the existing
   store, and the property is needed either way.  If adopted later, `bag`
   stores are the mechanism it desugars to.

4. **Track disjointness at every coarsening of the key automatically**, so a
   pair-keyed split would also expose entity-level overlap.  This attacks the
   leakage hole without bag stores, but it multiplies the lineage state the
   checker must carry (a region per coarsening) and does not address the
   vacuous-completeness cost.  Out of scope here; noted for the lineage
   document.

## Open questions

- **Surface.**  Keyword vs annotation vs `card` clause, and how it reads next
  to `unit { U }` and the attribute blocks.  Belongs in the store language
  document once this ADR is accepted.
- **Ordering / determinism.**  A `bag` store has no natural row order; window
  operations need one from the dependency qualifier.  Does a `bag` store carry
  an ordering, or is that always established in the pipeline?
- **Shapes and conformance.**  Does a shape claim constrain a store's declared
  cardinality, or only its content (mirroring the view rule in `0012`)?
- **Interaction with `domain` resolution.**  A unit-reference field into a
  `bag` store resolves to *which* observation?  `0002`'s one-level resolution
  assumed a functional (`singletons`) target.
- **Migration.**  Whether any current worked example or corpus program should
  be re-modelled as a `bag` store, or whether the default keeps them all valid
  unchanged (expected: unchanged).
