# Typing-rule reference

Version 0.1 (M0 freeze candidate).

This document collects the typing rules of the Mensura core language into one
place.  It is the M0 deliverable that the roadmap describes as "a versioned
typing-rule reference collecting the rules from the design docs into one place,
detailed enough that two people implementing independently would build
compatible compilers" (`ROADMAP.md`, M0).  Until now those rules were correct
but scattered: expressions in `06-expressions.md`, pipeline primitives in
`07-pipelines.md`, the disjointness algebra in `08-lineage.md`, and the proofs
in `formal/Mensura/{Table,Completeness}.lean`.

## Scope of this freeze

This reference makes the **four tracked properties explicit in the table type**
and freezes the algebra that threads them:

- the pipeline algebra (the eight primitives), split-invariance, and the
  Tier A / Tier B boundary;
- **cardinality** (table-scoped) and **totality** (column-scoped), carried as
  qualifiers in `Qs`;
- **completeness** and **disjointness** (via a lineage hierarchy), table-scoped
  qualifiers in `Qs`.

It deliberately defers the **extensible qualifier meta-calculus** of
`docs/decisions/0004-qualifier-mechanism.md` (user-definable qualifiers, the
rule-combinator DSL) and the two qualifiers with no rules yet written,
**sampling** and **dependency**.  In the frozen core `Qs` is therefore
**concrete and closed**: it holds exactly the four built-in qualifiers above
(cardinality, totality, completeness, lineage), not an open, user-extensible
row.  The boundary between `Qs` and the content `C` is structure versus
propagated fact, and a qualifier's **scope** (table or column) is a field of the
qualifier, not what selects its group
(`docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`).  When the
meta-calculus arrives, this closed set may be opened to user-defined qualifiers
and the row made extensible again.

For disjointness it adopts the **lineage hierarchy** model (a tag tree, decided
structurally) and defers `08-lineage.md`'s heavier predicate-region elaboration
(the symbolic key-predicate region, the linear-arithmetic decidable fragment,
and the full `disjointness_check` / `@disjoint_partition` surface).  Anything the
hierarchy cannot decide is delegated to `assert` or `assume`.

## How to read this document

This reference is the consolidated normative ruleset.  It restates, in one
notation, what the per-concept documents decided, choosing the settled subset.
Authority is layered:

- The **Lean formalization** (`formal/Mensura/`) is ground truth for the
  algebra.  Every split-safety, disjointness, and reindexing claim here is
  backed by a named theorem, cited inline and indexed in section 11.
- The **per-concept documents** (`00`, `06`, `07`, `08`, and the ADRs) remain
  authoritative for rationale, examples, and any prose this reference
  compresses.
- This reference is the place that states the frozen core all at once.  Where it
  disagrees with a per-concept document or a Lean theorem, that is a bug in this
  reference to be reconciled, not a new decision.

The surface syntax shown is preliminary, as in `06`/`07`/`08`; the typing
content is not.  Snippets here are illustrative and are not check-gated (only
`book/` examples and `docs/examples/*.mensura` are compiled).

## 1.  The table type `Table<Qs, C>`

Every table has type `Table<Qs, C>` (ADR 0004).  The boundary between the two
parts is **structure versus propagated fact**
(`docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`): `C` is the
pure structure of the data, and `Qs` is every fact threaded through the algebra,
each a **qualifier** with a declared **scope**.  This freeze makes both parts
concrete and closed.

```
Table<Qs, C>

C   structure (what the data is)
      index columns
      non-index columns
      column domains

Qs  qualifiers (propagated facts; concrete and closed in this freeze)
      cardinality   (table):   singletons (card <= 1)  |  bag (card 0..many)
      totality      (column):  total | optional, per non-index column
      completeness  (table):   whether each key's bag holds all its rows
      lineage       (table):   a hierarchy of tags, the carrier for disjointness
```

- `C` is the **content**: the index (key) columns, the non-index columns, and
  their domains.  It carries nothing propagated.  Reindexing moves columns
  between the key and the non-key part.
- `Qs` is the **qualifier row**.  Each qualifier declares a **scope** that fixes
  which structural node it rides: **table** (one value for the whole table,
  possibly a universal over keys) or **column** (one value per column, spanning
  index and non-index columns).  In this freeze `Qs` is the closed set
  cardinality (section 3.2), totality (section 3.3), completeness (section 3.4),
  and lineage (section 3.5); the extensible qualifier framework, sampling, and
  dependency are deferred (section 13).
- There is **no per-key scope**.  A fact "about keys" is either a universal over
  all keys (table scope: cardinality, completeness, the disjointness invariant)
  or a fact on an index column (column scope on a key column).  A value that
  varies per runtime key-tuple is not type-level trackable (ADR 0013).

The older `Table<S, D, L, C>` quadruple from the overview is subsumed:
`Table<Qs, C>` carries the same facts, now as scoped members of `Qs` rather than
fixed type slots.

## 2.  Judgment notation

Two judgment forms run through this reference.  Both use ASCII only.

**Expressions.**  `Gamma |- e : tau` reads "in context `Gamma`, expression `e`
has type `tau`".  A context `Gamma` is a site-supplied pair (section 5.1): the
named values in scope, and the result type the site requires.  The type `tau` of
a column read reflects both content axes: a read at a single row is one value
(whose totality may be optional), and a read at a group is a bag (the key's
cardinality made visible).

**Operations.**  A pipeline primitive is a pure function over tables, written

```
op : Table<Qs, C>  ->  Table<Qs', C'>     [side conditions]   (Tier X)
```

with the effect on each property named explicitly, the Tier (A or B, section 7),
and the backing Lean theorem.  `Qs'` differs from `Qs` only where the operation
changes completeness or lineage (for example, `split` adds lineage branches,
`shrink_key` drops the lineage fact).  An operation that *demands* a fact lists
it under side conditions; an operation that *establishes* one says so.

## 3.  The four tracked properties

A table carries more than its rows.  Four facts are made first-class and
threaded by every operation; the type checker rejects a pipeline that would
violate one.  All four are qualifiers in `Qs`, distinguished by **scope** (table
or column), not by which group they live in; `C` carries only the structure they
qualify (ADR 0013).

### 3.1  Content schema (`C`)

The index columns and the non-index columns with their domains: the pure
structure of the data, the ordinary record-of-columns part of the type.
Cardinality and totality are not part of `C`; they are qualifiers in `Qs`
(sections 3.2, 3.3).  Reindexing moves columns between the index and the
non-index part.

### 3.2  Cardinality (table-scoped qualifier)

How many nested rows share a key.  At the type level it is a **two-value chain**:

```
singletons (card <= 1)   ⊑   bag (card 0..many)
```

`singletons` guarantees at most one row per key (a partial function from key to
row); `bag` allows any number, including none (`card 0`, "not sampled").
Cardinality is a single table-scoped classification: it is one uniform bound
that holds for every key, so "per key" names the *subject* of the bound, not its
scope (ADR 0013).  Operations move it along the chain (section 6); `singletons`
is the stronger fact and never arises by accident.

A third notion, **exhaustive** (every key has exactly one row), is *derived*, not
a stored level: `exhaustive = singletons and completeness` (section 3.4).  It is
a corollary of two properties, so the lattice needs only two points.

### 3.3  Totality (column-scoped qualifier)

Whether a non-index value is known or may be missing.  A cell is
`Cell = Option` (`formal/Mensura/Table.lean`): known or missing, always 0 or 1.
Totality is a **per-column** fact: a value is **total** (always known) by
default, and an **optional** value carries a `?` on its type (ADR 0010).  It is
orthogonal to cardinality: cardinality counts rows at a key, totality asks
whether one value is present.  Totality is column-scoped over both index and
non-index columns; the index requires total values, so an `extend_key` that
promotes a column into the key demands it be total first, a constraint of the
totality qualifier rather than a structural axiom (ADR 0013).

### 3.4  Completeness (table-scoped qualifier)

Whether **each key's bag holds all its possible rows**.  This is the useful,
tracked fact, and it reads uniformly across the cardinality chain:

- on a `bag` table, every bag is full (no rows missing at any key);
- on a `singletons` table, every key that should exist does (no empty keys),
  which is exactly the `exhaustive` corollary above.

Completeness here is a fact about the **current** key.  "Completeness over a
*partial* (coarser) key" is not a second standing property: it is the
`shrink_key` obligation, read operationally as "shrinking to that key still
yields complete bags".  It is established and consumed where `shrink_key` needs
it (section 8), not carried on every table.

### 3.5  Lineage and disjointness (table-scoped qualifier)

Each table carries a **lineage hierarchy**: a tree of tags recording the splits
it descends from.  Lineage is the carrier that makes **disjointness** (a
relation between two tables) a property each table holds on its own.  Two tables
are disjoint when their tags sit in exclusive branches of a common split,
decided structurally from the tree (section 9).  Relationships the hierarchy
cannot decide are delegated to `assert` or `assume`.

## 4.  Property axes are orthogonal

The four facts answer different questions and live on different axes, all as
qualifiers in `Qs`:

- **cardinality**: the row axis (`singletons` / `bag`), a table-scoped
  qualifier;
- **totality**: the value axis (`Cell = Option`), a column-scoped qualifier;
- **completeness**: a unary fact about one whole table, table-scoped;
- **disjointness**: a relation between two tables, carried by the table-scoped
  lineage hierarchy.

They compose but do not substitute for one another, with one deliberate
exception: `exhaustive = singletons and completeness`, so that corner is a
derived corollary rather than a fifth fact.

## 5.  Expression typing rules

Consolidates `06-expressions.md`.  Mensura has one expression sublanguage
(ADR 0007): the same grammar and the same rules at every site
(`when:`/`where:`, `@auto(...)`, and every pipeline operation).  A site differs
only in its context and required result type.

### 5.1  Context and purity

Every expression is pure and lazy: it reads no external state, performs no side
effect, and does not decide when it runs (`06`, "Purity").  A site supplies a
context `Gamma`:

- the **named values** in scope (an auth predicate exposes `principal`/`row`; a
  pipeline operation exposes the current table's columns through the lambda it
  is given);
- the **result type** the site checks against (a boolean for a predicate, a
  value for `@auto` or a derived column).

A bare name resolves against the context; member access `a.b` is typed against
the named value's type.  Which builtins (`now`, `env`, `lookup`, the
aggregates, ...) are in scope is a property of the context, not the grammar
(`06`, "The context model").

### 5.2  Application and precedence

Application is juxtaposition, left-associative: `f x y` is `(f x) y`; functions
are curried, so partial application is an ordinary value (this is what lets
pipeline stages compose under `|>`).  Application binds tighter than every infix
operator and looser than member access.  Operator precedence, loosest to
tightest (`06`, "Operators and precedence"; grammar in `04-grammar.md`):

| Operators | Assoc. | Notes |
| --- | --- | --- |
| `\|>` | left | the pipe |
| `or` | left | |
| `and` | left | |
| `not` | prefix | sits below the comparisons |
| `== != < <= > >=`, `in`, `is known`, `is missing` | non-assoc. | do not chain |
| `+ -` | left | |
| `* /` | left | |
| `-` (unary) | prefix | |
| `^` | right | binds tighter than unary minus |
| application | left | juxtaposition |
| `.` | postfix | member access, tightest |

Comparisons do not chain (`a < b < c` is rejected).  `-` between two atoms is
subtraction; a negated argument must be parenthesized, `f (-x)`.

### 5.3  The scalar rule: one known value

A **scalar operator** (`+ - * / ^`, the comparisons, `and`/`or`/`not`) requires
**a single known value**: `card 1` and not missing.  Applying one to a bag, or
to a value that may be missing, is a hard type error, never an implicit fold or
default (`06`, "Cardinality and missing values").  So `r.temperature > 30.0`
type-checks only when `temperature` is read at one row and is total.

The scalar domain also gates which operator applies, strictly and without
coercion (ADR 0014): numeric `number` splits into `int` and `real`; `+ - * ^`
need matching numeric operands; `/` is `real`-only; `< <= > >=` and `min`/`max`
take the orderable domains (`int`, `real`, `date`); and `== !=` take the
equatable domains, so they are **not** defined on `real`.

### 5.4  Bag combinators: many to one

A bag is consumed only deliberately.  The **bag combinators** are the explicit
way: `in` tests membership, `count`/`any`/`all` summarize, and the aggregates
`sum`/`min`/`max` reduce (`mean` is not a primitive: it is
`sum(x) / to_real(count(x))`; ADR 0014).  An aggregate requires a total bag;
`count` yields `int`, `sum` preserves a numeric domain, `min`/`max` preserve an
orderable domain, and `any`/`all` take a bag of `bool`.  Each returns a single
value.  The `g` of a group lambda `|k, g| ...` sees the whole bag at a key (so
`g.credits` is the bag of `credits`), and a scalar comparison on a bag is a type
error until a combinator collapses it (`max g.readings > 30.0`).

### 5.5  `is known` narrows

`is missing` / `is known` apply to values only and test the optional axis.  On a
total value `is known` is always true.  `is known` **narrows**: inside a branch
guarded by `r.x is known`, and on every row a `map` keeps with
`if r.x is known then r else ()`, the optional `x` is treated as total, so a
scalar operator may then use it.
This is one of the three ways to make an optional value known, alongside a
default/coalesce and an aggregate defined over missingness (ADR 0010).  Testing
a *row* for absence (`card 0`) is not an expression-level operation for now
(`06`, "Known and missing values").

### 5.6  Enumerated values

An `enum` is declared by name; its variants are string literals.  In an
expression an enumerated value is compared as a string (`r.status == "active"`),
and the checker validates the literal against the variant set, so `== "activ"`
is a compile error (`06`, "Enumerated values").

### 5.7  Conditionals

`if c then a else b` (ADR 0015): the condition `c` is a known `bool`, and the
two branches type to the same `Ty`, which is the result; if either branch is
optional the result is optional.  A non-`bool` condition or mismatched branches
is a type error.  The conditional is an ordinary value, valid in a field value
(`.flag = if r.hot then 1 else 0`) and as a `map` body branch
(`if c then r else ()`); it is the introduction site for the deferred `is
known` narrowing.

## 6.  Pipeline primitive rules

Consolidates `07-pipelines.md`.  A pipeline is an ordinary expression of table
type built from the one sublanguage: stages compose with `|>`, intermediates
are named with `let`, and several tables are tupled for a merge
(`(train, test) |> bind`).  There is no separate pipeline grammar.  This round
specifies the **primitives**; the named sugar (`filter`/`mutate`/`select`/
`aggregate`/windows/`tagged_*`) is deferred (section 13).

Pipeline lambdas are **key-first** (ADR 0015): `|k, r|` binds the key `k` (the
index columns as single values) and the value row `r` (the non-index columns as
single values); `|k, g|` binds `k` and the group `g` (the non-index columns as
bags); `split`'s `|k|` binds the key alone.  `|_, r|` ignores the key.  Read the
key with `k.id` and a value with `r.x`.  Each entry states the effect on
cardinality, totality, completeness, and lineage.

### 6.1  `map` (row multiset) -- Tier A

```
data |> map |k, r| (.bmi = r.mass / r.height ^ 2.0)   // transform
data |> map |_, r| if r.degraded then r else ()       // filter
data |> map |k, r| r                                  // keep (identity)
```

The key-first lambda receives the key `k` and value row `r` and returns a
**collection of value rows** (the formal `Multiset`, ADR 0015): `()` drops the
row, a bare row or record keeps one, `(a, b, ...)` expands to several (all
sharing one schema), and `if c then ... else ...` branches between collections
(a `()` branch adopts the other's schema).  Content: the non-index columns are
the collection's row schema; the **index is preserved**, so an output record may
not name an index column.  Cardinality: the **maximum collection size** -- `<=
1` preserves the input bound (so filtering keeps `singletons`), `>= 2` yields
`bag`.  Totality: as returned (optional if any contributing row's field is).
Completeness: preserved.  Lineage: preserved.  Tier A (`map_splitSafe`,
`map_bindHom`, `map_preservesDisjoint`).

Because the body is the formal multiset, **filtering and row-expansion are the
same primitive**: there is no `filter` primitive (`filterRows_splitSafe` is
derived), and a named `filter` may later be sugar for `if c then r else ()`.

### 6.2  `group_map` (per-key whole-group transform) -- Tier A

```
data |> group_map |k, g| (.total = sum g.credits)
```

The key-first lambda receives the key `k` (a single value, constant within the
group) and the group `g` (the non-index columns as bags).  Content: the output
columns are the return's.  Cardinality:
**inferred from the return** -- a single record yields `singletons` (one row per
key, the aggregate shape, which later lets `pivot` meet its precondition); a bag
yields `bag` (the window shape, one output row per input row).  Completeness:
preserved.  Lineage: preserved.  Tier A (`fiberMap_splitSafe`,
`fiberMap_preservesDisjoint`).  Window-shaped returns (`rank`, `cumsum`)
additionally need an ordering, a dependency-qualifier concern (deferred);
split-safety holds regardless.

### 6.3  `extend_key` / `shrink_key` (reindexing)

Reindexing is one idea in two directions; the direction fixes the Tier.

**`extend_key cols`** promotes non-index columns into the key.  Content: the
named columns join the index.  Each promoted column must be **key-eligible**
(equatable) and total, since it becomes part of the identity; a continuous
`real` measurement is rejected (ADR 0014).  Cardinality: an entity's rows are
redistributed across the finer key, so the bound cannot grow; preserved.
Completeness: preserved.  Lineage: preserved.  Tier A (`ungroup_splitSafe`,
`ungroup_preservesDisjoint`).

**`shrink_key cols`** drops index components into the non-index part.  Content:
the named key columns become ordinary columns.  Cardinality: rows that differed
only in the dropped component now share a key, so the bound rises to **`bag`**
(unless a following `group_map` reduces it).  Completeness: **demanded** at the
coarser retained key -- shrinking is split-safe only over a partition complete
there, so `shrink_key` consumes that obligation (section 8) and the result is
complete over the new key.  Lineage: **dropped** -- the branch structure over
the old key no longer applies, so the disjointness fact falls out of scope and
must be re-established (`assert`) or assumed (section 9).  Tier B
(`project_not_preservesDisjoint`).

### 6.4  `left_join` / `inner_join` (join a fixed table) -- Tier A

```
readings |> left_join machines (|k, l| l.machine)
```

Joins against a fixed right table; the key-first lambda maps a left row (key `k`,
value `l`) to the right table's key.  Content: the right table's columns are
added.  Cardinality: preserved when the right table is functional (`singletons`);
a non-functional right table multiplies rows in, raising the bound to `bag`.
Totality:
`left_join` makes the added right columns **optional** (an unmatched left row is
kept with them missing); `inner_join` drops unmatched rows and adds no
optionality.  Completeness: preserved on the left.  Lineage: preserved.  Tier A
(`leftJoin_splitSafe`, `innerJoin_splitSafe`, `leftJoin_preservesDisjoint`,
`innerJoin_preservesDisjoint`).

### 6.5  `split` / `bind` (partition and merge) -- Tier A

```
let (train, test) = data |> split |k| hash k < threshold
let full          = (train, test) |> bind
```

**`split |k| pred`** routes each entity (each key) wholly to one side of a pair
by a predicate over the key, never cutting a key's rows apart.  Content,
cardinality, completeness: unchanged on both sides.  Lineage: **adds two sibling
branch tags** under the current node, one per side; the halves are disjoint by
construction because they sit in exclusive branches (section 9).  Tier A
(`split_disjoint`; `bind_split` shows `bind` undoes it).

**`(a, b) |> bind`** is the multiset union of two tables of the same schema at
each key.  It is **total**: no precondition, always split-safe, associative and
commutative (`bind_comm`, `bind_assoc`).  Content: unchanged.  Cardinality:
binding inputs whose lineage is **disjoint** preserves `singletons`; binding
**overlapping** inputs may push a key above one row, raising the bound to `bag`.
Completeness: the union is complete over a key iff both inputs are.  Lineage:
**unions** the two tag-sets, so the result is disjoint from a third table iff
both inputs were (`bind_disjoint_iff`).  Tier A.

### 6.6  `unpivot` / `pivot` (reshape long and wide)

**`unpivot cols`** turns the named value columns into rows, spreading the column
*name* into the key.  Content: the names move into the index, the values into a
single column.  Cardinality: preserved.  Completeness: preserved.  Lineage:
preserved.  Tier A (`unpivot_splitSafe`, `unpivot_preservesDisjoint`).

**`pivot name value`** is the inverse, with two forms of different status:

- **Attribute form** (`name` is a non-index column): split-safe, admissible
  exactly when each (key, name) cell is **`singletons`** -- the guarantee an
  upstream `group_map`/aggregate provides.  Lineage preserved.  Tier A
  (`pivotAttr_splitSafe`; reversible against `unpivot` via
  `pivotAttr_reversible`).
- **Index form** (`name` is part of the key): not split-invariant, because a
  split can cut across the spread names; lineage dropped.  Tier B
  (`pivot_not_splitInvariant`).

The spread key column (for the index form, and for `unpivot`'s inverse) must be
**finite-enumerable**, i.e. an `enum`, since its values become column names
(ADR 0014); `bool` is excluded because `true`/`false` as column names break the
round-trip.

So `pivot` is where cardinality tracking pays off: the attribute form
type-checks only when the spread cell is `singletons`.

## 7.  Tier A / Tier B and split-safety

The central guarantee is **split-safety**.  In the formalization,

```
SplitSafe op  :=  PreservesDisjoint op  and  SplitInvariant op
```

(`formal/Mensura/Table.lean`).  Split-safe operations are closed under
composition (`SplitSafe.comp`) and identity is split-safe (`SplitSafe.id`), so a
pipeline built only from Tier A operations commutes with a split: running it on
the whole table equals running it on each side of a split and re-binding.  That
is the formal content of "no leakage between train and test".

Both halves of the definition are now tracked: `SplitInvariant` is the Tier
boundary, and `PreservesDisjoint` is what lets the lineage hierarchy carry a
disjointness fact through a Tier A pipeline intact (section 9).

- **Tier A** (split-safe): `map`, `group_map`, `extend_key`, `left_join`,
  `inner_join`, `split`, `bind`, `unpivot`, attribute `pivot`.  They compose
  freely and carry cardinality, completeness, and lineage facts end to end.
- **Tier B** (split-breaking): `shrink_key` and index `pivot`.  Each is sound
  only over a complete partition, so each must discharge a **completeness
  obligation** to be admitted (section 8), and each drops the lineage fact.

## 8.  Completeness: establish, preserve, consume

Completeness (each key's bag holds all its rows, section 3.4) is established in
one of three ways (`07`, "Tier B and completeness"):

- **mechanism**: a `collect` source is complete by construction (overview
  pillar 7), so a Tier B operation over it needs no further discharge;
- **check**: `completeness_check { assert ... }`, a pipe stage that establishes
  the fact locally; each `assert` is a boolean expression, and together they
  witness that the partition is complete over the relevant key.  The stage is
  placed ahead of the consuming operation;
- **annotation**: `@complete_over(col)` on a source store, establishing the fact
  globally (grammar deferred to the annotation family, section 13).

Tier A operations **preserve** completeness; `shrink_key` (and index `pivot`)
**consume** it, at the coarser retained key.  This coarser-key obligation is the
operational reading of "completeness over a partial key": it is discharged where
`shrink_key` runs, and the shrunk result is then complete over the new key.
`assume { ... }` is the escape hatch when the obligation cannot be discharged.

```
enrollments
|> completeness_check { assert row_count open_offerings == 0 }   // establish over student
|> shrink_key course                                             // consume; result complete over student
|> group_map |k, g| (.total_credits = sum g.credits)            // back to singletons
```

Remove the check (and `@complete_over`, and `assume`) and `shrink_key` is
rejected.

## 9.  Lineage and disjointness (the tag hierarchy)

Split-safety is defined with `PreservesDisjoint` (section 7), so disjointness is
part of the proven algebra.  This freeze *tracks* it with a **lineage
hierarchy** rather than a symbolic key-predicate region.  In the formalization
(`formal/Mensura/Table.lean`),

```
Disjoint T0 T1  :=  forall k, T0.rows k = 0  or  T1.rows k = 0
```

at every key at least one side is empty.  The hierarchy is the carrier: each
table holds a set of **tags** marking the branches of the splits it descends
from (the formal `addTag`/`dropTag`/`taggedSplit`/`taggedBind` machinery, with
`taggedSplit_taggedBind_left`/`_right`).  Disjointness is decided **structurally**:

> two tables are disjoint when their tags sit in **exclusive branches of a
> common split**.

Because structural exclusivity implies the semantic `Disjoint` (a split's sides
are disjoint, `split_disjoint`), the check is sound; because it is a tree-position
test, it is decidable with no solver.  What each primitive does to the tags:

| operation | lineage effect | disjointness | theorem |
| --- | --- | --- | --- |
| `map` / `group_map` | tags carried | preserved | `map_preservesDisjoint`, `fiberMap_preservesDisjoint` |
| `extend_key` | tags carried | preserved | `ungroup_preservesDisjoint` |
| `left_join` / `inner_join` | tags carried | preserved | `leftJoin_preservesDisjoint`, `innerJoin_preservesDisjoint` |
| `unpivot` | tags carried | preserved | `unpivot_preservesDisjoint` |
| `split` | adds two sibling branch tags | establishes | `split_disjoint` |
| `bind` | unions the tag-sets | disjoint from `c` iff both were | `bind_disjoint_iff` |
| `shrink_key` / index `pivot` | tags dropped (key change) | re-establish or assume | `project_not_preservesDisjoint`, `pivot_not_splitInvariant` |

Anything the hierarchy cannot decide is delegated:

- **`assert`** establishes the fact by a boundary check on the actual data, when
  two tables have no shared split ancestor but a checkable key witnesses
  non-overlap;
- **`assume`** admits the obligation by fiat, locally and visibly, for external
  data of opaque provenance.

A site that *demands* disjointness (notably `fit`/`evaluate`, deferred with the
learning operations, section 13) consumes the fact: it type-checks only when the
two tables are structurally disjoint, asserted, or assumed.

## 10.  Consolidated effect matrix

One row per primitive (pres. = preserved).  "card" gives the cardinality bound
after the operation; "lineage" the effect on the tag hierarchy.  Theorems are
the primary split-safety / disjointness backing; section 11 has the full index.

| op | content | card | total | complete | lineage | Tier | theorem |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `map` | cols := row schema | pres. if max size `<= 1`, else `bag` | as ret. | pres. | carried | A | `map_splitSafe` |
| `group_map` | cols := return | `singletons` or `bag` (per return) | as ret. | pres. | carried | A | `fiberMap_splitSafe` |
| `extend_key` | cols join index | pres. | pres. | pres. | carried | A | `ungroup_splitSafe` |
| `shrink_key` | key cols -> non-key | **-> bag** | pres. | **demanded** | **dropped** | B | `project_not_preservesDisjoint` |
| `left_join` | + right cols | pres. if right `singletons`, else `bag` | right **optional** | pres. left | carried | A | `leftJoin_splitSafe` |
| `inner_join` | + right cols | pres. if right `singletons`, else `bag` | pres. | pres. left | carried | A | `innerJoin_splitSafe` |
| `split` | unchanged | unchanged | unchanged | unchanged | adds branches | A | `split_disjoint` |
| `bind` | unchanged | `singletons` if disjoint, else `bag` | pres. | iff both | unions tags | A | `bind_split` |
| `unpivot` | names -> key | pres. | pres. | pres. | carried | A | `unpivot_splitSafe` |
| `pivot` (attr) | gather by name | demands `singletons` | pres. | pres. | carried | A | `pivotAttr_splitSafe` |
| `pivot` (index) | names leave key | -- | -- | -- | dropped | B | `pivot_not_splitInvariant` |

## 11.  Lean theorem index

Each rule above is backed by a theorem in the Lean formalization.  Names are
verbatim; the two files are `formal/Mensura/Table.lean` and
`formal/Mensura/Completeness.lean`.

**`Table.lean`** -- core algebra, split-safety, disjointness, lineage tags:

- composition: `SplitSafe.comp`, `SplitSafe.id`; definitions `SplitSafe`,
  `SplitInvariant`, `PreservesDisjoint`, `Disjoint`.
- `map`: `map_splitSafe`, `map_preservesDisjoint`, `map_splitInvariant`.
- `extend_key` (`ungroup`): `ungroup_splitSafe`, `ungroup_preservesDisjoint`,
  `ungroup_splitInvariant`.
- joins: `leftJoin_splitSafe`, `leftJoin_preservesDisjoint`,
  `innerJoin_splitSafe`, `innerJoin_preservesDisjoint`.
- `unpivot`: `unpivot_splitSafe`, `unpivot_preservesDisjoint`.
- `split` / `bind`: `split_disjoint`, `bind_split`, `bind_comm`, `bind_assoc`,
  `bind_disjoint_iff`.
- lineage tags: `addTag`, `dropTag`, `taggedBind`, `taggedSplit`,
  `taggedSplit_taggedBind_left`, `taggedSplit_taggedBind_right`.
- `shrink_key` (`project`): `project_not_preservesDisjoint`.
- index `pivot`: `pivot_not_splitInvariant`; `pivot_unpivot`.

**`Completeness.lean`** -- reindexing layer, group/fiber operations:

- `group_map` (`fiberMap`): `fiberMap_splitSafe`,
  `fiberMap_preservesDisjoint`, `fiberMap_splitInvariant`.
- attribute `pivot` (`pivotAttr`): `pivotAttr_splitSafe`,
  `pivotAttr_reversible`.
- sugar already proved Tier A: `filterRows_splitSafe`, `mutateCol_splitSafe`,
  `antiJoin_splitSafe`, `distinct_splitSafe` (these back named forms deferred in
  section 13, recorded here so implementers know the proofs exist).

## 12.  Conformance cases (seed)

The roadmap's M0 calls for a must-accept / must-reject suite (`ROADMAP.md`, M0;
"Validation criterion").  This section seeds it with canonical cases drawn from
the worked examples in `07`/`08` and `docs/examples/*.mensura`; the executable
suite itself is M1 work (`ROADMAP.md`, M1).

**Must accept:**

- Summarize by an attribute (`07`): `extend_key machine |> group_map |k, g| ...`,
  all Tier A, result `singletons` per key.
- Filter with `map` (ADR 0015): `map |_, r| if r.degraded then r else ()` keeps
  or drops a row and stays `singletons`; `map |k, r| (r, r)` expands to `bag`.
- Coarsen with the fact established first (`07`): `completeness_check { ... }
  |> shrink_key course |> group_map ...`.
- Split and re-merge (`07`): `split |k| ...` then `(train, test) |> bind`
  reconstructs the input (`bind_split`); the disjoint halves keep `singletons`.
- Split then demand: `split` establishes structural disjointness that a later
  disjointness-demanding site consumes without a check (the learning-operation
  syntax itself is deferred, section 13).
- Attribute `pivot` after an aggregate that yields `singletons`.

**Must reject:**

- `shrink_key` with no completeness fact (no check, no `@complete_over`, no
  `assume`).
- A disjointness-demanding site fed two tables that are not structurally
  disjoint and were neither asserted nor assumed.
- A scalar operator applied to a bag, or to an optional value without narrowing
  (`r.x > 30` where `x` is optional or read at a `bag`).
- Comparison chaining (`a < b < c`); a mixed positional/labeled `( )`.
- A `map` body that names an index column in its output record, or one that
  always drops (`map |k, r| ()`, no schema to infer); an `if` with a non-`bool`
  condition or branches of different type (ADR 0015).
- Attribute `pivot` where the spread cell may hold more than one value (not
  `singletons`).

## 13.  Open points (the deferred ledger)

What this freeze deliberately leaves open, so its scope is unambiguous.  Each is
specified ahead of the milestone that needs it (`ROADMAP.md`, "specs first").

- **The extensible qualifier meta-calculus (ADR 0004).**  User-definable
  qualifiers, the rule-combinator DSL, and the open `Qs` row are deferred.  In
  this freeze `Qs` is the closed pair completeness + lineage; reconciling this
  narrower scope with ADR 0004 (which anticipated freezing the full
  meta-calculus at M0) is a follow-up.
- **Sampling and dependency qualifiers.**  Both are `std` qualifiers with no
  propagation rules yet written; they join `Qs` once the meta-calculus lands.
- **The predicate-region elaboration of lineage (`08-lineage.md`).**  The
  symbolic key-predicate region, the linear-arithmetic decidable fragment, and
  the full `disjointness_check` / `@disjoint_partition` surface are deferred;
  the frozen core decides disjointness structurally from the tag hierarchy and
  delegates the rest to `assert` / `assume`.
- **`fit` / `evaluate` typing.**  The learning operations that *demand*
  disjointness are unspecified; when written they consume the lineage fact of
  section 9.
- **Cardinality-type surface notation.**  How `singletons` / `bag` (and the
  derived `exhaustive`) are written in a `Type` is the content/types document's
  job (`07`, "Forward references").  The total/optional `?` axis is settled
  (ADR 0010).
- **Named sugar.**  `mutate`, `select`, `aggregate`, `group`/`ungroup`/`project`,
  window functions (`rank`, `cumsum`), and `tagged_bind`/`tagged_split` are sugar
  over the primitives (their Tier-A proofs exist, section 11) and get their own
  round.  `filter` is now derivable as `map |k, r| if c then r else ()` (ADR
  0015), so it too is sugar, not a primitive.
- **Expression features the fuller surfaces need.**  Row-dropping and
  row-expanding `map` now land (the `( )` collection and `if`/`then`/`else`, ADR
  0015); bag-returning `group_map` (windows) still needs an ordering.  The
  `const`/`var` record-field marker is parsed but its column-scoped meaning and
  the view shape-conformance check are the next design.
- **Annotation grammar.**  `@audited`, `@versioned`, `@auto`, `@complete_over`,
  `@disjoint_partition` are named here but their surface lands with the
  annotation family.
- **Streaming.**  `sliding_window`, `latest`, window-closedness, and `on_change`
  refresh extend these rules (M5).
- **Physical units, precision, and measure semantics.**  Dimensional units, the
  `NxE` measured literal, and `@additive`/`@foldable` are M3.
- **The companion LL(1) grammar proof.**  The other M0 freeze artifact (core
  grammar proven LL(1)) lives with `04-grammar.md` and is not duplicated here;
  the freeze is contingent on it (`ROADMAP.md`, M0).
