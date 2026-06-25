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
  year: number
}
```

The fields are the index.  There is no nested `index { ... }` block
inside the unit; everything between the unit's braces is part of the
identity discipline.  Two observations of `Course` are observations of
the same Course iff they agree on `(name, year)`.

Each field has a name and a type.  The type determines the value space
of that index column.  Type annotations may carry domain restrictions
(regex constraints, numeric ranges, precision, length).  The syntax of
those annotations is part of the broader type system, not specific to
units.

A unit declaration introduces nothing besides identity.  It does not
declare attributes, mutability, audit policy, or how observations
enter the system.  Those concerns belong on stores.

## Cardinality

For any unit `U` and any tuple of index values `r`, any tabulation of
observations of `U` has cardinality 0 or 1 at `r`: the entity is
either observed (cardinality 1) or not (cardinality 0).  This is
Wickham's rule that each row is one observation, restated as a
property of the unit.

The chapter's algebra (Chapter 5 of Data Science Project: An Inductive
Learning Approach, F. A. N. Verri, 2026, doi: 10.5281/zenodo.14498010)
allows row cardinality greater than 1, where one row of an indexed
table can carry tuples of values per cell.  Mensura accepts this as a
*transient state inside the algebra*: an operation like `project` can
produce a result in which one index tuple carries multiple values, and
a later operation (`ungroup`, `aggregate`) reduces it back to
cardinality 0 or 1.  Transient states are well-formed inside a
pipeline; they are ill-formed at unit boundaries (a `store`, a
`collect`, a `view`, a function signature that promises a tabulation
of a unit).

The practical consequence: if your data has cardinality greater than 1
for the chosen indexes, the unit's identity criterion is wrong.  Add
the disambiguating column to the index, or split the unit.

This row cardinality (how many rows a key has, 0 for "not sampled") is a
different axis from whether a *value* is missing.  An index field is
always known, so it never carries the `?` optional marker; only `const`
and `var` values may be missing
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
  year: number
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
`(department: (code: string), name: string, year: number)`, where
`department` is itself a tuple.  Mensura presents this hierarchy in
syntax (a user writes `course.department.code`).

The chapter's algebra takes flat tuples of index values.  A
hierarchical index and a flat one are interchangeable: flattening a
hierarchical index gives a flat tuple of scalars, and the algebra
operates on the flat form.  The hierarchy is presentation, not a new
mathematical object, and the chapter's typing rules apply unchanged.

## Naming convention

Units have **singular** names: `Person`, `Course`, `Enrollment`.
Stores, which tabulate observations of units, have **plural** names:
`Students`, `Courses`, `Enrollments`.  The convention is soft.
Following it makes source code easier to scan: a reader can tell from
a name alone which kind of declaration they are looking at.

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
  year: number
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
  semantics of `bind` and `join` and will get its own document.
- **Schema reconciliation under `bind`/`join`** depends on attribute
  identity and is deferred to the algebra document.
- **How operations transform units** is treated in the algebra
  document.  Briefly: split-invariant operations preserve the unit;
  `project` and aggregating operations change it.
- **`assume` and units** is deferred until the algebra is in place
  and a concrete need has been established.
