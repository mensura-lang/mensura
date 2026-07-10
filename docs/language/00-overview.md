# Mensura: overview

Mensura is a statically typed language for **data handling**, in which the
type system encodes properties that data manipulation libraries (pandas,
tidyverse, Polars, dplyr, …) leave to runtime, convention, or the
programmer's discipline.  The compiler rejects programs whose data-handling
operations are *syntactically valid but semantically wrong*: data leakage
between training and test sets, the wrong cross-validation strategy on
temporal data, biased sub-sampling, broken split-invariance, physical-unit
mismatches, and so on.

The novelty is not in the surface syntax but in the
typing rules attached to each operation.  Those rules are collected, for the
settled core, in `docs/language/09-typing-reference.md`.

## Motto

*Measure twice, run once.*

## Glossary

Several terms in Mensura overlap with everyday English, with statistics, or
with database and programming-language theory, but carry meanings that differ
from, or are more precise than, the common use.  This section defines every
such term before the rest of the documentation relies on it.  The definitions
are ordered from the most fundamental outward; later entries may cite earlier
ones.

### Observational unit (unit)

A **unit** is the *kind* of entity being observed: "Person", "Course",
"Transaction".  It is a type, not an instance.  In Wickham's tidy-data
vocabulary, a unit is an *observational unit*; Mensura makes the concept
syntactic with the `unit` declaration.

A unit declaration contains only an identity discipline: the list of
**index fields** whose values jointly name one distinct instance.  It carries
no attributes, no change-control policy, and no storage commitment.  Those
concerns belong on stores.

The word "unit" is overloaded in measurement theory (a kilogram is a unit of
mass) and in testing (a unit test).  In Mensura, "unit" always means
*observational unit* unless a qualifier like "physical unit" or "test unit"
is present.

### Index field, index column, index

An **index field** is a field declared inside a `unit { ... }` block.  It
contributes to the identity of observations of that unit.  Its type must be
*key-eligible* (equatable: `string`, `int`, `bool`, `date`, or `enum`);
continuous `real` is excluded because identity is decided by equality and
float equality is unreliable.

When the unit is tabulated in a store or view, each index field becomes an
**index column** of the resulting table.  The set of index columns is the
**index** of the table.

"Index" is used in three senses in the broader literature: a database index
(a data structure for fast lookup), a positional index (a row number), and an
identity index (a set of columns that name a row).  Mensura uses it
exclusively in the third sense.  A Mensura index is never a row number and
has nothing to do with B-trees.

### Key

A **key** is a concrete tuple of index-column values.  It is the runtime
counterpart of the index (which is the schema-level description of which
columns form the identity).  The key `("MATH-101", 2025)` is a particular
value of the index `(name: string, year: int)`.

Two rows that agree on every index-column value share the same key.  A key
does not by itself constitute a row; it is the address at which rows may be
found.  That is an important distinction of Relational algebra.

In pipeline lambdas, the key is bound to the parameter conventionally named
`k`.  Fields are accessed as `k.name`, `k.year`, and so on.

### Entity

An **entity** is a specific instance of a unit, picked out by a particular
key.  Alice (identified by `id = "alice-42"`) is an entity of unit `Person`.
MATH-101 in 2025 (identified by `(name = "MATH-101", year = 2025)`) is an
entity of unit `Course`.

The distinction between unit and entity mirrors the distinction between type
and value.  A unit is a category; an entity is a member.

An entity is distinct from a row.  A key identifies an entity; a row is a
record of non-index column values at that key.  When cardinality is at most 1
(the normal state at unit boundaries), entity and row coincide in practice,
but they are conceptually separate: an entity may be unobserved (cardinality
0, no rows at its key) or observed once (cardinality 1, exactly one row).
Inside a pipeline, transient states with cardinality greater than 1 are
permitted; there, one entity can have multiple rows simultaneously.  This
is the Mensura representation of a group.

### Row, observation

A **row** is a record of values, one per column (index and non-index), stored
in a table.  Within a key's group, there may be zero, one, or many rows.

An **observation** is a row viewed from the unit perspective: one recorded
instance of a unit.  At unit boundaries (stores, collect declarations), the
0-or-1 rule applies: each entity is either unobserved (no row at its key) or
observed exactly once (one row).  Inside pipelines, the two terms are
interchangeable; "observation" is preferred when emphasising the statistical
or unit-theoretic character of the data.

"Row" is used in database contexts to mean any record in a relation, and in
pandas/Polars to mean a positionally indexed record.  Mensura does not give
rows positional identities; the key is the only address a row has.  That
means that rows have no implicit order.

### Column, attribute

A **column** is a named field in a table's schema.  Index columns form the
key; non-index columns carry the per-entity data.

**Attribute** is a synonym for non-index column, preferred in declaration
contexts (store and shape bodies), where attributes are listed in an `attr`
block.  An attribute is a name and a type; store and shape bodies use the
same attribute language.

The word "variable" sometimes appears as a synonym for attribute in the
statistical literature.  Mensura avoids "variable" as a general synonym
for column.  "Feature" and "predictor" are not Mensura vocabulary; they
are ML-community names for specific uses of columns.

### Cardinality

**Cardinality** in Mensura describes how many rows a single key may hold.
It is a table-level qualifier with two states:

- **Singletons** (written `card ≤ 1`): each key has at most one row.  A key
  with no row is simply unobserved.  Singletons is the normal state at unit
  boundaries and after operations that reduce each group to one representative.
- **Bag** (written `card 0..*`): a key may hold any number of rows, including
  zero.  Bag cardinality is the transient state produced by `shrink_key` and
  any `group_map` that returns more than one row per group.

Cardinality is *not* a count stored in the data; it is a compile-time bound
on the number of rows per key.

The term "cardinality" has two other common meanings: in set theory, the size
of a set; in database design, the number of distinct values in a column (used
to assess selectivity).  Neither meaning applies here.  Mensura cardinality is
always a per-key row count, not a whole-table or whole-column measure.

### Totality, optional, missing

A column is **total** if its value is always known when a row is present.
Totality is the default.  A column marked `?` is **optional**: its value may
be **missing** (absent, null) even when the row exists.

Index columns are always total; a key field cannot be missing.  Only
non-index attributes may be optional.

Totality and cardinality are independent axes.  A table can have singletons
cardinality (at most one row per key) with optional columns (the row exists
but some values are unknown), or bag cardinality with total columns (many rows
per key, all values known).

"Nullable" is the SQL term for what Mensura calls optional.  Mensura avoids
"nullable" because it obscures that missingness is a property of the column's
type, not a database implementation detail.

### Completeness

**Completeness** is a table-level qualifier that says: for every key that
appears in the table, the table holds *all* the rows that belong to that key
under the current grouping.  It is not the same as totality (which is about
individual cell values) and not the same as cardinality (which counts rows).

Completeness is demanded where its absence would silently corrupt a
result: at a **reducing `group_map`**, whose group-wise aggregate over an
incomplete group would silently ignore the missing rows
(`docs/decisions/0023-completeness-consumed-by-the-reducer.md`).
`shrink_key` propagates the fact from the fine key to the coarser one on
the way there, and on a `singletons` input the reducer's demand discharges
trivially (a present key's single row is its whole group), so the ordinary
aggregation over a plain store needs no ceremony.  (`pivot` needs no such
fact either: an absent row simply becomes a missing cell; the related,
domain-relative fact `exhaustive` decides whether its spread columns come
out total.  See `docs/decisions/0020-reshape-as-a-true-inverse-pair.md`.)

Completeness is established by mechanism (a `collect` source guarantees it),
by explicit check (`completeness_check { ... }`), by annotation
(`@complete_over(col)`), or by fiat (`assume { complete }`).

### Table

A **table** in Mensura is inspired by the indexed-table model of Chapter 5 of
*Data Science Project: An Inductive Learning Approach* (F. A. N. Verri, 2026;
doi: 10.5281/zenodo.14498010): a mathematical object `(K, H, c)` where `K` is
the set of index columns, `H` is the set of non-index columns, and `c` is a
cell function mapping each `(key, column)` pair to an optional value.  A key
may map to zero, one, or many rows; each cell in a row is either known or
missing.

The Mensura type of a table is `Table<Qs, C>`: a row of **qualifiers** `Qs`
and a **content** schema `C`.

The chapter's model permits bag cardinality (multiple rows per key) as a
general case.  Mensura further restricts this at unit boundaries to the
0-or-1 rule and uses bag cardinality only as a transient state inside
pipelines.

The formal model in `formal/Mensura/Core/Defs.lean` represents a table as
`K → Multiset (Row H σ)`, where a `Row` is a dependent function from column
names to optional typed values (`(h : H) → Option (σ h)`), and the content at
each key is a multiset of such rows.  This differs from the chapter's
column-major aligned tuples in three ways: row order is not asserted (a
multiset is explicitly unordered); cross-column associations are structural
rather than positional (a row is one function, so its fields cannot desync);
and `bind` becomes a genuine commutative monoid (multiset union), which makes
split-invariance proofs unconditional rather than contingent on alignment
invariants.  The surface language and the typing rules follow the chapter's
presentation; the formal model is the proof-level encoding.

"Table" in SQL means a relation with no duplicate rows; in pandas, it means
a DataFrame with a positional row index.  Mensura's table differs from both:
rows have no positional address, duplicates (same key, same values) are
possible under bag cardinality, and the key is always explicit.

### Cell, bag (in the expression model)

A **cell** is a single optional value: either known (`some v`) or missing
(`none`).  It is the elementary unit of data at one `(row, column)`
intersection.

Inside pipeline lambdas, when the lambda receives a *group* (the `|k, g|`
form), each column of `g` is a **bag**: the multiset of all values of that
column across the rows at key `k`.  A bag is not a single value; scalar
operators cannot be applied to it.  Bag combinators (`count`, `sum`, `min`,
`max`, `any`, `all`) collapse a bag to a single value.

### Store, collect

A **store** is a Mensura declaration that creates a persistent, updatable
tabulation of observations of a unit.  It declares the unit being tabulated,
its attributes, domain resolution (for compound units), and change-control
policy.  A store is the primary source of raw data in Mensura.

A **collect** is a process-style counterpart to a store, where data arrives
through a streaming or ingestion mechanism rather than through CRUD
operations.  A collect carries a type-level completeness guarantee that stores
do not.

### Shape

A **shape** is a named structural contract for a table: an optional unit
clause plus `attr` blocks.  A shape is abstract: it describes structure only,
carrying no storage commitment, no domain resolution, and no policy.  Stores
and functions are typed against shapes.

A shape can be unit-fixing (`unit { Person }`), parameterised over a unit
(`unit { U }` where `U` is a `Unit` parameter), or unit-agnostic (no unit
clause).  The same shape may be claimed by many stores; one store may claim
many shapes.

### Qualifier

A **qualifier** is a compile-time fact about a table or a column that the
type system tracks and propagates through every operation.  Each primitive
operation carries rules for how each qualifier changes.

The four built-in qualifiers are cardinality, totality, completeness, and
lineage.

### Lineage, disjointness

The **lineage** qualifier is a tag tree that records the split ancestry of a
table.  Every `split` operation attaches a pair of sibling branch tags to its
two outputs, so their lineages sit in exclusive branches.

Two tables are **disjoint** when their lineage tag-sets lie in exclusive
branches of a common ancestor.  Disjointness is what licenses leak-free
train/test validation: if training and test tables are disjoint by lineage,
no entity can appear in both, and the compiler can certify that test metrics
are uncontaminated by training data.

Disjointness can also be established by explicit check, by annotation, or by
`assume`.

### Split-invariance, split-safety, Tier A, Tier B

An operation is **split-invariant** if the result of running it on a whole
table equals the result of running it independently on each side of a split
and then re-binding the outputs.  Formally, for an operation `f` and a split
`(L, R)` of table `T`: `f(T) = bind(f(L), f(R))`.

An operation is **split-safe** if it is split-invariant *and* it preserves
the disjointness facts in the lineage qualifier.

**Tier A** operations are split-safe: `map`, `group_map`, `extend_key`,
`left_join`, `inner_join`, `split`, `bind`, and `unpivot`.  They compose
freely and require no extra ceremony around splits.

**Tier B** operations are not split-invariant: `shrink_key` and `pivot`.
Both drop the lineage qualifier on their output, and that is the whole
content of the Tier: neither demands completeness (`shrink_key` propagates
it to the coarser key, ADR 0023; for `pivot` an absent row becomes a
missing cell, ADR 0020).  The completeness demand sits downstream, at the
reducing `group_map`.

The central guarantee of Mensura is that a pipeline composed entirely of
Tier A operations cannot introduce data leakage between disjoint partitions.
Split-invariance is also what makes Tier A pipelines data-parallel: each
partition can be processed independently and the results re-bound.

### Pipeline, view

A **pipeline** is a sequence of operations applied to one or more tables,
producing a new table.  Pipelines are expressed with the `|>` operator
(or equivalently by juxtaposition application).

A **view** is a named, materialised pipeline result.  A view declaration
hosts `let` bindings, `assert` statements, and a trailing table-valued
expression.  The view's content and qualifiers are computed from the pipeline,
not declared.

### Change control (deferred)

How persisted data may evolve, which changes are routine, which are
exceptional and audited, which values are versioned or auto-filled, is
change-control policy, expressed by store-only annotations (`@audited`,
`@versioned`, `@auto`, `@allowcreate`).  The whole family, including any
per-attribute mutability distinction (earlier drafts had `const`/`var`), is
deferred to a future policy document
(`docs/decisions/0019-attr-blocks-and-dropped-const-var.md`).

### `assume`

`assume` is an escape-hatch pipeline stage that admits an obligation by fiat,
locally and visibly in the source.  Writing `assume { complete }` tells the
compiler to treat the current table as complete without a runtime check.
Every `assume` is a local, readable acknowledgement that the programmer is
bypassing a check the type system would otherwise require.

`assume` is not a suppression pragma that silences a warning.  It appears in
the pipeline as ordinary syntax, is visible in diffs and code review, and can
be audited systematically.

### Domain

"Domain" has two distinct uses in Mensura and should not be confused.

A **domain block** in a store declaration resolves unit-reference fields to
concrete stores.  When a `Course` unit references a `Department` unit, the
store's domain block says which store holds the `Department` observations.

A **domain annotation** on a scalar column narrows the column's value space
beyond its primitive type: `code: string @domain(~/[A-Z]{5}/)` restricts the
string to a five-letter uppercase code.  Domain annotations on scalars are
deferred to a later implementation slice.

### Enum

An **enum** is a named finite set of string-valued variants:
`enum Status { "active" "inactive" "suspended" }`.  Enum names follow
PascalCase (they are types); variants are string literals and may contain
characters that are not valid in identifiers.

Enums have two important properties.  First, they are **equatable** (can be
used as index fields) because equality between enum values is exact string
comparison.  Second, they are **finite-enumerable**: their variants can be
spread across column names, which is what makes them the valid domain for
`unpivot`'s synthesised name column and for the key column being spread by
`pivot`.

`bool` is equatable but not finite-enumerable in Mensura, because spreading
`true`/`false` across column names and then pivoting back would not round-trip
correctly in the general case.

### Key-eligible, equatable, orderable, numeric

These are domain-level properties that determine which operations are
available for a column's type.

- **Key-eligible**: may appear as an index field.  Requires equatability.
  Key-eligible types: `string`, `int`, `bool`, `date`, `enum`.  `real` is
  excluded because float equality is unreliable.
- **Equatable**: supports `==` and `!=`.  Same set as key-eligible.
- **Orderable**: supports `<`, `<=`, `>`, `>=`, `min`, `max`.  Types:
  `int`, `real`, `date`.  Strings are not orderable (treated as opaque
  identifiers).  Enums are not orderable (no declared order on variants).
- **Numeric**: supports `+`, `-`, `*`, `^`, `sum`.  Types: `int`, `real`.
  No implicit widening between `int` and `real`.

### Naming conventions: PascalCase and snake_case

Mensura enforces two naming conventions at the compiler level, not merely by
convention.

**PascalCase** is for types: unit names (`Person`, `Course`), shape names
(`PersonRecord`), and enum names (`Status`).  The first character must be an
uppercase letter (or the name must be entirely caseless, see below).

**snake_case** is for terms (runtime values and declarations): store names
(`students`, `course_offerings`), view names, attribute names (`last_name`,
`year_founded`), and string-valued shape parameters.  All cased characters
must be lowercase.

Identifiers whose characters have no case (CJK characters, digits, and
similar) satisfy neither rule and are exempt from the check; they may appear
in both type and term positions.

The case distinction is how a reader can tell, from the name alone, whether
they are looking at a type or a term.  Violations are hard errors, not
warnings.

## Design pillars

1. **Tables are the central object.** A Mensura table is inspired by the indexed table
   of Chapter 5 of Data Science Project: An Inductive Learning Approach
   (F. A. N. Verri, 2026; doi: 10.5281/zenodo.14498010): a tuple
   `(K, H, c)` of index columns, non-index columns, and a cell function.
   Each row is an observation of an entity, addressed by its key (the tuple
   of index-column values); a key may have zero, one, or many rows
   (cardinality), and individual cell values may be missing (optional).
   Values are total by default, with optional ones marked `?` (see
   `docs/decisions/0010-attribute-totality.md`).

2. **The type of a table is `Table<Qs, C>`.** A table binding carries a row
   of **qualifiers** `Qs` and a **content** schema `C`, both checked at
   compile time. `C`, the **content**, is the schema: index columns, non-index columns,
   their domains, physical units, and semantic types.  Operations are
   typed as transformations on `Qs` and `C`; every primitive carries rules
   for how each qualifier propagates and how the content changes.

3. **Split-invariance is the default.** Chapter 5's Tier A operations
   (`bind`, `split`, `unpivot`, `map`, `group_map`, `extend_key`, and
   `left_join`/`inner_join` against a fixed table) are split-invariant by
   construction and require no extra ceremony.  The Tier B operations break
   split-invariance and drop the lineage qualifier: `shrink_key`, which
   drops a key component, and `pivot`, which spreads a key axis.  Neither
   demands completeness; the demand sits at the reducing `group_map`, which
   over a bag requires an explicit `completeness_check { … }` stage, a
   `@complete_over` annotation on its source, or an `assume`
   (ADR 0020, ADR 0023).  See `docs/language/07-pipelines.md`.

4. **Indexes and physical units are part of the type.** Each table declares its
   index columns, and each column declares its domain, including physical
   units and semantic refinements (CPF, email, regex-constrained strings).
   Physical-unit and semantic mismatches are compile errors, not runtime
   conversions.

5. **Change control is declared, not assumed.** How persisted data may
   evolve is store policy, to be expressed by declared annotations
   (`@audited`, `@versioned`, `@auto`, `@allowcreate`) rather than left to
   convention.  The family, including any per-attribute mutability
   distinction, is deferred to a future policy document.

6. **No defaults that hide assumptions.** Where existing tools silently
   pick a row order, a join key, an imputation strategy, or a CV scheme,
   Mensura requires the user to state it. Where the user wants to bypass a
   check, they write `assume`, locally and visibly.

7. **Properties are derived from mechanism, not declared.** When data
   enters Mensura through a `store` or `collect` declaration, the sampling,
   dependency, and (initial) lineage of the resulting table are fixed by
   the declaration's mechanism, not chosen by the programmer.

## In scope

- A core algebra for data handling (the operations of Chapter 5, with
  their typing rules).
- The `Table<Qs, C>` type (qualifiers plus content), with sampling,
  dependency, and lineage as standard-library qualifiers, and the
  disjointness constraint hook over lineage.
- A Polars-backed interpreter sufficient to run typed pipelines
  end-to-end.
- Compile-time prevention of the specific bug classes the postdoc report
  promises to address: leakage, wrong-CV-on-temporal-data, physical-unit
  mismatch, group-leak, broken split-invariance.
- Validation strategies and ML algorithm signatures as typed primitives
  (random forest, ARIMA, mixed-effects, k-fold, stratified, temporal,
  grouped), each with its disjointness obligations.
- A `mensura` toolchain: `check`, `run`, `test`, `fmt`, `repl`, `lsp`.

## Out of scope (for the academic deliverable)

- A general-purpose data-transformation language competing with pandas,
  Polars, or tidyverse on coverage. The postdoc report explicitly scopes
  the work to ML-validation correctness; breadth is a non-goal.
- Storage engines, query planning beyond what Polars provides, distributed
  execution.
- The web-service surface (`store`/`collect` endpoints, OAuth, REST,
  auditing/versioning at the HTTP layer) is **deferred** to M4 and may be
  spun off as a companion `mensura-server` project. The core language is
  usable without it. Its design is settled in
  `docs/decisions/0005-identity-and-authorization.md` and
  `docs/decisions/0006-transport-agnostic-surface.md`.
- A new query language for analytics. Mensura is a transformation
  language; it does not aim to replace SQL.

## Non-goals

- Turing-completeness as a goal in itself. Mensura is expressive enough
  for data-handling pipelines and ML validation; it deliberately stops
  short of becoming a general-purpose programming language.
- Performance parity with hand-tuned Polars. The interpreter exists to
  prove the type system carries through to execution.

## Where this fits in the literature

Existing formalizations of data-handling algebras (LaraDB by Hutchison
et al., 2017; Modin by Petersohn et al., 2020; SDTA/SDTL by Song et al.,
2021/2022) answer *can we express this operation?* and *can we execute
it efficiently?* They do not answer *should this operation be allowed?*
Mensura is the answer to the third question, built on top of the
indexed-table model and the split-invariance property developed in
Chapter 5.

## What this document is not

This is an orientation, not a specification. The typing rules, the grammar,
the disjointness solver, the algebra, and the toolchain each have their own
document under `docs/language/` and `docs/toolkit/`. The roadmap
(`ROADMAP.md`) lists them and the order in which they are written.
