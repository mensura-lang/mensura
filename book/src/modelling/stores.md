# Stores and attributes

A **store** tabulates observations of a unit.  Where a unit declares identity,
a store declares the attributes carried for each observation.

```mensura
{{#include ../examples/store-machines.mensura}}
```

The `unit { Machine }` clause says which unit these rows are about.  The store's
key is that unit's index, so each `Machine` has at most one row in `machines`.

## Attributes

The `attr` block lists the non-index attributes, each a `name: type` pair.  A
store may write several `attr` blocks; they merge into one attribute list.
How attributes may change over time (auditing, versioning, per-attribute
mutability) is change-control policy that later milestones attach to stores;
today a store records structure only.

## Optional values

By default every attribute is **total**: each row has a value for it.  A
trailing `?` marks an attribute whose value may be **missing** for a row,
without the row itself being absent.

```mensura
{{#include ../examples/optional-values.mensura}}
```

A machine that has never been serviced has no `last_service` date: the value is
missing, but the row still exists.  `operating_hours`, with no `?`, is always
present.

## Recurring observations

By default a store holds **at most one row per key**: a `Machine` is observed
once or not at all, and an accidental duplicate is rejected.  Some
observations of an entity genuinely recur, though: sensor readings from a
machine, transactions of an account.  Writing the attribute block as `attr*`
(the `*` is "many") declares a **bag store**, which holds many rows per key:

```mensura
{{#include ../examples/bag-store.mensura}}
```

The key still says what a row is *about* (the machine), no longer that a row
is unique.  A store is one or the other, never mixed: all of its attribute
blocks are `attr`, or all are `attr*`.  Keeping the entity as the key, rather
than working a timestamp into the index, is what later lets a split route
whole machines to one side, so a training set and a test set cannot share a
machine.  A per-entity constant (a machine's commissioning date) does not
belong in the bag; it lives in a companion default store of the same unit,
like `machines` above.

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
