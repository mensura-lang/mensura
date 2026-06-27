# Scalar types

Every attribute and index field has a **scalar type**, the domain of values it
may hold.  Mensura keeps this set small on purpose: a few well-understood
domains, each with clearly defined behaviour, rather than a sprawling type
zoo.  More will arrive (physical units and precision on `real`, for instance),
but the core stays minimal and grows only when a feature needs it.

There are six scalar types today:

```mensura
{{#include ../examples/scalar-types.mensura}}
```

- `string` for text labels and identifiers.
- `int` for discrete whole numbers: counts, years, integer identifiers.
- `real` for continuous measurements: a temperature, a vibration amplitude.
- `bool` for a yes/no flag.
- `date` for a calendar day or instant.
- A named `enum` for a fixed, finite set of string variants.

## Properties

A type's behaviour is described by four properties.  Each one decides which
operations a value of that type admits.

| type     | equatable | orderable | numeric | enumerable |
| ---      | ---       | ---       | ---     | ---        |
| `string` | yes       | no        | no      | no         |
| `int`    | yes       | yes       | yes     | no         |
| `real`   | **no**    | yes       | yes     | no         |
| `bool`   | yes       | no        | no      | no         |
| `date`   | yes       | yes       | no      | no         |
| `enum`   | yes       | no        | no      | **yes**    |

- **equatable** values support `==` and `!=`.  Every type is equatable except
  `real`: two continuous measurements are never reliably "equal" (the
  floating-point equality problem), so the type system forbids the question
  rather than letting it mislead.
- **orderable** values support `<`, `<=`, `>`, `>=` and the `min`/`max`
  aggregates.  The ordered types are the two numerics and `date`, so
  `temperature > 30.0` and `min visit_date` are well-typed, while strings and
  enums are compared only for equality.
- **numeric** values support arithmetic (`+ - * / ^`) and the `sum` aggregate.
  Arithmetic is strict: both operands must be the *same* numeric type, with no
  silent `int`-to-`real` conversion, and `/` (which can produce a fraction) is
  defined on `real` only.  Convert explicitly with `to_real` when you need to,
  for example `sum(g.x) / to_real(count(g.x))`.
- **enumerable** types have a finite, listable set of values.  Only `enum`
  qualifies; this is what lets a reshape spread the values across column names.

## Keys

An index field (a unit's identity, [Units and indices](units.md)) must be
**key-eligible**, which is exactly *equatable*: identity is decided by equality,
so a key must support it.  That admits `string`, `int`, `bool`, `date`, and
`enum`, and excludes `real`.  A continuous measurement is not an identity; if
you find yourself wanting to key by one, the value you mean is almost always a
discrete `int` (a tick, a bucket) or an `enum` (a category).
