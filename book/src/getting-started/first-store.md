# Your first store

A Mensura program describes data.  The smallest useful program declares a
**unit**, the kind of entity your rows are about, and a **store**, the table
that holds their attributes.  Here is one in full:

```mensura
{{#include ../examples/first-store.mensura}}
```

Save it as `readings.mensura`.  Two things are worth noticing before we run it.

**Every word is an identifier.**  Mensura has no reserved keywords.  `unit`,
`store`, `const`, and `var` are ordinary identifiers that the parser recognises
by their position.  That is why the highlighting in this book comes from the
compiler itself rather than a word list: only the parser knows that the first
`unit` opens a declaration while the second, inside the store, names which unit
the rows are about.

**Constants and variables are different.**  A store splits its non-index
attributes into two groups:

- `const` is for facts that should not change once a row exists.  A sensor's
  installation date is a fact about that sensor.
- `var` is for data that evolves.  A human-readable label can be corrected or
  updated.

The distinction is part of the type, not a comment.  Later milestones attach
auditing and versioning rules to it; for now it records intent precisely.

The `id: string` inside `unit Sensor` is the unit's **index**: the field that
identifies one sensor from another.  Index fields become the store's primary
key when it is created.  The next chapter runs this program and shows what it
produces.
