# 0031: Fold and scan are the primitives; the rest are const bindings

## Status

Accepted.  This ADR landed as a documentation-only pull request; the
implementation followed separately, with its checker rules behind the ADR
0021 proof gates exactly as ADR 0029 staged them.

**Implementation status.**  Both halves are **implemented**.  The fold half
landed first, behind ADR 0029's Stage 1 (`formal/Mensura/Fold.lean`):
Decisions 1, 2, 3, 4, 5, 6 (the `fold` rows), 8 (the `bag` module), 9, 10,
and 11.  The ordered half followed, behind Stage 2
(`formal/Mensura/Arranged.lean`): Decision 7 (`scan`, `prescan`, `desc`),
Decision 6's `scan`-only tack rows as *usable* combiners rather than merely
rejected ones, and Decision 8's `series` module.  Applying a tack under
`fold` is still an error naming the reason.

**Two corrections this ADR needs.**  Both were found by implementing it.

*Decision 8's mappers cannot typecheck as written.*  The module source
writes `let cumsum { scan `+` (|v| v) }`, but both of a scan's lambdas read
a **row**, not an element, and the trailing argument is the fiber rather
than a projected bag.  That is forced, not chosen: sorting values by a
sibling column requires seeing the row that carries both, so a projected
bag of bare scalars has nothing left to order by.  The bindings therefore
fix only the *combiner* and take the value extractor from the call site
(`series.cumsum (|r| r.energy) (|r| r.taken_at) b`).  Decision 7's own
`lead`, `|key| prescan `:>` (|v| v) (|r| desc (key r))`, already showed the
inconsistency: its key binder is `r` and reads a row.

*`lead` must curry explicitly.*  `|value, key| ...` binds a single 2-tuple
under ADR 0030 Decision 2, so it is written `|value| |key| ...`.

**Three prerequisites this ADR does not name** turned out to be
load-bearing, and are recorded here for the next reader:
**module-qualified function application** worked nowhere (the checker
rejected a non-name application head, and lowering mapped module members to
literals only, since `si` exports no function); **`Ty::Fn` is not an arrow
type** but a closure body, so the builtin kind of Decision 11 could not
reuse it; and **a const function returning a partially applied primitive
could not be applied further** (the application loop followed closures only,
so the argument after the one that saturated the closure reported "cannot
apply a value of type `function`").  Decision 8's entire surface rests on
the first, and `series.lead` on the third.

**Enabled by ADR 0030.**  ADR 0029 was written before const functions
existed; ADR 0030 landed lambdas, explicit currying, and partial
application as compile-time values, which is what lets `fold` and `scan`
be ordinary curried function values and lets every derived operation be a
const binding rather than a builtin.  This ADR is the re-founding of ADR
0029's surface on that machinery.

Relation to earlier decisions:

- **Revises `docs/decisions/0029-fold-and-scan.md` in part**: Decision 2
  (the fold surface: the bag becomes an explicit trailing argument and the
  mapper is per-element), Decision 3 (the table's mapper column), Decision
  6 (the scan surface: the order key becomes an ordinary argument),
  Decision 9 (the sorted map is dissolved; its hosts are scan-derived),
  and the spelling of Decision 11's second tier.  0029's model content is
  untouched: the closed combiner table, the open mapper, the accumulator
  identity (Decision 4), short-circuiting (Decision 5), local ordering
  (Decision 8), and the tie tiers (Decision 11) all carry over.  It also
  settles four of 0029's open questions; the notes live in 0029.
- **Amends `docs/decisions/0027-modules-and-imports.md` Decision 4**: the
  aggregate combinators that decision named as intrinsics **leave the
  initial environment**.  They become const bindings in qualified bundled
  libraries imported like `si` (Decision 8), and cardinality becomes the
  `#` operator (Decision 9).  The intrinsic environment shrinks to the
  base units, `fold`/`scan`/`map`, `desc`, `to_real`, and the pipeline
  operations; Decision 4's "no implicit prelude" rule now holds without
  the aggregate exception it used to carry.
- **Builds on `docs/decisions/0030-const-functions.md`** (curried
  builtins, partial application, beta-reducing lowering) and
  **`docs/decisions/0018-application-piping-equivalence.md`** (the
  trailing-argument pipe, which the explicit-bag surface composes with).

Formal backing: **no theorem lands with this ADR**, and the gates are
unchanged.  Per ADR 0021, `fold`'s checker rules wait on Stage 1
(`foldBag` over a commutative monoid, the shard lemma, the `Option`
completion), and `scan` plus every scan-derived binding waits on Stage 2
(the arranged structure, blueprint node `def:arranged`), whose demanded
content this ADR *grows*: the exclusive scan, its coherence with the
inclusive one, and the derivation lemmas that make the libraries'
definitions theorems.  Notably, one piece of Stage 0 work disappears: the
fiber-as-rows model (Decision 1) needs nothing from `formal/`, because the
fiber already *is* `Multiset (Row H σ)` there; it is the surface that
catches up.

It deliberately does **not** include:

- any implementation (the builtin function kind, the rows type, the
  combiner literal, the `#` token, the `bag`/`series` modules, and the
  call-site migration are named as the follow-on);
- **measure semantics** (the M5 item; the combiner table remains the
  object it will gate);
- **nested collections**: the rows type is dedicated, and `bag` keeps its
  scalar element;
- a generic **`zip`** of bags (bags are unordered; see Alternatives);
- any mandated **runtime representation** change: columnar storage remains
  a valid executor choice behind the row-typed surface;
- M5's streaming operators: `latest` looks scan-derivable, but that is the
  streaming document's business.

## Context

### What changed since ADR 0029

ADR 0029 fixed the model: a closed combiner table (associativity and
commutativity are laws the checker cannot verify on a user lambda), an
open mapper (its obligation is a type check), the identity in the
accumulator, and `fold`/`scan`/sorted-map as the primitive family.  Its
surface, however, was designed for a language without function values:
`fold` read the enclosing `map_bags` group implicitly and could not be
piped, the combiner token's grammar was left open, and the six aggregates
could only be "spellings" of `fold`, because there was no way to *bind*
a partial application to a name.

ADR 0030 removed that constraint.  A const binding can be a lambda,
application is saturated-or-error, currying is explicit, and partial
applications are ordinary values that beta-reduce at lowering.  Every
awkward corner of 0029's surface dissolves against that machinery, and
something stronger becomes available: the derived operations can be
*definitions in the language*, shipped as a bundled source and readable
by anyone who asks what `rank` means.

### The representation gap, and the model that closes it

A row-mapper fold such as `fold `+` (|r| r.mass / r.height ^ 2)` needs a
bag of *rows*.  The bag lambda's `b` is presented today as a record of
bags (`b.x` is `bag<T>`; ADR 0015), which is row-major data flattened to
columns.  The transpose back is sound only for `b` itself, because its
columns are provenance-aligned (they come from the same rows of the
group); it is not a generic operation on bags.

The resolution is to recognize which representation is the model.  In
`formal/`, a fiber is `Multiset (Row H σ)`: a bag of rows.  The columnar
record-of-bags is the checker's *presentation*, and the docs already
teach the projection reading ("`b.credits` is the bag of `credits`
across the rows at the key", `docs/language/07-pipelines.md`).  So `b`
should simply *be* the bag of rows, with `b.x` as projection sugar; the
transpose the surface seemed to need is the identity in the model.

### Two primitives suffice

With `b` as rows and functions first-class, the family collapses:

- `rank` is the running count: a scan of ones.
- `lag` is the previous row's value: an *exclusive* scan (`prescan`,
  Decision 7) with keep-right, each position receiving the proper
  prefix's last element.
- `lead` is `lag` over the dual order: the same keep-right `prescan`,
  with the key marked descending (`desc`, Decision 7).

Keep-left and keep-right (the tacks `<:` and `:>`, Decision 6) are
associative but not commutative, which is exactly the ordered-only table
column 0029's Decision 10 reserved; it is now load-bearing.  The sorted
map (0029 Decision 9) existed to host `rank`,
`lag`, and `lead`; with all three scan-derived, it dissolves.  `fold` is
not scan-derivable (it works over unordered bags with no key), so the
primitive set is exactly two, plus `map` standing apart as projection,
which is not a reduction and not derivable from one.

## Decision

### 1.  The bag lambda's `b` denotes the fiber: a bag of rows

`b` in `map_bags |k, b| ...` is a bag of rows, matching the fiber
`Multiset (Row H σ)` of `formal/Mensura/Core/Defs.lean` exactly.  The
columnar record-of-bags of ADR 0015 becomes the derived presentation, not
the model.  `k` is unchanged: a record of the key columns as total
values.

Two things fall out immediately.  A row-mapper fold applies to `b`
directly (`fold `+` (|r| r.mass / r.height ^ 2) b`), and `#b`
(Decision 9) is the natural row count of the group, where today one
writes `count b.x` and arbitrarily picks a column.

Bare `b` in a scalar position was a type error before (a record is not a
value) and remains one (a bag of rows is not a value), so no existing
program reads differently.

### 2.  `b.x` is projection sugar

```
b.x  ==  map (|r| r.x) b
```

Every existing aggregate site keeps its exact spelling and meaning:
`sum b.credits` still consumes the bag of `credits`.  The sugar equation
is the definition, stated once; in `formal/` it is `Multiset.map` at a
field projection.

The soundness caveat, stated so nobody generalizes it: projection is
well-defined because `b`'s columns are jointly indexed by the group's
rows.  That alignment is provenance, not structure a type could carry,
which is why there is **no generic `zip`** of two arbitrary bags: bags
are unordered, so there is no "i-th element" to pair (see Alternatives).

### 3.  `map` is a curried builtin, and the explicit form of projection

```
map : (element -> value) -> bag -> bag
```

`map` is the general element-wise transform: `map (|r| r.mass) b` is the
explicit spelling of `b.mass`, and `map (|r| r.mass / r.height ^ 2) b`
is a computed bag no projection sigil could express.  There is no
`.*field` operator: the sugar covers the common case, `map` covers the
general one, and two spellings of one operation with a sigil in between
is exactly the duplication this repository's nomenclature sweeps exist
to remove.

`map` stands apart from the reduction family: it is the projection
functor, needed by Decision 2's sugar, and not derivable from `fold` or
`scan` (a scan demands an order; `map` is order-free).

### 4.  `fold` is a curried builtin with an explicit trailing bag

```
fold : combiner -> (element -> value) -> bag -> value
```

`fold` is an ordinary function value (ADR 0030): partial applications
are values, the bag is the last argument, and piping composes by ADR
0018 (`b.x |> fold `+` (|v| v)`).  This replaces 0029 Decision 2's
implicit-group design, under which `fold` read the enclosing lambda's
group and could not be piped at all.

The mapper is per-element.  Over a projected bag the element is a value
(`fold `+` (|v| v * v) b.x`, the sum of squares); over `b` itself the
element is a row, which is how 0029's headline example is finally
expressible.  The mapper remains fully open, the combiner fully closed:
the obligation asymmetry of 0029 Decision 2 (a type versus a law) is
restated here unchanged.

### 5.  No user seed; the identity still lives in the accumulator

Reaffirming 0029 Alternative 3 against the tempting
`fold start op` shape: a seed is counted once per shard under partial
folding, so it is redundant when it equals the combiner's identity and
unsound otherwise, and `<<`/`>>` (binary minimum and maximum, Decision 6)
have no identity in the domain to write.  The identity comes from the
combiner table; the absent-identity rows fold through the accumulator
`Option` of 0029 Decision 4, which is unchanged, as is the
surface-totality rule derived from identity and emptiness.

### 6.  Every combiner is a surface operator, quoted by backticks

The combiner argument is a backticked **operator**:

| operator | scalar meaning | algebra | identity | absorber | admitted under |
| --- | --- | --- | --- | --- | --- |
| `+` | addition | commutative monoid | `0` | none | `fold` and `scan` |
| `*` | multiplication | commutative monoid | `1` | `0` | `fold` and `scan` |
| `<<`, `>>` | binary minimum, maximum | commutative semigroup | none | none | `fold` and `scan` |
| `or`, `and` | boolean | commutative monoid | `false`, `true` | `true`, `false` | `fold` and `scan` |
| `<:`, `:>` | keep-left, keep-right | semigroup (not commutative) | none | none | `scan` only |

The identity and absorber columns are not symmetric decorations; they
play opposite roles.  An **absorber is read**: the executor merely
recognizes it in the data and may stop early (0029 Decision 5's
licensed short-circuit), so an absorber need only be a value that can
legitimately occur, which is why `*`'s `0` and `or`'s `true` qualify.
An **identity is written**: the machinery fabricates it into results,
as the empty window's answer, `prescan`'s first output, and the
parallel shard's base, so an identity must be the *genuinely true
answer of the empty case*, storable and arithmetic-safe as data.  `0`
is the true sum of nothing; there is no true minimum of nothing, which
is what "none" in the `<<`/`>>` row means.  IEEE `+Inf` would fill that
cell only by extending the domain, and the extension fails citizenship
three ways: `inf - inf` mints `NaN`, which destroys the total order the
row's admission rests on; ADR 0026 already bans dimensioned infinities;
and a fabricated `+Inf` is a sentinel where `0`-for-`sum` is an honest
value.  Extending `real` to `[-Inf, +Inf]` is recorded as an open
question, not an inconsistency: on the extended domain the lattice
closes uniformly, and the finite domain is a *choice*.

One row carries a domain restriction the others do not.  A fold's
accumulator type must be invariant, so a combiner is admitted only
where it is type-preserving, `T -> T -> T`.  Dimensioned `*` is not: it
*adds* exponent vectors (ADR 0026), so folding it would give a product
whose dimension depends on the bag's cardinality, which no static type
can carry.  The `*` row is therefore admitted at the **dimensionless**
numeric domains only (`int`, bare `real`); `prod` over a
`temperature[real]` bag is a type error, while `sum` (whose `+`
requires equal dimensions and preserves them) works at every dimension.

Two operator families are new, introduced here so that the table is
operators **uniformly**, with no wordy residents:

- **`a << b` and `a >> b`** are the binary minimum and maximum: both
  operands of one orderable domain (dimension included, so the earlier
  of two dates and the smaller of two temperatures both work), the
  result of that domain.  They bind looser than `+ -` and tighter than
  the comparisons, so `a + b << c` is `(a + b) << c` and `a << b < c`
  is `(a << b) < c`.  (The ranking against the comparisons is
  superseded: ADR 0040 makes the tacks unranked, so that meeting now
  takes parentheses.)  Independently useful at the surface (clamping,
  earlier-of-two-dates); in the table they are the rows the aggregate
  `min`/`max` derive from.
- **`a <: b` and `a :> b`** are keep-left and keep-right, APL's tacks:
  `a <: b` is `a`, `a :> b` is `b`, both operands of one domain.  Their
  algebra is compiler-owned and two lines deep: associative (left- and
  right-zero semigroups), not commutative, no identity.  Their scalar
  use is trivial by design; their habitat is the backticked combiner
  slot, where they are what make `first_value`, `lag`, and `lead`
  scan-derivable (Decision 8).  Being non-commutative they are admitted
  under `scan` only; under `fold` they are an error.

This settles 0029's open question on where the combiner token lives, and
closes it without residue: a backtick always quotes an operator, and the
words `min`, `max`, `first`, `last` never enter the table.  The spelling
is nearly free: a backticked name already lexes (`lex_template` produces
a `Template` token whose `{}`-free content is a single literal), and
because `` `or` `` and `` `and` `` are `Template` tokens rather than
words, the reserved-operator collision that made bare combiner names
impossible never arises.  What the implementation owes is the four new
operator tokens (`<<`, `>>`, `<:`, `:>` collide with nothing: the lexer
has no shift tokens, and `:` followed by `>` occurs in no existing
production), an expression-position parse arm for the backticked form,
and a highlight class; the token is resolved against the closed table,
and an unknown combiner is an error naming the table.  The set extends
by ADR, never by a user assertion (0029 Alternative 1 stands).

### 7.  `scan` is the second primitive; the order key is an argument

```
scan : combiner -> (element -> value) -> (row -> key) -> rows -> bag
```

`scan` is the ordered sibling: same combiner table, same mapper, plus an
order key, emitting every intermediate where `fold` keeps the last.  Two
revisions to 0029 Decision 6's surface, both forced by the const-binding
goal:

- **The order key is an ordinary argument**, not a `by` clause.  A
  clause is not partially applicable, so `let cumsum { scan `+` (|v| v) }`
  could not exist with a clause in the signature.  Everything 0029
  Decisions 7 and 8 established about the key carries over verbatim: it
  is an open lambda whose obligation (an orderable domain) is decidable,
  and it establishes the order locally, demanding no qualifier fact.
- **Tuple-valued keys subsume `then`-chaining**: `|r| (r.date, r.seq)`
  orders lexicographically, which is what 0029 Decision 11's second tier
  spelled as `by g then h`.  The three-tier totality model itself is
  unchanged; only the spelling of tier 2 moves.

**Descending order is an intrinsic word, `desc`, marking key values.**
`desc e` wraps an orderable value in its order dual: orderable in,
orderable out, never storable and never ascribable (checker-internal,
like the rows type).  Because the marker sits on *values*, direction is
**per-component**: `|r| (r.date, desc r.priority)` is SQL's
`ORDER BY date ASC, priority DESC`, which no global reverse flag could
express, and `desc` of a whole tuple distributes to its components.  The
tie tiers are untouched (the dual of a total order is total, and `desc`
of an injective column is injective).  A direction marker rather than a
comparator is forced by the same epistemics as the combiners: a
comparator's obligation is a law (a strict total order), unverifiable on
a lambda, so keys stay values in orderable domains and direction stays a
marker.  In `formal/` the marker is Mathlib's `OrderDual`, instances
included, so Stage 2's `arrange` absorbs it at no proof cost.  `desc`
joins the intrinsic words (`fold`, `scan`, `map`, `to_real`): it is an
order constructor the libraries are written *with*, not an operation
derivable from them.

Scan-only combiner rows need associativity but **not commutativity**,
since the key supplies the order a bag lacks: the tacks `` `<:` `` and
`` `:>` `` join the table for scan, realizing 0029 Decision 10's
ordered-only column.  Applying them under `fold` remains an error.

**Scan has an inclusive form (`scan`) and an exclusive form
(`prescan`)**, after Blelloch's naming.  `scan`'s position i carries the
fold of elements 1..i; `prescan`'s carries 1..i-1, so its first output
is the combiner's identity, or a missing value for the identity-free
rows, by the same identity-and-emptiness rule that governs empty
windows.  The exclusive form is not decoration: `lag` is
`prescan `:>``, each position receiving the proper prefix's keep-right,
that is, the previous element, with the first row correctly missing
because keep-right has no identity.  The missingness of `lag`'s first
row is not designed; it falls out of the rule.

### 8.  The derived operations are const bindings in qualified libraries

Every aggregate and window operation the documents have ever promised is
a **definition**, written in the language, shipped as bundled sources
compiled at build time exactly like `stdlib/si.mensura` (parsed, const
evaluated, oracle-tested in CI), and **imported and qualified like `si`**.
The organization follows the derivation structure, which is also the
proof-stage structure: one module per primitive, gated by that
primitive's Lean stage.

```mensura
// stdlib/bag.mensura -- the fold-derived reductions (Stage 1)
let sum  { fold `+`  (|v| v) }
let prod { fold `*`  (|v| v) }
let min  { fold `<<` (|v| v) }
let max  { fold `>>` (|v| v) }
let any  { fold `or`  (|v| v) }
let all  { fold `and` (|v| v) }
```

```mensura
// stdlib/series.mensura -- the scan-derived windows (Stage 2)
let cumsum      { scan `+`  (|v| v) }
let rank        { scan `+`  (|_| 1) }    // the running count
let running_min { scan `<<` (|v| v) }
let running_max { scan `>>` (|v| v) }
let first_value { scan `<:` (|v| v) }    // every row sees the group's
                                         // first value under the order
let lag         { prescan `:>` (|v| v) } // the previous row's value;
                                         // the first row is missing
let lead        { |key| prescan `:>` (|v| v) (|r| desc (key r)) }
                                         // lag over the dual order:
                                         // the next row's value
```

A descending `rank` needs no binding of its own: it is the ones-scan
over a `desc` key at the call site.

A view writes `import bag` and `bag.max b.temperature`.  There is **no
implicit prelude and no unqualified loading**: with `fold` and `scan` as
builtins there is no reason to keep so many *names* in the language, and
ADR 0027 Decision 4's rule ("nothing else is in scope that you did not
import") now applies to the aggregate vocabulary too.  This is a genuine
amendment to that decision, which had named the aggregate combinators as
intrinsics; the intrinsic initial environment shrinks to the base units,
`fold`/`scan`/`map`, `desc` (Decision 7), `to_real` (pending a `math`
module; Open questions), and the pipeline operations.  A corollary: the
names `sum`,
`min`, `max`, `any`, `all`, and `count` return to users, and the
redeclaration protection shrinks with the intrinsics.

`any` and `all` keep `bag<bool> -> bool`, settling 0029's open question
on the six's signatures: the predicate-taking form needs no second
signature because it is written directly, `fold `or` p b`.  `rank` is
the ones-scan because Decision 11's total-order requirement means there
are no ties to break.  `mean` becomes *expressible*
(`|b| bag.sum b / to_real (#b)`); whether it joins `bag` or waits for a
`stats` module stays open.  Note `count` appears in neither module: it
is the `#` operator (Decision 9).

The migration is real and named: every bare-aggregate call site in the
corpus, the examples, and the book gains an `import` and a qualifier
(or `#`), in the implementation's reconciliation pull request.  The
corpus is young and CI-gated, so the change is mechanical and cannot
rot.

### 9.  `#` is the cardinality operator

`count` is the most frequent aggregate, and cardinality has an
established notation.  The word leaves the language; the operator
arrives:

```
#e  ==  fold `+` (|_| 1) e
```

`#b` is the group's row count and `#b.x` counts a projected bag (the
prefix binds looser than member access, so `#b.x` is `#(b.x)`, and
tighter than the comparisons, so `#b > 3` reads as expected).  The
meaning is fixed by the sugar equation, exactly like Decision 2's
projection sugar, so the everything-derives-from-`fold`-and-`scan`
claim survives: only the *spelling* is language.

The cost is one lexer token (`#` is free today; comments are `//`), one
prefix production with an LL(1) note in `04-grammar.md`, and a highlight
class.  The division of labour is deliberate: **operators are language,
words are library**, and count crosses the frequency threshold that
earns an operator where the rest of the vocabulary does not.

### 10.  The type model gains a dedicated rows type

The checker's `bag` type keeps its scalar element.  `b` types at a new,
dedicated rows type (the group's fields with their domains and
optionality); member access on it yields today's `bag<T>` per Decision
2.  Nested collections do not arrive by the back door: rows is not a
`bag` of records, it is the fiber's type, constructible only where
groups are.

This type outlives fold: it is the substrate `scan`'s key orders, so
Stage 2 inherits it rather than inventing one.

### 11.  Builtin function values join closure values

`fold`, `scan`, and `map` have function types but no lambda bodies: the
language deliberately has no recursion and cannot express bag iteration,
so they are primitives whose *types* are function-shaped.  `ConstValue`
and the checker's function type therefore each need a builtin-backed
kind alongside ADR 0030's closure kind, with per-slot application rules
(a combiner token, then functions, then the bag).  This is the
implementation's first task and is mechanical now that ADR 0030 built
the closure kind it mirrors.

## What this needs from `formal/`

Nothing new for the rows model: the fiber already is
`Multiset (Row H σ)`, and Decision 2's sugar equation is `Multiset.map`
at a projection, a one-line lemma if ever wanted.  The surface is
catching up to the formalization, not the reverse.

Stage 1 (gates `fold`, `#`, and the `bag` module) is exactly ADR
0029's: `foldBag` over a commutative monoid, the shard lemma over
arbitrary shards, the `Option` completion with its presence lemma, and
`aggregate` derived as the monoid-fold `fiberMap`.

Stage 2 (gates `scan` and the `series` module) keeps ADR 0029's
demands (the `arrange` operation, `scanBag`, the fold-coherence theorem,
the prefix decomposition) and **grows** by:

- the exclusive scan (`prescan`) and its coherence with the inclusive
  one (drop the last element of the inclusive scan, prepend the
  identity);
- the derivation lemmas, so the libraries are theorems rather than
  slogans: `rank` is the ones-scan, `lag` is the keep-right `prescan`,
  and `lead` is `lag` at the dual key (`desc` is Mathlib's `OrderDual`,
  so `arrange` absorbs it with no new structure).

Same blueprint node (`def:arranged`); nothing may be named
`Mensura.Arranged` before Stage 2 lands, per the stale-marker check.

## Consequences

**Positive.**  Two primitives instead of three, and the third's hosts
are now definitions anyone can read: the whole aggregate and window
vocabulary is greppable `.mensura` source, oracle-tested, with each
operation's combiner named at its definition.  The language itself
shrinks: with `fold` and `scan` as builtins there is no reason to keep
so many names in the initial environment, and ADR 0027 Decision 4's
no-implicit-prelude rule now holds without exceptions.  The names `sum`,
`min`, `max`, `any`, `all`, and `count` return to users.  The surface
finally matches `formal/` (the fiber is a bag of rows in both).  `fold`
pipes ordinarily, fixing 0029's no-piped-input corner.  The six are
genuine library instances, which is what 0029 Decision 2 promised and
could not yet deliver.  `#b` and row-mapper folds fall out of Decisions
1 and 9.  `mean` is expressible.  The rows type is Stage 2's substrate,
decided once.

**Negative.**  **Every bare-aggregate call site breaks**: the corpus,
the examples, and the book gain an `import` and a qualifier (or `#`).
The corpus is young and CI-gated, so the migration is a mechanical,
one-time reconciliation pull request, but it is the largest surface
change since the pipeline algebra landed and the book's first examples
now carry an `import` line.  A builtin function kind in `ConstValue` and
the checker's function type (mechanical after ADR 0030, but real).  A
new rows variant in the checker's type with the usual exhaustive-match
sweep.  Five new operator tokens (`#`, `<<`, `>>`, `<:`, `:>`) with
their grammar notes, plus the intrinsic word `desc` and its
checker-internal order-dual wrapper.  `prescan` is new design surface
(its coherence theorem joins Stage 2).  Scan's key-as-argument
diverges from 0029's `by` clause: stated here as a revision with its
reason (partial applicability), not slipped in.  `bag` and `series` are
the second and third bundled modules, so the module loader's
single-module assumptions (diagnostic text, oracle-test shape)
generalize.

**Neutral.**  The runtime may keep columnar group storage; rows is a
type-level notion, and the executor already holds the member indices it
needs for row iteration.  Qualified aggregate names read one token
longer (`bag.max b.temperature`); the `exposing` refinement ADR 0027
contemplates remains available if that ever grates.

## Alternatives considered

### 1.  An explicit transpose operator

`!b` as a symbol (no `Bang` token exists; a sigil in a language that
spells things as words) or `rows b` as a named builtin (additive and
workable, but it leaves two models alive: `b` would remain columnar
while its transpose is row-major, and every future operation would
choose).  Rejected in favour of making the model unambiguous: `b` *is*
the rows, and the columnar view is the sugar, matching `formal/`.

### 2.  A projection sigil (`b.*mass`)

Rejected: there is no ambiguity for it to resolve, since `b.x` reads
identically before and after Decision 1 and the docs already teach the
projection reading; `map` is the explicit form and strictly more useful;
and the sigil costs a token, a production, and a second spelling of one
operation.

### 3.  A user-supplied seed (`fold start op ...`)

Re-rejected; 0029 Alternative 3's argument is unchanged by the curried
surface.  The seed is redundant where it is safe and unsound where it is
not, and four of the six aggregates have no seed to write.

### 4.  An open combiner lambda

Re-rejected; 0029 Alternative 1 and Decision 2.  A fold over an
*unordered* bag is deterministic only for associative-commutative
combiners, and those laws cannot be checked on a lambda.  The naive
`|start| |op| |f| |sequence| op (op start (f sequence[0])) ...` sketch
presumes an indexed order a bag does not have; that assumption is the
bug the closed table exists to prevent.

The rejection covers manifestly-correct lambdas too:
`fold (|a, b| if a < b then a else b) (|v| v)` *is* the minimum
semantically, but the backticked `` `<<` `` and the lambda differ
epistemically, not semantically.  The operator's algebra is compiler
knowledge; the lambda's would rest on trust, and admitting one lambda
admits them all, including `|a, b| a - b` and the one-character typo
`if a < b then a else a`.  Decision 6's operators exist precisely so
that no combiner ever needs to be a lambda.

### 5.  A generic `zip` of bags

Rejected: bags are unordered, so pairing two arbitrary bags element-wise
is meaningless.  The alignment that makes Decision 2's projection sound
is provenance of the group record, not structure that a `zip` could
demand of its operands.

### 6.  Keeping the sorted map as a primitive

Rejected: its three hosts (`rank`, `lag`, `lead`) are scan-derived, so
the primitive would have no resident.  Re-add it only if a positional
operation appears that is not expressible as a scan; until then, fewer
primitives is the stronger closure principle.

### 7.  The order key as a `by` clause (0029 Decision 6's spelling)

Rejected on partial applicability: a clause is not an argument, so no
scan-derived operation could be a const binding, and Decision 8's
libraries would be impossible.  Tuple-valued keys recover the chained
spelling, and Decisions 7-8 of 0029 (open key, decidable obligation,
locally established order) carry over verbatim.

### 8.  An unqualified prelude, loaded without an `import`

An earlier draft of this ADR shipped the derived operations as a prelude
merged into the initial environment, so every bare `sum b.x` kept
working.  Rejected: it was compatibility-driven rather than principled,
and it carved the exact exception ADR 0027 Decision 4's "nothing else is
in scope that you did not import" exists to forbid.  With `fold` and
`scan` as builtins there is no reason to keep so many names in the
language; the corpus is young and CI-gated, so the migration is a
mechanical one-time cost, and the freed names (`sum`, `min`, `max`,
`any`, `all`, `count`) are worth more than the compatibility.

### 9.  `first`/`last` as backticked words rather than operators

The tacks could have stayed wordy table rows (`` `first` ``,
`` `last` ``), leaving the convention "a backtick names a combiner-table
row, most rows coincide with operators".  Rejected in favour of `<:` and
`:>`: the uniform statement "a backtick quotes an operator" has no
caveat to remember, the tacks' algebra is as compiler-ownable as `+`'s,
and APL's tacks are precedent that keep-left and keep-right are
respectable operators even with trivial scalar readings.

### 10.  Other descending-order designs

Three rejected in favour of Decision 7's `desc` marker.  **Comparator
lambdas** (`|a, b| ...` deciding the order): a comparator's obligation
is a law, a strict total order, unverifiable on a lambda; a broken one
makes `arrange` nondeterministic, the combiner epistemics one level up.
**Suffix-scan variants** (a backward `scan`/`prescan` pair): solves
`lead` but doubles the scan builtins, reverses only globally (the
mixed-direction key `date` ascending, `priority` descending stays
unexpressible), and everything a suffix scan computes, `scan` over a
`desc` key already does, since reversal just swaps the tacks.  **Other
spellings** of the marker: an operator (`~e`) does not clear the
frequency bar that `#` cleared and the tilde reads as "approximately";
`series.desc` would demand an `import` to state a key direction for the
builtin `scan`, which needs none.

## Open questions

- **Whether the module organization is final** (`bag`, `series`): the
  cut follows the derivation and the proof stages, but finer names
  (`order`, `logic`) remain reachable later by re-exports if a module
  grows enough to warrant splitting.
- **`to_real`'s home.**  It is the one remaining word-builtin that is
  neither `fold`, `scan`, `map`, nor a pipeline operation; by this ADR's
  own logic it belongs in a future `math` module (with `sqrt`, `log`,
  `abs`, and the half-exponent dimension question), and stays intrinsic
  only until that module exists.
- **Whether the rows type is user-writable in type position.**  Not for
  now: it is constructible only where groups are, and nothing needs to
  ascribe it.
- **How tier 3 of the tie model attaches.**  0029 Decision 11's
  arbitrary-tiebreak escape hatch was designed against a clause; with
  the key as an argument, the hatch needs a home (an `assume`-shaped
  wrapper on the scan, most likely).  The three-tier model is unchanged.

  **Settled while implementing this ADR, and not by a new decision.**  The
  hatch is `assume { arranged }`: ADR 0017 wrote that its block form
  "generalizes later without a surface change", and this ADR's own Decision
  11 already calls ties "structurally the same problem as completeness", so
  the two decisions meeting is what fixes the spelling.  A scan now
  *demands* tie-freedom, discharged either from a grading (tier 1, checked
  by the rule `Mensura.keyInjOn_demote_tag` backs) or by the claim (tier 3).
  An undischarged key is an error rather than a silent stable sort, which is
  what makes the obligation the same shape as the reducer's completeness
  demand.  Tier 2 remains unexpressible, below.
- **Tuple keys need a value-tuple type, which does not exist.**  Decision 7
  says a tuple key orders lexicographically and thereby subsumes 0029's
  `then`-chaining, so tier 2 of the tie model rests on it.  But the checker
  has no `Ty::Tuple`: the tuple syntax exists only to destructure a tupled
  lambda's parameters, and adding a *value* tuple collides with ADR 0030
  Decision 2's convention, under which `f (a, b)` binds two parameters
  rather than passing one pair.  Resolving that is an ADR-level question,
  not an implementation choice.  Scalar keys cover every binding in
  `series`, so nothing shipped waits on it, and a tuple in a key position
  reports the gap by name.

  **When tier 2 lands, the tie rule must extend with it.**  A tuple key is
  injective when its whole component set is, which is a grading over
  `key + {c1, c2, ...}`, so the lookup generalizes from one column to a set
  rather than needing a new mechanism.  Two things to get right: the
  components' *union* is what must be graded, not each part separately (a
  tuple can be injective when no single component is, which is the entire
  reason tier 2 exists), and a `desc` marker on a component stays
  transparent.  Extending the lookup and the claim in the same change is
  what keeps a tuple key from bypassing a check its scalar counterpart has
  to pass.
- **`map` and the window shape.**  A bag-valued field produced by `map`
  is the window shape of `map_bags`; confirm the shape rules compose
  when the docs are reconciled.
- **`stats` timing**: `mean` and `sd` are expressible now; whether they
  join `bag` or wait for a `stats` module (ADR 0028 Decision 4) is
  open.
- **Extending `real` to `[-Inf, +Inf]`.**  On the extended domain the
  order rows close into a bounded lattice: `<<` gains identity `+Inf`
  and absorber `-Inf`, dually for `>>`, and the table becomes uniform
  with the boolean rows (which *are* `<<`/`>>` on `false < true`).
  This ADR chooses the finite domain because the extension currently
  fails citizenship three ways (Decision 6): `inf - inf` mints `NaN`,
  which breaks the total order; ADR 0026 bans dimensioned infinities;
  and empty reductions would fabricate `+Inf` where data semantics
  wants *missing*.  Related and unspecified either way: the
  implementation can already mint `+Inf` today (`1.0 / 0.0` divides
  unguarded in the const evaluator and the runtime), so
  division-by-zero semantics must be fixed regardless; under the finite
  stance it becomes a diagnostic or a missing result.

## Forward references

- `docs/decisions/0029-fold-and-scan.md` (the model this re-founds; its
  revision notes point back here) and
  `docs/decisions/0030-const-functions.md` (the enabler).
- `docs/decisions/0027-modules-and-imports.md` Decision 4 (the initial
  environment; revised in part here) and
  `docs/decisions/0028-standard-library-si.md` (the bundled-module
  discipline `bag` and `series` reuse).
- `docs/decisions/0018-application-piping-equivalence.md` (the pipe the
  trailing-bag surface composes with) and
  `docs/decisions/0015-map-row-multiset-and-key-first-lambdas.md` (the
  columnar presentation Decision 1 demotes to sugar).
- `docs/decisions/0021-formal-proof-pipeline.md` (the gates), and
  `formal/Mensura/Core/Defs.lean` (the fiber the surface now matches).
- `docs/language/07-pipelines.md` and `09-typing-reference.md` (the
  reconciliation targets when the implementation lands).
