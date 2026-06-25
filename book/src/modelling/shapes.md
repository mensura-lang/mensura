# Shapes

A **shape** is a structural contract a store can claim to satisfy.  It names a
set of attributes; a store that declares it must carry them.  Shapes let
several stores share a guarantee ("every record of this kind has an admission
date") without repeating the reasoning.

```mensura
{{#include ../examples/shape-conformance.mensura}}
```

`store students : PersonRecord` claims the shape.  The compiler checks that
`students` is a store of `Person` and carries a `const admission: date`, exactly
what `PersonRecord` requires.  Drop the `admission` attribute and the store no
longer conforms, and the program is rejected.

A shape may fix the unit, as `PersonRecord` does with `unit { Person }`, so that
only stores of that unit can claim it.

## Parameterised shapes

A shape can take parameters, which let one contract fit stores that differ in a
detail.  A `string` parameter can name an attribute, using a backtick template
for the name:

```mensura
{{#include ../examples/templated-shape.mensura}}
```

`Ageable["birthdate"]` renders the templated attribute name to `birthdate`, and
`persons` conforms because it carries `const birthdate: date`.  The same shape
applied with a different argument names a different attribute, so one `Ageable`
contract fits a person measured from a `birthdate` and, say, a department
measured from a `foundation_day`.

Shapes describe structure, not identity: that is what keeps them reusable
across units when they do not fix one.  The full design lives in
`docs/language/03-shapes.md`.
