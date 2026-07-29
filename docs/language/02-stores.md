# Stores

A store is a tabulation of observations of a unit.  It is where unit
observations live: what attributes accompany each observation, what
foreign-key constraints they obey, what change-control policy applies
to them.

This document defines what a store is and how it is declared.  The
unit being tabulated is defined separately (`01-units.md`).  The
process variant of a store, `registry`, is treated in its own document.
The API surface a store may expose (REST endpoints, authentication,
permissions) is part of the web-service work and is out of scope
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
- which other stores their unit-reference fields resolve into,
- which audit, version, or auto-fill policy applies,
- the API surface (when one is exposed).

What they cannot disagree on is the unit itself: identity is fixed by
the unit declaration.

## Store declaration

A store declaration consists of a name, a unit reference, an optional
`domain` block, and one or more `attr` blocks.

```
enum Status { "active" "inactive" }

store persons {
  unit { Person }
  attr {
    birthdate: date
    status:    Status
  }
}
```

The store's name is the identifier other stores and pipelines use to
refer to it.  The `unit { U }` line says which unit is being
tabulated.  The `attr` block lists the attributes attached to each
observation.  Repeated `attr` blocks are allowed and merged.  This
surface is decided in
`docs/decisions/0019-attr-blocks-and-dropped-const-var.md`.

## Basic and compound stores

A store of a *basic* unit (whose key fields are all scalar) needs no
foreign-key resolution: its key values are concrete primitives.

A store of a *compound* unit (whose key has at least one
unit-reference field) must declare where each unit-reference field
resolves.  The `domain` block does this:

```
store student_grades {
  unit { Enrollment }
  domain {
    student: students
    course:  courses
  }
  attr {
    class_id: string
    grade:    real
  }
}
```

`Enrollment` was declared in `01-units.md` with key fields
`student: Person` and `course: Course`.  The `domain` block resolves
each: rows of `student_grades` are constrained to `student` values
that appear as observations in `students`, and `course` values that
appear as observations in `courses`.

The block has one entry per unit-reference field of the store's unit.
Resolution is one level deep: `student_grades.domain` says only where
`student` and `course` resolve.  How `Course.department` resolves is
the responsibility of `courses`, declared in *its* `domain` block.
Transitivity follows the store graph.

## Attributes

The `attr` block lists the attributes that accompany each observation.
Each attribute has a name and a type, exactly as in a shape
(`03-shapes.md`).  The type may be a primitive (`string`, `int`,
`real`, `date`, ...) or a unit reference, in the same way unit key
fields can be either.

A value is **total** by default: in an observed row every attribute is
known.  Marking the type with a trailing `?` makes the value
**optional**, so it may be missing even when the row is present:

```
attr { last_service: date? }
```

Whether a value may be missing is independent of how many rows a key has
(its cardinality); `?` is the only per-attribute control over it, and an
key field is always known, so `?` is not allowed there.  The default
and the marker are decided in `docs/decisions/0010-attribute-totality.md`.

When an attribute is a unit reference, the `domain` block must also
resolve it.  The `domain` block does not distinguish between key
unit-references and attribute unit-references; both are unit-reference
fields needing FK resolution.  The block has one entry per
unit-reference field, drawn from the unit's key and from the store's
own attributes alike.

```
unit Program {
  code: string
}

store programs {
  unit { Program }
  domain { coordinator: persons }
  attr {
    name:        string
    coordinator: Person
  }
}
```

Here `programs.coordinator` is an attribute of type `Person`, resolved
into `persons`.

## Store cardinality: `attr` versus `attr*`

A store declares how many observations it holds per identity, and the
attribute block is where it says so
(`docs/decisions/0022-observations-as-bags-declared-store-cardinality.md`).

A store whose attributes are in plain `attr` blocks is a **`singletons`**
tabulation, the default and the historical rule
(`docs/decisions/0001-unit-as-identity-discipline.md`): for any tuple of
key values there is **at most one** observation, and an accidental
duplicate is rejected.  This is right for entities: a `Person` or a
`Course` is observed once or not at all.

A store whose attributes are in **`attr*`** blocks (the `*` is "many") is a
**`bag`**: it holds many observations per key, and its key is the *entity*
the observations are about.  This is the form for recurring observations,
sensor readings from a machine, transactions of an account, events in a
session:

```
store readings {
  unit { Machine }
  attr* {
    kelvin: real
  }
}
```

Keying the bag by the entity is what aligns the tracked guarantees with the
science: a `split` over `readings` routes whole machines, so disjointness is
tracked at the leakage boundary, and "complete over the key" (all of a
machine's readings) is a contentful fact establishable at the source.  The
identity criterion still says what a row is *about*; it no longer says a row
is *unique*.  Cardinality is a store concern, not a unit concern: the unit
stays pure identity, and the tabulation declares how many observations it
holds, alongside the attribute and change-control concerns already placed
here.

A `bag` store's non-key columns are the columns of the same rows, so they
co-vary and share one length per key by construction.  A store therefore
does **not mix** the two block forms: an `attr` (singleton) column inside a
`bag` store is deferred until an expression-level syntax exists for aligning
one value to many rows, and mixing is rejected as not yet supported.
Per-entity constant facts (a machine's `location`) live in a companion
`singletons` store keyed by the same unit and joined via `domain`, keeping
normalization explicit.

Two consequences to know about:

- **Ordering.**  A `bag` store carries no row order.  When an operation
  needs one (a window such as a running sum, a rank, a lag), the order is
  named at the operator, not carried by the store.  The spelling is an
  ordinary **key argument** to `scan`, `|r| r.taken_at`, rather than the `by`
  clause this document originally anticipated: a clause cannot be partially
  applied, and the derived window vocabulary is partial applications
  (`09-typing-reference.md` section 5.4, ADR 0031 Decision 7).
- **Storage.**  A `bag` store cannot use its key columns as a primary
  key; it maps to a table with a surrogate row identifier and a non-unique
  covering index over the key columns
  (`docs/toolkit/00-storage-backend.md`).  Per-row addressability is lost,
  by definition.

### Mutability is deferred

Earlier drafts distinguished immutable facts (`const`) from evolving
values (`var`) per attribute.  That distinction is change-control
policy, not structure, and the change-control family it belongs to
(`@audited`, `@versioned`, `@auto`, `@allowcreate`) is itself
deferred, so the language currently records no mutability at all: a
store attribute is a name and a type, nothing more.  How mutability
returns, for instance per-tabulation-kind defaults with annotations on
exceptional attributes only, is an open question of the change-control
document.  Decided in
`docs/decisions/0019-attr-blocks-and-dropped-const-var.md`.

## Multiple stores of the same unit

A unit can be tabulated by any number of stores.  This is a feature,
not a quirk: different stores serve different purposes.

```
store persons {
  unit { Person }
  attr { birthdate: date }
}

store students {
  unit { Person }
  attr { admission: date }
}

store alumni_snapshot {
  unit { Person }
  attr { graduation_year: int }
}
```

`persons`, `students`, and `alumni_snapshot` all tabulate `Person`
observations, with different attribute sets.  A row may be present in
`students` and
absent from `persons`; a row may move from `students` to
`alumni_snapshot` when a person graduates; the same `Person.id` may
appear in two stores at the same time.

Crucially, *which* store another store's `domain` block resolves a
unit-reference field into is a per-store choice.  An `Enrollment`
whose `student` resolves into `students` is about a current student;
one resolved into `alumni_snapshot` is about a graduate.  The choice
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
referencing `students` and one referencing `alumni_snapshot`) is also
normal.

## What is not in a store

A store declaration cannot contain:

- **The identity criterion** of its unit.  That is fixed by the unit
  declaration; the store cannot extend, restrict, or redefine it.
- **Pipeline operations.**  Mapping, joining, rekeying, reshaping
  belong to views and transforms (treated in the algebra document),
  not to store declarations.
- **A per-row cardinality knob.**  A store's cardinality is declared once,
  by the uniform `attr` / `attr*` block form (see Store cardinality above):
  `singletons` (the 0-or-1 rule of `01-units.md`, the default) or `bag`.
  There is no finer-grained control: no per-attribute mix (deferred,
  ADR 0022), no bounded counts, no per-key exceptions.  Whether a *value*
  may be missing is a separate axis, declared per attribute with `?` (see
  Attributes above).

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

enum Weekday {
  "Monday", "Tuesday", "Wednesday", "Thursday", "Friday"
}

store departments {
  unit { Department }
  attr { name: string }
}

store persons {
  unit { Person }
  attr {
    birthdate: date
    last_name: string
  }
}

store students {
  unit { Person }
  attr { admission: date }
}

store courses {
  unit { Course }
  domain {
    department: departments
  }
  attr {
    weekday: Weekday
  }
}

store student_grades {
  unit { Enrollment }
  domain {
    student: students
    course:  courses
  }
  attr {
    class_id: string
    grade:    real
  }
}
```

Five stores.  `departments`, `persons`, `students` are basic.
`courses`, `student_grades` are compound.  `persons` and `students`
both tabulate `Person` with different attribute sets.

The dependency graph: `student_grades` references `students` and
`courses`; `courses` references `departments`; the others reference
nothing.  Acyclic, well-formed.

## Forward references and open questions

- **`registry`.**  A process-style variant of `store`, where data
  enters through an ingestion mechanism rather than CRUD.  Treated in
  its own document.  Briefly: registry declarations carry a
  completeness guarantee at the type level that ordinary stores do
  not.
- **Change control.**  The syntax and semantics of `@audited`,
  `@versioned`, `@auto`, `@allowcreate`, and how per-attribute
  mutability returns (see "Mutability is deferred" above), belong in a
  separate policy document.
- **API surface.**  REST endpoints, authentication, and permission
  checking are part of the web-service work, not the language
  core.  This document is silent on whether or how any particular
  store is exposed over HTTP.  The design is settled in
  `docs/decisions/0005-identity-and-authorization.md` (identity and
  `auth {}`) and `docs/decisions/0006-transport-agnostic-surface.md`
  (transport projection).
- **Attribute identity.**  When are two attributes (in two stores, or
  in two intermediate tables) "the same thing"?  Unsettled, important
  for `union` and `join`, has its own document pending.
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
