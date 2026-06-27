# Stores and attributes

A **store** tabulates observations of a unit.  Where a unit declares identity, a
store declares the attributes carried for each observation and how they may
change.

```mensura
{{#include ../examples/store-machines.mensura}}
```

The `unit { Machine }` clause says which unit these rows are about.  The store's
key is that unit's index, so each `Machine` has at most one row in `machines`.

## Constants and variables

Non-index attributes are split by whether they evolve:

- `const` attributes are facts fixed when the row is created.  When a machine
  was commissioned does not change.
- `var` attributes hold evolving state.  A machine's status moves between
  values over its life.

The split is part of the store's type.  It records intent precisely and is what
later milestones hang auditing and versioning rules on.

## Optional values

By default every attribute is **total**: each row has a value for it.  A
trailing `?` marks an attribute whose value may be **missing** for a row,
without the row itself being absent.

```mensura
{{#include ../examples/optional-values.mensura}}
```

A machine that has never been serviced has no `last_service` date: the value is
missing, but the row still exists.  `operating_hours`, with no `?`, is always
present.  Both `const` and `var` attributes may be optional.

## Enumerations

A named `enum` is a fixed set of string values, referenced by name as an
attribute type:

```mensura
{{#include ../examples/enum-status.mensura}}
```

A `status` value must be one of the three variants; anything else is a type
error.  When the store is created, the column is stored as text constrained to
those values.

Attribute types today are the primitives (`string`, `int`, `real`, `bool`, `date`)
and named enums.  Physical units and precision on attributes are a later
feature; see [What's next](../whats-next.md).
