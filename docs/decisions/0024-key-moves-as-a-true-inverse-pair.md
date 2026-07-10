# 0024: The key moves as a true inverse pair

## Status

Proposed.  Realized alongside the ADR 0022/0023 implementation (PR #40).
Extends the `extend_key`/`shrink_key` rules of
`docs/language/09-typing-reference.md` section 6.3 as amended by
`docs/decisions/0023-completeness-consumed-by-the-reducer.md`, and mirrors
the inverse-pair contract of
`docs/decisions/0020-reshape-as-a-true-inverse-pair.md`.  The formal
backing lands in `formal/Mensura/Laws.lean` (`project_ungroup`,
`ungroup_project`); the checker realization in `mensura-types`.

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

Both directions are equalities of tables, so the round trip preserves
*every* property at once: cardinality, completeness, `exhaustive`,
lineage, and content are facts about the table, and the table is the
same.  In particular the composite is the identity, which is `SplitSafe`
(`SplitSafe.id`), so the Tier B lineage drop of a lone `shrink_key` does
not apply to an exact round trip.

### 2.  Tracking: the key-move frame

The checker's table type gains a **key-move frame**: after
`extend_key cols` (or `shrink_key cols`), the output records the moved
column set and the input table exactly as it stood, nested frames
included.  The exactly inverse move (a `shrink_key`, respectively
`extend_key`, naming the same column set) returns the snapshot verbatim
instead of applying its conservative rules.  Any other operation clears
the frame at the dispatch, in one central place.

The frame is **not** a syntactic peephole and not a history hack: it is a
semantic fact about the table it is attached to.  A promotion frame says
"demoting these columns yields `saved`", which is `project_ungroup`
applied to the current table; a demotion frame says the dual via
`ungroup_project`.  Because the fact is self-justifying, frames nest and
unwind correctly through chains
(`extend_key a |> extend_key b |> shrink_key b |> shrink_key a`
restores the original table in two steps), and a *mismatched* inverse
move simply falls through to the conservative rules, burying the old
frame inside the new one's snapshot, where it remains true.

Clearing on every other operation is deliberately maximal for this
first realization.  Several intermediate operations plainly preserve the
fact (`assume`, `completeness_check`, a non-dropping `map` that neither
reads nor writes the moved columns); admitting them is the per-op
transport table sketched in Open questions, and each row of it needs its
own mechanized witness before the checker may use it, the same standard
ADR 0020 set for the `exhaustive` propagation rows.

### 3.  What changes downstream

- `T |> extend_key c |> shrink_key c` now types exactly as `T`, in
  content (original column order included) and in all four qualifier
  axes.  A `singletons` source round-trips to `singletons`, so a
  downstream reducing `group_map` is admitted by the ADR 0023 trivial
  discharge instead of demanding a vacuous `assume { complete }`.
- `T |> shrink_key c |> extend_key c` likewise restores `T`.
- A `shrink_key` that is *not* the exact inverse of the pending
  promotion (different columns, or any operation intervened) behaves
  exactly as ADR 0023 specifies: cardinality to `bag`, completeness
  propagated, lineage dropped, `exhaustive` forfeited.  Nothing in ADR
  0022/0023 moves.

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
  program with a `bag` type and a lost lineage when nothing actually
  happened between the moves.
- One mechanism covers both directions and chains, with a single
  clearing point in the dispatch, so the conservative behavior of every
  other operation is untouched by construction.

Negative:

- The frame is all-or-nothing: any intervening operation forfeits the
  restoration, even one that provably preserves the fact.  The cost is a
  conservative type, never unsoundness; the transport table (Open
  questions) is the path to lifting it.
- A snapshot of the table type rides along inside the checked type.  The
  nesting depth is bounded by the pipeline's length, and view boundaries
  (`resolve`) read only content and qualifiers, so the frame never
  escapes into a schema.

Neutral:

- ADR 0023's placement of the completeness obligation is unaffected: the
  reducing `group_map` still demands the fact; an exact round trip now
  simply presents the input's own cardinality to it.
- The typing reference (`09` section 6.3) needs a paragraph for the
  frame at its next editorial pass; until then this ADR is the
  specification.

## Alternatives considered

1. **A syntactic peephole** (rewrite adjacent `extend_key c |>
   shrink_key c` in the AST).  Rejected: it states a fact about syntax,
   not about the table, so it cannot compose across parenthesized
   sub-pipelines or future view inlining, and it silently diverges from
   the formal statement, which is about tables.
2. **Key-graded qualifiers**: state cardinality, completeness,
   `exhaustive`, and lineage at every coarsening of the current key
   ("functional over `K`", the axis-aware lineage of ADR 0020's open
   questions).  The principled generalization, and it would subsume the
   frame (a promotion re-grades the input's full-key facts to the
   subkey; a demotion re-grades them back).  Deferred: it touches every
   propagation rule in the checker and needs a per-fact mechanization,
   while the frame captures the exact-inverse fragment with two proven
   equalities.  The frame is forward-compatible with it.
3. **Teaching `shrink_key` to keep `singletons` when the retained key is
   provably functional.**  A special case of alternative 2 (it needs
   subkey cardinality anyway), and it restores only cardinality, not
   lineage or `exhaustive`.

## Open questions

- **The transport table.**  Which operations between the two moves
  preserve the frame: `assume` and `completeness_check` (they touch no
  content), a provably non-dropping `map` that neither reads nor writes
  the moved columns, `left_join`?  Each admitted row needs a mechanized
  commutation witness (the op commutes with `project` on the frame's
  domain), the ADR 0020 standard.
- **Key-graded facts** (alternative 2) as the eventual home of the
  frame, unifying it with axis-aware lineage.
- **Partial inverses.**  `extend_key a b |> shrink_key b` restores
  nothing today (the sets differ); with per-column frames it could
  restore the `a`-only promotion.  Deferred until a use case demands it.
- **The eval shortcut.**  When `shrink_key` becomes executable, lower an
  exact round trip as the identity.
