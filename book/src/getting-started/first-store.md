# Your first store

A Mensura program describes data.  The smallest useful program declares a
**unit**, the kind of entity your rows are about, and a **store**, the table
that holds their attributes.  Here is one in full:

```mensura
{{#include ../examples/first-store.mensura}}
```

Save it as `machines.mensura`.  Two things are worth noticing before we run it.

**Every word is an identifier.**  Mensura has no reserved keywords.  `unit`,
`store`, and `attr` are ordinary identifiers that the parser recognises by
their position.  That is why the highlighting in this book comes from the
compiler itself rather than a word list: only the parser knows that the first
`unit` opens a declaration while the second, inside the store, names which unit
the rows are about.

**Attributes are the data carried per row.**  The `attr` block lists the
non-key attributes, each a name and a type.  A machine's commissioning date
and its human-readable label travel with each observation of that machine.

The `id: string` inside `unit Machine` is the unit's **key**: the field that
identifies one machine from another.  Key fields become the store's primary
key when it is created.  The next chapter runs this program and shows what it
produces.
