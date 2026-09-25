# 0044: `last`, the arrangement-relative reduction

## Status

Accepted.  Resolves issue #76.  Renames the point-reduction of
`docs/decisions/0037-streaming-windows-and-closedness.md` decision 7
from `latest` to `last`, with no change to its semantics, its demands,
or its formal backing.  Touches `mensura-types` (`pipe_check`),
`mensura-runtime` (`eval`), `docs/language/04-grammar.md`,
`07-pipelines.md`, `09-typing-reference.md`, `10-views.md`, the book's
`windows/over-time.md`, the fleet example, and the corpus.  No
`formal/` work follows: no Lean definition carries the name.

This ADR is the **translation reference** for ADR 0037, in the manner
of `docs/decisions/0025-nomenclature-consistency-sweep.md`.  ADRs are
append-only, so 0037 (and the one mention in
`docs/decisions/0031-fold-and-scan-primitives.md`) keep `latest`;
read every `latest p` there as `last p`, and `latest (desc p)` as
`last (desc p)`.

## Context

ADR 0037 decision 7 settled the dual of the reduction as a marked
point, `latest (desc p)`, and recorded the cost: "the latest by
descending `p`" is the earliest, so the spelling says the opposite of
what it does.  That is one of two problems with the name.

1. **The name asserts a direction**, so it fights the marker whose
   whole job is direction.
2. **The name asserts a temporal reading the operation does not
   have.**  The point may be any orderable column (`int`, `real`, a
   dimensioned real, `date`, `instant`), so `latest score` and
   `latest peak_load` are well-typed and nonsense as English.

The operation is `getLast (arrange p fiber)`: it keeps the last row of
the fiber's arrangement by `p`.  The arrangement is exactly what `desc`
reverses, so a name relative to the arrangement composes with the
marker instead of contradicting it.

## Decision

### 1.  The reduction is spelled `last`

```mensura
readings |> demote taken_at |> assume { complete } |> last taken_at
```

- `last taken_at` keeps the newest row, as `latest taken_at` did.
- `last (desc taken_at)` keeps the oldest row, and now reads as what
  it does: the last row of the descending arrangement.
- `last score` keeps the maximal-score row, with no false temporal
  claim.

Everything else in ADR 0037 decision 7 and its two annotations stands
as written: the result is `singletons` at the current key; tie-freedom
and completeness are demanded and discharge by the same rules; the
point must already be an attribute, total, and orderable; the `desc`
marker must be parenthesized; ties resolve to the earlier row at
either order; and the backing is `IsArrangement.unique`, at `ω` or
`ωᵒᵈ`.

### 2.  `latest` is not kept as an alias

Nothing outside the repository uses the name, so there is no one to
break.  An unknown `latest` falls through to the generic unknown
operation diagnostic, whose edit-distance hint already suggests
`last`.

### 3.  The naming family is arrangement-relative

`series.first_value` and `series.last_value` already name positions in
the arrangement.  Under this rename `last` picks the row and
`last_value` picks the value, and any later point-reduction takes a
name relative to the arrangement rather than to time.

## Consequences

- The dual needs no second name, and now needs no apology either: the
  "`earliest` as sugar" escape ADR 0037 left open loses its motivation,
  since `last (desc p)` already reads correctly.
- The common call reads slightly worse (`last taken_at` rather than
  `latest taken_at`), the one cost weighed against the general and the
  dual cases.
- The book no longer justifies the operation as temporal.  It stays in
  `windows/over-time.md`, where its motivating query lives, but the
  prose says the point is any orderable column.
- Corpus files and book examples named after `latest` are renamed with
  it.

## Alternatives considered

- **Keep `latest`.**  `latest taken_at` is the overwhelmingly common
  call and reads best there.  Declined because the temporal framing
  compounds: every future point-reduction would either inherit it or
  sit inconsistently beside it, and the vocabulary is cheapest to fix
  at today's handful of call sites.
- **`argmax`.**  Exact, but jargon, and direction-bearing, so it wants
  an `argmin` partner: the dual-name route ADR 0037 declined.  It also
  does not read as a row.
- **`top`.**  Arrangement-relative but ambiguous: the top of a ranking
  is the maximum, the top of a list is its first element.
- **`first` with the minimum as the default.**  A change of meaning
  rather than of name: every existing call site would silently invert.
- **`pick` or `select` with a marker.**  Direction-neutral but vague,
  and `select` carries the SQL folklore ADR 0025 exists to avoid.
