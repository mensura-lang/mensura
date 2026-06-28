# 0017: Completeness establish and consume surface

## Status

Accepted.  Ratifies the surface for establishing and consuming the
completeness fact in M1: the `completeness_check { assert ... }` and
`assume { complete }` stages that establish it, and the Tier B operations
(`shrink_key`, index `pivot`) that consume it.  The typing effects are frozen
in `docs/language/09-typing-reference.md` sections 6.3, 6.6, 8, and 9; this ADR
fixes only the surface.  Implemented for M1 on the `m1-completion` branch.

## Context

A Tier B operation is sound only over a partition that is complete over the
key it retains (`09` sections 7, 8).  A bare `store` is `Incomplete`
(`TableType::from_store`), so without a way to establish completeness, every
Tier B program is rejected.  `09` section 8 names three establish mechanisms:

- **mechanism**: a `collect` source is complete by construction;
- **check**: `completeness_check { assert ... }`, a pipe stage;
- **annotation**: `@complete_over(col)` on a source store.

Of these, only **check** is available in M1: `collect` arrives with ingestion
(M4) and the annotation family is deferred (`09` section 13).  The escape hatch
`assume` is also needed, for partitions whose completeness cannot be witnessed
by an in-language check.

Disjointness has a parallel escape hatch in `09` section 9, but in M1 nothing
*consumes* disjointness (the learning operations `fit`/`evaluate` that demand
it are M6).  So in M1 `assume` need only discharge the completeness obligation.

## Decision

### 1.  `completeness_check { assert <bool>; ... }`

A pipe stage whose block holds one or more boolean `assert`s over the current
table.  It establishes `Complete` over the current key and preserves every
other qualifier.  Each `assert` is a boolean expression typed against the
**key** context (the same context `split`'s predicate uses): it reads index
columns through `k`.

```
readings |> extend_key machine |> completeness_check { assert k.ts > 0 }
         |> shrink_key machine
```

The narrowing to a key-context boolean (rather than the cross-table
`row_count open_offerings == 0` shown illustratively in `09`) keeps the asserts
inside the M1 expression sublanguage, which has no cross-table aggregate
builtins yet.  The richer witness forms arrive with the builtins that back
them.

### 2.  `assume { complete }`

A pipe stage that admits the completeness obligation by fiat, locally and
visibly.  The block holds the single recognized claim `complete`.

```
external |> assume { complete } |> shrink_key region
```

`assume` is scoped to the completeness claim in M1 because completeness is the
only obligation consumed at this milestone.  The block form generalizes later
(e.g. a disjointness claim) without a surface change.

### 3.  Tier B consumers

`shrink_key cols` and the index form of `pivot` (ADR 0016) **demand**
`Complete` over the retained key, **consume** that obligation, yield a result
that is `Complete` over the new (coarser) key, and **drop** the lineage fact
(`project_not_preservesDisjoint`, `pivot_not_splitInvariant`).  Remove the
establishing stage and the Tier B operation is rejected.

## Consequences

Positive:

- Tier B operations are usable in M1 with a visible, in-language establish
  step; nothing is silently assumed.
- `assume` stays minimal (one claim), matching what M1 can consume, and is
  forward-compatible with later claims.
- The `assert` keyword (already in the grammar, ADR 0015) finds its first
  typed use inside `completeness_check`.

Deferred:

- `collect`-by-mechanism completeness (M4) and the `@complete_over` annotation
  (the annotation family, `09` section 13).
- The cross-table witness forms for `completeness_check` (need cross-table
  aggregate builtins).
- A disjointness `assume`/`assert` claim (no consumer until M6).

## Alternatives considered

1. **A view-body `assert` statement** (rather than a `completeness_check`
   stage).  Rejected for M1: a stage composes in the pipeline where the
   obligation is discharged, adjacent to the consuming operation; a loose
   body-level `assert` is left deferred (it already errors with a clear
   message).
2. **A bare `assume` operation with no block.**  Rejected: the block names the
   claim, so the program states *what* is assumed, and the form extends to
   later claims without new surface.

## Forward references

- `docs/language/09-typing-reference.md` sections 6.3, 6.6, 8, 9 (the frozen
  effects), section 11 (`project_not_preservesDisjoint`,
  `pivot_not_splitInvariant`).
- `docs/decisions/0016-reshape-surface.md` (the index form of `pivot`).
- `docs/decisions/0015-map-row-multiset-and-key-first-lambdas.md` (the `assert`
  statement and the key-first lambda context reused here).
