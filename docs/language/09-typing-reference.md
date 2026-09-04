# Typing-rule reference

Version 0.1 (M0 freeze candidate).

This document collects the typing rules of the Mensura core language into one
place.  It is the M0 deliverable that the roadmap describes as "a versioned
typing-rule reference collecting the rules from the design docs into one place,
detailed enough that two people implementing independently would build
compatible compilers" (`ROADMAP.md`, M0).  Until now those rules were correct
but scattered: expressions in `06-expressions.md`, pipeline primitives in
`07-pipelines.md`, the disjointness algebra in `08-lineage.md`, and the proofs
in the themed modules under `formal/Mensura/`.

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
qualifier, not a partition of `Qs`
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
  algebra.  Every split-safety, disjointness, and rekeying claim here is
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
      key columns
      non-key columns
      column domains

Qs  qualifiers (propagated facts; concrete and closed in this freeze)
      cardinality   (table):   singletons (card <= 1)  |  bag (card 0..many)
      totality      (column):  total | optional, per non-key column
      completeness  (table):   whether each key's bag holds all its rows
      lineage       (table):   a hierarchy of tags, the carrier for disjointness
```

- `C` is the **content**: the key columns, the non-key columns, and
  their domains.  It carries nothing propagated.  Reindexing moves columns
  between the key and the non-key part.
- `Qs` is the **qualifier row**.  Each qualifier declares a **scope** that fixes
  which structural node it rides: **table** (one value for the whole table,
  possibly a universal over keys) or **column** (one value per column, spanning
  key and non-key columns).  In this freeze `Qs` is the closed set
  cardinality (section 3.2), totality (section 3.3), completeness (section 3.4),
  and lineage (section 3.5); the extensible qualifier framework, sampling, and
  dependency are deferred (section 13).
- There is **no per-key scope**.  A fact "about keys" is either a universal over
  all keys (table scope: cardinality, completeness, the disjointness invariant)
  or a fact on a key column (column scope on a key column).  A value that
  varies per runtime key-tuple is not type-level trackable (ADR 0013).

The older `Table<S, D, L, C>` quadruple from early drafts of the overview is
subsumed: `Table<Qs, C>` carries the same facts, now as scoped members of
`Qs` rather than fixed type slots.

## 2.  Judgment notation

Two judgment forms run through this reference.  Both use ASCII only.

**Expressions.**  `Gamma |- e : tau` reads "in context `Gamma`, expression `e`
has type `tau`".  A context `Gamma` is a site-supplied pair (section 5.1): the
named values in scope, and the result type the site requires.  The type `tau` of
a column read reflects both content axes: a read at a single row is one value
(whose totality may be optional), and a read at a key's whole bag is a bag (the key's
cardinality made visible).

**Operations.**  A pipeline primitive is a pure function over tables, written

```
op : Table<Qs, C>  ->  Table<Qs', C'>     [side conditions]   (Tier X)
```

with the effect on each property named explicitly, the Tier (A or B, section 7),
and the backing Lean theorem.  `Qs'` differs from `Qs` only where the operation
changes completeness or lineage (for example, `split` adds lineage branches,
`demote` drops the lineage fact).  An operation that *demands* a fact lists
it under side conditions; an operation that *establishes* one says so.

## 3.  The four tracked properties

A table carries more than its rows.  Four facts are made first-class and
threaded by every operation; the type checker rejects a pipeline that would
violate one.  All four are qualifiers in `Qs`, distinguished by **scope** (table
or column), not by separate families; `C` carries only the structure they
qualify (ADR 0013).

### 3.1  Content schema (`C`)

The key columns and the non-key columns with their domains: the pure
structure of the data, the ordinary record-of-columns part of the type.
A column's domain includes its physical dimension when it has one
(`temperature[real]`, `11-physical-units.md`): dimension is *what the
data is*, and dimensional arithmetic is type computation over `C`, not a
propagated fact in `Qs` (ADR 0026, revising ADR 0013's anticipation that
units would be a per-column qualifier; precision keeps that slot).
Cardinality and totality are not part of `C`; they are qualifiers in `Qs`
(sections 3.2, 3.3).  Reindexing moves columns between the key and the
non-key part.

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
is the stronger fact and never arises by accident.  A source enters at its
**declared** cardinality
(`docs/decisions/0022-observations-as-bags-declared-store-cardinality.md`):
a plain `attr` store is `singletons` (the ADR 0001 boundary discipline) and
an `attr*` store of recurring observations is a `bag` keyed by the entity.

A third notion, **exhaustive** (every key has exactly one row), is *derived*, not
a stored level: `exhaustive = singletons and completeness` (section 3.4).  It is
a corollary of two properties, so the lattice needs only two points.

### 3.3  Totality (column-scoped qualifier)

Whether a non-key value is known or may be missing.  A cell is
`Cell = Option` (`formal/Mensura/Core/Defs.lean`): known or missing, always
0 or 1.
Totality is a **per-column** fact: a value is **total** (always known) by
default, and an **optional** value carries a `?` on its type (ADR 0010).  It is
orthogonal to cardinality: cardinality counts rows at a key, totality asks
whether one value is present.  Totality is column-scoped over both key and
non-key columns; the key requires total values, so an `promote` that
promotes a column into the key demands it be total first, a constraint of the
totality qualifier rather than a structural axiom (ADR 0013).

### 3.4  Completeness (table-scoped qualifier)

Whether **each key's bag holds all its possible rows**.  This is the useful,
tracked fact, and it reads uniformly across the cardinality chain:

- on a `bag` table it is contentful: every bag is full (no rows missing at
  any present key);
- on a `singletons` table it is trivially held: a present key's single row
  is its whole bag (`fiberCompleteWrt_of_functional`), so the qualifier is
  derived from the cardinality, at the source lift and at the key moves
  alike.  Key coverage ("every key that should exist does") is a different
  fact that nothing tracks table-wide; an absent key is an honest absence,
  not a partial bag, until a coarsening merges it into one.

Completeness here is a fact about the **current** key, against the fixed
intended population (the mechanized `FiberCompleteWrt`,
`formal/Mensura/Completeness/CompleteOver.lean`): every fiber of the
current key that is present is whole.  Because the reference is fixed,
the fact does not survive a genuine coarsening: an absent fine key is
invisible to the fiber-level fact, and `demote` merges it into a coarse
fiber as a gap (the fiber-gap counterexample of ADR 0035), so the key moves
re-derive the qualifier from the ADR 0024 gradings (section 6.3,
`docs/decisions/0035-completeness-cleared-by-demote.md`).  The operation
that *demands* it is the reducing `map_bags`, whose fold is silently
wrong on a partial bag (section 8,
`docs/decisions/0023-completeness-consumed-by-the-reducer.md`).

A second, **domain-relative** grade is tracked
(`docs/decisions/0020-reshape-as-a-true-inverse-pair.md`): for an
enum-domained key column `A`, **`exhaustive(A)`** says every residual key
present in the table carries its `(k, v)` row for every variant `v`.  Its
reference is `A`'s finite type domain, not the population, so it is
extensional and decidable per fiber; neither fact implies the other (a
faithfully recorded but sparse table is complete without being exhaustive).
It is established by `unpivot` by mechanism, by a `completeness_check`
witness, or by `assume`, and consumed by `pivot`'s totality upgrade
(section 6.6).

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
pipeline stages compose under `|>`).  The pipe is reversed application
(`x |> g` means `g x`); the checker routes both forms through one path, per
`docs/toolkit/01-application-checking.md`.  Application binds tighter than every
infix operator and looser than member access.  Operator precedence is
partial (ADR 0040; `06`, "Operators and precedence"; grammar in
`04-grammar.md`).  The ordered spine, loosest to tightest:

| Operators | Assoc. | Notes |
| --- | --- | --- |
| `\|>` | left | the pipe; accepts anything |
| `or`, `and` | left | one homogeneous level: a chain is one word |
| `not` | prefix | sits below the comparisons |
| `== != < <= > >=`, `in`, `is known`, `is missing` | non-assoc. | do not chain |
| `+ -` | left | |
| `* /` | left | |
| `-` (unary) | prefix | |
| `^` | right | binds tighter than unary minus |
| application | left | juxtaposition |
| `.` | postfix | member access, tightest |

`??` (ADR 0039) and the tacks `<< >>`, `<: :>` (ADR 0031) are
**unranked**, one level between the comparisons and `+ -`: operands are
arithmetic-or-tighter, self-chains keep their associativity (`??`
discharges right, a tack folds left), and meeting anything else, a
different unranked operator, a comparison, `is`, or a logic word, is a
parse error asking for parentheses.  In one breath: school math chains,
one logic word chains, parenthesize the rest.

Comparisons do not chain (`a < b < c` is rejected), and `and`/`or` do not
mix bare (`(a and b) or c`).  `-` between two atoms is subtraction; a
negated argument must be parenthesized, `f (-x)`.

### 5.3  The scalar rule: one known value

A **scalar operator** (`+ - * / ^`, the comparisons, `and`/`or`/`not`)
requires **a single value**: `card 1`.  Applying one to a bag is a hard
type error, never an implicit fold (`06`, "Cardinality and missing
values").  The missing axis **lifts** (ADR 0039, gated by
`formal/Mensura/Expr/Missing.lean`): an optional operand is accepted, the
result is optional, absent when any operand is absent, and the domain
rules below apply unchanged under the `?`.  A comparison over an optional
value is therefore an optional boolean (there is no three-valued logic:
absence absorbs, so `false and missing` is missing), and `??` is the only
discharge: `e ?? d` with a same-domain default, dimension included, total
exactly when the (chained) default is.  The decision boundaries stay
total: an `if` condition demands a total boolean (state the policy,
`?? false`), a fold or scan accumulates total values, `in` requires a
total bag, and keys are total (ADR 0010).

The scalar domain also gates which operator applies, strictly and without
coercion (ADR 0014): numeric `number` splits into `int` and `real`; `+ -`
need matching numeric operands; `< <= > >=` and `min`/`max`
take the orderable domains (`int`, `real`, `date`, `instant`); and `== !=`
take the equatable domains, so they are **not** defined on `real` or any
`real`-backed domain.

The two temporal domains are different families (ADR 0036): `date` is a
civil calendar day, `instant` an absolute moment (UTC, millisecond
precision).  Both are equatable, orderable, and key-eligible; neither is
numeric, and comparing one against the other is a domain mismatch no
conversion repairs.  Their arithmetic is the torsor rule of ADR 0036
decision 4, implemented and gated by `formal/Mensura/Units/Torsor.lean`
(ADR 0021): `instant - instant : time[real]`, and
`instant +/- time[real] : instant`, with translation exact-or-error at
evaluation (the duration must be a whole number of milliseconds, and the
result must stay within years 0001-9999; `06-expressions.md`, "Temporal
arithmetic").  Two instants do not add, a point never scales, and `date`
arithmetic is deferred outright.

Dimensions refine the numeric rules (`11-physical-units.md`, ADR 0026).
A `real`-backed domain carries a dimension exponent vector, with bare
`real` the zero vector; `int` is never dimensioned.  Then:

- `+ -` require *equal* domains, dimension included, so `meter + second`
  and `meter + 1.0` are rejected;
- `*` and `/` combine two `real`-backed operands, adding (respectively
  subtracting) their exponent vectors; a zero result vector is bare
  `real`.  `*` on matching `int`s stays `int`; `/` never applies to
  `int` (unchanged);
- `^` on a dimensioned base takes an integer-literal exponent (optionally
  negated) and scales the vector; `^ 0` yields bare `real`.  On
  dimensionless operands `^` keeps the matching-domain rule
  (`real ^ real`, `int ^ int`).

### 5.4  Bag reduction: many to one

A bag is consumed only deliberately.  Since ADR 0031 there are **two
primitives** that consume one, plus two spellings of language around them:

```
fold    : combiner -> (element -> value) -> bag -> value
scan    : combiner -> (row -> value) -> (row -> key) -> rows -> bag
prescan : combiner -> (row -> value) -> (row -> key) -> rows -> bag
map     : (element -> value) -> bag -> bag
desc    : orderable -> orderable
```

`fold` reduces, `scan` and `prescan` reduce *in order* (emitting every
intermediate where `fold` keeps the last), `map` transforms element-wise, and
all are curried, so a partial application is an ordinary value.  Everything
else is derived.

**The ordered primitives take the fiber, and both their lambdas take a row.**
One extracts the value to accumulate, the other the key to order by.  Sorting
values by a sibling column requires seeing the row that carries both, so the
trailing argument is `rows` (the fiber) rather than a projected bag.  ADR 0031
Decision 8 writes the derived bindings with a `(|v| v)` mapper, which cannot
typecheck for that reason; the value extractor comes from the call site.

**The order key's obligation is orderable *and* total.**  Orderability is a
decidable type fact, which is why the key may be an open lambda while the
combiner may not.  Totality is required because a missing value has no
position in the order; the fix is upstream (filter, or coalesce the key).
Note `string` is not orderable (ADR 0014), so ordering by a name is not
expressible today: a real limitation rather than a design intent.

**`desc` marks a key value as ordered by its dual.**  Never storable and never
ascribable, like the rows type: it is checker-internal, so it has no spelling
in type position and cannot reach a column.  Because the marker sits on
*values*, direction is per-component, which no global reverse flag could
express.  A direction marker rather than a comparator is forced by the same
epistemics as the combiner table: a comparator's obligation is a law (a strict
total order), unverifiable on a lambda.  In `formal/` it is Mathlib's
`OrderDual`, so the arrangement absorbs it at no proof cost.  Its consumers
are the ordered primitives' order keys and `latest`'s point (section 6.9).

**A scan's result is a bag, so it is the window shape**, one output row per
input row.  Its completeness demand is nonetheless per **combiner row**, not
per shape (ADR 0037 decision 5, settling ADR 0029's flag): under a
fold-admitting combiner a scan *contains* its reduction (its last entry is
the fold, `Mensura.scanl_getLast_eq_foldBag`, and every entry folds a
prefix), so it demands the fiber-completeness fact a reducing `map_bags`
demands (section 6.2, ADR 0023), discharged the same two ways.  The keep
combiners (`<:`, `:>`) demand nothing: their outputs are claims about
adjacency among *present* rows, which a partial bag represents honestly.
The demand axis is "does any output row's claim quantify over absent rows",
and the closed table's admission column is the line.

**Optionality follows the combiner's identity.**

| | identity (`+ * or and`) | identity-free (`<< >> <: :>`) |
| --- | --- | --- |
| `scan` | total | total |
| `prescan` | total | **optional** |

An inclusive scan is total at every row because position *i* folds elements
`1..i`, never empty, so the identity is never consulted.  An exclusive scan's
first position folds the *empty* prefix, so an identity-free combiner has no
answer there.  **`series.lag`'s missing first row is this rule, not a rule
about `lag`**: it is `prescan` at keep-right, and keep-right has no identity.
`series.lead` is the mirror, being `lag` at the dual key, so there the *last*
row is missing.

**The order key must be tie-free, and a scan demands it.**  A scan's
arrangement is unique only when the key is injective on each fiber (ADR 0029
Decision 11's tier 1, `Mensura.IsArrangement.unique`); with ties the
intermediate values depend on how the rows happened to be stored, so the same
input can give different output.  That is the ordered counterpart of the
reducing shape's completeness demand, and the same *kind* of obligation: a
property of the data, undecidable in general, established upstream or admitted
by fiat.  ADR 0029 Decision 11 already says ties are "structurally the same
problem as completeness"; the checker enforces it that way.

Two ways to discharge it:

- **A grading (tier 1), checked.**  A projection key `|r| r.c` is tie-free when
  `key + {c}` contains a grading (section 3, ADR 0024), because two rows of one
  fiber agreeing on `c` then agreed on a whole grading and are the same row.
  `Mensura.keyInjOn_demote_tag` is that argument.  This makes the common window
  shape ceremony-free: a history keyed by `(entity, time)` and `demote`d to
  `entity` carries the grading through the key move unchanged, so the time is
  unique within each group by construction.  A `desc` marker is transparent to
  the question, since the dual of an injective key is injective.
- **`assume { arranged }` (tier 3), claimed.**  For an ungraded column or a
  computed key, the obligation is admitted locally and visibly, exactly as
  `assume { complete }` admits completeness (section 8).  This is the home ADR
  0029 Decision 11 left open for its arbitrary-tiebreak hatch; ADR 0017's block
  form was written to generalize this way, so it needs no new surface.

A key the checker can neither prove nor see claimed is an **error**, not a
silent stable sort.  Tier 2 (lexicographic tuple keys) is still not
expressible, and when it lands the grading lookup must extend to the tuple's
whole component set, since a tuple can be injective when no single component
is.  Where ties are genuinely unresolvable the arrangement is still a stable
sort, so a claimed-but-false key gives a reproducible answer rather than a
nondeterministic one; that is a courtesy of the implementation, not a
guarantee the type carries.

**What tie-freedom does not give you.**  The tie model settles whether the
arrangement is *determined*, not whether it is *gap-free*.  The `series`
vocabulary is defined over the rows present in the fiber, so `lag` means
"the previous row in this bag", not "the previous time step".  Over an
order key reading `1, 2, 4, 5` every obligation above is discharged and
`lag` at `4` still reports `2`'s value, which a caller differencing
consecutive readings will take for `3`'s.  `rank` counts present rows
rather than positions, and `cumsum` totals whatever the bag holds.
The ordered operations carry no contiguity obligation of their own, which
is settled rather than open (ADR 0037 decision 6): a dense and an
irregular series are indistinguishable over a raw order key, so no
mechanism could discharge such an obligation and no marker should claim
it.  A discharged `arranged` therefore means the window is deterministic,
not that it is faithful to an underlying regular series, and the fix for
positional-as-temporal code is to stop writing rates positionally.  Over
a **window grid** contiguity is decidable, and there it is established by
a mechanism rather than obliged: `dense` (section 6.10) completes the
grid, after which the previous present row *is* the previous slot.  The
general case over a raw order key, an explicit `resample` with a stated
step, stays deferred for the reason above.

**The combiner is closed, the mapper is open.**  A fold over an *unordered*
bag is deterministic only when the combiner is associative and commutative,
and those are laws no checker can verify on a user-supplied lambda.  So the
combiner is a backticked operator drawn from a fixed table, whose algebra is
compiler knowledge; the mapper is any expression, because its obligation is
merely a type check.  The table:

| combiner | folds | identity | admitted under |
| --- | --- | --- | --- |
| `` `+` `` | a numeric domain, dimension included | `0` | `fold`, `scan` |
| `` `*` `` | a **dimensionless** numeric domain | `1` | `fold`, `scan` |
| `` `<<` ``, `` `>>` `` | an orderable domain, dimension included | none | `fold`, `scan` |
| `` `or` ``, `` `and` `` | `bool` | `false`, `true` | `fold`, `scan` |
| `` `<:` ``, `` `:>` `` | any one domain | none | `scan` only |

Three rows carry a restriction worth stating outright.  `` `*` `` is
dimensionless-only because a fold's accumulator type must be invariant, while
dimensioned `*` *adds* exponent vectors (ADR 0026), so the product's dimension
would depend on the bag's cardinality; `` `+` `` requires equal dimensions and
preserves them, so `bag.sum` works at every dimension while `bag.prod` does
not.  `` `<<` ``/`` `>>` `` have no identity, because there is no smallest
element of nothing; a group arises from rows and is never empty, which is what
keeps their result total (`Mensura.foldBagOpt_isSome_of_ne_zero`).  The tacks
are associative but not commutative, so they are admitted only where a key
supplies the order a bag lacks.  An unknown combiner is an error naming the
table, and the table extends by decision record, never by a call site.

**The derived vocabulary is a library, not language.**  `bag.sum`, `bag.prod`,
`bag.min`, `bag.max`, `bag.any`, and `bag.all` are const bindings in the
bundled `bag` module (`12-modules-and-imports.md`), each a partial application
of `fold` at one row of the table, and they are imported like any other
module.  There is no implicit prelude: `import bag` or the name is unknown.
`mean` is expressible rather than primitive (`bag.sum b.x / to_real (#b.x)`).

**`#` is cardinality**, replacing `count`: `#b` is the group's row count and
`#b.x` a projected bag's size.  Unlike the value reductions it does not demand
a total bag, since it never reads a value: a row whose column is missing still
counts, and it is always a dimensionless `int`.

`in` still tests membership.  The `b` of a bag lambda `|k, b| ...` is the
**fiber**, the bag of rows at that key (section 5.5), and a scalar comparison
on a bag is a type error until a reduction collapses it
(`bag.max b.readings > 30.0`).

Backed by ADR 0029's Stage 1 in `formal/Mensura/Fold.lean` (section 11).

### 5.5  The fiber: a bag of rows

The `b` of a bag lambda types at a dedicated **rows** type: the bag of rows at
one key, matching `Table.rows : K -> Multiset (Row H σ)` in
`formal/Mensura/Core/Defs.lean` exactly.  Member access on it is *projection
sugar*, defined by one equation:

```
b.x  ==  map (|r| r.x) b
```

So `b.credits` is the bag of `credits` across the rows at the key, exactly as
before; the columnar record-of-bags is now the derived presentation rather
than the model.  Two things follow: `#b` counts the group's rows without
naming an arbitrary column, and a fold's mapper over `b` itself receives a
whole *row*, so `fold `+` (|r| r.mass / (r.height * r.height)) b` is
expressible.

Projection is sound because the fiber's columns are jointly indexed by the
group's rows.  That alignment is **provenance**, not structure a type carries,
which is why it does not generalize: there is no `zip` of two arbitrary bags,
since bags are unordered and have no i-th element to pair.

The rows type is not a `bag` of records, and nested collections do not arrive
through it: it is the fiber's type, constructible only where groups are, it
never enters a column, and it is not writable in type position.  Bare `b` in a
value position is a type error, as it was when `b` was a record.

### 5.6  `is known` and the `??` discharge

`is missing` / `is known` apply to values only, lifted results included, and
test the optional axis, returning a *total* boolean.  On a total value
`is known` is always true.  They do **not** narrow: flow-sensitive narrowing
is deferred (ADR 0039 alternative 4), and the way to make an optional value
total is the `??` discharge: `e ?? d`, the present value or the default,
same domain, dimension included; right-associative, so a chain discharges at
its first present value and is total exactly when its final default is.
Testing a *row* for absence (`card 0`) is not an expression-level operation
for now (`06`, "Known and missing values").

### 5.7  Enumerated values

An `enum` is declared by name; its variants are string literals.  In an
expression an enumerated value is compared as a string (`r.status == "active"`),
and the checker validates the literal against the variant set, so `== "activ"`
is a compile error (`06`, "Enumerated values").

### 5.8  Conditionals

`if c then a else b` (ADR 0015): the condition `c` is a *total* `bool` (the
branching boundary of ADR 0039 decision 3: an optional comparison must state
its absent-row policy, `(...) ?? false` or `?? true`), and the two branches
type to the same `Ty`, which is the result; if either branch is optional the
result is optional.  A non-`bool` condition or mismatched branches is a type
error.  The conditional is an ordinary value, valid in a field value
(`.flag = if r.hot then 1 else 0`) and as a `flat_map` body branch
(`if c then r else ()`).

### 5.9  Const functions (ADR 0030)

A top-level const binding may be a **function**: a lambda evaluated at
compile time to a closure (`12-modules-and-imports.md`).  Its type is a
function value carrying the closure itself; parameters are unannotated, so
no signature is inferred at the definition.  A **saturated application** is
typed by substituting the argument expressions into the body and typing the
result in the caller's context, which is exact per call site: `add1 1` is
`int`, `add1 r.temp` is `temperature[real]`.

Two rules fix the surface.  A multi-parameter lambda is **tupled**
(`|a, b| e` binds one 2-tuple parameter; the pipeline lambdas `|k, r|` /
`|k, b|` read the same way), so currying is written explicitly as nested
lambdas.  And **every application is saturated or an error**: partial
binding is ordinary application of a curried function (`add 1` where
`add` is `|a| |b| a + b`), never an arity-tracking mechanism.

A function value never enters a column (a function-valued record field is
rejected), cannot be ascribed (the type grammar has no arrow), and cannot
be *created* in a view body, where lambdas remain pipeline-operation
arguments; a view body may *use* a const function by name.  Recursion is a
compile error.  This realizes section 5.2's "partial application is an
ordinary value" for user functions and settles ADR 0018's open question 2.

## 6.  Pipeline primitive rules

Consolidates `07-pipelines.md`.  A pipeline is an ordinary expression of table
type built from the one sublanguage: stages compose with `|>`, intermediates
are named with `let`, and several tables are tupled for a merge
(`(train, test) |> union`).  There is no separate pipeline grammar.  This round
specifies the **primitives**; the named sugar (`filter`/`mutate`/`select`/
`aggregate`/windows/`tagged_*`) is deferred (section 13).

Pipeline lambdas are **key-first** (ADR 0015): `|k, r|` binds the key `k` (the
key columns as single values) and the value row `r` (the non-key columns as
single values); `|k, b|` binds `k` and the bag `b` (the non-key columns as
bags); `split`'s `|k|` binds the key alone.  `|_, r|` ignores the key.  Read the
key with `k.id` and a value with `r.x`.  Each entry states the effect on
cardinality, totality, completeness, and lineage.

### 6.1  `flat_map` (row multiset) -- Tier A

```
data |> flat_map |k, r| (.bmi = r.mass / r.height ^ 2.0)   // transform
data |> flat_map |_, r| if r.degraded then r else ()       // filter
data |> flat_map |k, r| r                                  // keep (identity)
```

The key-first lambda receives the key `k` and value row `r` and returns a
**collection of value rows** (the formal `Multiset`, ADR 0015): `()` drops the
row, a bare row or record keeps one, `(a, b, ...)` expands to several (all
sharing one schema), and `if c then ... else ...` branches between collections
(a `()` branch adopts the other's schema).  Content: the non-key columns are
the collection's row schema; the **key is preserved**, so an output record may
not name a key column.  Cardinality: the **maximum collection size** -- `<=
1` preserves the input bound (so filtering keeps `singletons`), `>= 2` yields
`bag`.  Totality: as returned (optional if any contributing row's field is).
Completeness: preserved.  Lineage: preserved.  Tier A (`flatMap_splitSafe`,
`flatMap_unionHom`, `map_preservesDisjoint`).

Because the body is the formal multiset, **filtering and row-expansion are the
same primitive**: there is no `filter` primitive (`filterRows_splitSafe` is
derived), and a named `filter` may later be sugar for `if c then r else ()`.

### 6.2  `map_bags` (per-key whole-bag transform) -- Tier A

```
data |> map_bags |k, b| (.total = bag.sum b.credits)
```

The key-first lambda receives the key `k` (a single value, constant within the
bag) and the **fiber** `b`, the bag of rows at that key, whose member
access is projection sugar (section 5.5).  Content: the output
columns are the return's.  Cardinality:
**inferred from the return** -- a single record yields `singletons` (one row per
key, the aggregate shape, which later lets `pivot` meet its precondition); a bag
yields `bag` (the window shape, one output row per input row).  Completeness:
**demanded by the aggregate (reducing) shape** (ADR 0023): a
fold over a partial bag is silently wrong, so a reducing body over a `bag`
input requires the fact "complete over the current key".  The fact is
carried through the operation: every body expressible today emits at least
one output row per present fiber (the non-emptying hypothesis of
`fiberMap_exhaustive`), so a present key stays present; whether every
future body preserves this is an open question of `07-pipelines.md`.  Over a
`singletons` input the obligation discharges trivially -- a present key's
single row is the identity's whole fiber (`fiberCompleteWrt_of_functional`)
-- so the checker recognizes that base case from the input cardinality and
the ordinary aggregation over a plain store needs no establishment step.
The window shape's demand follows its combiner, not its shape: a
fold-admitting scan contains its reduction and demands the same fact, and
the keep combiners demand nothing (section 5.4, ADR 0037 decision 5).
Lineage: preserved.  Tier A
(`fiberMap_splitSafe`, `fiberMap_preservesDisjoint`; a monoid fold's case is
`foldFiber_splitSafe`, and a scan's is `scanFiber_splitSafe`).  Window-shaped
returns (`series.rank`, `series.cumsum`) additionally need an ordering, which
the call site names by a `scan`'s key argument (section 5.4).
Split-safety holds regardless, because a split routes a key's *whole* bag to
one side, so neither the bag a reduction sees nor the fiber a scan arranges is
ever torn.

### 6.3  `promote` / `demote` (rekeying)

Reindexing is one idea in two directions; the direction fixes the Tier.

Cardinality at the key moves is **key-graded** (ADR 0024): the qualifiers
carry *gradings*, column sets over the flat table (key or not) over which
the table is known functional (`Mensura.Functional`), and the scalar
cardinality is derived as "some grading is a subset of the current key".
A grading is a fact about the flat table, indifferent to which columns
currently form the key, so the key moves change the key, leave the
gradings untouched, and re-run the subset check; the content-identity
stages (`assume`, `completeness_check`) carry the gradings; every other
operation resets them to match its own output cardinality until its
transport row is mechanized.  A `singletons` source seeds its key as a
grading, which is what makes the pair truly inverse: either round-trip
order restores `singletons` (`demote_promote`, `promote_demote`), and a
`bag` whose grading fits the grown key promotes back to `singletons`.

**`promote cols`** promotes non-key columns into the key.  Content: the
named columns join the key.  Each promoted column must be **key-eligible**
(equatable) and total, since it becomes part of the identity (and totality
is the `demote_promote` inverse-domain side condition); a continuous
`real` measurement is rejected (ADR 0014).  Cardinality: derived from the
gradings; a `singletons` input stays `singletons` (`promote_functional`),
and a `bag` carrying a grading inside the new key becomes `singletons`.
Completeness: re-derived from the graded cardinality (ADR 0035): a result
graded `singletons` is `Complete` (a present singleton fiber is whole,
`fiberCompleteWrt_of_functional`), and a `bag` result preserves the
incoming fact (refining the key partitions each fiber by row content, and
a whole fiber partitions into whole sub-fibers).  Lineage: preserved.
Tier A (`promote_splitSafe`, `promote_preservesDisjoint`).

**`demote cols`** drops key components into the non-key part.  Content:
the named key columns become ordinary columns.  Cardinality: derived from
the gradings; on a genuine coarsening no grading fits the retained key and
the bound rises to **`bag`** (unless a following `map_bags` reduces it),
while an exact round trip re-derives `singletons`.  Completeness:
re-derived from the graded cardinality (ADR 0035): an exact round trip is
graded `singletons` and is `Complete` (`fiberCompleteWrt_of_functional`),
while a genuine coarsening **clears** the fact, because merging fibers
turns an absent fine key into a gap inside a coarse fiber
(the ADR 0035 fiber-gap counterexample; the reference-relative
`demote_completeWrt` remains true but is not the tracked fact), except
where every demoted column is an `exhaustive` axis (section 6.6), which
rules those absences out and keeps the fact
(`demote_fiberCompleteWrt_of_exhaustive`).  A
`demote` with no downstream reducer is still admitted with no discharge;
a reducer over the coarsened bag establishes the fact *after* the
`demote` (section 8).
Lineage: **dropped** -- the branch structure over the old key no longer
applies, so the disjointness fact falls out of scope and must be
re-established (`assert`) or assumed (section 9); this lineage break is what
makes the operation Tier B (`demote_not_preservesDisjoint`).

### 6.4  `lookup` / `lookup_total` (join a fixed table) -- Tier A

```
readings |> lookup machines (|k, r| r.machine)
```

Joins against a fixed right table; the key-first lambda maps a left row (key `k`,
value `r`) to the right table's key.  Content: the right table's columns are
added.  Cardinality: preserved when the right table is functional (`singletons`);
a non-functional right table multiplies rows in, raising the bound to `bag`.
Totality:
`lookup` makes the added right columns **optional** (an unmatched left row is
kept with them missing); `lookup_total` drops unmatched rows and adds no
optionality.  Completeness: preserved on the left.  Lineage: preserved.  Tier A
(`lookup_splitSafe`, `lookupTotal_splitSafe`, `lookup_preservesDisjoint`,
`lookupTotal_preservesDisjoint`).

### 6.5  `split` / `union` (partition and merge) -- Tier A

```
let (train, test) = data |> split |k| hash k < threshold
let full          = (train, test) |> union
```

**`split |k| pred`** routes each entity (each key) wholly to one side of a pair
by a predicate over the key, never cutting a key's rows apart.  Content,
cardinality, completeness: unchanged on both sides.  Lineage: **adds two sibling
branch tags** under the current node, one per side; the halves are disjoint by
construction because they sit in exclusive branches (section 9).  Tier A
(`split_disjoint`; `union_split` shows `union` undoes it).

**`(a, b) |> union`** is the multiset union of two tables of the same schema at
each key.  It is **total**: no precondition, always split-safe, associative and
commutative (`bind_comm`, `bind_assoc`).  Content: unchanged.  Cardinality:
binding inputs whose lineage is **disjoint** preserves `singletons`; binding
**overlapping** inputs may push a key above one row, raising the bound to `bag`.
Completeness: the union is complete over a key iff both inputs are.  Lineage:
**unions** the two tag-sets, so the result is disjoint from a third table iff
both inputs were (`union_disjoint_iff`).  Tier A.

### 6.6  `unpivot` / `pivot` (reshape long and wide)

The surface is ADR 0016 as amended by
`docs/decisions/0020-reshape-as-a-true-inverse-pair.md`: the column list is
gone (the fold is total over the attributes; exclusion is upstream
projection), a missing cell yields no row, and `pivot` keeps one form.

**`unpivot name value`** folds **all** attribute columns, which must share
one domain, into rows, spreading the column *name* into the key.  Content:
the names move into a new `enum` key column `name`, the values into a
single column `value`.  **A missing cell yields no row** (drop semantics),
so `value` is total by construction.  Cardinality: preserved.
Completeness: establishes `exhaustive(name)` exactly when every folded
column is total (section 3.4).  Lineage: preserved.  Tier A
(`unpivotDrop_splitSafe`).

**`pivot name value`** is the inverse and has one form: `name` must be an
enum-domained **key** column (`name` in attribute position is rejected,
with a hint to `promote` first).  Admissible exactly when the input is
**`singletons`** and its attributes are exactly `value` (drop or aggregate
others first).  It consumes **no completeness fact**: an absent
(key, variant) row becomes a missing cell.  The spread columns are total
iff `exhaustive(name)` holds and `value` is total, optional otherwise
(`pivot_total_of_exhaustive`).  Not split-invariant, because a split can
cut across the spread names, so lineage is dropped.  Tier B
(`pivot_not_splitInvariant`).

The pair is mutually inverse on functional, minimal tables:
`pivot (unpivot W) = W` (`pivot_unpivotDrop`) and `unpivot (pivot L) = L`
with no completeness side condition (`unpivotDrop_pivot`).  Value-missing
in the wide table and row-absent in the long table carry the same
information; that transposition is what the drop semantics makes bijective.

The spread key column must be **finite-enumerable**, i.e. an `enum`, since
its values become column names (ADR 0014); `bool` is excluded because
`true`/`false` as column names break the round-trip.

So `pivot` is where cardinality tracking pays off: it type-checks only when
each spread cell is known to hold at most one value, which the long form's
key discipline provides.

### 6.7  `window` (replicate onto a time grid) -- Tier A

```
window : w -> p -> diff(domain(p)) -> diff(domain(p)) -> Table -> Table
```

`window w p size stride` replicates each row into every window containing
its point `p` and adds the window start as a fresh key column `w` with
`p`'s domain (ADR 0037 decision 1).  `w` must not already exist, which is
the reverse of every other column argument; `p` must, in the key or among
the attributes, and it stays where it is.

Window starts are the multiples of `stride` from the domain's zero
(ADR 0036 decision 5), and a row lands in every `w` with
`w <= p < w + size`, so tumbling windows are `stride == size` and no
second operation is needed.  An empty window is not a row: `window`
replicates, and with no row there is nothing to replicate (ADR 0038 is
where materializing the empty ones belongs).

The extents are **const expressions** (ADR 0030) of type
`diff(domain(p))` (ADR 0036 decision 4): `time[real]` for an `instant`
point, `int` for an `int` point, both positive, and for an instant a
whole number of milliseconds, so the grid cannot drift.  A `date` point
waits on `diff(date)`.

Properties:

- **Content**: the key gains `w` at `p`'s domain; nothing else moves.
- **Cardinality**: `singletons` at `K` becomes `singletons` at `K + {w}`,
  a `bag` stays a `bag`, because the replication is injective on
  (input identity, `w`).
- **Gradings**: each `G` becomes `G + {w}` (`window_functional`), the one
  operation besides the key moves that transforms them rather than
  resetting them.  This is what keeps a downstream scan ceremony-free:
  after `window` then `demote p`, the points are still unique inside one
  window fiber, so tier 1 discharges the order key with no claim.
- **Completeness, totality, lineage**: as the derived form transports
  them (a replicating `flat_map`, then `promote w`).
- **`exhaustive`**: cleared, as under the other key moves.
- Tier A (`window_splitSafe`, by composition).

The checker additionally records the **windowing fact** (`w` windows `p`
at this extent and stride, over a source whose intake contract it
inherits), the sibling of `exhaustive(axis)`: established here, consumed
by `closed`, and reset conservatively by any operation that is not
content-identity in ADR 0024's sense.

### 6.8  `closed` (keep the final windows) -- Tier A

```
closed : Table -> Table
```

Takes no arguments: the extent comes from the `window` stage and the
bound from the source's declaration.  Drops every window that can still
receive a row and **establishes `Complete`** at the current key on the
survivors, which is a new establishment mechanism for the existing fact
(section 8), not a new qualifier.  A window survives when
`w + size + lateness <= watermark` against the watermark of its own
grain (ADR 0041), which is `max(observed, floor)`: the maximum point
accepted in that grain, raised by the deployment's declared closure
floor.

Demands, each rejected with the fix named: a window column in the key
carrying a live windowing fact; its point demoted into the fiber; a
`lateness` contract on that point (without one there is no mechanism and
`assume { complete }` is the visible fallback); and the contract's
watermark grain still in the key, since a row can only be measured
against the producer it came from.

Establishes `Complete`; carries every other fact, being content-identity
apart from the row filter.  Tier A.

The invariant it buys is **finality**: rerunning after further ingestion
adds newly closed windows and never changes a previously emitted one
(`closedWindow_stable`), which is what makes a closed-window view safe to
maintain incrementally.  Note what the fact says: the *arrival*
completeness of ADR 0033 transported to the window key, not a claim that
the device was working.

### 6.9  `latest` (the newest row per group) -- Tier A

```
latest : p -> Table -> Table
latest : desc p -> Table -> Table
```

Keeps, per fiber, the row with the maximal point `p`, which is
`getLast (arrange p fiber)`, deterministic by `IsArrangement.unique`
given tie-freedom.  A **reduction**, fiber-to-row: the result is
`singletons` at the current key with `p` a total attribute, and
completeness is re-established on the output, a present singleton fiber
being its whole fiber.

Demands both ordered-reduction facts: **tie-freedom** of `p` (a grading,
or `assume { arranged }`, exactly as a scan, since the argmax of a tied
key is not determined) and **completeness** at the current key (ADR 0023,
since a partial bag's latest is silently wrong), the latter discharged
trivially on a `singletons` input.

`p` must be an attribute and orderable and total.  A **key** column is
rejected: fusing the coarsening into the operation would leave the
completeness demand undischargeable, so the coarsening is written out
(`demote p`, then the claim, then `latest p`).  Tier A.

The point takes the `desc` marker (section 5.4), so `latest (desc p)` is
the argmin: `getLast (arrange p fiber)` at the dual order, which is
`IsArrangement.unique` instantiated at `ωᵒᵈ` rather than a new theorem.
The obligations are unchanged, the dual of a total order being total and
the dual of an injective key injective, and ties resolve to the earlier
row either way.  The marker must be parenthesized, an unparenthesized
`latest desc p` being two arguments.  No `earliest` exists (ADR 0037
decision 7, direction settled).

### 6.10  `dense` (complete the window grid) -- Tier A

```
dense : w -> population -> bound -> Table -> Table
```

Adds one row per population entity per closed grid slot that produced
none, and **establishes `Complete`** at the current key (ADR 0038).  The
establishment is mechanism-grade, the mechanism being the grid
enumeration: stride and origin are compile-time constants, closedness
bounds the run above, so every row the ideal rectangle has is present.
Runs **after** the reduction, which preserves ADR 0029 decision 4's
empty-bag guarantee and computes the ideal grid's answer anyway
(`dense_fiberMap_foldFiber`).

Demands, each rejected with the fix named: a window column in the key
carrying a live windowing fact; `closed` upstream on that grid, since
closedness is the upper bound and filling past the watermark would
declare unelapsed time confirmed empty; a `singletons` input, since the
grid is one row per (entity, slot) and a bag input has not been reduced
yet; a `singletons` population keyed like the windowed rows without the
window column; and a total lower-bound column of that population in the
window column's domain.  The population and the bound are policy and are
never inferred (ADR 0038 decision 3): the windowed bag knows nothing of
an entity that never sent a row, and the earliest observed window would
confuse "offline since before we watched" with "not yet installed".

**Totality**: a column that is a single fold at a combiner carrying an
identity fills with that identity and stays total; every other column
becomes **optional** and is absent on a filled row (ADR 0038 decision 2,
`foldBagOpt_eq_none_iff`).  The recognition is syntactic plus const
resolution over the fold shapes of section 5.4, so `#b`, `bag.sum b.x`
and a written-out `fold` at `+` qualify and a compound expression does
not.  **Cardinality** and the gradings are preserved (rows are added at
fresh keys of the same key).  **Rectangularity** is recorded beside the
completeness fact and consumed by one rule: the `demote` of section 6.3
re-derives `Complete` from it instead of clearing it
(`demote_fiberCompleteWrt_dense`), which is the one exception beside the
`exhaustive`-axis rule and holds only over a grid.  Tier A
(`dense_idem`); a closed slot's row is final across the fill
(`dense_stable_of_closed`), so a rectangularized view grows without
retracting.

## 7.  Tier A / Tier B and split-safety

The central guarantee is **split-safety**.  In the formalization,

```
SplitSafe op  :=  PreservesDisjoint op  and  SplitInvariant op
```

(`formal/Mensura/Core/Defs.lean`).  Split-safe operations are closed under
composition (`SplitSafe.comp`) and identity is split-safe (`SplitSafe.id`), so a
pipeline built only from Tier A operations commutes with a split: running it on
the whole table equals running it on each side of a split and re-binding.  That
is the formal content of "no leakage between train and test".

Both halves of the definition are now tracked: `SplitInvariant` is the Tier
boundary, and `PreservesDisjoint` is what lets the lineage hierarchy carry a
disjointness fact through a Tier A pipeline intact (section 9).

- **Tier A** (split-safe): `flat_map`, `map_bags`, `promote`, `lookup`,
  `lookup_total`, `split`, `union`, `unpivot`, `window`, `closed`,
  `latest`, `dense`.  They compose freely and carry cardinality, completeness, and
  lineage facts end to end.
- **Tier B** (split-breaking): `demote` and `pivot`.  Each drops the
  lineage fact, and that is the whole content of the Tier: `demote`
  demands no completeness itself (the demand sits at the reducing
  `map_bags`, section 8, ADR 0023, and a genuine coarsening clears the
  fact rather than carrying it, ADR 0035), and `pivot` demands nothing
  (an absent row becomes a missing cell) and instead upgrades its spread
  columns' totality under `exhaustive` (section 6.6, ADR 0020).

## 8.  Completeness: establish, clear, consume

Completeness (each key's bag holds all its rows, section 3.4) is established in
one of five ways (`07`, "Completeness: establish, clear, consume"):

- **mechanism**: a `registry` source is complete by construction at its
  **own declared key** (overview pillar 7, `13-registries.md`, ADR 0033
  as amended by ADR 0035).  The fact holds at that boundary whatever the
  cardinality: on a `singletons` registry it is the
  `fiberCompleteWrt_of_functional` corollary and discharges what
  cardinality already would, while on an `attr*` registry it is
  contentful, pinning the reference population per entity, so a reducer
  at the registry's own key needs no further discharge;
- **check**: `completeness_check { assert ... }`, a pipe stage that establishes
  the fact locally; each `assert` is a boolean expression, and together they
  witness that the partition is complete over the current key.  The stage is
  placed ahead of the consuming operation and after the last coarsening;
- **annotation**: `@complete_over(col)` on a source store, establishing the fact
  globally (grammar deferred to the annotation family, section 13);
- **closedness**: `closed` (section 6.8) over a windowed table whose
  source declares a `lateness` bound.  Mechanism-grade like the registry
  rule and for the same kind of reason: the extent and the bound are both
  enforced, so "no row of this window can still arrive" is a theorem
  about the intake (ADR 0037 decision 4);
- **enumeration**: `dense` (section 6.10) over a closed grid, which
  materializes the slots the reduction produced no row for, so the fact
  holds by construction rather than by claim (ADR 0038 decision 4).  The
  only establishment that also survives a coarsening `demote`, of the
  window column it completed.

Row-wise Tier A operations **preserve** completeness (they map whole
fibers to whole fibers); the key moves **re-derive** it from the ADR 0024
gradings, and a `demote` that genuinely coarsens **clears** it, because an
absent fine key becomes a gap inside a coarse fiber
(the fiber-gap counterexample, ADR 0035).  A coarsening whose every
demoted column is an `exhaustive` axis (section 6.6) is the exception:
exhaustiveness rules those absences out, so the fact survives
(`demote_fiberCompleteWrt_of_exhaustive`, ADR 0035 decision 6).  A
coarsening of a window column whose grid `dense` completed is the second:
the fill left no absent fine key inside the grid, so the coarse fiber is
the whole rectangle (`demote_fiberCompleteWrt_dense`, ADR 0038
decision 4).  A
**reducing `map_bags`**
**consumes** it, because a fold over a partial bag is silently wrong
(ADR 0023, amending ADR 0017's consumer placement).  *Consume* names the
demand (the reducer is rejected without the fact), not the fate of the
fact, which is carried through the operation (section 6.2).  Over a
`singletons` input the reducer's obligation discharges trivially
(`fiberCompleteWrt_of_functional`), so only a reduction over a `bag` -- a
coarsened key, or a `bag` store -- needs an establishment step, placed
after the coarsening it folds under.  `assume { ... }` is the escape
hatch when the obligation cannot be discharged.  (`pivot` formerly
carried an obligation too; ADR 0020 dissolves it into the `exhaustive`
totality upgrade of section 6.6.)

```
enrollments
|> demote course                                             // coarsen; the fine-key fact is forfeited
|> completeness_check { assert row_count open_offerings == 0 }   // establish at the folded key
|> map_bags |k, b| (.total_credits = bag.sum b.credits)            // consume; back to singletons
```

Remove the check (and `@complete_over`, and `assume`) and the reducing
`map_bags` is rejected; the `demote` alone would still be admitted.  Move
the check ahead of the `demote` and it is rejected too: the check would
witness the fine key, and the coarsening forfeits that fact.

## 9.  Lineage and disjointness (the tag hierarchy)

Split-safety is defined with `PreservesDisjoint` (section 7), so disjointness is
part of the proven algebra.  This freeze *tracks* it with a **lineage
hierarchy** rather than a symbolic key-predicate region.  In the formalization
(`formal/Mensura/Core/Defs.lean`),

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
| `flat_map` / `map_bags` | tags carried | preserved | `map_preservesDisjoint`, `fiberMap_preservesDisjoint` |
| `promote` | tags carried | preserved | `promote_preservesDisjoint` |
| `lookup` / `lookup_total` | tags carried | preserved | `lookup_preservesDisjoint`, `lookupTotal_preservesDisjoint` |
| `unpivot` | tags carried | preserved | `unpivotDrop_preservesDisjoint` |
| `split` | adds two sibling branch tags | establishes | `split_disjoint` |
| `union` | unions the tag-sets | disjoint from `c` iff both were | `union_disjoint_iff` |
| `demote` / `pivot` | tags dropped (key change) | re-establish or assume | `demote_not_preservesDisjoint`, `pivot_not_splitInvariant` |

Anything the hierarchy cannot decide is delegated:

- **`assert`** establishes the fact by a boundary check on the actual data, when
  two tables have no shared split ancestor but a checkable key witnesses
  non-overlap;
- **`assume`** admits the obligation by fiat, locally and visibly, for external
  data of opaque provenance.

A site that *demands* disjointness (notably `fit`/`evaluate`, deferred with the
learning operations, section 13) consumes the fact: it type-checks only when the
two tables are structurally disjoint, asserted, or assumed.

**M1 scope.**  The implemented checker (`mensura check`) tracks disjointness
*only* through this tag hierarchy, with `assume` as the escape hatch.  The
symbolic key-predicate region of `08-lineage.md` (and any decision procedure
over it, such as the linear-arithmetic fragment) is **deferred to M6**, where
`fit`/`evaluate` become the first operations to consume disjointness; until
then nothing consumes it, so the predicate fragment buys nothing.

`assume` therefore carries **two** claims, and both are obligations something
downstream consumes:

| claim | admits | consumed by |
| --- | --- | --- |
| `complete` | every key's bag is whole | a reducing `map_bags` (ADR 0023) |
| `arranged` | a scan's order keys are tie-free | `scan` / `prescan` (section 5.4) |

`arranged` is the second claim ADR 0017 anticipated when it wrote that "the
block form generalizes later without a surface change", and it is the home ADR
0029 Decision 11 left open for its tier 3 hatch.  Neither claim is a fifth
qualifier axis in spirit: each records an assertion about the data, not a
derived fact, and each is scoped to the pipeline stage that makes it.

Prefer deriving over claiming where the shape allows it.  A tie-free order key
projected out of the key needs no claim at all (section 5.4,
`Mensura.keyInjOn_demote_tag`), and `assume { arranged }` is for orders that are
genuinely ambiguous rather than a line to paste.

## 10.  Consolidated effect matrix

One row per primitive (pres. = preserved).  "card" gives the cardinality bound
after the operation; at the key moves it is derived from the gradings
(ADR 0024, section 6.3).  "lineage" is the effect on the tag hierarchy.
Theorems are the primary split-safety / disjointness backing; section 11 has
the full key.

| op | content | card | total | complete | lineage | Tier | theorem |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `flat_map` | cols := row schema | pres. if max size `<= 1`, else `bag` | as ret. | pres. | carried | A | `flatMap_splitSafe` |
| `map_bags` | cols := return | `singletons` or `bag` (per return) | as ret. | pres.; **demanded** by the reducing shape on a `bag` input | carried | A | `fiberMap_splitSafe`, `fiberCompleteWrt_of_functional` |
| `promote` | cols join key | graded: pres.; a fitting grading promotes `bag` -> `singletons` | pres. | re-derived: `Complete` at graded `singletons`, pres. at `bag` | carried | A | `promote_splitSafe`, `promote_functional`, `fiberCompleteWrt_of_functional` |
| `demote` | key cols -> non-key | graded: **-> bag** on a genuine coarsening; a round trip re-derives `singletons` | pres. | re-derived: **cleared** on a genuine coarsening, `Complete` at graded `singletons` or when every demoted column is an `exhaustive` axis | **dropped** | B | `demote_not_preservesDisjoint`, `demote_promote`, `fiberCompleteWrt_of_functional`, `demote_fiberCompleteWrt_of_exhaustive`; the clearing arm is conservative (ADR 0035) |
| `lookup` | + right cols | pres. if right `singletons`, else `bag` | right **optional** | pres. left | carried | A | `lookup_splitSafe` |
| `lookup_total` | + right cols | pres. if right `singletons`, else `bag` | pres. | pres. left | carried | A | `lookupTotal_splitSafe` |
| `split` | unchanged | unchanged | unchanged | unchanged | adds branches | A | `split_disjoint` |
| `union` | unchanged | `singletons` if disjoint, else `bag` | pres. | iff both | unions tags | A | `union_split` |
| `unpivot` | all attrs -> (name, value); missing cells drop | pres. | `value` total | establishes `exhaustive` | carried | A | `unpivotDrop_splitSafe` |
| `pivot` | name leaves key, variants spread | demands `singletons` | per `exhaustive` | not consumed | dropped | B | `pivot_not_splitInvariant`, `pivot_total_of_exhaustive` |
| `window` | `w` joins the key | pres.; each grading gains `w` | pres. | pres. | carried | A | `window_splitSafe`, `window_functional` |
| `closed` | unchanged (rows dropped) | pres. | pres. | **establishes** | carried | A | `closedWindow_stable` |
| `latest` | `p` becomes an attribute | `singletons` | `p` total | re-established; **demanded** | carried | A | `IsArrangement.unique`, `fiberCompleteWrt_of_functional` |
| `dense` | unchanged (rows added) | pres. | no-identity columns -> **optional** | **establishes**, plus rectangularity | carried | A | `dense_fiberMap_foldFiber`, `dense_idem`, `dense_stable_of_closed` |

The `demote` row's completeness arm reads the rectangularity fact as its
second exception: a coarsening of a window column whose grid `dense`
completed re-derives the fact (`demote_fiberCompleteWrt_dense`).

## 11.  Lean theorem catalogue

Each rule above is backed by a theorem in the Lean formalization.  Names are
verbatim; the development lives in themed modules under `formal/Mensura/`
(`Core/`, `SplitSafety.lean`, `Reshape.lean`, `Rectangle.lean`, `Laws.lean`,
and `Completeness/`).

**Core algebra** (`Core/`, `SplitSafety.lean`, `Reshape.lean`,
`Rectangle.lean`, `Laws.lean`) -- split-safety, disjointness, reshape,
lineage tags:

- composition: `SplitSafe.comp`, `SplitSafe.id`; definitions `SplitSafe`,
  `SplitInvariant`, `PreservesDisjoint`, `Disjoint`.
- `flat_map`: `flatMap_splitSafe`, `map_preservesDisjoint`, `map_splitInvariant`.
- `promote` (`promote`): `promote_splitSafe`, `promote_preservesDisjoint`,
  `promote_splitInvariant`; the grading transport `promote_functional`
  (over the `Functional` definition of `Reshape.lean`, ADR 0024).
- joins: `lookup_splitSafe`, `lookup_preservesDisjoint`,
  `lookupTotal_splitSafe`, `lookupTotal_preservesDisjoint`.
- `unpivot` (the drop variant, `unpivotDrop`): `unpivotDrop_splitSafe`,
  `unpivotDrop_unionHom`, `unpivotDrop_preservesDisjoint`,
  `unpivotDrop_minimal`, `unpivotDrop_exhaustive`.  (The reify variant's
  `unpivot_splitSafe` / `pivot_unpivot` remain for comparison.)
- `split` / `union`: `split_disjoint`, `union_split`, `bind_comm`, `bind_assoc`,
  `union_disjoint_iff`.
- lineage tags: `addTag`, `dropTag`, `taggedBind`, `taggedSplit`,
  `taggedSplit_taggedBind_left`, `taggedSplit_taggedBind_right`.
- `demote` (`project`): `demote_not_preservesDisjoint`; the key-move
  round trips `demote_promote` and `promote_demote` (ADR 0024).
- `pivot`: `pivot_not_splitInvariant`; the round trips `pivot_unpivotDrop`
  and `unpivotDrop_pivot`; the totality upgrade `pivot_total_of_exhaustive`.
- `exhaustive` propagation: `map_exhaustive` (non-dropping maps),
  `lookup_exhaustive`, `aggregate_exhaustive`, `bind_exhaustive`,
  `exhaustive_of_subsingleton` (a single-variant axis is exhaustive
  trivially); `split_not_exhaustive` witnesses the destroyed row.

**Completeness layer** (`Completeness/`) -- rekeying layer, fiber
operations, and population-relative completeness:

- `map_bags` (`fiberMap`): `fiberMap_splitSafe`,
  `fiberMap_preservesDisjoint`, `fiberMap_splitInvariant`,
  `fiberMap_exhaustive` (a presence-preserving fiber action carries the
  rectangle: both `map_bags` shapes).
- population-relative completeness (`CompleteOver.lean`, ADR 0023,
  ADR 0035): `CompleteWrt` (a row wherever the reference has one) with
  the reference-relative `demote_completeWrt` (retained; not the tracked
  fact), and `FiberCompleteWrt` (the tracked fact, section 3.4) with
  `fiberCompleteWrt_of_functional` (at `card <= 1` a present key carries
  its whole fiber: the reducer's trivial discharge and the key moves'
  `singletons` re-derivation), `ExhaustiveOn` with
  `demote_fiberCompleteWrt_of_exhaustive` (a coarsening along an
  exhaustive axis keeps the fact, ADR 0035 decision 6), and the negative
  witness recorded fiber-gap counterexample (a genuine coarsening turns
  an absent key into a fiber gap, the clearing rule; it fails exactly the
  `ExhaustiveOn` hypothesis).
- `pivotAttr`: `pivotAttr_splitSafe`, `pivotAttr_reversible` (these back the
  bag-long alternative recorded, and not adopted, in ADR 0020; retained for
  a possible future fused attribute-position form).
- sugar already proved Tier A: `filterRows_splitSafe`, `mutateCol_splitSafe`,
  `antiJoin_splitSafe`, `distinct_splitSafe` (these back named forms deferred in
  section 13, recorded here so implementers know the proofs exist).

**Bag reduction** (`Fold.lean`, ADR 0029 Stage 1) -- the monoid-parameterized
fold behind section 5.4, and the gate `fold`, `#`, and the `bag` module ship
behind:

- `foldBag` over a commutative monoid, with `foldBag_add` (two shards) and
  `foldBag_shards` (arbitrarily many, including empty ones): the theorem that
  licenses partial and parallel folding.  `foldBag_add_seed_counterexample`
  witnesses why there is no user-supplied seed -- a seed that is not the
  identity is counted once per shard.
- the identity-free rows: `optionLift` completes a commutative semigroup, and
  `foldBagOpt` folds through it, with `foldBagOpt_isSome_of_ne_zero` and
  `foldBagOpt_eq_none_iff` pinning it from both sides.  The first is what
  licenses the *total* surface type of `bag.min` / `bag.max`, since a group
  arises from rows and is never empty; the second says a missing result is
  exactly an absent fiber and never a data-dependent surprise.
- placement in the algebra: `foldFiber_eq_aggregate` (a monoid fold *is* an
  `aggregate`, so this carves out a well-behaved subclass rather than
  generalizing -- `aggregate` takes an arbitrary whole-bag function),
  `foldFiber_strict`, `foldFiber_splitSafe`, `foldFiber_exhaustive`.

**The ordered structure** (`Arranged.lean`, ADR 0029 Stage 2, ADR 0031
Decision 7) -- the gate `scan`, `prescan`, `desc`, and the `series` module ship
behind.  The obstacle was structural rather than incidental: a table's content
is a `Multiset` and `Core/Defs.lean` argues *for* multisets precisely because
order should not be asserted when it is not used, so neither a scan nor a
positional map is expressible over one.

- the arrangement: `IsArrangement` (a *relation* between a bag and a list of
  its elements in key order, claiming the blueprint's reserved `def:arranged`
  node), `exists_isArrangement`, `arrange`, `arrangeList`.  Stated relationally
  because existence and uniqueness have different hypotheses and a sort
  conflates them: `Multiset.sort` wants antisymmetry as a typeclass instance,
  which for a key-induced order is *global* key injectivity, while tier 1
  supplies only the per-fiber fact.
- Tier 1 determinism: `IsArrangement.unique` (a key injective on the fiber
  arranges it uniquely), with `KeyInjOn` as the hypothesis.
  `keyInjOn_of_functional` bridges to `Functional`, which ADR 0029 asked for,
  but **vacuously**: a functional table's fibers hold at most one row, so
  nothing can tie and any scan over one is a one-element list.  The
  *substantive* discharge is `keyInjOn_demote_tag`: `demote` merges the rows of
  every key `(k, d)` into one output key, tagging each with its own `d`, so a
  functional input yields fibers holding at most one row per tag and the tag
  column is injective on a genuinely multi-row bag.  That is the theorem the
  checker's grading rule cites (section 5.4).
- the two scans: `scanBag` and `prescanBag`, the `tail` and `dropLast` of one
  `List.scanl`.  So the inclusive/exclusive coherence ADR 0031 demanded is list
  slicing rather than a second induction, and `lag`'s missing first row falls
  out of `dropLast` at the `Option` completion.
- coherence: `scanl_getLast_eq_foldBag` (a scan's last element *is* the
  corresponding fold) and `scanl_getLast_eq_foldBagOpt` for the identity-free
  pair.  This is what makes "same combiner, two variants" a theorem.  It needs
  no injectivity, because a commutative-associative combiner cannot observe a
  tie's permutation: the total is determined even when the intermediates are
  not, which is exactly why `fold` needs no order and `scan` does.  Stated per
  combiner class, since for the associative-only tacks no `foldBag` exists to
  cohere with, which is the formal content of the surface rule admitting them
  under `scan` only.
- parallel scan: `scanl_append_decomp`, the prefix decomposition, Stage 2's
  analogue of `foldBag_shards` (and needing no laws at all, since a list scan
  asserts no order-independence).
- placement in the algebra: `scanFiber` with `scanFiber_strict`,
  `scanFiber_splitSafe`, and `scanFiber_exhaustive`.  Split-safety is inherited
  from `fiberMap_splitSafe` unchanged, because a `split` routes a key's *whole*
  multiset to one side, so an arranged verb sorts an intact fiber.  The
  blueprint node's old claim that arranged verbs are "deliberately not
  split-invariant" concerned lifting to the list monad *in general*; what stays
  out of scope is an order *across* keys.

**Streaming windows** (`Window/`, ADR 0037 decision 8, ADR 0038
decision 7) -- the gate `window`, `closed`, and `dense` ship behind.  The
operation is specified as a derived form (a replicating `flatMap` then
`promote`), so safety comes from the composition lemmas rather than a new
argument:

- `Window/Defs.lean`: `window` with `window_splitSafe` and
  `window_unionHom` (by composition), the fiber characterization
  `window_rows`, the grading extension `window_functional` (decision 2's
  "extended, not reset"), and `closedWindow_stable`, which is the
  soundness of `closed`'s establishment given the enforced contract and
  the finality invariant the refresh slice inherits.  The watermark is
  indexed by a grain, and the theorem's two hypotheses read the same
  watermark, so a mixed admission-and-closure grain is not expressible
  (ADR 0041 decision 1).  `demote_congr` lifts fiber agreement through the
  coarsening the surface performs first.  The concrete grid is
  `Units.Instant.windowStarts`, characterized by `mem_windowStarts` as the
  interval test on the stride grid.
- `Window/Dense.lean`: `dense` with `dense_fiberMap_foldFiber` and its
  `Option` mirror `dense_fiberMap_foldFiberOpt` (filling after the
  reduction computes the reduction over the completed grid, the licence for
  the cheap order of operations), `dense_present_of_mem_grid`,
  `dense_idem`, `Units.Instant.dense_stable_of_closed` (a closed slot's row
  is final across the fill), and `demote_fiberCompleteWrt_dense` with
  `dense_eq_rectangle` (after the fill the table *is* the ideal rectangle,
  which is why the coarsening keeps the fact).  Slots are an abstract
  `Fintype`, since the grid's provenance is a side condition of the surface
  rule rather than a hypothesis of any theorem.

**Physical dimensions** (`Units/Dimension.lean`, ADR 0026) -- the group
behind the section 5.3 dimensional rules: `Dimension` (the free abelian
group over the seven `Base` dimensions) with its `CommGroup` and
`DecidableEq` instances, `exponents`, `base`, and `base_injective` (the
seven axes are distinct).  The planned follow-ups, per ADR 0026 Decision
10, are dimensional-arithmetic soundness (the checker's `*`/`/`/`^` match
the group operations) and conversion correctness (scale-factor
normalization preserves the quantity); they are white nodes in the
blueprint until proved.

## 12.  Conformance cases (seed)

The roadmap's M0 calls for a must-accept / must-reject suite (`ROADMAP.md`, M0;
"Validation criterion").  This section seeds it with canonical cases drawn from
the worked examples in `07`/`08` and `docs/examples/*.mensura`; the executable
suite itself is M1 work (`ROADMAP.md`, M1).

**Must accept:**

- Summarize by an attribute (`07`): `promote machine |> map_bags |k, b| ...`,
  all Tier A, result `singletons` per key.
- Filter with `flat_map` (ADR 0015): `map |_, r| if r.degraded then r else ()` keeps
  or drops a row and stays `singletons`; `flat_map |k, r| (r, r)` expands to `bag`.
- Coarsen, then establish at the folded key (`07`): `demote course
  |> completeness_check { ... } |> map_bags ...` (the coarsening clears,
  the check establishes at the retained key, the reducer consumes;
  ADR 0023, ADR 0035).
- Reindex only: `demote` with no downstream reducer and no establish
  step (a possibly partial bag is an honest rekey; ADR 0023).
- Reduce a plain store: `map_bags |k, b| (.m = bag.max b.x)` straight over a
  `singletons` source, with no establish step (the trivial discharge).
- Split and re-merge (`07`): `split |k| ...` then `(train, test) |> union`
  reconstructs the input (`union_split`); the disjoint halves keep `singletons`.
- Split then demand: `split` establishes structural disjointness that a later
  disjointness-demanding site consumes without a check (the learning-operation
  syntax itself is deferred, section 13).
- `pivot` of an enum key axis on a `singletons` long table whose only
  attribute is the value column; a sparse input is admitted, with optional
  spread columns (ADR 0020).

**Must reject:**

- A reducing `map_bags` over a `bag` with no completeness fact (no check,
  no `@complete_over`, no `assume`); e.g. `demote` then an aggregate
  with no establish step, or with the establish step placed *before* the
  `demote` (ADR 0023, ADR 0035).
- A `scan`/`prescan` at a **fold-admitting** combiner over the same
  fact-less `bag` (`series.cumsum`, `series.running_max`, `series.rank`
  included): the scan contains its reduction, so it carries the reducer's
  demand; the keep combiners (`series.lag`, `series.lead`,
  `series.first_value`, `series.last_value`) stay accepted there (ADR 0037
  decision 5).
- A disjointness-demanding site fed two tables that are not structurally
  disjoint and were neither asserted nor assumed.
- A scalar operator applied to a bag (`r.x > 30` where `x` is read at a
  `bag`); an optional operand *lifts* instead (ADR 0039), but an optional
  boolean reaching an `if` condition, or an optional mapper reaching a fold
  or scan, is rejected until discharged with `??`.
- Comparison chaining (`a < b < c`); a mixed positional/labeled `( )`.
- A `flat_map` body that names a key column in its output record, or one that
  always drops (`flat_map |k, r| ()`, no schema to infer); an `if` with a non-`bool`
  condition or branches of different type (ADR 0015).
- `pivot` of a `bag` input (a (key, name) cell may hold more than one
  value); `pivot` naming a non-key column (`promote` it first); and
  `unpivot` over attributes of differing domains (project first).

## 13.  Open points (the deferred ledger)

What this freeze deliberately leaves open, so its scope is unambiguous.  Each is
specified ahead of the milestone that needs it (`ROADMAP.md`, "specs first").

- **The extensible qualifier meta-calculus (ADR 0004).**  User-definable
  qualifiers, the rule-combinator DSL, and the open `Qs` row are deferred.  In
  this freeze `Qs` is the closed set of four built-ins (cardinality, totality,
  completeness, lineage; section 1); reconciling this narrower scope with
  ADR 0004 (which anticipated freezing the full meta-calculus at M0) is a
  follow-up.
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
- **Named sugar.**  `mutate`, `select`, `reduce`, and
  `tagged_union`/`tagged_split` are sugar over the primitives (their Tier-A
  proofs exist, section 11) and get their own round.  `filter` is now derivable
  as `flat_map |k, r| if c then r else ()` (ADR 0015), so it too is sugar, not a
  primitive.  The window functions have **landed** rather than remaining sugar
  to schedule: `rank` and `cumsum` are bindings in the bundled `series` module,
  backed by `scanFiber_splitSafe` (section 11).
- **Expression features the fuller surfaces need.**  Row-dropping and
  row-expanding `flat_map` now land (the `( )` collection and `if`/`then`/`else`, ADR
  0015); bag-returning `map_bags` (windows) now has its ordering, named at the
  operator by a `scan`'s key argument (section 5.4).  The
  `const`/`var` record-field marker that ADR 0015 reserved is dropped by ADR
  0019, which drops the `const`/`var` concept altogether.
- **Annotation grammar.**  `@audited`, `@versioned`, `@auto`, `@complete_over`,
  `@disjoint_partition` are named here but their surface lands with the
  annotation family.
- **Compound keys.**  A compound unit's key flattens to dotted scalar
  columns before typing (`01-units.md`, ADR 0032), so every rule in this
  reference applies unchanged to the flat form; the hierarchy is
  presentation only.  Key moves (`promote`/`demote`) and reshape selectors
  naming a flattened component or a unit-reference group are deferred
  (ADR 0032).
- **Streaming.**  `window`, `closed`, `latest`, and `dense` have landed
  (sections 6.7 to 6.10).  Per-window sampling inference and `on_change`
  refresh extend these rules (M5), and with the latter the honest exit for
  the frontier window: a reduction over the *open* windows that carries the
  bound it was computed over, so a provisional row says so (ADR 0037, open
  questions).  Deferred beside them: fill policies that narrow a `dense`
  column back to total, the general `resample` over a raw order key, and
  completeness transport through per-row filters (ADR 0038, open
  questions).
- **Precision and measure semantics.**  Dimensional units are now
  specified (`11-physical-units.md`, section 5.3 above; ADR 0026).
  Precision (a library extension of `real`; the deferred `NxE` literal)
  and `@additive`/`@foldable` measure semantics remain open, each with
  its own document to come.
- **The companion LL(1) grammar proof.**  The other M0 freeze artifact (core
  grammar proven LL(1)) lives with `04-grammar.md` and is not duplicated here;
  the freeze is contingent on it (`ROADMAP.md`, M0).
