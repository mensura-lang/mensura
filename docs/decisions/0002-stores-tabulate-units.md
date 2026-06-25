# 0002: Stores tabulate units

## Status

Accepted.  The `store` declaration is specified in
`docs/language/02-stores.md`.  It builds directly on the unit/tabulation
split established in
`docs/decisions/0001-unit-as-identity-discipline`.

## Context

`0001` decided that a `unit` carries identity and nothing else.  Something
still has to say where observations of a unit live, what attributes accompany
them, how those attributes are change-controlled, and (for compound units)
where their unit-reference fields resolve.  That something is the store.

The design questions are: how many stores may a unit have, what does a store
add on top of the unit, and how are cross-store references kept sound.

## Decision

A `store` declaration names a tabulation, points at the unit it tabulates
(`unit { U }`), and adds attributes in `const` and `var` blocks; compound
units additionally get a `domain` block.  A store cannot restate, extend, or
restrict the identity criterion: that is fixed by the unit.

- **Many stores per unit.**  A unit may be tabulated by any number of stores,
  each with its own attribute set and change-control discipline.  `Persons`,
  `Students`, and `AlumniSnapshot` can all tabulate `Person`; a row may be in
  one and absent from another, and the same `Person.id` may appear in several
  at once.  This is the payoff of the `0001` split.

- **`const` versus `var` is a real, load-bearing distinction.**  `const`
  attributes are facts that should not change (a birthdate); `var` attributes
  evolve routinely (a status).  Both kinds of change are observed by the
  language, but the two categories are what audit and version policy attach
  to.  The policy syntax (`@audited`, `@versioned`, ...) is deferred to a
  separate document; the `const`/`var` split itself is settled here.

- **`domain` resolves unit references, one level deep.**  A store of a
  compound unit (or a store with a unit-reference *attribute*) declares, in a
  `domain` block, which store each unit-reference field resolves into.  The
  block has one entry per unit-reference field, drawn from the index and the
  attributes alike; it does not distinguish the two.  Resolution is one level
  deep: a store says only where *its* references land, and transitivity
  follows the store graph.  *Which* store a reference resolves into is a
  per-store choice (an `Enrollment.student` into `Students` is a current
  student; into `AlumniSnapshot` is a graduate).

- **The store dependency graph must be acyclic.**  Every `domain` entry is an
  edge from the referencing store to the referenced store.  Acyclicity is a
  compile-time check that guarantees references resolve without infinite
  recursion, that initialization and migration have a well-defined order, and
  that any single store's well-formedness can be checked locally given the
  stores it references.

## Consequences

Positive:

- Local well-formedness: because resolution is one level deep and the graph is
  acyclic, a store can be checked against just the stores it names, not the
  whole program at once.
- The per-store choice of resolution target turns "which population does this
  reference mean" into a typed, explicit decision rather than a runtime
  convention.
- `const`/`var` gives audit and version policy a structural place to attach,
  so change control is part of the schema rather than bolted on.

Negative:

- Compound stores carry boilerplate: every unit-reference field needs a
  `domain` entry, and the resolution target must be chosen explicitly even
  when there is only one plausible store.
- Acyclicity forbids genuinely mutually-referential store designs; those must
  be re-expressed (for example, by introducing an intermediate store) to fit
  a DAG.

Neutral:

- The word `domain` covers two related mechanisms: the store-level block that
  resolves unit references, and a primitive-field annotation
  (`code: string @domain(~/[A-Z]{5}/)`) that narrows a scalar's value space.
  They sit in different syntactic positions; the overlap is flagged as a
  thing to watch, not a known conflict.

## Alternatives considered

1. **One store per unit (the table-is-the-entity model).**  Simpler, but it
   collapses the `0001` distinction and makes the current-student / alumnus /
   payroll case inexpressible.  Rejected.

2. **Implicit foreign-key resolution.**  Resolve a unit reference into "the"
   store of that unit automatically.  Rejected: there is often more than one
   store of a unit, and which one is meant is exactly the semantically
   significant choice; making it implicit hides it.

3. **Transitive `domain` resolution.**  Let a store specify resolution for
   references several levels down.  Rejected: it breaks local checking and
   duplicates information that the referenced store already owns.  One level
   deep, with transitivity following the graph, keeps each store's
   declaration self-contained.

4. **Allowing cycles in the store graph.**  More expressive, but it forfeits
   the well-defined initialization order and the local well-formedness
   guarantee.  Rejected.

## Open questions

- **`collect`.**  A process-style variant of `store` where data enters by
  ingestion rather than CRUD, carrying a type-level completeness guarantee
  ordinary stores lack.  Treated in its own document.
- **Audit, version, auto-fill policy.**  The syntax and semantics of
  `@audited`, `@versioned`, `@auto`, `@allowcreate`, and whether `const`
  implies `@audited`, belong in a separate policy document.
- **API surface.**  REST endpoints, authentication, and permissions are M4
  web-service concerns; this decision is silent on them.  See
  `docs/decisions/0005-identity-and-authorization` and
  `docs/decisions/0006-transport-agnostic-surface`.
- **Initialization semantics.**  How a store starts (empty, loaded, replayed)
  is a runtime concern not addressed here.
- **Attribute identity.**  Shared with `0001`; still open.
