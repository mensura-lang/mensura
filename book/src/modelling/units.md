# Units and indices

A **unit** is the kind of entity your rows are about.  It is Wickham's
*observational unit* made syntactic: "Person", "Course", "Machine" are units;
a particular Alice or a particular machine is an *observation* of one.  Declaring
a unit introduces a name and the fields that identify one observation from
another.

```mensura
{{#include ../examples/unit-person.mensura}}
```

The fields between the braces are the unit's **index**.  Two observations are
observations of the same `Person` exactly when they agree on `id`.

## Composite indices

An index can have more than one field.  A course is identified by its name and
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

Index field types are the key-eligible primitives (`string`, `int`, `bool`, `date`) and named enums; a continuous `real` measurement cannot be a key (ADR 0014).
Indices whose fields reference other units (compound units) are a later
feature; see [What's next](../whats-next.md).  The full design lives in
`docs/language/01-units.md`.
