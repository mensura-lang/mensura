# 0026: Dimensional physical units

## Status

Accepted.  Opens M3 (`ROADMAP.md`, "Physical units, precision, and measure
semantics") and realizes design pillar 4 ("keys and physical units are part of
the type") of `docs/language/00-overview.md`.  The formal backing lands with
this ADR in `formal/Mensura/Units/Dimension.lean` (blueprint chapter "Physical
dimensions"), per the ADR 0021 rule that a checker propagation rule ships only
when a theorem under `formal/` backs it.

This ADR fixes the *model* of dimensional units and the units-and-values surface
built on it.  Two companion ADRs land in the same pull request, because the
units model is stated in terms of const bindings and imports:
`docs/decisions/0027-modules-and-imports.md` introduces top-level const bindings
and the module system, and `docs/decisions/0028-standard-library-si.md` ships
the `si` library that defines the unit symbols, prefixes, and named derived
units.

It deliberately does **not** include: the full language design doc
(`docs/language/11-physical-units.md`, with the surface grammar, the typing
rules, and a worked fleet example); the lexer/parser/checker/runtime
realization; or the reconciliation edits this decision forces on
`docs/language/06-expressions.md`, `docs/language/04-grammar.md`,
`docs/language/05-naming-and-casing.md`,
`docs/language/09-typing-reference.md`,
`docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`,
`docs/decisions/0014-scalar-domain-taxonomy.md`, and
`docs/toolkit/00-storage-backend.md`.  Those are follow-ups named at the end.

It answers ADR 0014's open question ("exactly how M3's dimensional units and
precision refine `real`") and revises ADR 0013's anticipation that physical
units are a per-column `Qs` qualifier (see Decision 4).

## Context

Existing tools leave the physical unit of a measurement to a column name, a
comment, or the programmer's memory, so adding gallons to liters or reading an
acceleration as a length is a silent runtime error.  Mensura's promise (pillar
4) is that a physical-unit mismatch is a compile error.  The fleet example
carries a temperature today as a bare `real` with the unit baked into the field
name (`kelvin: real`, `docs/examples/fleet-monitoring.mensura:64`); M3 turns
that placeholder into a checked type, and the IIoT driving application also needs
acceleration (`length / time^2`), energy (`mass * length^2 / time^2`), and other
derived quantities.

The reserved `NxE` measured literal (`10x3`) fused two things into one token: a
dimension *and* a measurement precision (`06-expressions.md:119-124`).  That
coupling is the mistake this ADR avoids.  Precision is a hard, separable
concern, better delivered later as a library that extends `real` than baked into
the core literal now.  So M3 starts with *dimensions only*; precision and
measure semantics (`@additive`/`@semiadditive`/`@foldable`) are separate, later
documents.  The reserved unit-attachment-by-juxtaposition surface
(`06-expressions.md:341-346`) is also superseded here: units are ordinary
dimensioned constants combined with explicit `*` (Decision 7), so no
juxtaposition rule and no `SI(...)` constructor are needed.

The intended dimension representation is settled evidence: dimensions form a
free abelian group, and canonicalization is "sort the exponent vector," a
trivial canonical form (`untracked/egg.md:76-79`).  No dimensional analysis
exists in `formal/` today; the algebra there is about tables, keys, and
multisets only.  This ADR's formal module is new territory.

## Decision

### 1.  The seven base dimensions and their base units

The SI base quantities map to seven type-level base dimensions, each with one
base unit:

| SI base quantity     | Base dimension | Base unit (symbol) |
| -------------------- | -------------- | ------------------ |
| time                 | `time`         | second (`s`)       |
| length               | `length`       | meter (`m`)        |
| mass                 | `mass`         | kilogram (`kg`)    |
| electric current     | `current`      | ampere (`A`)       |
| temperature          | `temperature`  | kelvin (`K`)       |
| amount of substance  | `amount`       | mole (`mol`)       |
| luminous intensity   | `luminosity`   | candela (`cd`)     |

`luminosity` names luminous intensity: astrophysics defines it slightly
differently (total radiant power), but as a base-type name it reads
unambiguously as "light power," matching the IIoT background material.

Derived dimensions need no unit choice: their base unit is *induced* by the
group structure (coherent SI, scale 1), e.g. the base unit of `length / time^2`
is `m/s^2`.  The base-unit choice is **semantically invisible**: values
normalize to base at ingestion, so it is a storage/interop convention, recorded
where the storage doc decides persistence, not a fact the type system tracks.

Two footnotes the SI forces:

- `kg` is the base *scale* for mass, but SI prefixes attach to the gram `g`
  (`mg`, `g`, `kg`), never to `kg` (`mkg` is not a unit).  The `si` library
  (ADR 0028) defines `g` and prefixes it.
- Timestamps are affine (offsets from an epoch) and stay **outside** the
  dimension system; only *durations* are `time[real]`.  An epoch timestamp is
  not `time[real]` any more than a calendar date is a duration.

### 2.  A dimension is an element of the free abelian group over the seven

A physical dimension is an integer exponent vector over the seven base
dimensions, i.e. an element of the free abelian group of rank seven, written
multiplicatively: dimensions *multiply* (`length * length = length^2`) while
their exponent vectors *add*.  The group identity (all exponents zero) is the
**dimensionless** quantity.  Derived dimensions are products, quotients, and
integer powers of the seven.

The canonical form is the exponent vector itself; dimension equality is vector
equality and is decidable.  Decidable equality is what makes "dimension mismatch
is a compile error" a decision procedure rather than a heuristic.  Equality
saturation is unnecessary machinery for a group with a trivial canonical form
(Alternatives, 5).

### 3.  A dimensioned type is a dimension applied to a backing numeric type

The type of a measured column is a **dimension applied to a backing scalar
type**, written `D[backing]`:

```
temperature[real]
(length / time^2)[real]
```

`real` is the only backing today.  Keeping the dimension a parametric
constructor over its backing (rather than a fixed refinement of `real`) makes
the dimension orthogonal to the numeric representation: a future library
refinement of `real` (measurement precision, intervals, rationals) substitutes
into the bracket without the dimension system knowing about it.  Bare `real` is
the dimensionless quantity (the group identity).  `int` is never dimensioned; it
remains counts and identities (ADR 0014).  This still "attaches dimensional
units to `real`" as ADR 0014 anticipated: `real` is the backing, not replaced,
so the closed scalar taxonomy is not reopened.

**The type tracks the dimension, not the unit.**  `m` and `km` both have type
`length[real]`; magnitudes are always normalized to the base unit, so the
specific unit is invisible at the type level.  This is a deliberate divergence
from F# units-of-measure and boost::units, which track `m` and `km` as distinct
types.  The trade: conversion within a dimension becomes automatic and free
(Decision 7), at the cost that the compiler catches *dimension* mismatches, not
*unit-labeling* mistakes ("kilometers stored in a column documented as meters").
Unit labeling for storage/display is an interop convention, not a typed fact.

**Unit constants are `real`-backed for now.**  `meter : length[real]`, so
dimensional arithmetic combines `real`-backed values.  Making unit constants
backing-*polymorphic* (so `a * m` would be `length[T]` for `a : T`) needs a rule
for how backings combine under `*`, which is the precision library's concern;
it is deferred (Open questions).  The Rust representation (a parametric
`Quantity { dimension, backing }` domain, or a `Dimension` payload on the
numeric domain) is a follow-up implementation choice; this ADR fixes the model.

### 4.  Dimension lives in the content `C`, not the qualifier row `Qs`

A dimension is part of a column's **domain**: it is *what the data is*, so under
the ADR 0013 boundary ("`C` is structure, `Qs` is propagated fact") it belongs
in `C`.  This **revises ADR 0013's anticipation** that units are a per-column
`Qs` qualifier alongside totality (0013:63, 139).

Dimensional arithmetic is genuine *type computation*, not fact propagation: the
result dimension of `a * b` is a function of the operand dimensions
(`length * length = length^2`); the result dimension of `sum b.x` is the
dimension of `x`; adding two quantities requires their dimensions *equal* or the
program is rejected.  These are domain-inference rules of the same kind as "what
is the type of this expression," which live in `C`, not the preserve/forfeit
propagation rules that characterize a `Qs` qualifier.  The clean line for the M3
family: **dimension** = domain in `C` (this ADR); **precision** (future library)
= an extension of the *backing* `real`, which may surface as a refined backing
type or a per-column `Qs` qualifier (0013's slot); **measure semantics**
(`@additive` etc., later) = per-column annotations gating rollups.

### 5.  Casing: dimension names are lowercase built-in type names

Dimension names (`temperature`, `length`) and dimension aliases (`speed`,
Decision 8) are **lowercase built-in type names**, the same category as `int`,
`real`, `string`, `bool`, and `date`.  They are not user-declared PascalCase
types.  The PascalCase-for-types rule (`05-naming-and-casing.md`) governs
user-declared type *names* (`unit`, `shape`, `enum`); built-in type names are a
fixed lowercase vocabulary the resolver matches rather than case-checks, and
dimensions extend it.  Unit names and symbols (`meter`, `m`, `kg`) are
value-level terms and follow snake_case.

The `[...]` bracket is uniformly **type-level parameter application**: shape
parameters (`Shape[U: Unit]`), the backing of a dimension (`temperature[real]`),
and the parameters of a generic alias (`speed[T]`, Decision 8) are one construct.

### 6.  Minting: the seven base units are intrinsic bindings

The seven base units are **intrinsic value bindings** named in full: `second`,
`meter`, `kilogram`, `ampere`, `kelvin`, `mole`, `candela`.  They live in the
initial environment, not as keywords (the lexer stays keyword-free); the
precedent is the ambient aggregate combinators (`sum`, `count`, ...) and
pipeline operations, which are already entries in the checker's typing context,
not grammar.

There is **no constructor form**: `time(1)` and any ambient
dimensionless-to-dimensioned cast are rejected, because such a cast is the exact
escape hatch a dimensional type system exists to make unnecessary.  Because
there is no constructor, the base units **must** be language intrinsics: they
are the root of the dimensioned-value world, and everything else (short symbols,
prefixes, named derived units) is written in terms of them in the `si` library
(ADR 0028).  So exactly **two ways a dimensioned value comes to exist**:
arithmetic on the intrinsics, or ingestion through a declared column type.

### 7.  Units are ordinary dimensioned constants; value syntax is explicit `*`

A unit is a **positive dimensioned constant** (an element of `R+ x Dim`).  With
top-level const `let` (ADR 0027) and explicit `*`, units need no new machinery:
`*` already computes the constant's magnitude and dimension.

- **Value syntax is explicit multiplication**: `9.8 * m / s^2`.  Juxtaposition
  is dropped (it was only reserved, never implemented); `*`, `/`, `^` are the
  existing operators, now defined over dimensioned values with the existing
  precedence (`^` tighter than `*`/`/`, so `9.8 * m / s^2` is
  `(9.8 * m) / (s^2)`).
- **Named derived units are ordinary bindings**: `let N = kg * m / s^2`,
  `let h = 3600.0 * s`.  **SI prefixes are a generated table of bindings**
  (`let km = 1000.0 * m`), not syntax.  These are library const bindings shipped
  by `si` (ADR 0028); they require top-level const `let` and imports (ADR 0027).
  This **resolves the earlier "named derived units and prefixes" open
  question**: they are library bindings, not language surface.
- **Dimensionless results are `real`**: a ratio of same-dimension values cancels
  to the group identity, i.e. plain `real`.
- **Conversion is linear and automatic within a dimension**: a unit is a
  positive scale factor relative to the base, and a value normalizes to base by
  multiplying by the ratio.  **Mixing different dimensions is a compile error**
  (`length + time` does not type).
- **Affine (offset) units are not dimensional units.**  Celsius and Fahrenheit
  differ from the temperature base (Kelvin) by an offset, not a scale, and once
  an offset is present multiplying or dividing quantities is ill-defined (an
  absolute temperature versus a difference).  An offset unit is handled by an
  explicit value-level conversion at ingestion that yields an absolute base-unit
  (Kelvin) quantity; it is never first-class inside dimensional arithmetic
  (matches the IIoT intent, Celsius in / Kelvin base, `iiot.md:37-38`).
- **Aggregates and keys**: `sum`/`min`/`max`/`mean` over a dimensioned column
  preserve its dimension; `count` is dimensionless `int`.  A dimensioned `real`
  column is not key-eligible (`real` is not, ADR 0014).  *Which* rollups are
  semantically valid (temperature is non-additive, energy additive) is the
  measure-semantics document's concern, not this ADR.

### 8.  Dimension aliases via generic `let`

A dimension alias is a generic `let` whose body is a dimension:

```
let speed[T] = (length / time)[T]
let accel[T] = speed[T] / time[T]
```

No `dimension` keyword is introduced; aliases sit at the same kind as base
dimensions and compose.  Aliases are **transparent** (expanded before
exponent-vector equality, so decidability is untouched), **fully applied** (no
partial application; the Haskell type-synonym rule), and **non-recursive**.
There is no `T: type` bound: the body's bracket already enforces a valid
backing, and a `T: backing` bound is a one-line upgrade when precision makes
multiple backings real.

`let` thus serves two kinds, disambiguated by the body: a **value binding** when
the body is a value (Decision 7, `let N = kg * m / s^2`) and a **type-level
alias** when the body is a dimension and the name carries `[T]` parameters (this
decision).  The general top-level `let` binding form is specified in ADR 0027.
Value-level generic `let`s are unnecessary: a unit is already combined through
its operand by `*` (`1.0 * m : length[real]` today; the operand-backing story
generalizes when precision adds backings, Decision 3).

### 9.  Precision leaves the core

Measurement precision is out of scope for M3's core and will arrive as a
**library extension of `real`**.  The bracket model of Decision 3 gives it a
natural home: a precision-carrying refinement of `real` substitutes into the
backing slot, so precision and dimension compose without either owning the
other.  The reserved `NxE` measured literal (`04-grammar.md:339`,
`06-expressions.md:119`) is deferred with precision.

### 10.  Formal backing

`formal/Mensura/Units/Dimension.lean` mechanizes Decisions 1 and 2: the seven
base dimensions, the dimension type as the free abelian group over them, its
commutative-group structure (well-definedness of `*`, `/`, and integer powers),
and its decidable equality (the mismatch decision procedure).  The seven base
dimensions are proved pairwise distinct, so the group is genuinely of rank seven.

The next formal targets in this area, which back the deferred checker
propagation per ADR 0021, are **dimensional-arithmetic soundness** (the
checker's `*`/`/`/`^` on dimensioned types match the group operations) and
**conversion correctness** (scale-factor normalization preserves the represented
quantity).  ADR 0027's "import as disjoint environment union" is a near-trivial
finite-map lemma and is not a priority formal target.

## Consequences

Positive:

- A physical-unit mismatch is a compile error backed by a decidable equality on
  a mechanized group, delivering pillar 4.
- Units are ordinary dimensioned constants: no unit syntax, no `NxE` literal, no
  `SI(...)` constructor, no juxtaposition rule.  The whole units surface is the
  existing operators plus library bindings.
- The language core stays tiny: seven intrinsic bindings; symbols, prefixes, and
  named derived units all live in `si` (ADR 0028).
- Dimension, numeric representation, and future per-column qualifiers compose
  orthogonally, undoing the `NxE` literal's coupling of dimension and precision.
- The `C`/`Qs` boundary is sharper: dimension is domain structure (type
  computation), refining ADR 0013.

Negative:

- Dimensioned columns require top-level const bindings and a module system
  (ADR 0027) plus a standard library (ADR 0028): a larger surface than units
  alone, and the reason this PR carries three ADRs.
- The type tracks dimension, not unit, so unit-labeling mistakes (km stored as
  m) are *not* caught: a deliberate divergence from F# units-of-measure, whose
  cost is accepted for automatic conversion.
- Dimensional inference threads a new computed attribute through every
  expression/pipeline rule that produces or combines numeric columns.
- The storage backend must decide how a dimensioned magnitude persists (a
  follow-up).

Neutral:

- `int` and plain `real` are unaffected: `real` is dimensionless, `int` is never
  dimensioned.  Existing programs keep typing.
- Dimension names are lowercase built-in type names; unit names/symbols are
  snake_case terms.
- Affine units live outside the dimension system as ingestion-time conversions,
  so dimensional arithmetic never sees an offset.
- The formal module is independent of the table algebra.

## Alternatives considered

1. **Dimension as a per-column `Qs` qualifier** (ADR 0013's original slot).
   Rejected: a dimension is part of what a value *is*, and dimensional
   arithmetic is type computation, not preserve/forfeit propagation.  Precision,
   which genuinely *is* a propagated per-column fact, keeps that slot.

2. **`real` refined by a concrete unit** (`real<m/s^2>`).  Rejected: it
   parameterizes by *unit* not *dimension*, so same-dimension columns in
   different units get distinct types needing conversion at assignment.

3. **A `@unit(...)` domain annotation on `real`.**  Rejected for the column type
   for the same reason as 2, and because it reads as an add-on fact rather than
   the column's domain.

4. **Keep precision (the `NxE` literal) in the M3 core.**  Rejected: precision
   is complex and separable, better as a library extension of `real`.

5. **Equality saturation for unit normalization** (an egraph).  Rejected: a free
   abelian group has a trivial canonical form (the sorted exponent vector), so
   canonicalize-and-compare is a few lines (`untracked/egg.md:76-79`).

6. **Constructor minting** (`time(1)`, or an ambient dimensionless-to-dimensioned
   cast).  Rejected: it is the escape hatch a dimensional type system exists to
   avoid.  The seven base units are language intrinsics instead (Decision 6).

7. **Track the specific unit at the type level** (F# units-of-measure: `m` and
   `km` as distinct types).  Rejected: it makes conversion explicit and
   non-free and multiplies types per unit.  Mensura tracks the dimension and
   normalizes magnitude to base, so conversion is automatic; the recorded cost
   is that unit-labeling mistakes are uncaught (Consequences).

8. **Juxtaposition for units** (`9.8 m/s^2`).  Rejected in favor of explicit `*`
   (`9.8 * m/s^2`): juxtaposition is already application, and once units are
   ordinary constants the special attachment reading and whitespace concerns add
   grammar for no gain.

9. **A `dimension` keyword for aliases.**  Rejected: aliases are generic `let`,
   keeping the lexer keyword-free and putting aliases at the dimension kind.

## Open questions

- **Persisting a dimensioned magnitude.**  The storage backend maps a
  dimensioned column to its base-unit magnitude in a `REAL` column with the
  dimension tracked at the type level, or carries companion metadata; the
  storage doc decides (`00-storage-backend.md:95`).
- **Backing-polymorphic unit constants.**  Whether unit constants become
  backing-polymorphic (`a * m : length[T]`) when precision adds a second
  backing, or stay `real`-backed; deferred to the precision document (Decision
  3).
- **LL(1) grammar for dimension expressions in type position.**
  `(length / time^2)[real]` adds an operator-precedence sub-grammar to the type
  grammar; the exact productions and their LL(1) proof are the language doc's
  job (`04-grammar.md`).
- **Dimensionless-but-distinct quantities** (angle/radian, ratios).  Whether any
  dimensionless quantities must stay distinct from bare `real` is deferred.

## Forward references

- The reserved surface it supersedes is in `docs/language/06-expressions.md`
  (the `NxE` literal, the juxtaposition note) and `docs/language/04-grammar.md`.
- Top-level const bindings, imports, and the intrinsic/library split are in
  `docs/decisions/0027-modules-and-imports.md`; the `si` library that ships the
  symbols, prefixes, and named derived units is
  `docs/decisions/0028-standard-library-si.md`.
- The `C`/`Qs` boundary this sharpens is
  `docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`; the scalar
  taxonomy whose open question it answers is
  `docs/decisions/0014-scalar-domain-taxonomy.md`; the casing rule it extends is
  `docs/language/05-naming-and-casing.md`.
- The formal-proof pipeline governing the Lean module and blueprint node is
  `docs/decisions/0021-formal-proof-pipeline.md`.
- The full language design doc (`docs/language/11-physical-units.md`), the
  checker/runtime realization, and the reconciliation edits listed under Status
  follow this ADR's acceptance.
