# Units

A unit is a Mensura encoding of Wickham's *observational unit*: the
kind of entity being observed.  A unit declaration introduces a name
for that kind and the columns that identify one observation of it.

This document defines what a unit is and how it is declared.  How
units are tabulated (stores, attributes, audit policy, API surface)
lives in `02-stores.md`.  How operations on tables transform units is
treated in the algebra document.  The syntax shown here is preliminary
and may evolve as the surface grammar is finalized; the design content
is not.

## Observational units

Wickham's tidy data has three rules; the third is *each type of
observational unit forms a table*.  An observational unit is a *type*,
not an instance: it names the *category* of entity being observed.
"Person", "Course", "Transaction", "Sensor reading at time T" are
observational units.  A particular Alice, a particular MATH-101, a
particular transaction #4729 are *observations* of those units.

Mensura makes the distinction syntactic.  A `unit` declaration creates
a unit; a `store` (defined elsewhere) creates a tabulation of
observations of a unit.  The same unit can be tabulated by multiple
stores; different stores can disagree about attributes, audit policy,
and the API for the data, but they agree on what the unit is.

## Unit declaration

A unit declaration consists of a name and a list of *index fields*.

```
unit Person {
  id: string
}

unit Course {
  name: string
  year: int
}
```

The fields are the index.  There is no nested `index { ... }` block
inside the unit; everything between the unit's braces is part of the
identity discipline.  Two observations of `Course` are observations of
the same Course iff they agree on `(name, year)`.

Each field has a name and a type.  The type determines the value space
of that index column.  An index field's type must be **key-eligible**:
a stable, comparable identity (`string`, `int`, `bool`, `date`, `enum`),
never a continuous `real` measurement, since identity is decided by
equality (ADR 0014).  Type annotations may carry domain restrictions
(regex constraints, numeric ranges, precision, length).  The syntax of
those annotations is part of the broader type system, not specific to
units.

A unit declaration introduces nothing besides identity.  It does not
declare attributes, mutability, audit policy, or how observations
enter the system.  Those concerns belong on stores.

## Cardinality

For any unit `U` and any tuple of index values `k`, a **`singletons`**
tabulation of observations of `U` has cardinality 0 or 1 at `k`: the
entity is either observed (cardinality 1) or not (cardinality 0).  This
is Wickham's rule that each row is one observation, restated as a
property of the unit, and it is the default at every unit boundary.
Equivalently, the tabulation is **functional** over its index: there is
at most one row per key.  Functionality over a column set is
the fact the checker actually carries (a *grading*, ADR 0024), with
`singletons` as its reading at the current key; because the fact names
columns rather than the key, reindexing can move a column out of the key
and back without forgetting that the entity was observed at most once.

The chapter's algebra (Chapter 5 of Data Science Project: An Inductive
Learning Approach, F. A. N. Verri, 2026, doi: 10.5281/zenodo.14498010)
allows row cardinality greater than 1.  Mensura models this as a key
carrying many rows (a *bag*), following the row-multiset model
(ADR 0015).  A bag arises in two ways.  Inside the algebra it is a
*transient state*: an operation like `demote` can produce a result
in which one key carries multiple rows, and a later `map_bag` may
reduce each bag back to a single row.  At the store boundary it is a
*declared state*
(`docs/decisions/0022-observations-as-bags-declared-store-cardinality.md`):
a store of recurring observations may opt into `bag` cardinality with
`attr*` blocks (`02-stores.md`), keeping the entity as its key.  In
either case the cardinality is a property of the *tabulation*; the unit
itself stays pure identity, and an undeclared duplicate at a
`singletons` boundary (a plain `store`, a `registry`, a function
signature that promises a 0-or-1 tabulation) remains ill-formed.

The practical consequence: if your data has cardinality greater than 1
for the chosen indexes, either the observations genuinely recur for one
entity (declare a `bag` store keyed by that entity) or the unit's
identity criterion is wrong (add the disambiguating column to the
index, or split the unit).  Which modelling to pick is the author's
declaration of the validation granularity: entity-keyed bags make a
`split` route whole entities, while time-in-index keys are what
temporal cross-validation wants.

This row cardinality (how many rows a key has, 0 for "not sampled") is a
different axis from whether a *value* is missing.  An index field is
always known, so it never carries the `?` optional marker; only
non-index attribute values may be missing
(`docs/decisions/0010-attribute-totality.md`).

## Compositional units

An index field's type may be another unit.  When it is, the value of
that field is the index of an observation of the referenced unit.

```
unit Department {
  code: string
}

unit Course {
  department: Department
  name: string
  year: int
}
```

A `Course` is identified by `(department, name, year)`, where
`department` is itself the index of a `Department`.  This is what
Wickham gestures at when he writes about cross-table references in
tidy data: instead of a string foreign key, the field's type is the
referenced unit, and the value is the referenced unit's identity.

A unit with at least one unit-reference field is **compound**.  A unit
whose fields are all scalar is **basic**.  The distinction is
load-bearing for stores: a store of a compound unit must declare where
each unit-reference field resolves, while a store of a basic unit
needs no such resolution.  See `02-stores.md`.

### Hierarchical at the unit level, flat at the math level

A compound unit's index is a tree.  `Course`'s index is
`(department: (code: string), name: string, year: int)`, where
`department` is itself a tuple.  Mensura presents this hierarchy in
syntax (a user writes `course.department.code`).

The chapter's algebra takes flat tuples of index values.  A
hierarchical index and a flat one are interchangeable: flattening a
hierarchical index gives a flat tuple of scalars, and the algebra
operates on the flat form.  The hierarchy is presentation, not a new
mathematical object, and the chapter's typing rules apply unchanged.

## Naming convention

A unit is a type, so its name is **PascalCase**: `Person`, `Course`,
`Enrollment`.  A store, which tabulates observations of a unit, is a
term, so its name is **snake_case**: `students`, `courses`,
`enrollments`.  This case distinction is enforced (a non-PascalCase
unit name or a non-snake_case store name is a resolution error); see
`05-naming-and-casing.md`.

A softer style convention sits on top of the enforced case rule: units
read naturally as **singular** (`Person`) and stores as **plural**
(`students`), since a store holds many observations.  This part is not
enforced, but following it makes source code easier to scan.

## What is not in a unit

A unit declaration cannot contain:

- **Attributes** (constant facts, evolving variables).  These belong
  on stores of the unit.
- **Audit, version, or auto-fill policy** (`@audited`, `@versioned`,
  `@auto`, `@allowcreate`).  These belong on store attributes.
- **API surface** (REST endpoint, auth, permissions).  These belong
  on the store.
- **Cardinality declarations.**  The 0-or-1 rule is universal; there
  is nothing per-unit to set.
- **Schema extension.**  Mensura does not have an `is`-extension form.
  A new unit is its own declaration; relationships between units go
  through index-reference fields.

These are not arbitrary restrictions.  They reflect the design choice
that a unit is an identity discipline and nothing more.  Two stores
of the same unit can disagree about everything else.

## Worked example

```
unit Person {
  id: string
}

unit Department {
  code: string
}

unit Course {
  department: Department
  name: string
  year: int
}

unit Enrollment {
  student: Person
  course: Course
}
```

Two basic units (`Person`, `Department`) and two compound units
(`Course`, which references `Department`; `Enrollment`, which
references both `Person` and `Course`).  None of these declarations
say anything about how observations enter the system, what attributes
accompany them, or where the data lives.  Those concerns belong on
stores.

## Open questions and forward references

- **Attribute identity** (when are two columns in two stores referring
  to "the same thing") is not yet settled.  It is important for the
  semantics of `union` and `join` and will get its own document.
  A new `attribute` declaration may be needed to give a univocal name
  to an attribute and avoid accidental collisions of equivocal names.
- **Schema reconciliation under `union`/`join`** depends on attribute
  identity and is deferred to the algebra document.
- **How operations transform units** is treated in the algebra
  document.
