# Shapes

A shape is a named, possibly parameterised description of a table's
structure: the unit being tabulated and the attributes that accompany
each observation.  Shapes are the language form for talking about
table types in function signatures, transform inputs and outputs, and
contracts that multiple stores can claim.

This document defines what a shape is and how it is declared, the
parameter system (a single annotated list whose parameters may be units
(`Unit`) or compile-time values such as `string`), interpolation of
`string` parameters into attribute names using backticks, and the
conformance clause (`:`) on stores and on function signatures.

The syntax shown is preliminary, like the rest of the language docs at
this stage; the design content is not.

## What a shape is

A unit declares an identity discipline.  A store declares one
concrete tabulation of a unit.  A shape declares an *abstract* contract
about the structure of a tabulation, without committing to storage,
policy, or domain resolution.

Stores claim conformance to shapes with a `:` clause.  Functions
accept and return values typed by shape names.  The same shape may be
claimed by many stores; one store may claim many shapes.

```
shape PersonRecord {
  unit { Person }
  attr { admission: date }
}

store students : PersonRecord {
  unit { Person }
  attr { admission: date }
}

enum Rank {
  "assistant"
  "associate"
  "full"
}

store faculty : PersonRecord {
  unit { Person }
  attr {
    admission: date
    rank:      Rank
  }
}

let count = |t: PersonRecord| : real { ... }

let n_students = count(students);
let n_faculty  = count(faculty);
```

Both `students` and `faculty` conform to `PersonRecord`; both can be
passed to a function whose argument type is `PersonRecord`.  `faculty`
has an additional attribute (`rank`) that the shape does not require;
extra attributes are fine.

The `:` reads the same way it does everywhere else in the language:
the thing on the left has the type on the right.  An attribute
(`admission: date`), a lambda parameter (`t: PersonRecord`), and a
store (`students : PersonRecord`) all use the one colon for "has this
type."  A store is a structural subtype of every shape it claims: it
has everything the shape requires, possibly more.

Attribute types may carry the `?` optional marker like any other type
(`docs/decisions/0010-attribute-totality.md`).  Totality
participates in the subtyping: a total attribute satisfies an optional
requirement, since a known value is an acceptable optional one, but an
optional store attribute does *not* satisfy a total requirement, because
the shape promises the value is always known.

## Shape declaration

A shape declaration consists of a name, an optional parameter list, an
optional `unit { ... }` clause, and one or more `attr` blocks.

```
shape ShapeName[p1: T1, p2: T2, ...] {
  unit { ... }        // optional
  attr { ... }
}
```

Both the parameter list and the `unit` clause are optional.  A shape
with neither reads as a pure attribute contract; a shape with no
parameters reads exactly like the `PersonRecord` example above.

The body of a shape carries *structure only*: which unit and which
attributes, written in exactly the same attribute language as a store
body (`02-stores.md`).  A shape does not carry domain resolution,
policy annotations, an API surface, or any storage commitment; those
are store concerns.  A program containing only shape
declarations is well-typed but observes no data.

Because the attribute language is the store's, a shape also writes
`attr` and `attr*` blocks with the same meaning
(`docs/decisions/0022-observations-as-bags-declared-store-cardinality.md`,
amending `docs/decisions/0012-view-hosting.md`): a shape claim
constrains **cardinality**, not only content.  A shape with no `attr*`
block requires the conforming table to be `singletons` (the strict
reading, including a unit-only marker shape); an `attr*` shape requires
its columns to be bag-valued, which in the current uniform-store scope
means a `bag` table.  Like a store, a shape does not mix the two block
forms (rejected as not yet supported).

## The unit clause (optional)

A shape's `unit { ... }` clause says which unit a conforming table must
tabulate.  It has three forms:

- **Concrete:** `unit { Person }` requires the conforming store to
  tabulate `Person`.
- **Parameterised:** `unit { U }`, where `U` is a `Unit` parameter,
  requires the store's unit to equal the unit supplied for `U` at the
  use site.
- **Omitted:** a shape with no `unit` clause is *unit-agnostic*; any
  store conforms regardless of its unit, so long as it carries the
  required attributes.

A store always has exactly one unit, so for store conformance the unit
either must match (concrete or parameterised) or is simply not
constrained (omitted).  Unit-agnostic shapes are how one structural
contract applies across different units.

## Parameters

A shape takes a single parameter list.  Every parameter is written
`name: T`: a name, a colon, and the type of the argument it stands for.

- **`Unit`** is the kind of units.  A `Unit` parameter is a type-level
  variable filled in by a unit name when the shape is used; it is
  typically threaded into the `unit { U }` clause.
- **A primitive type** (`string`, ...) makes the parameter a
  compile-time value of that type.  A `string` parameter can be
  interpolated into attribute names within the shape body.

```
shape NumericCol[U: Unit, col: string] {
  unit { U }
  attr { `{col}`: real }
}

shape Ageable[date_field: string] {
  attr { `{date_field}`: date }
}
```

`NumericCol[Person, "height"]` is "a table of `Person` with at least an
attribute named `height` of type `real`."  `Ageable` is
unit-agnostic: `Ageable["birthdate"]` is "any table with an attribute
named `birthdate` of type `date`," whatever its unit.

The list is a **telescope**: parameters are positional, and a later
parameter (or the shape body) may refer to an earlier one.  A `Unit`
parameter therefore comes before the table parameter or `string`
parameter that depends on it, simply by being written first.

The annotation is always explicit; there is no privileged default.
This keeps one uniform form for every parameter and leaves room for
further value types (numbers, predicates) to be used as parameters
later, with no new syntax.  `string` is the value type consumed today
(by name interpolation); `Unit` feeds the `unit` clause.

At a **use site** arguments are supplied positionally, without
annotations.  A `Unit` argument is a bare unit name (`Person`); a
`string` argument is a string literal (`"height"`).  The bare-versus-
quoted distinction shows at a glance which argument is a unit and which
is a name.  A shape with no parameters omits the list entirely
(`PersonRecord`).

## Name interpolation

A backtick name is a uniform attribute-name form.  *Any* attribute
name (a unit index field, a store attribute, or a shape attribute) may
be written backtick-quoted, and `` `a` `` denotes the same attribute as
the bare `a`.  Backticks add one capability: inside them, `{param}`
interpolates a `string` parameter.

```
shape NormalizedCol[U: Unit, col: string] {
  unit { U }
  attr {
    `{col}`:    real
    `{col}_z`:  real
  }
}
```

Interpolation resolves only where parameters are in scope.  In a shape
that is the shape's `string` parameters; a `{param}` written in a unit
or store, which has no parameters, is a resolution error.  Backticks
are required to mark "this is a parametric identifier"; without them,
`{col}_z` would be a literal identifier.

This uniformity is deliberate and forward-looking.  The same backtick
mechanism is what will let **transform and function bodies derive new
columns as functions of the existing ones**: a transform that
standardises a column writes its result to an interpolated name, e.g.
`mutate { `{col}_z` = (t[col] - mean(t[col])) / sd(t[col]) }`, naming
the derived column by interpolation in exactly the same way a shape
names a required one.  Keeping one name form across declarations and
transforms means a derived-column name reads the same wherever it
appears.

Interpolation is resolved at compile time, when shape parameters are
bound.  The interpolated result must be a valid attribute identifier;
a `string` argument that does not render to one is rejected at the use
site.  Names cannot be derived from runtime values; if a transform
genuinely needs runtime column names, that is a different feature, not
yet in the language.

Only attribute-name positions support interpolation in this version.
Interpolating into unit names, shape names, or other identifier
positions is a much larger feature and is not yet on the table.

## Conformance: the `:` clause on stores

A store declares the shapes it conforms to with a `:` clause after the
store name.  The clause may list one or more shapes, separated by
commas, with their arguments supplied:

```
store students : PersonRecord, NumericCol[Person, "height"] {
  unit { Person }
  attr {
    admission: date
    height:    real
  }
}
```

Each conformance entry is checked at the store declaration site,
independently of the others.  After substituting the arguments, the
compiler verifies that:

- If the shape pins a unit (concrete or via a `Unit` parameter), the
  store's unit equals it; a unit-agnostic shape imposes no unit
  constraint.
- The store's declared cardinality matches the shape's (ADR 0022): an
  all-`attr` shape requires a `singletons` store, an `attr*` shape a
  `bag` store.
- Every attribute required by the shape is present in the store, with
  the same name (after interpolation) and the same type (including
  optionality).
- The store may have *additional* attributes beyond what the shape
  requires.

If any check fails, the declaration is rejected with a diagnostic that
names the missing or mismatched attribute.  The diagnostic mentions the
substituted argument so the reader can see which interpolation produced
the expected name.

The conformance clause and the `domain` block of a store are
independent: shape conformance checks structure, the `domain` block
resolves foreign keys.  Both are checked, neither implies the other.

## Conformance in function signatures

A function is an ordinary `let` binding of a lambda.  Each lambda parameter
carries a type ascription, and the return type follows the parameter bars,
both using the same `:` as every other ascription; the binding's own type
is inferred from the lambda, so every name is bound exactly once.  The unit
and value parameters a function is generic over sit in the *same* parameter
list as its table arguments, as one telescope:

```
let count = |t: PersonRecord| : real { ... }

let normalize = |U: Unit, col: string, t: NumericCol[U, col]| : NormalizedCol[U, col]
  mutate { `{col}_z` = (t[col] - mean(t[col])) / sd(t[col]) }
```

There is one list, not a separate "generics" list and "value" list.
What a parameter *is* follows from its annotation: a parameter
annotated with `Unit` or a primitive type is a compile-time argument,
resolved and erased before run time; a parameter annotated with a
**shape** (a table type, such as `NumericCol[U, col]`) is a run-time
table value.  Because the list is a telescope, a table parameter's type
may mention the unit and value parameters declared before it, which is
how `normalize` threads one unit identity through both its input and
its output.

At the call site, all arguments are supplied positionally:

```
let standardised = normalize(Person, "height", students);
```

The compiler resolves `U` and `col`, verifies that `students` conforms
to `NumericCol[Person, "height"]`, and then `standardised` has type
`NormalizedCol[Person, "height"]`.

Functions and transforms are a later implementation slice: only shape
declarations and the store conformance clause are implemented today.  Shape
*type application* uses square brackets (`NumericCol[U, col]`), as shown.  The
parameter ascriptions in the lambdas (not yet in the lambda grammar of
`04-grammar.md`) and the parentheses at the call sites are provisional: how
function signatures and calls delimit their arguments (brackets, the
juxtaposition application of the expression sublanguage, or otherwise) is
part of the deferred function-syntax design.

## Naming convention

A shape is a type, so its name is **PascalCase**: `PersonRecord`, `Ageable`,
`NumericCol`.  This case rule is enforced, like the unit and store rules;
see `05-naming-and-casing.md`.

A softer style convention sits on top of the enforced case rule, the
counterpart of "units singular, stores plural" (`01-units.md`).  A shape
name reads **singular**, as what one conforming table *is*, because it
appears to the right of a `:` and the colon reads "has this type":
`students : PersonRecord` reads "students is a PersonRecord table".  A
plural name would suggest the shape holds many of something; it holds
nothing, it classifies one table.

Two singular styles fit the two kinds of contract.  A contract about
*content* is a singular noun phrase naming what a conforming table is:
`PersonRecord`, `NumericCol`, `FeatureWindow`.  A contract about a
*property* is an **adjective** (or participle): `Ageable`, `Tabular`,
`Named`.  Prefer the adjective form whenever the shape asserts a quality
rather than enumerates structure; it is also the natural style for the
future marker shapes (`Independent`, `Exhaustive`), which assert a quality
and nothing else.  The test for both styles: the name should complete
"this table is (a) ___" naturally.  This part is not enforced, but
following it keeps conformance clauses readable as sentences.

## What shapes do not contain

A shape declaration cannot contain:

- **A `domain` block.**  Foreign-key resolution is per-store.
- **Policy annotations** (`@audited`, `@versioned`, `@auto`,
  `@allowcreate`).  These attach to store attributes.
- **API surface** (`endpoint`, `auth`).  These belong on the store
  and are web-service concerns.
- **Storage commitment.**  A shape is contract, not data.

A shape *does* contain:

- An optional unit: concrete, a `Unit` parameter, or omitted.
- `attr` blocks, listing required attributes as structure only.
- Optional parameters, each of kind `Unit` or a primitive type such as
  `string`.

Nothing else.

## A known cost

In this version, the result type of a function is exactly what the
function declares.  If `normalize` declares its return type as
`NormalizedCol[U, col]`, the value cannot be passed to a function
expecting `NumericCol[U, col]`, even though structurally a
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
  attr { admission: date }
}

shape Ageable[date_field: string] {
  attr { `{date_field}`: date }
}

shape NumericCol[U: Unit, col: string] {
  unit { U }
  attr { `{col}`: real }
}

shape NormalizedCol[U: Unit, col: string] {
  unit { U }
  attr {
    `{col}`:    real
    `{col}_z`:  real
  }
}

store students : PersonRecord, Ageable["birthdate"], NumericCol[Person, "height"] {
  unit { Person }
  attr {
    admission: date
    birthdate: date
    height:    real
  }
}

let normalize = |U: Unit, col: string, t: NumericCol[U, col]| : NormalizedCol[U, col]
  mutate { `{col}_z` = (t[col] - mean(t[col])) / sd(t[col]) }

let standardised = normalize(Person, "height", students);
// standardised has type NormalizedCol[Person, "height"]
```

`students` is a basic store of `Person`, claiming conformance to three
shapes: `PersonRecord` (a concrete-unit contract), `Ageable["birthdate"]`
(a unit-agnostic contract that also fits a `Department` store keyed on
`"foundation_day"`), and `NumericCol[Person, "height"]` (the structural
contract demanded by `normalize`).  All claims are checked at the store
declaration site.

The transform `normalize` is generic in both the unit and the column
name.  The same definition handles `normalize(Person, "height", ...)`,
`normalize(Person, "weight", ...)`, and `normalize(Course, "credits", ...)`
without modification.

## Forward references and open questions

- **Functions and transforms.**  The parameter form is settled (one
  telescope, kinds/value-types for compile-time arguments, shapes for
  run-time table values), but functions are not implemented yet.
- **Further parameter value types.**  Numbers, predicates, and other
  primitive-typed parameters would slot into the same list; only
  `string` is consumed today.
- **Marker shapes.**  Shapes with no body, used as user-defined
  property tags (Independent, Exhaustive, ...).  With the `unit` clause
  optional, a marker is simply `shape Independent {}`; the same
  conformance machinery becomes the carrier of table properties.
  Treated in its own document; supersedes the qualifier ADR when
  written.
- **Sub-shape relationships.**  See "A known cost" above.  The first
  obvious extension once the explicit cost becomes painful.
- **Associated types.**  Shape parameters are inputs only; they
  cannot, at present, *compute* derived shape information.  When this
  matters, it will get its own document.
- **Schema arithmetic.**  Operators on shapes (`+`, `\`, ...) for
  expressing schema deltas in function signatures: deferred.
- **Coherence and orphan rules.**  Whether one program may declare
  another program's store conforms to its own shape is unsettled.
- **"Any table" type.**  A unit-agnostic shape with no attributes
  (`shape Any {}`) is now expressible and any store conforms to it;
  whether that is the right spelling for "works on any table" is open.
- **Attribute identity** (cross-shape, cross-store).  Still open;
  important for `union` and `join`.
