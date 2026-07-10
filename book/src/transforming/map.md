# Reshaping rows with map

`map` is the per-row workhorse.  Its key-first lambda receives the key and one
value row and returns a **collection of value rows**: return one row to rewrite,
return `()` to drop the row, or return several to expand it.  Rewrite, filter,
and expand are therefore not three operations but three uses of one, because in
Mensura a row maps to a *multiset* of rows, not to exactly one.

## Rewrite: one row in, one row out

Returning a single record replaces each row with a new one.  The output columns
are those of the returned record; the index is preserved.

```mensura
{{#include ../examples/map-rewrite.mensura}}
```

Each reading in kelvin becomes a reading in celsius.  A record literal is
`(.name = expr, ...)`, and `r.kelvin` reads the `kelvin` cell of the current
row.  Because the body returns exactly one row, per-key cardinality is
unchanged.

## Drop: filtering is map returning nothing

Returning the empty collection `()` drops the row.  A filter is just a `map`
whose body keeps the row on one branch and returns `()` on the other, so there
is no separate `filter` primitive.

```mensura
{{#include ../examples/map-drop.mensura}}
```

The body returns the row `r` when the machine is degraded and `()` otherwise, so
the view keeps only the degraded machines.  The body returns at most one row, so
the result is still `singletons`.

## Expand: one row in, several out

Returning a tuple of rows emits several rows for one input row.

```mensura
{{#include ../examples/map-expand.mensura}}
```

Here each sample is emitted twice.  A body that may return two or more rows
raises the per-key cardinality to `bag`: the key no longer identifies a single
row, which is exactly the fact the type carries forward.  That distinction is
what a later `pivot` consults, since [pivoting](pivot.md) demands `singletons`.

## Why one primitive

Collapsing rewrite, filter, and expand into `map` keeps the algebra small and
makes the cardinality rule uniform: the output cardinality is the maximum
collection size the body can return.  A body that always returns one row
preserves cardinality; one that can return zero or many changes it, and the type
records the change so downstream stages that need `singletons` are checked
against it.
