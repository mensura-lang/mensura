# Units and keys

A **unit** is the kind of entity your rows are about.  It is Wickham's
*observational unit* made syntactic: "Person", "Course", "Machine" are units;
a particular Alice or a particular machine is an *observation* of one.  Declaring
a unit introduces a name and the fields that identify one observation from
another.

```mensura
{{#include ../examples/unit-person.mensura}}
```

The fields between the braces are the unit's **key**.  Two observations are
observations of the same `Person` exactly when they agree on `id`.

## Composite keys

A key can have more than one field.  A course is identified by its name and
the year it ran, so neither field alone is enough:

```mensura
{{#include ../examples/unit-course.mensura}}
```

Two observations are the same `Course` only when they agree on the whole tuple
`(name, year)`.  Everything inside a unit's braces is identity.

## What a unit is not

A unit declares *identity only*.  It says nothing about attributes, whether
they may change, or how observations enter the system.  Those belong to a
[store](stores.md).  This separation is deliberate: the same unit can be
tabulated by several stores that carry different attributes and policies but
agree on what the entity is.

Key field types are the key-eligible primitives (`string`, `int`, `bool`,
`date`) and named enums; a continuous `real` measurement cannot be a key
(ADR 0014).

## Compound units

A key field's type may also be another unit.  The field's value is then the
key of an observation of that unit, instead of a string foreign key:

```mensura
{{#include ../examples/unit-enrollment.mensura}}
```

An `Enrollment` is identified by *which student* and *which course*, and a
`Course` is itself identified by `(name, year)`.  A unit with a
unit-reference field is **compound**; a store of a compound unit must say
where each referenced unit's observations live, which is the `domain`
block of [stores](stores.md).  The full design lives in
`docs/language/01-units.md`.
