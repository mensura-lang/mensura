# 0024: The key moves as a true inverse pair

## Status

Proposed.  Realized alongside the ADR 0022/0023 implementation (PR #40).
Extends the `extend_key`/`shrink_key` rules of
`docs/language/09-typing-reference.md` section 6.3 as amended by
`docs/decisions/0023-completeness-consumed-by-the-reducer.md`, and mirrors
the inverse-pair contract of
`docs/decisions/0020-reshape-as-a-true-inverse-pair.md`.  The formal
backing lives in `formal/Mensura/Laws.lean` (`project_ungroup`,
`ungroup_project`, `ungroup_functional`); the checker realization in
`mensura-types` (key-graded cardinality).  An earlier draft of this same
ADR realized the contract with a key-move *frame* (a snapshot of the
pre-move table type); that mechanism is superseded here and recorded under
Alternatives considered.

## Context

ADR 0020 fixed the design bar for operation pairs that undo each other:
`pivot` and `unpivot` are **truly inverse** on a cleanly characterized
domain, and the checker tracks enough (the `exhaustive` fact) for the
round trip to restore types exactly.  The key moves fail that bar today.
`extend_key c |> shrink_key c` is semantically the identity, yet the
checker types it as a strict loss:

- **Cardinality** rises to `bag` (`shrink_key` coarsens unconditionally),
  so a `singletons` table comes back a bag and a downstream reducing
  `group_map` demands a completeness discharge that the input never
  needed (the ADR 0022/0023 discussion's "vacuous completeness" trap,
  re-entered through the back door).
- **Lineage** is dropped (`shrink_key` is Tier B), although the composite
  is the identity, which is trivially split-safe.
- **`exhaustive`** is forfeited at `extend_key` (the ADR 0020 erratum) and
  never comes back.

The same happens in the opposite order: `shrink_key c |> extend_key c`
re-files every row under exactly the key it came from, yet types as if
the table had been through a genuine reindex.

The motivating case reaches past a single pipeline.  A `singletons` store
keyed by `(machine_id, ts)` is reshaped by one view into a bag over
`machine_id` (`shrink_key ts`), and a *second* view promotes `ts` back
(`extend_key ts`), expecting the original `singletons` shape.  Whatever
tracks the inverse must therefore be a fact that can live on a table's
*type at a boundary* (a view's output schema), not a payload private to
one pipeline walk: `resolve` persists a view's output as columns plus
qualifiers (`ViewPlan`), and anything that cannot be stated there cannot
serve the cross-view round trip even in principle.

Which fact?  Completeness is the tempting candidate (the bag "still
carries `complete_over(machine_id, ts)`"), and it is the wrong one.
Completeness is a *presence* fact: no row that should exist is absent.
It says nothing about *multiplicity*: a bag in which one machine carries
two rows with the same `ts` can be perfectly complete, and promoting `ts`
then yields two rows at one `(machine_id, ts)` key, a bag, not the
original table.  What the promotion actually needs is that `ts` is
**distinct within each machine's group**, equivalently that the table is
*functional over the column set `{machine_id, ts}`*: grouping the flat
table by those columns yields at most one row.  That is a cardinality
fact graded by a column set, and it is exactly the information a
`singletons` source over `(machine_id, ts)` starts with and never loses
by being reindexed.

The formal development already contains both operations: `extend_key` is
the algebra's `ungroup` (def:grouping) and `shrink_key` its `project`
(def:projection), `formal/Mensura/Core/Ops.lean`.  What was missing is
the cancellation laws and a checker mechanism entitled to use them.

## Decision

### 1.  The inverse contract, mechanized

Two laws in `formal/Mensura/Laws.lean` make the key moves a mutually
inverse pair, exactly parallel to `pivot_unpivotDrop`/`unpivotDrop_pivot`:

- **`project_ungroup`**: `project (ungroup T) = T` whenever the promoted
  column is **total** on `T`.  The side condition is precisely the gate
  `extend_key` already enforces ("requires the column to be total; narrow
  it first"), so every promotion the checker admits sits inside the
  inverse domain.  A missing value in the promoted column would drop its
  row at `ungroup` (the honest ADR 0020 drop semantics), which is where
  the identity would fail.
- **`ungroup_project`**: `ungroup (project T) = T`, with **no side
  condition**: `project` tags every demoted row with its own key
  component, so the demoted column is total by construction and
  re-promoting it restores the original filing.

Both directions are equalities of tables, so semantically the round trip
preserves *every* property at once.  In particular the composite is the
identity, which is `SplitSafe` (`SplitSafe.id`), so the Tier B lineage
drop of a lone `shrink_key` does not apply to an exact round trip.

### 2.  Tracking: key-graded cardinality

The checker tracks cardinality **graded by column sets**.  The qualifiers
carry a set of *gradings*: column sets `S`, drawn from the flat table
(index and non-index columns alike), over which the table is known
**functional** (grouping by `S` yields at most one row;
`Mensura.Functional` in `formal/`).  The scalar cardinality is then
*derived*, not stored: a table is `singletons` exactly when some grading
is a subset of the current index.  Functionality is monotone upward (a
finer key can only shrink groups), so the subset check is sound.

The rules, per operation:

- **A source seeds the gradings.**  A `singletons` store contributes its
  index as a grading; a `bag` store contributes none (ADR 0022).
- **The key moves change the index, never the gradings.**  A grading is a
  fact about the flat table, indifferent to which of its columns are
  currently the key.  `extend_key C` grows the index and re-derives
  cardinality; `shrink_key C` shrinks the index and re-derives
  cardinality.  Everything else about the two operations stays per ADR
  0023: `shrink_key` propagates completeness, drops lineage (its Tier B
  break), and forfeits `exhaustive`; `extend_key` keeps its totality and
  key-eligibility gates.
- **Content-identity stages preserve the gradings.**  `assume` and
  `completeness_check` do not touch the rows, so the fact rides through
  them.
- **Every other operation resets the gradings** to match its own computed
  output cardinality: the current index if `singletons`, nothing if
  `bag`.  This is deliberately maximal for the first realization; the
  per-op transport table (which operations provably preserve a grading)
  is an open question, and each admitted row needs its own mechanized
  witness, the standard ADR 0020 set for the `exhaustive` rows.

Why this realizes the inverse contract with no further machinery:

- `T |> extend_key c |> shrink_key c`: the source grading (say `{ts}`)
  survives both moves untouched; after the shrink the index is `{ts}`
  again, the subset check finds the grading, and the table is
  `singletons`.  This is `project_ungroup` read at the type level.
- `T |> shrink_key c |> extend_key c`: the grading `{ts, c}` from the
  `singletons` source is not a subset of the shrunken index (correctly a
  `bag`), and is a subset again after the promotion (`singletons`
  restored).  This is `ungroup_project` read at the type level.
- The **cross-boundary round trip** falls out of the same check, because
  a grading is a plain set of column sets: it is representable on a
  view's output type, where a snapshot of a checker-internal table can
  never be.  A bag over `machine_id` carrying the grading
  `{machine_id, ts}` types `extend_key ts` as `singletons` no matter
  where the bag came from, another pipeline, a view boundary, or (later)
  a declared fact on a bag store.

Consumption is definitional rather than rule-shaped: the grading *is*
the statement `Functional (ungroup_C T)`, and `extend_key C` *is*
`ungroup_C`, so promoting simply exposes what the fact already said.  The
one propagation row that needs its own lemma is that `extend_key`
preserves `singletons` (`ungroup_functional`: `Functional T` implies
`Functional (ungroup T)`), which backs re-deriving `singletons` after a
promotion from an already-`singletons` table.

### 3.  What changes downstream

- `T |> extend_key c |> shrink_key c` and `T |> shrink_key c |>
  extend_key c` both restore the input's **cardinality**, content
  columns, and totality.  A `singletons` source round-trips to
  `singletons`, so a downstream reducing `group_map` is admitted by the
  ADR 0023 trivial discharge instead of demanding a vacuous
  `assume { complete }`.
- The restoration composes through content-identity stages: an
  `assume { complete }` or `completeness_check` *between* the moves does
  not forfeit it (a frame-style mechanism would have).
- Promoting a column set whose grading is already known types as
  `singletons` even when the bag was built elsewhere, which is the
  cross-view scenario above once view outputs become sources.
- Two restorations are deliberately **not** claimed, conservative but
  never unsound: `exhaustive` stays forfeited at `extend_key` (a graded
  `exhaustive` is an open question), and `shrink_key` still drops
  lineage even inside an exact round trip (today `Lineage::dropped()`
  equals the root lineage, so nothing observable is lost; a graded
  lineage is the principled fix).  Attribute *order* after a round trip
  may also differ (demoted columns re-enter at the end of the attribute
  list); the schema content is unchanged.
- A `shrink_key` that is not an inverse of anything behaves exactly as
  ADR 0023 specifies: cardinality to `bag` (no grading fits the retained
  key), completeness propagated, lineage dropped, `exhaustive`
  forfeited.  Nothing in ADR 0022/0023 moves.

### 4.  Runtime

Unchanged.  `shrink_key` remains Tier B and not executable
(`mensura-runtime`), so no lowering exists to reconcile; when it becomes
executable, an exact round trip may additionally be lowered as the
identity, which is then an optimization, not a semantics change.

## Consequences

Positive:

- The key moves join `pivot`/`unpivot` as a true inverse pair, proven
  both ways, with the inverse-domain side condition already enforced by
  the existing `extend_key` totality gate.
- The "promote, work at the finer key, demote" idiom stops taxing the
  program with a `bag` type when nothing between the moves disturbed the
  fact.
- The tracked fact is a set of column sets, the same shape as the
  `exhaustive` qualifier: boundary-representable, so it extends to view
  outputs (`ViewPlan`) and, later, to declared facts on bag stores and
  shapes, unifying with ADR 0022's deferred per-column refinements.
- Cardinality stops being a primitive two-state flag and becomes the
  derived, `S = index` instance of one graded fact, which is the
  direction the full key-graded design (Alternatives, 3) wants anyway.

Negative:

- The reset-everywhere-else rule is all-or-nothing outside the key moves
  and the content-identity stages: an intervening `map` forfeits the
  gradings even when it provably preserves them.  The cost is a
  conservative type, never unsoundness; the transport table (Open
  questions) is the path to lifting it.
- Exact round trips no longer restore `exhaustive`, and attribute order
  may change.  Both are strictly weaker restorations than a snapshot
  gives; both are the honest price of tracking facts instead of
  memoizing types, and neither loss is observable in any current corpus
  program.

Neutral:

- ADR 0023's placement of the completeness obligation is unaffected: the
  reducing `group_map` still demands the fact; a round trip now simply
  presents the input's own cardinality to it.
- The typing reference (`09` section 6.3) states the grading rules for
  the two key moves; this ADR is the specification of the mechanism.

## Alternatives considered

1. **A syntactic peephole** (rewrite adjacent `extend_key c |>
   shrink_key c` in the AST).  Rejected: it states a fact about syntax,
   not about the table, so it cannot compose across parenthesized
   sub-pipelines or future view inlining, and it silently diverges from
   the formal statement, which is about tables.
2. **The key-move frame** (this ADR's earlier draft, superseded).  After
   a key move, snapshot the pre-move table type; the exactly inverse
   move returns the snapshot verbatim; any other operation clears it.
   Sound (the snapshot is justified by the same two laws) and it
   restores *everything*, `exhaustive` and column order included.
   Rejected on three counts.  It is all-or-nothing by construction: only
   the literally adjacent inverse restores, and even `assume` between
   the moves forfeits it.  It is boundary-blind by construction: a
   snapshot of a checker-internal type cannot be persisted on a view's
   output schema, so the cross-view round trip that motivates the whole
   design is unservable.  And it memoizes a type instead of tracking a
   fact, which is against the grain of a checker whose every other
   property is derived from how operations transform facts.  The graded
   mechanism subsumes its cardinality restoration, which is the axis
   with observable consequences today.
3. **Key-graded everything**: state cardinality, completeness,
   `exhaustive`, and lineage at every coarsening of the current key
   (axis-aware lineage, ADR 0020's open questions).  The full design
   this ADR takes the first step of; adopting it wholesale touches every
   propagation rule in the checker and needs a per-fact mechanization.
   This ADR adopts exactly the cardinality axis, whose lemmas exist, and
   stays forward-compatible with the rest.

## Open questions

- **The transport table.**  Which operations preserve a grading:
  a non-dropping `map` that neither reads nor writes the graded columns,
  `left_join` against a functional right table, `split`?  Each admitted
  row needs a mechanized witness (the op preserves `Functional` on the
  graded regrouping), the ADR 0020 standard.
- **Graded `exhaustive` and graded lineage.**  The two axes an exact
  round trip currently loses; alternative 3 is their eventual home.
- **The declaration surface.**  A bag store or shape that *declares* a
  grading (the `ReadingBack` problem: stating that `ts` is distinct
  within each machine's bag), joining the `@complete_over` annotation
  family and ADR 0022's shape-strictness open question; and the
  persistence of gradings on `ViewPlan` once view outputs become sources
  for downstream views.
- **Partial inverses.**  `extend_key a b |> shrink_key b` restores the
  `a`-only promotion exactly when a grading fits the retained key, so
  the subset check already covers most of what per-column frames would
  have; whether any residue remains is deferred until a use case
  demands it.
- **The eval shortcut.**  When `shrink_key` becomes executable, lower an
  exact round trip as the identity.
