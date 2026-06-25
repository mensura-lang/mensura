# 0003: Shapes as structural contracts

## Status

Accepted.  The `shape` declaration and the conformance clause are specified in
`docs/language/03-shapes.md`.  Shapes are the table-type language that units
(`docs/decisions/0001-unit-as-identity-discipline`) and stores
(`docs/decisions/0002-stores-tabulate-units`) are written against, and they
are the carrier the later qualifier work reuses
(`docs/decisions/0004-qualifier-mechanism`).

## Context

`0001` and `0002` give concrete declarations: a unit is one identity
discipline, a store is one tabulation.  Functions, transforms, and reusable
contracts need to talk about a table's *structure* abstractly, without
committing to a particular store, its policy, or its storage.  A function that
counts rows should accept any table that has the attributes it reads,
regardless of which store produced it; a normalization transform should be
generic over both the unit and the column name it operates on.

The questions are: how is an abstract table type spelled, how does a concrete
store relate to it, and how are such types parameterised over units and over
compile-time values like column names.

## Decision

A `shape` is a named, optionally parameterised description of a table's
structure: an optional `unit` clause plus `const` and `var` attribute blocks.
It carries structure only, no `domain` block, no policy, no API, no storage.
A program of only shapes is well-typed but observes no data.

- **Structural conformance via `:`.**  A store claims conformance to shapes
  with a `:` clause (`store Students : PersonRecord`); a function parameter is
  typed by a shape name.  Conformance is structural: a store conforms if it
  has every attribute the shape requires (same name after interpolation, same
  type, same `const`/`var` block) and may have more.  A store is thus a
  structural subtype of every shape it claims.  The `:` reads the same as
  everywhere else in the language ("the left has the type on the right"),
  whether on an attribute, a function parameter, or a store.

- **The unit clause is optional and has three forms.**  Concrete
  (`unit { Person }`) pins the unit; parameterised (`unit { U }`) ties it to a
  `Unit` parameter; omitted makes the shape *unit-agnostic*, so one structural
  contract spans different units.

- **One telescoping parameter list.**  A shape (and, later, a function) takes
  a single positional, explicitly annotated parameter list, a telescope in
  which
  a later parameter or the body may refer to an earlier one.  A parameter
  annotated `Unit` is a type-level unit variable; one annotated with a
  primitive type (`string`) is a compile-time value.  Crucially there is *not*
  a separate "generics" list and "value" list: what a parameter is follows
  from its annotation, and compile-time parameters (`Unit`, `string`) are
  resolved and erased before run time while a shape-typed parameter is a
  run-time table.  This is how
  `normalize(U: Unit, col: string, t: ...[U, col])` threads one unit identity
  through input and output.

- **Backtick name interpolation, compile-time only.**  Any attribute name may
  be written backtick-quoted, and `` `a` `` is the same attribute as `a`.
  Inside backticks, `{param}` interpolates a `string` parameter, resolved at
  compile time where parameters are in scope.  The same mechanism will later
  let transform bodies name derived columns (`` `{col}_z` = ... ``), so a
  derived-column name reads identically wherever it appears.  Names cannot be
  derived from runtime values, and only attribute-name positions interpolate.

- **Type application uses square brackets.**  `NumericCol[U, col]`,
  `NumericCol[Person, "height"]`.  Bare names are units, quoted literals are
  values, so a use site shows at a glance which argument is which.

## Consequences

Positive:

- One contract, many stores, and many contracts per store: structural
  conformance lets `count(t: PersonRecord)` accept any conforming store and
  lets a store claim several shapes at once, each checked independently at the
  declaration site with a diagnostic that names the offending attribute.
- Unit-agnostic shapes plus `string` parameters give genuinely reusable
  contracts (`Ageable["birthdate"]`, `NumericCol[Person, "height"]`) without a
  separate macro or generic facility.
- The single telescope keeps one uniform parameter form, with room for future
  value types (numbers, predicates) to slot in with no new syntax.

Negative:

- No sub-shape relationships in this version: a `NormalizedCol` value cannot
  be passed where a `NumericCol` is expected even though it structurally
  contains everything required, because the "every NormalizedCol is a
  NumericCol" rule is not expressible yet.  Long pipelines may need a chain of
  explicitly declared intermediate shapes.  This is a deliberate cost, to be
  paid down by a sub-shapes document when it bites.
- Structural conformance means a store can satisfy a shape by accident (the
  right attributes for unrelated reasons).  This is the usual
  structural-typing trade-off, accepted for the flexibility it buys.

Neutral:

- Functions and transforms are specified here only as far as their parameter
  form; their bodies and call syntax are a later implementation slice, and
  whether calls use brackets or juxtaposition is part of the deferred
  function-syntax design.  Today only shape declarations and the store
  conformance clause are implemented.

## Alternatives considered

1. **Nominal table types.**  A store would declare itself *of* a named type
   and match only that name.  Rejected: it defeats the "any table with these
   columns" use case that functions and transforms need, and forces a naming
   ceremony for every structural contract.

2. **Separate generics list and value-parameter list.**  A conventional
   `fn f<U>(col: string)` split.  Rejected: two lists for one telescope is
   redundant, and it obscures that a `Unit` and a `string` parameter are both
   just compile-time arguments resolved before run time.

3. **Runtime-derived attribute names.**  Allow `{param}` to interpolate
   runtime values.  Rejected: names must be known at compile time for the type
   checker to verify conformance; runtime column names are a different,
   larger feature, not in this version.

4. **Built-in sub-shape (subtyping) lattice from the start.**  More
   convenient, but it commits the type system to a coercion story before the
   need is concrete.  Deferred deliberately; see "A known cost" in the doc.

## Open questions

- **Sub-shape relationships.**  The first obvious extension once the explicit
  intermediate-shape cost becomes painful.
- **Marker shapes.**  A bodyless `shape Independent {}` is now expressible and
  any store conforms to it; the same conformance machinery can carry
  user-defined table properties.  This is what the qualifier work
  (`docs/decisions/0004-qualifier-mechanism`) reconsiders: a marker-shape
  document, when written, is expected to supersede that ADR.
- **Further parameter value types.**  Numbers, predicates, and other
  primitive-typed parameters slot into the same telescope; only `string` is
  consumed today.
- **Associated types and schema arithmetic.**  Computing derived shape
  information from parameters, and operators on shapes (`+`, `\`) for schema
  deltas, are both deferred.
- **Coherence and orphan rules.**  Whether one program may declare that
  another program's store conforms to its own shape is unsettled.
- **Attribute identity.**  Shared with `0001` and `0002`; still open, and
  important for `bind` and `join`.
