# Stores

A store is a tabulation of observations of a unit.  It is where unit
observations live: what attributes accompany each observation, what
foreign-key constraints they obey, what change-control policy applies
to them.

This document defines what a store is and how it is declared.  The
unit being tabulated is defined separately (`01-units.md`).  The
process variant of a store, `collect`, is treated in its own document.
The API surface a store may expose (REST endpoints, authentication,
permissions) is part of the M4 web-service work and is out of scope
here.  The audit, version, and auto-fill policy syntax (`@audited`,
`@versioned`, `@auto`, `@allowcreate`) is treated in a separate policy
document.

The syntax shown is preliminary, like the rest of the language docs
at this stage; the design content is not.

## What a store is

A unit declares an identity discipline.  A store is the concrete place
where observations of that unit are tabulated, with whatever
attributes the application needs to record.

Two stores of the same unit observe the same kind of entity, but they
may disagree on:

- which attributes accompany each observation,
- whether those attributes are immutable facts or evolving values,
- which other stores their unit-reference fields resolve into,
- which audit, version, or auto-fill policy applies,
- the API surface (when one is exposed).

What they cannot disagree on is the unit itself: identity is fixed by
the unit declaration.

## Store declaration

A store declaration consists of a name, a unit reference, an optional
`domain` block, and any number of `const` and `var` blocks.

```
store Persons {
  unit { Person }
  const { birthdate: date }
  var   { status: enum("active", "inactive") }
}
```

The store's name is the identifier other stores and pipelines use to
refer to it.  The `unit { U }` line says which unit is being
tabulated.  The `const` and `var` blocks list the attributes attached
to each observation; their semantics are described below.

## Basic and compound stores

A store of a *basic* unit (whose index fields are all scalar) needs no
foreign-key resolution: its index values are concrete primitives.

A store of a *compound* unit (whose index has at least one
unit-reference field) must declare where each unit-reference field
resolves.  The `domain` block does this:

```
store StudentGrades {
  unit { Enrollment }
  domain {
    student: Students
    course:  Courses
  }
  const { class_id: string }
  var   { grade: number }
}
```

`Enrollment` was declared in `01-units.md` with index fields
`student: Person` and `course: Course`.  The `domain` block resolves
each: rows of `StudentGrades` are constrained to `student` values
that appear as observations in `Students`, and `course` values that
appear as observations in `Courses`.

The block has one entry per unit-reference field of the store's unit.
Resolution is one level deep: `StudentGrades.domain` says only where
`student` and `course` resolve.  How `Course.department` resolves is
the responsibility of `Courses`, declared in *its* `domain` block.
Transitivity follows the store graph.

## Attributes

The `const` and `var` blocks list the attributes that accompany each
observation.  Each attribute has a name and a type.  The type may be
a primitive (`string`, `number`, `date`, ...) or a unit reference, in
the same way unit index fields can be either.

When an attribute is a unit reference, the `domain` block must also
resolve it.  The `domain` block does not distinguish between index
unit-references and attribute unit-references; both are unit-reference
fields needing FK resolution.  The block has one entry per
unit-reference field, drawn from the unit's index and from the store's
own attributes alike.

```
unit Program {
  code: string
}

store Programs {
  unit { Program }
  domain { coordinator: Persons }
  const { name: string }
  var   { coordinator: Person }
}
```

Here `Programs.coordinator` is an attribute of type `Person`, resolved
into `Persons`.

### `const` and `var`

`const` attributes are *facts that should not change*: a person's
birthdate, a course's name under a given catalogue revision, a
registration's program.  They can be modified, but the language
treats every change as an exceptional event subject to audit.

`var` attributes are *data that evolves over time*: a student's
status, a course offering's open/closed state, a person's last name.
Changes are still observed by the language, but they are routine.

The semantic distinction between `const` and `var` is not just
documentation: it is what audit and version policy attach to.  The
exact policy syntax (`@audited`, `@versioned`, `@auto`,
`@allowcreate`) is treated in a separate document; for now it is
enough to know that `const` and `var` are real, distinct categories.

## Multiple stores of the same unit

A unit can be tabulated by any number of stores.  This is a feature,
not a quirk: different stores serve different purposes.

```
store Persons {
  unit { Person }
  const { birthdate: date }
}

store Students {
  unit { Person }
  const { admission: date }
}

store AlumniSnapshot {
  unit { Person }
  const { graduation_year: number }
}
```

`Persons`, `Students`, and `AlumniSnapshot` all tabulate `Person`
observations, with different attribute sets and different
change-control disciplines.  A row may be present in `Students` and
absent from `Persons`; a row may move from `Students` to
`AlumniSnapshot` when a person graduates; the same `Person.id` may
appear in two stores at the same time.

Crucially, *which* store another store's `domain` block resolves a
unit-reference field into is a per-store choice.  An `Enrollment`
whose `student` resolves into `Students` is about a current student;
one resolved into `AlumniSnapshot` is about a graduate.  The choice
is local to the referencing store.

## The store dependency graph

The `domain` blocks of all stores in a program form a directed graph:
each `domain` entry is an edge from the referencing store to the
referenced store.  This graph must be acyclic.

Acyclicity is a compile-time check.  It guarantees that:

- references can be resolved without infinite recursion,
- migrations and initialization have a well-defined order,
- the well-formedness of any single store can be checked locally,
  given the well-formedness of the stores it references.

A store may have any number of incoming edges.  Multiple stores
referencing the same store is normal.  Multiple stores referencing
different stores of the same unit (for example, several stores
referencing `Students` and one referencing `AlumniSnapshot`) is also
normal.

## What is not in a store

A store declaration cannot contain:

- **The identity criterion** of its unit.  That is fixed by the unit
  declaration; the store cannot extend, restrict, or redefine it.
- **Pipeline operations.**  Filtering, projecting, mutating, joining
  belong to views and transforms (treated in the algebra document),
  not to store declarations.
- **Cardinality declarations.**  The 0-or-1 rule is universal at
  unit boundaries (`01-units.md`), and a store is a unit boundary.

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

store Departments {
  unit { Department }
  const { name: string }
}

store Persons {
  unit { Person }
  const { birthdate: date }
  var   { last_name: string }
}

store Students {
  unit { Person }
  const { admission: date }
}

store Courses {
  unit { Course }
  domain {
    department: Departments
  }
  const {
    weekday: enum("Monday", "Tuesday", "Wednesday", "Thursday", "Friday")
  }
}

store StudentGrades {
  unit { Enrollment }
  domain {
    student: Students
    course:  Courses
  }
  const { class_id: string }
  var   { grade: number }
}
```

Five stores.  `Departments`, `Persons`, `Students` are basic.
`Courses`, `StudentGrades` are compound.  `Persons` and `Students`
both tabulate `Person` with different attribute sets.

The dependency graph: `StudentGrades` references `Students` and
`Courses`; `Courses` references `Departments`; the others reference
nothing.  Acyclic, well-formed.

## Forward references and open questions

- **`collect`.**  A process-style variant of `store`, where data
  enters through an ingestion mechanism rather than CRUD.  Treated in
  its own document.  Briefly: collect declarations carry a
  completeness guarantee at the type level that ordinary stores do
  not.
- **Audit, version, auto-fill policy.**  The syntax and semantics of
  `@audited`, `@versioned`, `@auto`, `@allowcreate`, and whether
  `const` always implies `@audited` (per the proposal), belong in a
  separate policy document.
- **API surface.**  REST endpoints, authentication, and permission
  checking are part of the M4 web-service work, not the language
  core.  This document is silent on whether or how any particular
  store is exposed over HTTP.  The design is settled in
  `docs/decisions/0005-identity-and-authorization.md` (identity and
  `auth {}`) and `docs/decisions/0006-transport-agnostic-surface.md`
  (transport projection).
- **Attribute identity.**  When are two attributes (in two stores, or
  in two intermediate tables) "the same thing"?  Unsettled, important
  for `bind` and `join`, has its own document pending.
- **Initialization semantics.**  How a store starts (empty, loaded
  from a file, replayed from a log) is a runtime concern this
  document does not address.
- **`@domain(...)` annotation versus the `domain { ... }` block.**
  The same word covers two related but distinct mechanisms: a
  primitive-field annotation (e.g.  `code: string @domain(~/[A-Z]{5}/)`)
  narrows the value space of a scalar; the store-level block resolves
  unit-references into stores.  They occupy different syntactic
  positions and the overlap should not cause ambiguity, but it is
  worth flagging in case it does.
