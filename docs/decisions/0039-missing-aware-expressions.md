# 0039: Missing-aware expressions

## Status

Accepted.  The consuming surface for ADR 0010's optional columns,
lifting the missing-aware deferral recorded in
`docs/decisions/0014-scalar-domain-taxonomy.md`.  Two consumers are
already waiting in the worked examples of the M5 design stack: the
cross-window rate idiom of ADR 0037 decision 6, inexpressible because
`series.lag` columns are optional, and the no-identity columns of
ADR 0038 decision 2, which are serve-only without a way to consume a
`T?`.  Lands with or immediately after the M5 windows slice.

Touches `mensura-syntax` (one operator token), `mensura-types`
(`expr_check`), `mensura-runtime` (`eval`), `formal/` (decision 5),
and `docs/language/06-expressions.md` and `09-typing-reference.md`.

## Context

ADR 0010 put missingness on the value axis: a `T?` column's row
exists and its value may be absent.  Nothing consumes the type: no
operator accepts an optional operand and no expression discharges
one, so any pipeline that produces a `T?` (a `series.lag`, a `dense`
fill) is terminal, and its columns can be served but not used.

SQL is the cautionary tale, and the failure must be located
precisely, because its NULL propagation is routinely blamed for
damage done elsewhere.  `1 + NULL = NULL` has never hurt anyone.
The damage comes from three leaks: propagation into predicates that
silently decide row fate (`WHERE x <> 5` drops the NULL rows with no
policy stated anywhere), aggregates that silently skip (`AVG`
ignores NULLs and the denominator changes without a trace), and
invisibility (nullability is not tracked through expressions, so no
one knows which hazard a query is exposed to).  The design below
adopts the propagation and blocks each leak.

## Decision

### 1.  Scalar operators lift over optionals

Every scalar operator and function of the expression sublanguage
accepts optional operands: if any operand of `op` is `T?`, the
result is optional, absent when any operand is absent, and `op`
applied to the present values otherwise.  This covers arithmetic,
the torsor rules of ADR 0036, dimension arithmetic (ADR 0026),
comparisons, and total functions such as `to_real`; the dimension
and domain checks are unchanged, applied under the `?`.

There is no nesting: the value axis is 0-or-1 (ADR 0010), so `T??`
does not exist and the lifting composes flat.  "Absent because an
operand was absent" and "absent result" are the same fact, which is
correct for value arithmetic: neither value exists.

The rate idiom this buys, over ADR 0038's `sensor_health`:

```mensura
(.rate = (r.peak - r.prev_peak) / (r.w - r.prev_w))
```

types as an optional rate, absent on each machine's first window and
wherever a `peak` is absent, which is the honest answer.

### 2.  `??` is the only exit from optionality

`e ?? d` evaluates to `e`'s value when present and to `d` otherwise.
`e` is `T?`; `d` is of the same scalar type, dimension included
(`?? 0.0 * kelvin`, not `?? 0`); the result is `T` when `d` is total
and `T?` when `d` is itself optional.  Right-associative, so a chain
discharges at its first present value and is total exactly when its
last default is.  One two-character operator token; the precedence
slot lands in `04-grammar.md` with the implementation.  (Resolved by
ADR 0040: `??` is unranked, and meeting a comparison or logic word
takes parentheses.)

Every `??` is a visible, grep-able policy statement: "when this
value is absent, this default is the true answer."  There is no
implicit collapse anywhere else in the language.

### 3.  Decision boundaries demand total values

Absence may flow through values; it may never flow past a decision
unconsulted.  Three rules, one per SQL leak:

- **Branching.**  The condition of `if` demands a total `bool`.  A
  comparison over optionals yields `bool?` (decision 1), so a filter
  over an optional column does not compile until the author writes
  `(...) ?? false` or `(...) ?? true`: the absent-row policy, stated
  in the program.  A branch *result* may be optional (the branches
  unify to the more optional type); the condition may not.
- **Aggregates.**  A fold or scan accumulates total values: the
  value lambda's result type must be total, so folding an optional
  column is a type error until it is discharged, visibly.  No
  aggregate skips absent values silently; `count` and `#b` count
  rows, never values, and are unaffected by optionality.  A
  count-of-present-values wants the presence predicate deferred
  below.
- **Keys.**  Unchanged and restated: `?` is a value-axis marker
  (ADR 0010), key columns are total, and absence never reaches
  equality-for-identity.

### 4.  Discharge late is the taught idiom

The checker cannot know that a default is a lie, so nothing stops
`?? 0.0 * kelvin` at the top of a pipeline, a sentinel with extra
steps (the move ADR 0031 decision 6 and ADR 0038 alternative 3
reject).  The worked examples therefore teach the opposite: carry
the `?` to the end, let the serving layer render absence, and write
`??` only where the default is a true statement about the domain.
This is a documentation obligation, not a typing rule, and it is the
residual risk this design accepts.

### 5.  Formal backing

Per ADR 0021, decision 1's lifted typing rules ship with theorems.
The lifting is the standard option applicative:
`formal/Mensura/Expr/Missing.lean` states the composition laws
(lifting distributes over composition, absence is absorbing, the
present case agrees with the unlifted operator) over an abstract
scalar operation, so the torsor and dimension instantiations come
for free and no per-operator proof is needed.

## Consequences

Positive:

- ADR 0037's rate idiom and ADR 0038's dense outputs become
  consumable in-language; the two worked examples complete.
- The three SQL leaks are compile errors: no silent row-dropping, no
  silent skipping, no invisible nullability.
- Every discharge is a searchable `??` with a reviewable default.

Negative:

- One optional column makes its downstream expressions optional, and
  the pressure to discharge early is real (decision 4 accepts it).
- `bool?` at every comparison over optionals adds ceremony to
  filters; the ceremony is the policy statement, but it is ceremony.

Implementation:

- `mensura-syntax`: the `??` token and its precedence row.
- `mensura-types`: `expr_check` lifts operator typing over `?`,
  types `??`, and enforces decision 3's totality demands.
- `mensura-runtime`: `eval` propagates absence and evaluates `??`;
  storage is unchanged (an absent value is already representable).
- Docs: `06-expressions.md` (lifting, `??`, the discharge-late
  idiom), `09-typing-reference.md` (the rules); the ADR 0037/0038
  worked examples gain the rate column in the implementation slice.

## Alternatives considered

1. **Three-valued logic** (SQL).  Rejected: propagation into
   branching is the leak, not the propagation; `unknown` collapsing
   to `false` at a filter is a policy nobody stated.  Decision 3
   makes the same collapse a one-token explicit choice.
2. **No lifting; explicit handling at every use.**  Rejected: the
   safety is nominal, because the ceremony pushes authors to
   discharge at first contact with a made-up default, which is the
   sentinel disease with worse ergonomics.  Lifting lets the honest
   absent flow to where an honest answer exists.
3. **Sentinel defaults in the machinery** (absent as `NaN` or
   `+Inf`).  Already litigated and rejected, ADR 0031 decision 6 and
   ADR 0038 alternative 3.
4. **Flow-sensitive narrowing** (an `is missing` test narrowing
   `T?` to `T` inside a branch).  Deferred, not rejected: it
   subsumes `??` but is a much larger typing feature, and no waiting
   consumer needs it.  `??` is forward-compatible with it.
5. **Skip-absent aggregates** (`AVG` semantics).  Rejected as a
   behavior of the existing folds; if wanted, it is a separately
   named binding whose skipping is in its name, with its own
   consumer.

## Open questions

- **A presence predicate.**  Resolved before it opened: `is known` /
  `is missing` already existed at the surface (typed and evaluated,
  `06-expressions.md`), which this ADR's survey missed.  They return a
  total boolean and do not narrow, so count-of-present-values was
  already expressible; only flow narrowing (alternative 4) remains
  open.
- **Fill policies on `dense`** (ADR 0038): carry-forward narrows
  `T?` to `T` by a mechanism; whether it lowers to a scan plus `??`
  or stays a distinct surface is decided there, with a consumer.
- **`??` on non-scalar positions** (rows, collections): out of
  scope; the operator is scalar until a consumer says otherwise.

## Forward references

- `docs/decisions/0010-attribute-totality.md` (the `?` axis this
  ADR consumes).
- `docs/decisions/0014-scalar-domain-taxonomy.md` (the missing-aware
  deferral lifted here).
- `docs/decisions/0021-formal-proof-pipeline.md` (decision 5).
- `docs/decisions/0026-dimensional-physical-units.md` and
  `docs/decisions/0036-temporal-domains-and-torsor-arithmetic.md`
  (the operator families decision 1 lifts).
- `docs/decisions/0031-fold-and-scan-primitives.md` (the sentinel
  rejection decision 4 inherits; the folds decision 3 constrains).
- `docs/decisions/0037-streaming-windows-and-closedness.md` (the
  rate idiom, first consumer).
- `docs/decisions/0038-rectangularization-over-the-window-grid.md`
  (the no-identity columns, second consumer; the fill-policy open
  question).
- `formal/Mensura/Expr/Missing.lean` (new).
