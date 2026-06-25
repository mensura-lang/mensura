# 0001: Unit as identity discipline

## Status

Accepted.  The `unit` declaration is specified in `docs/language/01-units.md`
and is the foundation the store and shape designs build on
(`docs/decisions/0002-stores-tabulate-units`,
`docs/decisions/0003-shapes-as-structural-contracts`).

## Context

Mensura is built on Wickham's tidy data, whose third rule is that *each type
of observational unit forms a table*.  An observational unit is a *type*
("Person", "Course", "Enrollment"), not an instance; a particular Alice is an
*observation* of `Person`.  The language needs a first-class construct for
this notion, and the question is what that construct should carry.

The tempting answer is to make the unit the whole schema: identity plus the
attributes, mutability, audit policy, and storage that go with it.  This is
what most ORMs and table-definition languages do.  It conflates two things
that the data-science workflow keeps separate.  The same kind of entity is
routinely tabulated more than once for different purposes (a `Person` as a
current student, as an alumnus snapshot, as a payroll record), and those
tabulations legitimately disagree about attributes, change control, and API
surface while agreeing completely about *what a Person is*.

If identity and tabulation live in one construct, that agreement cannot be
expressed: two tables of the same entity are just two unrelated schemas that
happen to share column names.

## Decision

A `unit` declaration introduces a name and a list of **index fields**, and
nothing else.  The index fields are the identity criterion: two observations
are observations of the same entity iff they agree on every index field.

A unit carries no attributes, no mutability or audit policy, no auto-fill
rule, no API surface, and no cardinality declaration.  All of those are store
concerns (`0002`).  A unit is an identity discipline and nothing more.

Three properties follow and are fixed at the unit level:

- **Cardinality is universally 0 or 1.**  For any unit and any tuple of index
  values, a tabulation has at most one observation there: the entity is
  either observed or not.  This is Wickham's "each row is one observation"
  restated as a property of the unit, so there is nothing per-unit to
  configure.  Row cardinality greater than 1 is allowed only as a transient
  state *inside* the algebra (between `project` and a later `ungroup` or
  `aggregate`); it is ill-formed at any unit boundary (`store`, `collect`,
  `view`, a signature promising a tabulation of a unit).  The `view` case is
  narrowed by `docs/decisions/0012-view-hosting.md`: a view is a unit boundary
  only when it claims a unit-fixing shape; a bare view is a general materialized
  table whose cardinality may be `bag`.
- **References are typed by unit, not by string.**  An index field's type may
  be another unit; its value is then the identity of an observation of that
  unit.  A unit with at least one unit-reference field is **compound**; one
  whose fields are all scalar is **basic**.  This distinction is load-bearing
  for stores, which must resolve where each unit-reference field lands
  (`0002`).
- **Hierarchical in syntax, flat in the algebra.**  A compound unit's index is
  a tree (`course.department.code`).  Flattening it yields the flat tuple the
  Chapter 5 algebra operates on, so the hierarchy is presentation, not a new
  mathematical object, and the chapter's typing rules apply unchanged.

Units have **singular** names by soft convention (`Person`, `Course`); stores
have plural names (`Persons`, `Courses`), so a reader can tell the kind of
declaration from the name alone.

## Consequences

Positive:

- One kind of entity can be tabulated by many stores that disagree about
  everything except identity.  The shared unit is what makes those tables
  recognizably about the same thing.
- The 0-or-1 rule is a free, universal invariant: a dataset with cardinality
  greater than 1 for its chosen index is a signal that the identity criterion
  is wrong (add the disambiguating column, or split the unit), caught at a
  unit boundary rather than discovered downstream.
- Typed unit references replace stringly-typed foreign keys; the compiler
  knows what a reference refers to, which is what later lets stores check
  foreign-key resolution and the algebra reason about `bind`/`join`.

Negative:

- Identity and tabulation living in separate constructs means a minimal "just
  store some people" program is two declarations (`unit` then `store`), not
  one.  This is a real ergonomic cost paid for the multi-store benefit.
- The transient-cardinality rule puts an obligation on the algebra: it must
  define exactly which operations may produce cardinality greater than 1 and
  which boundary checks reject it.  That obligation is deferred to the algebra
  document.

Neutral:

- The singular/plural naming split is convention, not enforced by the
  compiler.  Casing and naming rules are settled separately
  (`docs/language/05-naming-and-casing.md`).

## Alternatives considered

1. **Unit carries its own attributes and policy (the ORM model).**  One
   declaration per entity, fewer lines for simple cases.  Rejected: it makes
   "two tabulations of the same entity" inexpressible, which is the central
   thing the data-science workflow needs.

2. **String foreign keys for cross-unit references.**  Familiar from SQL.
   Rejected in favour of typed unit references, so the compiler can resolve
   and check references rather than trusting matching strings.

3. **Per-unit cardinality declarations.**  Let a unit opt into cardinality
   greater than 1.  Rejected: the 0-or-1 rule is what makes a row an
   observation; relaxing it per unit would undermine every downstream
   invariant.  Multiplicity is modeled by choosing the right index, not by a
   knob.

4. **Schema extension (an `is`-style form).**  Let one unit extend another.
   Rejected: relationships between units go through index-reference fields,
   which is more explicit and avoids inheritance semantics in the type system.

## Open questions

- **Attribute identity.**  When are two columns in two stores "the same
  thing"?  Unsettled, and important for the semantics of `bind` and `join`.
  It is an attribute question, so it does not block the unit design, but it
  must be settled before the algebra is complete.
- **How operations transform units.**  Treated in the algebra document.
  Briefly: split-invariant operations preserve the unit; `project` and
  aggregating operations change it.
- **`assume` and units.**  Deferred until the algebra is in place and a
  concrete need exists.
