# Pivoting and unpivoting

`unpivot` and `pivot` move between the two shapes of the same data: **long**,
where a column name becomes part of the key, and **wide**, where the key's
variants become columns.  They are a matched pair, designed to be true inverses,
and seeing where the inverse holds (and where it must not) is the point of this
chapter.

## unpivot: wide to long

`unpivot name value` folds **all** attribute columns into two columns: `name`
takes the folded column's name (as a new `enum` index column) and `value` takes
its cell.

```mensura
{{#include ../examples/unpivot-long.mensura}}
```

The wide `readings`, with a `temperature` and a `humidity` column, becomes a
long table keyed by `(ts, metric)` with a single `reading` column.  Two rules
follow from folding *all* attributes:

- The folded columns must share one domain (both `real` here); a heterogeneous
  wide table is projected down first.
- A missing cell yields **no row**, so the `reading` column is total by
  construction.

## pivot: long to wide

`pivot name value` is the inverse: for each residual key it gathers the values
indexed by the `name` key column into one wide row.  Because `unpivot` and
`pivot` are inverses on a functional table, folding and then spreading
reconstructs the original wide table.

```mensura
{{#include ../examples/reshape-roundtrip.mensura}}
```

`pivot` spreads an **index** column, so its `name` argument must already be in
the key.  An enum sitting in attribute position is rejected with a diagnostic
that names the fix, promote it with `extend_key` first:

```mensura
{{#include ../examples/pivot-promote.mensura}}
```

## pivot demands singletons

`pivot` type-checks only when each cell it spreads holds **at most one value**,
that is, when the input is `singletons`.  This is where cardinality tracking
pays off: a `(key, name)` pair that could hold several values has no single cell
to spread into.  If the pipeline first expands rows to a bag, the pivot is
rejected:

```mensura,ignore
readings
|> unpivot metric reading
|> map |k, r| (r, r)     // expands each row: now a bag
|> pivot metric reading  // rejected: pivot requires a singletons input
```

The long form's key discipline is what normally supplies the `singletons` fact:
one row per `(key, variant)`.  When several values genuinely share a
`(key, variant)`, the fix is an aggregate upstream (a `group_map` reducing the
bag to one value) before the pivot.

## When the wide columns are not total

`pivot` consumes **no completeness fact**.  An absent `(key, variant)` row
simply becomes a **missing cell** in the wide row, and the spread columns come
out total only when every variant is present for every key, a fact called
`exhaustive`.

This is why the round-trip above stays total: `unpivot` folds total columns, so
it establishes `exhaustive` by mechanism (every variant came from a column that
had a value in every row), and `pivot` spreads them back total.  A long table
that is *sparse*, missing some `(key, variant)` rows, is not exhaustive, so its
pivoted columns are optional: the wide form faithfully records that some cells
were never observed rather than inventing a value.  Totality is not lost
silently; it is tracked, and a downstream scalar operation on an optional column
must supply a default or narrow with `is known` first.

So the pair is a true inverse precisely because value-missing in the wide table
and row-absent in the long table carry the same information.  A sparse long
table round-trips as it is, with no completeness discharge and no `assume`.
