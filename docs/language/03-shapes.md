# Shapes

A shape is a named, possibly parameterised description of a table's
structure: the unit being tabulated and the attributes that accompany
each observation.  Shapes are the language form for talking about
table types in function signatures, transform inputs and outputs, and
contracts that multiple stores can claim.

This document defines what a shape is and how it is declared, the
two-kind parameter system (unit parameters in `<...>`, name parameters
in `(...)`), interpolation of name parameters into attribute names
using backticks, and the `implements` clause on stores and on
function signatures.

The syntax shown is preliminary, like the rest of the language docs at
this stage; the design content is not.

## What a shape is

A unit declares an identity discipline.  A store declares one
concrete tabulation of a unit.  A shape declares an *abstract* contract
about the structure of a tabulation, without committing to storage,
policy, or domain resolution.

Stores claim conformance to shapes with `implements`.  Functions
accept and return values typed by shape names.  The same shape may be
implemented by many stores; one store may implement many shapes.

```
shape PersonRecord {
  unit { Person }
  const { admission: date }
}

store Students implements PersonRecord {
  unit { Person }
  const { admission: date }
}

store Faculty implements PersonRecord {
  unit { Person }
  const { admission: date }
  var   { rank: enum("assistant", "associate", "full") }
}

fn count(t: PersonRecord) -> number { ... }

let n_students = count(Students);
let n_faculty  = count(Faculty);
```

Both `Students` and `Faculty` implement `PersonRecord`; both can be
passed to a function whose argument type is `PersonRecord`.  `Faculty`
has an additional attribute (`rank`) that the shape does not require;
extra attributes are fine.

## Shape declaration

A shape declaration consists of a name, optional unit and name
parameters, a `unit { ... }` clause, and any number of `const` and
`var` blocks.

```
shape ShapeName<UnitParams>(NameParams) {
  unit { ... }
  const { ... }
  var   { ... }
}
```

Both bracket pairs are optional.  A shape with no parameters reads
exactly like the `PersonRecord` example above.

The body of a shape carries *structure only*: which unit, which
attributes, and (per attribute) whether it is `const` or `var`.  A
shape does not carry domain resolution, policy annotations, an API
surface, or any storage commitment; those are store concerns.  A
program containing only shape declarations is well-typed but observes
no data.

## Parameters

Shapes admit two kinds of parameters.

### Unit parameters: `<U>`

Written in angle brackets.  A unit parameter introduces a *type-level*
variable that must be filled in by a unit name when the shape is used.

```
shape Tabular<U> {
  unit { U }
}
```

`Tabular<U>` is "any table of any unit, with no required attributes."
A function that does not care which unit it operates on can use it as
its argument type:

```
fn count<U>(t: Tabular<U>) -> number { ... }
```

### Name parameters: `(col: Name)`

Written in parentheses.  A name parameter introduces a *compile-time
value* of type `Name` that is filled in when the shape is used.
Names can be interpolated into attribute names within the shape body.

```
shape NumericCol<U>(col: Name) {
  unit { U }
  const { `{col}`: number }
}
```

`NumericCol<Person>(height)` is "a table of `Person` with at least a
`const` attribute named `height` of type `number`."

A shape may have any number of unit parameters and any number of name
parameters.  Other parameter kinds (numbers, types, predicates) are
not in this version of the language.

### Why two brackets

The split is deliberate.  Angle brackets carry type-level information
(units); parentheses carry compile-time values (names).  At the use
site, `NumericCol<Person>(height)` reads "the shape `NumericCol`
applied to the unit `Person` and the name `height`."  The reader can
tell at a glance which slot is which.

## Name interpolation

Name parameters can be interpolated into attribute-name positions
using **backticks**:

```
shape NormalizedCol<U>(col: Name) {
  unit { U }
  const {
    `{col}`:    number
    `{col}_z`:  number
  }
}
```

The same `` `{...}` `` syntax works in expression-side positions
within transform bodies (mutate keys, rename targets, etc.).
Backticks are required to disambiguate "this is a parametric
identifier" from "this is a fixed identifier"; without them,
`{col}_z` would be a literal identifier.

Interpolation is resolved at compile time, when shape parameters are
bound.  Names cannot be derived from runtime values; if a transform
genuinely needs runtime column names, that is a different feature,
not yet in the language.

Only attribute-name positions support interpolation in this version.
Interpolating into unit names, shape names, or other identifier
positions is a much larger feature and is not yet on the table.

## The `implements` clause on stores

A store declares the shapes it conforms to using `implements`.  The
clause may list one or more shapes, separated by commas, with their
parameter values supplied:

```
store Students implements PersonRecord, NumericCol<Person>(height) {
  unit { Person }
  const {
    admission: date
    height:    number
  }
}
```

Each `implements` entry is checked at the store declaration site,
independently of the others.  After substituting parameter values,
the compiler verifies that:

- The store's unit equals the shape's unit.
- Every attribute required by the shape is present in the store, with
  the same name (after interpolation), the same type, and in the same
  block (`const` or `var`).
- The store may have *additional* attributes beyond what the shape
  requires.

If any check fails, the declaration is rejected with a diagnostic that
names the missing or mismatched attribute.  The diagnostic mentions the
substituted parameter values so the reader can see which interpolation
produced the expected name.

The `implements` clause and the `domain` block of a store are
independent: shape conformance checks structure, the `domain` block
resolves foreign keys.  Both are checked, neither implies the other.

## `implements` in function signatures

A function or transform parameter is typed by referring to a shape.
Unit and name parameters of the shape become generic parameters of the
function:

```
fn count<U>(t: Tabular<U>) -> number { ... }

fn normalize<U, col: Name>(
  t: NumericCol<U>(col)
) -> NormalizedCol<U>(col) {
  mutate { `{col}_z` = (t[col] - mean(t[col])) / sd(t[col]) }
}
```

Function generics are listed once, in `<...>` after the function name,
with each parameter's kind made explicit by its annotation: a unit
parameter is bare (`U`); a name parameter is annotated as `col: Name`.
At the call site, generic arguments are supplied positionally:

```
let standardised = normalize<Person, height>(Students);
```

The compiler resolves the parameters and verifies that `Students`
implements `NumericCol<Person>(height)`.  If it does, the call is
well-typed and `standardised` has type `NormalizedCol<Person>(height)`.

The angle-bracket-and-parentheses split that shape *use sites* exhibit
(`NumericCol<U>(col)`) does not extend to the function generics list:
function generics mix kinds in a single `<...>`.  The split is a
property of how shapes are *applied*, not how function generics are
*declared*.

## What shapes do not contain

A shape declaration cannot contain:

- **A `domain` block.**  Foreign-key resolution is per-store.
- **Policy annotations** (`@audited`, `@versioned`, `@auto`,
  `@allowcreate`).  These attach to store attributes.
- **API surface** (`endpoint`, `auth`).  These belong on the store
  and are M4 concerns.
- **Storage commitment.**  A shape is contract, not data.

A shape *does* contain:

- A unit (concrete or unit-parameterised).
- Const and var attribute blocks.
- Optional unit and name parameters.

Nothing else.

## A known cost

In this version, the result type of a function is exactly what the
function declares.  If `normalize` declares its return type as
`NormalizedCol<U>(col)`, the value cannot be passed to a function
expecting `NumericCol<U>(col)`, even though structurally a
`NormalizedCol` already contains everything a `NumericCol` requires.

The reason is that **sub-shape relationships** (the rule "every
NormalizedCol is also a NumericCol") are not in this version.
Adding them is a small extension, but it is a deliberate choice to
state contract relationships explicitly when they exist.  The
practical cost: long pipelines may need to declare and maintain a
chain of intermediate shapes.

A document on sub-shapes will land when the cost becomes concrete.

## Worked example

Putting it all together with the college example:

```
unit Person {
  id: string
}

shape PersonRecord {
  unit { Person }
  const { admission: date }
}

shape NumericCol<U>(col: Name) {
  unit { U }
  const { `{col}`: number }
}

shape NormalizedCol<U>(col: Name) {
  unit { U }
  const {
    `{col}`:    number
    `{col}_z`:  number
  }
}

store Students implements PersonRecord, NumericCol<Person>(height) {
  unit { Person }
  const {
    admission: date
    height:    number
  }
}

fn normalize<U, col: Name>(
  t: NumericCol<U>(col)
) -> NormalizedCol<U>(col) {
  mutate { `{col}_z` = (t[col] - mean(t[col])) / sd(t[col]) }
}

let standardised = normalize<Person, height>(Students);
// standardised has type NormalizedCol<Person>(height)
```

`Students` is a basic store of `Person`, claiming conformance to two
shapes: `PersonRecord` (the project-domain contract) and
`NumericCol<Person>(height)` (the structural contract demanded by
`normalize`).  Both claims are checked at the store declaration site.

The transform `normalize` is generic in both the unit and the column
name.  The same definition handles `normalize<Person, height>`,
`normalize<Person, weight>`, and `normalize<Course, credits>`
without modification.

## Forward references and open questions

- **Marker shapes.**  Shapes with no structural body, used as
  user-defined property tags (Independent, Exhaustive, ...).  The
  same `implements` machinery becomes the carrier of table
  properties.  Treated in its own document; supersedes the qualifier
  ADR when written.
- **Sub-shape relationships.**  See "A known cost" above.  The first
  obvious extension once the explicit cost becomes painful.
- **Associated types.**  Shape parameters are inputs only; they
  cannot, at present, *compute* derived shape information.  When this
  matters, it will get its own document.
- **Schema arithmetic.**  Operators on shapes (`+`, `\`, ...) for
  expressing schema deltas in function signatures: deferred.
- **Coherence and orphan rules.**  Whether one program may declare
  another program's store implements its own shape is unsettled.
- **"Any table" type.**  A function genuinely working on any table
  (e.g.  `count_rows`) cannot be written without conformance to a
  `Tabular<U>` shape on every relevant store.  Whether this remains
  acceptable is open.
- **Attribute identity** (cross-shape, cross-store).  Still open;
  important for `bind` and `join`.
