# 0010: Attribute values are total by default

## Status

Accepted.  Realized in `docs/language/02-stores.md`,
`docs/language/06-expressions.md`, `docs/language/07-pipelines.md`, and
the type grammar in `docs/language/04-grammar.md`.

## Context

A `var` or `const` value may be missing for a particular row: a sensor
that dropped a reading, a person with no recorded last name, a machine
never serviced.  The language had no rule for this, and the notion was
running together with two others it must be kept apart from.

The formal model (`formal/Mensura/Table.lean`) is precise about which is
which:

- A table is `rows : K → Multiset (Row H σ)`.  At a key there is a
  **multiset of nested rows**; its `card` is the **cardinality**: `0` is
  an absent row ("not sampled"), `1` is one observation, two or more is a
  bag.  Cardinality is the only multiset in the model, and "not sampled"
  and `#row` are cardinality talk.
- A cell is `Cell β = Option β`: a single value is **known** (`some`) or
  **missing** (`none`), **always 0 or 1**.  A value is never a bag.  This
  is the chapter's missing marker `?`.
- **Completeness** is a third, separate notion: whether the observed rows
  cover a whole partition of the key, the fact a Tier B operation
  consumes (ADR 0004, ADR 0009, `08-lineage.md`).

Cardinality (how many rows) and value-missingness (whether a value is
there) are **orthogonal**, and this ADR fixes only the second.  It is a
content/types decision, not a surface convenience, for two reasons.
First, operations already produce missing values: `leftJoin` keeps an
unmatched left row and fills its right columns with `none`
(`Table.lean`, the `f.elim (fun _ => none)` branch).  So whether a value
is known is a per-column property that operations *introduce and
transform*, like cardinality and completeness.  Second, a missing value
is not an absent row: minimality (`Substantive`/`Minimal` in
`Table.lean`) requires every observed row to have at least one known
value, and `leftJoin` shows rows that plainly exist yet carry missing
columns.

Terminology, used consistently below: **cardinality** is rows per key
(0/1/many); a value is **known** or **missing**; a column is **total**
(every value known) or **optional** (values may be missing).  "0-or-1"
elsewhere in the docs (`07-pipelines.md`, ADR 0009) is *row* cardinality
(at most one row, as `pivot` demands), a separate axis from `?`.

## Decision

- **Value totality is a per-column property, orthogonal to
  cardinality.**  Each value is known or missing (`Cell = Option`); a
  column is total or optional.  This is independent of how many rows a
  key has.

- **Values are total by default.**  In an observed row, a declared
  attribute is known.  A bare type is a guarantee that the value is
  there.

- **`?` marks an optional value**, written as a postfix on the type:
  `last_service: date?` may be missing even in an observed row.  At most
  one `?`.  It is a type operator, so it is available wherever a type
  appears: store and shape attributes now, function signatures and
  lambda-return ascriptions later.

- **Index fields are always known.**  `?` is not allowed on an index
  field; whether the row exists at all is cardinality (the 0-or-1 rule of
  `01-units.md`), a separate axis.  Both `const` and `var` may be
  optional.

- **Totality is threaded by operations.**  `leftJoin` makes the right
  table's columns optional on the result (unmatched left rows carry them
  missing); `innerJoin` drops unmatched rows and introduces none.
  Totality propagates through a pipeline as cardinality and completeness
  do.

- **Consuming an optional value requires establishing that it is known.**
  Any value-demanding context (the scalar operators `+ - * / ^`, the
  comparisons, `and`/`or`/`not`) requires a **single known value**:
  cardinality 1 and not missing.  Applying one to an optional value is a
  hard type error, not silent propagation; there is no implicit "missing
  in, missing out".  This is a distinct obligation from collapsing a bag
  (a cardinality matter) with a bag combinator (`06-expressions.md`).

- **A value is established as known three ways:** a default or coalesce
  that supplies a value for the missing case; an aggregate or combinator
  defined over missingness; or **narrowing** through `is known`.  Inside
  the branch guarded by `r.x is known`, and on every row after a
  table-level `filter (|r| r.x is known)`, the optional `x` is treated as
  total.

## Consequences

Positive:

- A bare type is an enforced guarantee that the value is there, checked at
  compile time.  The risky case (it might not be) is the one that has to
  be written down, in keeping with Mensura's discipline that the strong
  property is the default and the weaker one is explicit.
- Defaulting and imputation become visible, typed operations that take a
  column from optional to total.  That is the hook a later decision needs
  to make imputation leakage-aware, the way `08-lineage.md` makes
  disjointness leak-aware.
- The storage mapping is mechanical: a total attribute is a `NOT NULL`
  column, an optional one is nullable
  (`docs/toolkit/00-storage-backend.md`).
- A `leftJoin` result type now *states* that its right columns are
  optional, instead of leaving the `none`-fill implicit in prose.

Negative:

- Gappy domains pay the annotation cost.  In the IIoT example, sensor
  dropouts mean many `var` columns are optional; in the college example,
  a value recorded only while enrolled is optional.  `?` will be common
  on `var` columns, and narrowing or defaulting is needed before
  arithmetic.
- Docs that described missing values loosely are reframed so the missing
  axis is `Cell = Option`, separate from the row multiset.
  `06-expressions.md` no longer calls "absence and multiplicity the two
  ends of the same axis"; `00-overview.md`'s "cells may carry tuples of
  values" is restated as the row multiset.

Neutral:

- The grammar gains one postfix type operator.  LL(1) is preserved: after
  a type the parser peeks one token, takes a single `?` if present, and
  proceeds.

## Alternatives considered

1. **Optional by default, with a marker to assert that a value is known.**
   Honest about messy data, and the usual objection (silent `NULL`
   propagation, as in SQL) does not apply here because operators refuse a
   possibly missing operand outright.  Rejected anyway: it inverts the
   language's "strong property is the default" stance, and it makes a bare
   type say nothing, shifting the burden to remembering to assert
   known-ness everywhere it matters.

2. **Model missingness as a row cardinality of 0-or-1.**  Rejected, and it
   is the conflation this ADR exists to prevent: a missing value is not an
   absent row.  `Cell = Option` is per value; minimality keeps an observed
   row substantive even when some columns are missing; and `leftJoin`
   produces rows that exist with missing columns.  Cardinality counts
   rows; `?` marks a value.

3. **An explicit propagating lift** (a `?.`-style operator that threads a
   missing value through an expression and yields a missing result)
   instead of strict elimination.  Deferred, not rejected: the strict rule
   is the safe floor, and a lift can be added later as sugar over it
   without reopening this decision.  See open questions.

4. **Per-kind defaults** (`var` optional, `const` total).  Rejected as a
   special-case rule; the language favours few general primitives, so one
   default and one marker apply uniformly, and `const` may be optional
   just as `var` may.

## Open questions

- **An explicit propagating lift.**  Whether to add a `?.`-style operator
  that carries a missing value through an expression, yielding a missing
  result, as sugar over the strict rule.
- **Narrowing scope.**  Exactly which guard forms narrow `optional` to
  `total`: a `when:`-style branch, a table-level `filter`, and how the
  narrowing is expressed in the type rules.
- **Row-cardinality notation.**  How `card 1` / `0-or-1` / many is written
  in a type stays open (ADR 0009); this ADR settles only the orthogonal
  total/optional axis, and the two notations must compose without
  confusion.
