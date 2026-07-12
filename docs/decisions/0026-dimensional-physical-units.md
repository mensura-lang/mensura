# 0026: Dimensional physical units

## Status

Accepted.  Opens M3 (`ROADMAP.md`, "Physical units, precision, and measure
semantics") and realizes design pillar 4 ("keys and physical units are part of
the type") of `docs/language/00-overview.md`.  The formal backing lands with
this ADR in `formal/Mensura/Units/Dimension.lean` (blueprint chapter "Physical
dimensions"), per the ADR 0021 rule that a checker propagation rule ships only
when a theorem under `formal/` backs it.

This ADR fixes the *model* of dimensional units.  It deliberately does **not**
include: the full language design doc (`docs/language/11-physical-units.md`,
with the surface grammar, the typing rules, and a worked fleet example); the
lexer/parser/checker/runtime realization; or the reconciliation edits this
decision forces on `docs/language/06-expressions.md`,
`docs/language/04-grammar.md`, `docs/language/09-typing-reference.md`,
`docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`,
`docs/decisions/0014-scalar-domain-taxonomy.md`, and
`docs/toolkit/00-storage-backend.md`.  Those are follow-ups named at the end of
this ADR.

It answers ADR 0014's open question ("exactly how M3's dimensional units and
precision refine `real`") and revises ADR 0013's anticipation that physical
units are a per-column `Qs` qualifier (see Decision 4).

## Context

Existing tools leave the physical unit of a measurement to a column name, a
comment, or the programmer's memory, so adding gallons to liters or reading an
acceleration as a length is a silent runtime error.  Mensura's promise (pillar
4) is that a physical-unit mismatch is a compile error.  The fleet example
carries a temperature today as a bare `real` with the unit baked into the field
name (`kelvin: real`, `docs/examples/fleet-monitoring.mensura:64`); M3 is the
milestone that turns that placeholder into a checked type, and the IIoT driving
application also needs acceleration (`length / time^2`), energy
(`mass * length^2 / time^2`), and other derived quantities.

Several commitments were reserved ahead of this ADR:

- `real` is the refinement target; the closed scalar taxonomy of ADR 0014 need
  not be reopened (0014:144-145).
- A physical unit attaches to a *literal* by juxtaposition (`... m`), with no
  `SI(...)` constructor, and the unit-expression grammar (`m/s^2`) uses
  ordinary operator precedence with no whitespace significance
  (`06-expressions.md:341-346`).
- ADR 0013 anticipated units as a per-column qualifier in `Qs` at `column`
  scope, alongside totality and PII taint (0013:63, 139).

The reserved `NxE` measured literal (`10x3`) fused two things into one token: a
dimension *and* a measurement precision (`06-expressions.md:119-124`).  That
coupling is exactly the mistake this ADR avoids.  Precision is a hard,
separable concern, and it is better delivered later as a library that extends
`real` than baked into the core literal now.  So M3 starts with *dimensions
only*; precision and measure semantics (`@additive`/`@semiadditive`/`@foldable`)
are separate, later documents.

The intended representation is settled evidence: dimensions form a free abelian
group, and canonicalization is "sort the exponent vector," a trivial canonical
form (`untracked/egg.md:76-79`).  No dimensional analysis exists in `formal/`
today; the algebra there is about tables, keys, and multisets only, with column
domains left abstract (`sigma : H -> Type`).  This ADR's formal module is new
territory.

## Decision

### 1.  The seven base dimensions

The SI base quantities map to seven type-level base dimensions:

| SI base quantity     | Mensura base dimension |
| -------------------- | ---------------------- |
| time                 | `time`                 |
| length               | `length`               |
| mass                 | `mass`                 |
| electric current     | `current`              |
| temperature          | `temperature`          |
| amount of substance  | `amount`               |
| luminous intensity   | `luminosity`           |

`luminosity` names luminous intensity.  Astrophysics defines luminosity
slightly differently (total radiant power), but as a base-type name it reads
unambiguously as "light power," and it already appears with this meaning in the
IIoT background material.

### 2.  A dimension is an element of the free abelian group over the seven

A physical dimension is an integer exponent vector over the seven base
dimensions, i.e. an element of the free abelian group of rank seven, written
multiplicatively: dimensions *multiply* (`length * length = length^2`) while
their exponent vectors *add*.  The group identity (all exponents zero) is the
**dimensionless** quantity.  Derived dimensions are products, quotients, and
integer powers of the seven (`length / time^2`, `mass * length^2 / time^2`).

The canonical form is the exponent vector itself (equivalently, the sorted
vector); dimension equality is vector equality and is decidable.  Decidable
equality is what makes "dimension mismatch is a compile error" a decision
procedure rather than a heuristic.  Equality saturation is unnecessary
machinery for a group with a trivial canonical form (Alternatives, 5).

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
the dimensionless quantity (the group identity).  `int` is never dimensioned;
it remains counts and identities (ADR 0014).

This still "attaches dimensional units to `real`" as ADR 0014 anticipated:
`real` is the backing, not replaced, so the closed scalar taxonomy is not
reopened.  The Rust representation (a parametric `Quantity { dimension, backing }`
domain, or a `Dimension` payload on the numeric domain) is a follow-up
implementation choice; this ADR fixes the model, not the encoding.

### 4.  Dimension lives in the content `C`, not the qualifier row `Qs`

A dimension is part of a column's **domain**: it is *what the data is*, so under
the ADR 0013 boundary ("`C` is structure, `Qs` is propagated fact") it belongs
in `C`.  This **revises ADR 0013's anticipation** that units are a per-column
`Qs` qualifier alongside totality (0013:63, 139).

The reason is that dimensional arithmetic is genuine *type computation*, not
fact propagation.  The result dimension of `a * b` is a function of the operand
dimensions (`length * length = length^2`); the result dimension of `sum b.x` is
the dimension of `x`; adding two quantities requires their dimensions to be
*equal* or the program is rejected.  These are domain-inference rules of the
same kind as "what is the type of this expression," which live in `C`, not the
preserve/forfeit propagation rules that characterize a `Qs` qualifier.

This draws a clean line for the whole M3 family and tightens ADR 0013 rather
than contradicting it (0013 predates the dimension-as-type surface):

- **dimension** = domain structure in `C` (this ADR);
- **precision** (future library) = an extension of the *backing* `real`, which
  may surface as a refined backing type in the bracket, a per-column `Qs`
  qualifier (0013's original slot), or both;
- **measure semantics** (`@additive` etc., later) = per-column annotations gating
  rollups.

### 5.  Surface syntax

- A **dimension expression** is built from the seven base names with `*`, `/`,
  and `^<int>` under ordinary operator precedence, no whitespace significance
  (as reserved in `06-expressions.md:341-346`).  A `[backing]` bracket applies
  to the *whole* dimension expression: `(length / time^2)[real]`, not per-atom.
  Bare `real` is the dimensionless quantity.
- A dimensioned **literal** attaches a concrete unit to an ordinary numeric
  literal by **juxtaposition** (`9.8 m/s^2`), with **no `SI(...)` constructor**.
  The `NxE` literal is *not* used here; it is deferred with precision (Decision
  6).
- **Conversion is automatic within a single dimension**: a value written in one
  unit of a dimension normalizes to that dimension's base unit.  **Mixing
  different dimensions is a compile error by construction** (`length + time`
  does not type).

### 6.  Precision leaves the core

Measurement precision is out of scope for M3's core and will arrive as a
**library extension of `real`**.  The bracket model of Decision 3 gives it a
natural home: a precision-carrying refinement of `real` substitutes into the
backing slot, so precision and dimension compose without either owning the
other.  The reserved `NxE` measured literal (`04-grammar.md:339`,
`06-expressions.md:119`) is deferred with precision.  Whether precision is
realized as a refined backing type, a `Qs` qualifier, or both is left to the
future precision document; this ADR only fixes that dimension does not own it.

### 7.  Formal backing

`formal/Mensura/Units/Dimension.lean` mechanizes Decisions 1 and 2: the seven
base dimensions, the dimension type as the free abelian group over them, its
commutative-group structure (the well-definedness of `*`, `/`, and integer
powers), and its decidable equality (the mismatch decision procedure).  The
seven base dimensions are proved pairwise distinct, so the group is genuinely of
rank seven and no two axes collapse.  The module is standalone; it does not yet
instantiate the abstract column-domain slot (`sigma : H -> Type`) of
`formal/Mensura/Core/Defs.lean`.  A blueprint chapter records the nodes.

## Consequences

Positive:

- A physical-unit mismatch becomes a compile error backed by a decidable
  equality on a mechanized group, delivering pillar 4.
- Dimension, numeric representation, and future per-column qualifiers compose
  orthogonally instead of fusing, undoing the `NxE` literal's coupling of
  dimension and precision.
- The `C`/`Qs` boundary gets sharper: dimension is recognized as domain
  structure (type computation), which distinguishes it from the propagated
  facts and refines ADR 0013.
- M3 is now scoped to one tractable concept; precision and measure semantics
  are cleanly deferred.

Negative:

- ADR 0013's forward reference to units-as-qualifier is now wrong and must be
  amended (a follow-up edit).  The prediction was reasonable before the
  dimension-as-type surface existed.
- Dimensional inference threads a new computed attribute through every
  expression and pipeline rule that produces or combines numeric columns
  (`*`, `/`, `^`, `sum`, `+`), which the checker did not carry before.
- The parametric `D[backing]` type is more machinery than a flat refinement of
  `real`, and the storage backend must decide how a dimensioned magnitude
  persists (a follow-up).

Neutral:

- `int` and plain `real` are unaffected: `real` is dimensionless, `int` is never
  dimensioned.  Existing programs keep typing.
- The formal module is independent of the table algebra; it neither changes nor
  depends on the existing `formal/` development.

## Alternatives considered

1. **Dimension as a per-column `Qs` qualifier** (ADR 0013's original slot).
   Rejected: a dimension is part of what a value *is*, and dimensional
   arithmetic is type computation, not the preserve/forfeit propagation a
   qualifier models.  Precision, which genuinely *is* a propagated per-column
   fact, keeps that slot.

2. **`real` refined by a concrete unit** (`real<m/s^2>`).  Rejected: it
   parameterizes by *unit* rather than *dimension*, so two columns of the same
   dimension in different units (`real<m>`, `real<km>`) get distinct types and
   need conversion at assignment.  Dimension-as-type unifies them and pushes
   unit reconciliation to the value level, where conversion belongs.

3. **A `@unit(...)` domain annotation on `real`** (consistent with the deferred
   `@domain(...)` family).  Rejected for the column type for the same reason as
   2 (it names a unit, not a dimension) and because it reads as an add-on fact
   rather than the column's domain.

4. **Keep precision (the `NxE` literal) in the M3 core.**  Rejected by the
   author: precision is complex and separable, better delivered as a library
   extension of `real` than fused into the dimensioned literal.  Decision 6
   records the redirection.

5. **Equality saturation for unit normalization** (an egraph, e.g. `egg`).
   Rejected: a free abelian group has a trivial canonical form (the sorted
   exponent vector), so canonicalize-and-compare is a few lines and equality
   saturation is unjustified heavy machinery (`untracked/egg.md:76-79`).

## Open questions

- **Affine conversions.**  Temperature units are affine, not linear: Celsius to
  Kelvin adds an offset, and once an offset is present, multiplying or dividing
  affine-unit quantities is ill-defined (an absolute temperature versus a
  temperature difference).  How conversion handles affine units, and whether
  affine units are barred from derived-dimension arithmetic, is settled in the
  language design doc, not here.
- **Default backing.**  Whether bare `temperature` sugars to `temperature[real]`
  (terse, keeps the fleet example readable) or the bracket is mandatory (pillar
  6, "no defaults that hide assumptions").  Recommended: default to `real` while
  it is the only backing; revisit when a second backing exists.
- **Named derived units and prefixes.**  Whether `newton`, `joule`, `pascal`
  and SI prefixes (`kilo`, `milli`) are surface sugar over base-dimension
  expressions, and how they are declared, is deferred to the language doc.
- **Persisting a dimensioned magnitude.**  The storage backend maps a
  dimensioned column to a base-unit magnitude in a `REAL` column with the
  dimension tracked at the type level, or carries companion metadata; the
  storage doc decides (`00-storage-backend.md:95`).
- **Dimensionless-but-distinct quantities** (angle/radian, ratios).  Whether any
  dimensionless quantities need to stay distinct from bare `real` is deferred.

## Forward references

- The reserved surface and the deferral notes are in
  `docs/language/06-expressions.md` (the `NxE` literal, juxtaposition) and
  `docs/language/04-grammar.md`.
- The `C`/`Qs` boundary this ADR sharpens is
  `docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`; the scalar
  taxonomy whose open question it answers is
  `docs/decisions/0014-scalar-domain-taxonomy.md`.
- The formal-proof pipeline that governs the Lean module and its blueprint node
  is `docs/decisions/0021-formal-proof-pipeline.md`.
- The full language design doc (`docs/language/11-physical-units.md`), the
  checker/runtime realization, and the reconciliation edits listed under Status
  follow this ADR's acceptance.
