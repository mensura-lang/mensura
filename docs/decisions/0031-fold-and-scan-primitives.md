# 0031: Fold and scan are the primitives; the rest are const bindings

## Status

Accepted, design-only.  This ADR lands as a documentation-only pull
request; the implementation follows separately and its checker rules stay
behind the ADR 0021 proof gates exactly as ADR 0029 staged them.

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
- **Refines `docs/decisions/0027-modules-and-imports.md` Decision 4**: the
  aggregate combinators that decision names as intrinsics of the initial
  environment are now *defined* by a bundled prelude source rather than
  hardcoded.  What is in scope does not change; how it is defined does.
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
inclusive one, and the derivation lemmas that make the prelude's
definitions theorems.  Notably, one piece of Stage 0 work disappears: the
fiber-as-rows model (Decision 1) needs nothing from `formal/`, because the
fiber already *is* `Multiset (Row H σ)` there; it is the surface that
catches up.

It deliberately does **not** include:

- any implementation (the builtin function kind, the rows type, the
  combiner literal, and the prelude module are named as the follow-on);
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
- `lag` is the previous row's value: an *exclusive* scan with the `last`
  combiner (each position receives the proper prefix's last element).
- `lead` is `lag` over the reversed order, with `first`.

`first` and `last` are associative but not commutative, which is exactly
the ordered-only table column 0029's Decision 10 reserved; it is now
load-bearing.  The sorted map (0029 Decision 9) existed to host `rank`,
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
directly (`fold `+` (|r| r.mass / r.height ^ 2) b`), and `count b` is
the natural row count of the group, where today one writes `count b.x`
and arbitrarily picks a column.

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
unsound otherwise, and `min`/`max` have no identity in the domain to
write.  The identity comes from the combiner table; the absent-identity
rows fold through the accumulator `Option` of 0029 Decision 4, which is
unchanged, as is the surface-totality rule derived from identity and
emptiness.

### 6.  The combiner is a backticked operator from the closed table

The combiner argument is written as a backticked operator name:
`` `+` ``, `` `*` ``, `` `min` ``, `` `max` ``, `` `or` ``, `` `and` ``,
and for scan only, `` `first` `` and `` `last` `` (Decision 7).  This
settles 0029's open question on where the combiner token lives.

The spelling is nearly free: a backticked name already lexes
(`lex_template` produces a `Template` token whose `{}`-free content is a
single literal), and because `` `or` `` and `` `and` `` are `Template`
tokens rather than words, the reserved-operator collision that made bare
combiner names impossible never arises.  What the implementation owes is
an expression-position parse arm and a highlight class; the token is
resolved against the closed table, and an unknown combiner is an error
naming the table.  The set extends by ADR, never by a user assertion
(0029 Alternative 1 stands).

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

Scan-only combiner rows need associativity but **not commutativity**,
since the key supplies the order a bag lacks: `` `first` `` and
`` `last` `` join the table for scan, realizing 0029 Decision 10's
ordered-only column.  Applying them under `fold` remains an error.

**Scan has an inclusive and an exclusive form.**  The inclusive form's
position i carries the fold of elements 1..i; the exclusive form carries
1..i-1, so its first output is the combiner's identity, or a missing
value for the identity-free rows, by the same identity-and-emptiness
rule that governs empty windows.  The exclusive form is not decoration:
it is what makes `lag` derivable (Decision 8).  Its surface name is an
open question; this ADR fixes its existence and semantics.

### 8.  The derived operations are const bindings in a bundled prelude

Every aggregate and window operation the documents have ever promised is
a **definition**, written in the language, shipped as a bundled source
compiled at build time exactly like `stdlib/si.mensura` (parsed, const
evaluated, oracle-tested in CI):

```mensura
let sum    { fold `+`   (|v| v) }
let count  { fold `+`   (|_| 1) }
let min    { fold `min` (|v| v) }
let max    { fold `max` (|v| v) }
let any    { fold `or`  (|v| v) }
let all    { fold `and` (|v| v) }

let cumsum { scan `+` (|v| v) }
let rank   { scan `+` (|_| 1) }          // the running count
// lag: the exclusive scan with `last`; lead: dually, with `first`
// over the reversed order.  Spelled once the exclusive form's surface
// name is fixed (Open questions).
```

`any` and `all` keep `bag<bool> -> bool`, settling 0029's open question
on the six's signatures: the predicate-taking form needs no second
signature because it is written directly, `fold `or` p b`.  `rank` is
the ones-scan because Decision 11's total-order requirement means there
are no ties to break.  `mean` becomes *expressible*
(`|b| sum b / to_real (count b)`); whether it ships here or in a `stats`
module stays open.

**Unlike `si`, the prelude loads into the initial environment
unqualified and without an `import`.**  This refines ADR 0027 Decision
4, which already names the aggregate combinators as intrinsics of the
initial environment ("these are *language*, always in scope"): what is
in scope does not change, and no implicit prelude beyond the initial
environment appears; what changes is that the initial environment's
aggregate vocabulary is now *defined* by a source file rather than
hardcoded in the checker.  Every existing `sum b.x` keeps working at
every site.  Explicit imports remain qualified-only, and the
redeclaration protection ("`sum` is an ambient builtin and cannot be
redeclared") becomes a collision with a prelude binding.

### 9.  The type model gains a dedicated rows type

The checker's `bag` type keeps its scalar element.  `b` types at a new,
dedicated rows type (the group's fields with their domains and
optionality); member access on it yields today's `bag<T>` per Decision
2.  Nested collections do not arrive by the back door: rows is not a
`bag` of records, it is the fiber's type, constructible only where
groups are.

This type outlives fold: it is the substrate `scan`'s key orders, so
Stage 2 inherits it rather than inventing one.

### 10.  Builtin function values join closure values

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

Stage 1 (gates `fold` and the fold-derived prelude bindings) is exactly
ADR 0029's: `foldBag` over a commutative monoid, the shard lemma over
arbitrary shards, the `Option` completion with its presence lemma, and
`aggregate` derived as the monoid-fold `fiberMap`.

Stage 2 (gates `scan` and every scan-derived binding) keeps ADR 0029's
demands (the `arrange` operation, `scanBag`, the fold-coherence theorem,
the prefix decomposition) and **grows** by:

- the exclusive scan and its coherence with the inclusive one (drop the
  last element of the inclusive scan, prepend the identity);
- the derivation lemmas, so the prelude is theorems rather than slogans:
  `rank` is the ones-scan, and `lag` is the exclusive `last`-scan.

Same blueprint node (`def:arranged`); nothing may be named
`Mensura.Arranged` before Stage 2 lands, per the stale-marker check.

## Consequences

**Positive.**  Two primitives instead of three, and the third's hosts
are now definitions anyone can read: the whole aggregate and window
vocabulary is greppable `.mensura` source, oracle-tested, with each
operation's combiner named at its definition.  The surface finally
matches `formal/` (the fiber is a bag of rows in both).  `fold` pipes
ordinarily, fixing 0029's no-piped-input corner.  The six are genuine
library instances, which is what 0029 Decision 2 promised and could not
yet deliver.  `count b` and row-mapper folds fall out of Decision 1.
`mean` is expressible.  The rows type is Stage 2's substrate, decided
once.

**Negative.**  A builtin function kind in `ConstValue` and the checker's
function type (mechanical after ADR 0030, but real).  A new rows variant
in the checker's type with the usual exhaustive-match sweep.  The
exclusive scan is new design surface with an unresolved name.  Scan's
key-as-argument diverges from 0029's `by` clause: stated here as a
revision with its reason (partial applicability), not slipped in.  The
prelude is a second bundled module, so the module loader's single-module
assumptions (diagnostic text, oracle-test shape) generalize.

**Neutral.**  Every existing aggregate call site is unchanged.  The
runtime may keep columnar group storage; rows is a type-level notion,
and the executor already holds the member indices it needs for row
iteration.

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
prelude would be impossible.  Tuple-valued keys recover the chained
spelling, and Decisions 7-8 of 0029 (open key, decidable obligation,
locally established order) carry over verbatim.

## Open questions

- **The prelude's name** (`prelude`, `std`): it never appears at a use
  site (it loads unqualified), so the choice matters for diagnostics and
  the repository layout only.
- **The exclusive scan's surface name** (`scan_x`, `prescan`, a flag on
  `scan`): its existence and semantics are fixed by Decision 7; `lag`
  and `lead`'s prelude definitions are spelled once this is.
- **Whether the rows type is user-writable in type position.**  Not for
  now: it is constructible only where groups are, and nothing needs to
  ascribe it.
- **How tier 3 of the tie model attaches.**  0029 Decision 11's
  arbitrary-tiebreak escape hatch was designed against a clause; with
  the key as an argument, the hatch needs a home (an `assume`-shaped
  wrapper on the scan, most likely).  The three-tier model is unchanged.
- **`map` and the window shape.**  A bag-valued field produced by `map`
  is the window shape of `map_bags`; confirm the shape rules compose
  when the docs are reconciled.
- **`stats` timing**: `mean` and `sd` are expressible now; whether they
  join the prelude or wait for a `stats` module (ADR 0028 Decision 4) is
  open.

## Forward references

- `docs/decisions/0029-fold-and-scan.md` (the model this re-founds; its
  revision notes point back here) and
  `docs/decisions/0030-const-functions.md` (the enabler).
- `docs/decisions/0027-modules-and-imports.md` Decision 4 (the initial
  environment; revised in part here) and
  `docs/decisions/0028-standard-library-si.md` (the bundled-module
  discipline the prelude reuses).
- `docs/decisions/0018-application-piping-equivalence.md` (the pipe the
  trailing-bag surface composes with) and
  `docs/decisions/0015-map-row-multiset-and-key-first-lambdas.md` (the
  columnar presentation Decision 1 demotes to sugar).
- `docs/decisions/0021-formal-proof-pipeline.md` (the gates), and
  `formal/Mensura/Core/Defs.lean` (the fiber the surface now matches).
- `docs/language/07-pipelines.md` and `09-typing-reference.md` (the
  reconciliation targets when the implementation lands).
