# Lineage and the disjointness check

This document specifies **lineage** and the **disjointness** property it
carries.  It is a proposal: the design content is firm, the surface syntax is
preliminary, in the same spirit as `07-pipelines.md`.  It is written against
the qualifier mechanism of `docs/decisions/0004-qualifier-mechanism.md`:
lineage is a standard-library qualifier, and disjointness is its first worked
**constraint hook**.

The ROADMAP names this document `05-lineage.md` in its planning list; the
actual `docs/language/` sequence has since filled `05` with
`naming-and-casing.md`, so the lineage document lands here at `08`.

## What this document adds

`07-pipelines.md` makes two properties first-class beyond plain content:
**cardinality** and **completeness**.  Each is a *tracked fact*: every
operation states how it transforms the fact, the fact is *established* by a
mechanism or a check or an annotation, and a downstream operation *demands*
it.  Completeness is the template (`07-pipelines.md:39-42`, `:208-238`):

- established by mechanism (`collect` is complete by construction), by a check
  (`completeness_check { assert ... }`), or by a source annotation
  (`@complete_over(col)`);
- preserved by Tier A operations, *demanded* (consumed) by the Tier B
  operations `shrink_key` and the index form of `pivot`;
- relaxable by `assume { ... }` when the obligation cannot be discharged.

This document gives **disjointness** the same surface.  Disjointness is the
precondition for not leaking across a split, and it is the fact that licenses
leak-free validation, which is the correctness goal of the whole language.
`07-pipelines.md:178-180` already places it in the lineage qualifier rather
than in the algebra; what follows is the operational account of how that fact
is established, propagated, demanded, and assumed.

## Disjointness is a relation, tracked through a unary state

Completeness is a unary fact about one table (`complete_over(k)`).
Disjointness is a relation between *two* tables: in the formalization
(`formal/Mensura/Table.lean`, `def:disjoint-tables`),

```
Disjoint T0 T1  :=  forall k, T0.rows k = 0  or  T1.rows k = 0
```

at every key at least one side is empty.  A relation seems unlike a unary
property, but completeness already backs a *relational* guarantee (a partition
over a key) from a *unary* fact consumed by a Tier B operation.  Disjointness
works the same way once it is given a unary carrier:

> each table carries a **lineage region**, the set of keys at which it is
> present (its support), tracked symbolically as a predicate over the key.
> Two tables are disjoint when their regions are provably non-overlapping.

The region is the qualifier state (ADR 0004's "structural value", an
accumulating value the framework supports, line 46).  Operations move the
region; a binary **constraint hook** at the point of use checks two regions
for disjointness.  This is exactly the unary-state / binary-check shape that
completeness already uses, so disjointness is first-class in the same sense,
not a fourth slot in the table type.

## Establishing disjointness

A disjointness fact enters a program in one of four ways, mirroring
completeness.

**By mechanism: `split`.**  `split |k| pred` routes each entity wholly to one
side of a pair by a predicate over the key.  The two halves carry the regions
`R and pred` and `R and not pred`, which are disjoint because the predicates
are mutually exclusive.  This is disjointness *by construction*: the
formalization proves it as `split_disjoint` (`Table.lean`), the companion of
`bind_split` (bind undoes split).  `split` is to disjointness what `collect`
is to completeness, the mechanism that needs no further discharge.

```
let (train, test) = data |> split |k| hash k < threshold
// train : disjoint from test, established here
```

**By check: `disjointness_check`.**  When two tables arrive without a shared
`split` ancestor, a check stage establishes the fact locally, the analogue of
`completeness_check`:

```
(cohort_a, cohort_b)
|> disjointness_check { assert disjoint_on patient_id }
|> bind
```

Each `assert` is a boolean expression over the two regions; together they
witness that the supports do not overlap.  As with `completeness_check`, the
fact must hold where it is consumed, so the check is placed ahead of the
consuming operation.  The surface of the check itself (in particular whether
`disjoint_on` must name an identity/key column, and how its assertion language
relates to the key-predicate region) is deferred; see the open questions.

**By annotation: `@disjoint_partition`.**  A source store may declare that it
is one block of a named partition, establishing the fact globally so no
per-use check is needed:

```
@disjoint_partition(cohort, block = "2024")  store enrollments_2024 ...
@disjoint_partition(cohort, block = "2025")  store enrollments_2025 ...
// two blocks of the same partition are disjoint by declaration
```

This is the disjointness counterpart of `@complete_over(col)`; like it, the
annotation lands with the annotation family (`@audited`, `@versioned`, ...),
so this document names it but does not fix its grammar.

**By assumption: `assume`.**  `assume { ... }` admits a disjointness obligation
by fiat, locally and visibly, when it cannot be discharged.  External data
whose provenance the type system cannot see is the common case (see
Decidability below).

## Propagating disjointness through the primitives

A table's region is moved by every operation, so a disjointness fact
established upstream survives, strengthens, or is lost downstream.  In the
formalization this is the `PreservesDisjoint` predicate (`Table.lean`,
`def`): an operation preserves disjointness when it sends disjoint inputs to
disjoint outputs.  `SplitSafe` is exactly `PreservesDisjoint and
SplitInvariant`, and split-safe operations compose (`SplitSafe.comp`), so a
Tier A pipeline carries a disjointness fact end to end.  Per primitive:

| operation | effect on the region | disjointness | theorem |
| --- | --- | --- | --- |
| `map` | key-preserving, support can only shrink | preserved | `map_preservesDisjoint` |
| `filter` | region narrows to `R and q` | preserved (strengthened) | `map_preservesDisjoint` |
| `group_map` | one output key per input key | preserved | `fiberMap_splitSafe` |
| `extend_key` | key refines, support splits | preserved | `ungroup_preservesDisjoint` |
| `left_join` / `inner_join` | fixed-right, key-preserving | preserved | `leftJoin_preservesDisjoint`, `innerJoin_preservesDisjoint` |
| `unpivot` | names move into the key | preserved | `unpivot_preservesDisjoint` |
| `split` | region splits by `pred` / `not pred` | established | `split_disjoint` |
| `bind` | regions union | weakened (see below) | `bind_disjoint_iff` |
| `shrink_key` | key coarsens, rows merge | **not preserved** | `project_not_preservesDisjoint` |
| index `pivot` | names leave the key | not preserved | `pivot_not_splitInvariant` |

**`split` refines.**  As above, `split` is the establishing mechanism; in
region terms it cuts `R` into two exclusive sub-regions.

**`bind` unions, hence weakens.**  `(a, b) |> bind` has region `R_a or R_b`.
For the result to stay disjoint from a third table `c`, *both* `a` and `b`
must have been disjoint from `c`:

```
Disjoint (bind a b) c   iff   Disjoint a c  and  Disjoint b c
```

(proved as `bind_disjoint_iff` in `Table.lean`, a direct consequence of the
`bind` and `Disjoint` definitions).  So merging can only *grow* a
region and therefore only *lose* disjointness facts: binding in a table that
overlaps `c` destroys `Disjoint _ c`.  This is the precise content of "the
property is changed by merge".  Note that `bind` itself is total and always
split-safe (`07-pipelines.md:169-176`); disjointness is not a precondition for
`bind` to be defined, it is a fact `bind` consumes when two halves are
recombined and a guarantee (`card <= 1`) that disjoint inputs buy.

**`shrink_key` and index `pivot` break it.**  These are the key-changing
operations, and they are exactly where disjointness is lost.  `shrink_key`
drops an index component, merging rows that a split had separated by that
component; the formalization proves the underlying `project` does not preserve
disjointness (`project_not_preservesDisjoint`), which is why
`07-pipelines.md:140` already cites that theorem for `shrink_key`.  The index
form of `pivot` is not even split-invariant (`pivot_not_splitInvariant`).
Past one of these, an upstream disjointness fact no longer holds over the new
key and must be re-established (by a check) or assumed.

## Demanding disjointness

A disjointness fact is *consumed* by an operation that is only leak-free over
disjoint inputs, the way `shrink_key` consumes a completeness fact.  Two sites
consume it:

- The learning and validation operations (model `fit` on one table,
  `evaluate`/`predict` on another) demand that the two tables are disjoint, so
  that a metric computed on the evaluation table is not contaminated by the
  training table.  These operations are not yet specified in the algebra
  documents; when they are, the disjointness demand is their defining
  precondition, the direct analogue of a Tier B completeness demand.
- `bind` consumes a disjointness fact to *preserve* a cardinality guarantee:
  binding disjoint inputs keeps `card <= 1` per key, binding overlapping
  inputs yields card many (`07-pipelines.md:172-175`).

An operation that demands disjointness type-checks only when the fact is in
scope, established by one of the four mechanisms above and propagated, intact,
to the point of use.

## Decidability and the key-change cliff

The constraint hook reduces to **predicate disjointness**: are the two
regions' guard predicates jointly unsatisfiable?  This has a decidable
fragment, linear arithmetic over numeric key fields, and falls back to
`assume` outside it (ROADMAP, "Implementation choices": "Predicate
disjointness has a decidable fragment ... and falls back to `assume`").
`split` predicates and `@disjoint_partition` blocks sit inside the decidable
fragment by construction.

The hard case is the key-change cliff: after `shrink_key`, a join that changes
the unit, or an index `pivot`, a region expressed over the old key no longer
denotes the same entities, so the solver cannot transport the fact.  Modeling
disjointness as a *check* (rather than a monolithic solver over full lineage
trees) is what lets the language degrade gracefully here: the fact simply
falls out of scope, and the program re-establishes it with a
`disjointness_check` or relaxes it with `assume`, locally and visibly, instead
of the whole solver entering an undecidable fragment.  This is the concrete
shape of the decidability worry ADR 0004 flags (open question on decidability
bounds, and the consequence that the lineage hook "becomes load-bearing and
must be specified carefully").

## Worked examples

**Train/test, the canonical leak-free pipeline.**

```
let (train, test) = enrollments |> split |k| hash k < 0.8
                                 // Disjoint train test  (split_disjoint)
let model         = train |> fit logistic_regression
let score         = test  |> evaluate model
                                 // demands Disjoint train test, in scope: ok
```

The fact established by `split` is carried, unchanged, by `fit` (key
preserving) and demanded by `evaluate`.  No annotation or check is needed; the
mechanism discharged it.

**Recombining safely, then losing the fact.**

```
let folds = data |> split |k| hash k < 0.5      // (a, b), disjoint
let all   = (folds.0, folds.1) |> bind          // region a or b; Disjoint _ c
                                                // now requires both a,b vs c
let coarse = all |> shrink_key region           // key changes: fact dropped
let safe   = (coarse, holdout)
             |> disjointness_check { assert disjoint_on student_id }
             |> evaluate model                  // re-established before use
```

`bind` unions the regions, `shrink_key` drops the fact at the key change, and
a `disjointness_check` re-establishes it over the new key before `evaluate`
consumes it.

**External sources, assert or assume.**

```
@disjoint_partition(cohort, block = "site_a")  store site_a ...
@disjoint_partition(cohort, block = "site_b")  store site_b ...
let combined = (site_a, site_b) |> bind         // disjoint by declaration

store vendor_extract ...                         // opaque provenance
let merged = (combined, vendor_extract)
             |> assume { disjoint_on subject_id } // no proof available
             |> bind
```

Two declared blocks of one partition are disjoint without a check; an opaque
external store carries no region the solver can compare, so its disjointness
must be asserted (if it shares a checkable key) or assumed.

## Relation to ADR 0004

This document is the first concrete instance of a lineage qualifier built from
the ADR 0004 framework, and disjointness is its constraint hook:

- **State space**: the lineage region, a key predicate (ADR 0004's structural
  value).
- **Propagation rules**: `preserve` for the key-preserving Tier A operations,
  `accumulate`/union for `bind`, a refining rule for `split`, and *drop* at the
  key-changing operations, each backed by a `PreservesDisjoint` theorem (or its
  refutation) in `Table.lean`.
- **Constraint hook**: predicate disjointness at the consuming operations, with
  the uniform `assume` escape.

Specifying disjointness this way de-risks ADR 0004 rather than competing with
it: the ADR notes that "the lineage disjointness solver moves from being a core
feature to being a `std::lineage` constraint hook" and that "the hook interface
becomes load-bearing and must be specified carefully".  The establish /
propagate / demand / assume surface above is that specification, exercised on
the one hook the ADR singles out.

## Open questions and proof obligations

- **`bind` weakening lemma** (discharged).  `Disjoint (bind a b) c  iff
  Disjoint a c and Disjoint b c` is proved as `bind_disjoint_iff` in
  `formal/Mensura/Table.lean`, backing the propagation rule for `bind`.
- **Region re-expression across key changes.**  Whether any key change admits a
  sound automatic transport of the region (rather than always dropping the
  fact) is open; the safe default specified here is to drop and re-establish.
- **`fit`/`evaluate` typing.**  The learning operations that *demand*
  disjointness are not yet specified; their typing rules belong in a future
  operations document and should cite this one for the demanded fact.
- **`disjointness_check` surface and the key it checks** (deferred).  The check
  is sketched here (`disjointness_check { assert disjoint_on <col> }`) but not
  worked out.  The open design point: the lineage region is a predicate over
  the *key*, yet `disjoint_on` names a column that need not be the key.  A
  future round must settle whether `disjoint_on` requires an identity/key
  column (so the check reduces to predicate disjointness over the region), or
  whether the region generalizes to non-key supports, and how the assertion
  language inside the block (`disjoint_on`, and any region combinators) is
  defined.  Until then the check reads as established-by-assertion, like
  `assume`, but with a column named.
- **Naming** (settled: "disjointness").  This document, and the term going
  forward, use "disjointness", matching `Table.lean`'s `Disjoint`.  The earlier
  working-branch spelling "disjointedness" is dropped.  (The ADR 0004
  "qualifier" naming question is separate and still open.)
