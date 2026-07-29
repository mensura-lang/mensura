# Physical units: dimensional quantities

This document specifies the surface and the typing rules for dimensional
physical units: the realization of design pillar 4 ("keys and physical
units are part of the type", `00-overview.md`).  The model is decided in
`docs/decisions/0026-dimensional-physical-units.md`; the module machinery
that ships the unit library is `12-modules-and-imports.md` (ADR 0027); the
`si` library itself is `docs/decisions/0028-standard-library-si.md`.

The one-sentence version: **a measured column's type carries its physical
dimension, dimensional arithmetic is checked at compile time, and a
dimension mismatch is a compile error**.

## Dimensions

A **dimension** is an element of the free abelian group over the seven SI
base dimensions:

| SI base quantity     | Base dimension | Base unit (intrinsic) |
| -------------------- | -------------- | --------------------- |
| time                 | `time`         | `second`              |
| length               | `length`       | `meter`               |
| mass                 | `mass`         | `kilogram`            |
| electric current     | `current`      | `ampere`              |
| temperature          | `temperature`  | `kelvin`              |
| amount of substance  | `amount`       | `mole`                |
| luminous intensity   | `luminosity`   | `candela`             |

Concretely a dimension is an integer exponent vector over the seven,
written multiplicatively: dimensions multiply while their exponent vectors
add, and the group identity (all exponents zero) is the **dimensionless**
quantity.  Derived dimensions are products, quotients, and integer powers
of the seven: acceleration is `length / time^2`, energy is
`mass * length^2 / time^2`.  Dimension equality is exponent-vector
equality, so the mismatch check is a decision procedure, not a heuristic.
The group is mechanized in `formal/Mensura/Units/Dimension.lean`
(ADR 0026, Decision 10).

Dimension names are **lowercase built-in type names**, the same fixed
vocabulary category as `int` and `real` (`05-naming-and-casing.md`).  They
are not user-declared PascalCase types.

## Dimensioned types

The type of a measured column is a **dimension applied to a backing
scalar type**, written with the uniform type-level application bracket:

```mensura
attr* {
  temperature: temperature[real]
  vibration:   (length / time^2)[real]
}
```

`real` is the only backing today; the bracket keeps the dimension
orthogonal to the numeric representation so a future precision-carrying
refinement of `real` substitutes into the same slot (ADR 0026, Decision
9).  Bare `real` is the dimensionless quantity.  `int` is never
dimensioned (ADR 0014: `int` is counts and identities).  A bare dimension
is not a type: `x: temperature` is rejected with a pointer to
`temperature[real]`.

**The type tracks the dimension, not the unit.**  A value's magnitude is
always normalized to the coherent SI base unit (meters, seconds, kelvin,
...), so `meter` and `kilometer` produce values of one type,
`length[real]`, and conversion within a dimension is automatic and free.
The deliberate cost (ADR 0026, Decision 3): the compiler catches
*dimension* mismatches, not unit-labeling mistakes; "kilometers stored in
a column documented as meters" is an interop concern, not a typed fact.

A dimensioned column follows `real`'s scalar-domain properties (ADR
0014): numeric and orderable, but not equatable and not key-eligible.

### Type-level grammar

Type positions accept a **type-level expression**:

```ebnf
type       = tl_expr [ "?" ] ;
tl_expr    = tl_term { ( "*" | "/" ) tl_term } ;
tl_term    = tl_factor [ "^" [ "-" ] int ] ;
tl_factor  = ident [ "[" ident "]" ]
           | "(" tl_expr ")" "[" ident "]" ;
```

A lone `ident` is today's form (a primitive, an `enum`, or a unit
reference); an `ident` or parenthesized dimension expression followed by
`[backing]` is a dimensioned type; the bracket argument is a single
identifier (`real`, or an alias parameter inside an alias body).  `*`,
`/`, and `^` have their expression-level precedences (`^` tighter);
exponents are integer literals, optionally negated.

This sub-grammar is LL(1) in every type position; the argument, including
the one genuinely hazardous FOLLOW set, lives with the grammar
(`04-grammar.md`, "Why the type grammar is LL(1)").

### Dimension aliases

A dimension alias is a generic top-level `let` whose body is a type-level
expression over its parameter (ADR 0026, Decision 8):

```mensura
let speed[T] { (length / time)[T] }
let accel[T] { speed[T] / time[T] }
```

Aliases are **transparent** (expanded to exponent vectors before
equality, so decidability is untouched), **fully applied** (no partial
application), and **non-recursive** (a cycle is a compile error).  An
alias name is a lowercase type name, like the base dimensions it
abbreviates.  `speed[real]` and `(length / time)[real]` are the same
type.

## Dimensioned values

There is **no constructor form** and no dimensionless-to-dimensioned
cast: `time(1)` does not exist, because such a cast is the escape hatch a
dimensional type system exists to make unnecessary.  Exactly two ways a
dimensioned value comes to exist (ADR 0026, Decision 6):

1. **Arithmetic on the intrinsic base units.**  The seven base units are
   intrinsic value bindings, always in scope, each of magnitude one:
   `second : time[real]`, `meter : length[real]`, and so on.  A unit is
   an ordinary positive dimensioned constant, and value syntax is
   explicit multiplication with the existing operators:

   ```mensura
   9.8 * meter / second^2
   ```

   Named derived units and prefixed units are ordinary `let` bindings
   shipped by the `si` library (`import si`, then `si.km`, `si.newton`;
   ADR 0028).  There is no juxtaposition attachment (`9.8 m` is not a
   value) and no `NxE` measured literal; both reserved surfaces are
   superseded (ADR 0026, Context).

2. **Ingestion through a declared column type.**  A column declared
   `temperature[real]` yields dimensioned values when read in a pipeline.

## Typing rules

For operands whose domains are dimensionless `real` or a dimensioned
`D[real]` (write `dim(x)` for the exponent vector, with `dim = 0` for
bare `real`):

- **`+` and `-`** require equal domains, so equal dimensions:
  `meter + second` and `meter + 1.0` are compile errors.
- **`*`** multiplies dimensions: `dim(a * b) = dim(a) + dim(b)`.
- **`/`** divides dimensions: `dim(a / b) = dim(a) - dim(b)`.  A
  same-dimension ratio cancels to the group identity, and **a
  dimensionless result is bare `real`**.
- **`^`** on a dimensioned base takes an **integer literal** exponent
  (optionally negated): `second^2`, `meter^-1`.  The exponent must be a
  literal because the result dimension is computed at compile time.  A
  zero exponent collapses to dimensionless `real`.  On dimensionless
  operands `^` keeps its existing rule (matching numeric domains), so
  `x ^ 2.0` on reals and `n ^ 2` on ints are unchanged and `real ^ int`
  remains rejected outside the dimensioned case.  This asymmetry is
  deliberate: the literal-exponent form exists exactly where the type
  system needs to compute a dimension.
- **Unary `-`** preserves the domain, dimension included.
- **Comparisons** (`< <= > >=`) require equal orderable domains, so
  cross-dimension comparison is rejected; `==`/`!=` stay undefined on any
  `real`-backed domain (ADR 0014).
- **Aggregates**: `sum`, `min`, and `max` over a dimensioned bag preserve
  its dimension; `count` is dimensionless `int`.  Which rollups are
  *semantically* valid (temperature is non-additive) is the future
  measure-semantics document's concern.
- **`to_real`** converts `int` to dimensionless `real`, as before.
- **Keys**: a dimensioned column is not key-eligible, exactly as `real`
  is not.

Mixing `int` with any `real`-backed domain remains a type error (no
coercion, ADR 0014).

## Conversion and normalization

A unit is a positive scale factor relative to its dimension's base unit,
and a value normalizes to base at the moment it is computed:
`2.0 * si.km` *is* `2000.0` at dimension `length`.  Conversion within a
dimension is therefore linear, automatic, and invisible; there is no
conversion function to call.  Storage persists the base-unit magnitude in
an ordinary `REAL` column with the dimension tracked only at the type
level (`docs/toolkit/00-storage-backend.md`).

**Affine (offset) units are not dimensional units** (ADR 0026, Decision
7).  Celsius and Fahrenheit differ from kelvin by an offset, not a scale,
and offset quantities do not multiply or divide meaningfully.  An offset
unit is handled by an explicit value-level conversion at ingestion that
yields an absolute base-unit quantity; it is never first-class inside
dimensional arithmetic.  Likewise epoch timestamps are affine and stay
outside the dimension system; only durations are `time[real]`.

## Worked example

The fleet's readings store carries a dimensioned temperature, and the
rollup preserves the dimension:

```mensura
import si

unit Machine {
  id: string
}

store readings {
  unit { Machine }
  attr* {
    temperature: temperature[real]
  }
}

view machine_temperature {
  readings |> assume { complete }
           |> map_bags |_, b| (.max_temperature = bag.max b.temperature)
}
```

`bag.max b.temperature` is `temperature[real]`; adding it to a duration, or
comparing it to a bare `3.0`, is a compile error.  A threshold constant
is written in whatever unit is convenient and normalizes automatically:
`let limit { 350.0 * kelvin }`.

## Deferred

- **Precision** and the `NxE` measured literal: a future library
  extension of `real` that substitutes into the backing slot (ADR 0026,
  Decision 9).
- **Measure semantics** (`@additive`, `@semiadditive`, `@foldable`):
  a later document gating window rollups.
- **Backing-polymorphic unit constants** (`a * meter : length[T]`):
  deferred to the precision document.
- **Dimensionless-but-distinct quantities** (angle, ratios): whether any
  must stay distinct from bare `real` is open (ADR 0026).

## Forward references

- The decision record: `docs/decisions/0026-dimensional-physical-units.md`.
- Modules, imports, and top-level `let`: `12-modules-and-imports.md`
  (ADR 0027); the `si` library: ADR 0028.
- The grammar and its LL(1) proof: `04-grammar.md`.
- Casing of dimension names, aliases, and unit terms:
  `05-naming-and-casing.md`.
- The consolidated typing rules: `09-typing-reference.md`, section 5.
- Storage mapping: `docs/toolkit/00-storage-backend.md`.
- The mechanized group: `formal/Mensura/Units/Dimension.lean`; the
  formal-proof pipeline: `docs/decisions/0021-formal-proof-pipeline.md`.
