# 0016: Reshape surface (pivot and unpivot)

## Status

Accepted.  Ratifies the surface syntax of the two reshape primitives,
`unpivot` and `pivot`, whose typing effects are already frozen in
`docs/language/09-typing-reference.md` sections 6.6 and 10.  This ADR fixes
only the surface; it adds no typing rule.  Implemented for M1 on the
`m1-completion` branch (the attribute form of `pivot` and `unpivot` are
Tier A; the index form of `pivot` is Tier B, landing with the completeness
machinery of ADR 0017).

## Context

`09` (the typing reference) marks its surface syntax "preliminary".  For the
reshape pair it shows `unpivot cols` and `pivot name value` illustratively,
but does not pin how the new columns an `unpivot` introduces are named, nor
how the two `pivot` forms (attribute and index) are written.  Per the
specs-first rule (`CLAUDE.md`), the surface needs ratifying before code.  The
reshape primitives are the last two of the eight in the frozen algebra; the
other six already have a settled surface (ADR 0015 and `10-views.md`).

`unpivot` folds several value columns into one, spreading their *names* into a
new index column; that name column must be **finite-enumerable** (an `enum`)
because its values become column names on the inverse (`09` section 6.6).
`ColumnType::Enum { name, variants }` and `is_enumerable()` already exist in
`mensura-types`, so the requirement is checkable today.

## Decision

### 1.  `unpivot name value (col1, col2, ...)`

Fold the listed non-index value columns into a single non-index column
`value`, tagged by a new index column `name`.  The folded columns must all be
non-index and share one domain (they collapse into one `value` column).  The
new `name` index column has a synthesized `enum` whose variants are the folded
column names.

```
readings |> unpivot metric reading (temperature, humidity)
```

turns the wide row `(ts | temperature, humidity)` into the long form
`(ts, metric | reading)`, where `metric` is an `enum` over
`{ temperature, humidity }`.  Cardinality, completeness, and lineage are
preserved (Tier A, `unpivot_splitSafe`, `unpivot_preservesDisjoint`).

### 2.  `pivot name value`

The inverse of `unpivot`: `name` is an existing column whose values become new
column names; `value` is an existing column whose values fill them.  The form
is selected by where `name` sits:

- **Attribute form** (`name` is a non-index column): Tier A, split-safe.
  Admissible only when the input is `singletons`, so each (key, name) cell
  holds at most one value (`pivotAttr_splitSafe`).  The `name` column must be
  a finite-enumerable `enum`; each variant becomes a `value`-typed column.
  Lineage and completeness preserved.
- **Index form** (`name` is an index column): Tier B, not split-invariant.
  Demands completeness over the retained key and drops lineage
  (`pivot_not_splitInvariant`); see ADR 0017 for the establish/consume
  mechanism.

```
readings |> unpivot metric reading (temperature, humidity)
         |> pivot metric reading        // attribute form, round-trips
```

### 3.  The enum requirement

The spread name column (the `enum` `unpivot` synthesizes, and the column
`pivot` reads) must be finite-enumerable, since its values become column
names (`09` section 6.6, ADR 0014).  `bool` is excluded: `true`/`false` as
column names break the round-trip.

## Consequences

Positive:

- The reshape surface is fixed and matches the frozen effects in `09`.
- The attribute/index split is visible at the call site (it is the position
  of `name`), with no new keyword.
- `unpivot` and attribute `pivot` need no completeness machinery (Tier A), so
  they land before ADR 0017's work.

Deferred:

- `collect`-by-mechanism completeness and the `@complete_over` annotation
  (`09` section 8/13) are not part of this surface.
- A heterogeneous unpivot (folding columns of differing domains) is rejected;
  it would need a sum domain, out of scope.

## Alternatives considered

1. **`unpivot cols` with implicit output names.**  Rejected: the name and
   value columns need explicit identifiers so downstream stages can refer to
   them; an implicit convention is invisible at the call site.
2. **A keyword to distinguish the two `pivot` forms.**  Rejected: the form is
   already determined by whether `name` is in the key, so a keyword would be
   redundant.

## Forward references

- `docs/language/09-typing-reference.md` sections 6.6, 10 (the frozen
  effects), section 11 (`unpivot_splitSafe`, `pivotAttr_splitSafe`,
  `pivot_not_splitInvariant`).
- `docs/decisions/0017-completeness-establish-consume.md` (the index form's
  completeness obligation).
- `docs/decisions/0014-scalar-domain-taxonomy.md` (enum is the only
  finite-enumerable domain).
