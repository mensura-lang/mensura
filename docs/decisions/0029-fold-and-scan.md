# 0029: Bag aggregates as a fold, scan, and sorted-map family

## Status

Accepted, design-only.  This ADR lands as a documentation-only pull request:
it fixes the model and the surface intent, and no code, no Lean, and no
language-doc reconciliation ship with it.

It opens **no milestone**.  It generalizes a surface already frozen at M0
(`docs/language/09-typing-reference.md` section 5.4) and supplies the primitive
that M5's window rollups (`ROADMAP.md`, "Streaming and reactive") and the
still-open M3 measure-semantics document both need to refer to.  It is
deliberately *not* the M3 measure-semantics item, which it scopes out below.

Relation to earlier decisions:

- **Amends `docs/decisions/0014-scalar-domain-taxonomy.md`** twice.  It closes
  0014's open question "Reintroducing `mean` and other statistics as named
  sugar" by answering *not in the language*, and it reframes 0014's six
  aggregate signatures as library instances of one primitive rather than six
  independent builtins.
- **Answers the open question in
  `docs/decisions/0007-single-expression-sublanguage.md`**, which asks which
  builtins "are in the core, and how is the set extended".  The answer is a
  closed combiner table plus an open mapper, and the extension mechanism is a
  new table row by ADR.
- **Refines `docs/decisions/0015-map-row-multiset-and-key-first-lambdas.md`**
  and the aggregate-shape / window-shape distinction of
  `docs/language/07-pipelines.md` by giving both shapes one combiner table.
- **Does not amend
  `docs/decisions/0023-completeness-consumed-by-the-reducer.md`.**  The
  completeness obligation stays exactly where 0023 put it, since `fold` *is*
  the reducing shape.  0023's decision text lists `mean` among the reducers
  and inherits a wording fix.
- **Anticipates, does not decide,** the measure-semantics document.

Formal backing: **no theorem lands with this ADR.**  Unlike
`docs/decisions/0026-dimensional-physical-units.md`, which shipped
`formal/Mensura/Units/Dimension.lean` alongside its rules, this ADR states
what `formal/` must supply *before* any checker rule follows, in the section
"What this needs from `formal/`".  That is the ADR 0021 discipline: a checker
propagation rule ships only when a theorem under `formal/` backs it.  The
existing results it builds on are `Mensura.fiberMap`,
`Mensura.aggregate_eq_fiberMap`, `Mensura.flatMap_eq_fiberMap`, and
`Mensura.fiberMap_exhaustive` in
`formal/Mensura/Completeness/FiberMap.lean`; the planned node it claims is
`def:arranged` / `Mensura.Arranged` in `formal/blueprint/src/content.tex`.

It deliberately does **not** include:

- the surface grammar for `fold`, `scan`, the sorted map, `by`, and the
  tiebreak chain (a `docs/language/04-grammar.md` change, including how a
  combiner token interacts with the keyword-free lexer and the lambda-bar
  hazard);
- the language-doc reconciliation edits, listed under "Follow-ons" below;
- any Lean development (this ADR states the demands, not the proofs);
- any change under `crates/`, including the `Agg` enum;
- **measure semantics** (`@additive` and its family): which combiners a given
  column may be folded with, and the annotation surface that declares it;
- **top-k** and other bounded accumulators, which are neither folds nor
  positional maps.

`rank`, `lag`, `lead`, and the treatment of ties are **not** exclusions: they
are Decisions 9 and 11.

## Context

### The set is closed by enumeration, not by a principle

Six bag aggregates exist today: `sum`, `min`, `max`, `count`, `any`, and
`all`.  They are specified as six independent signatures
(`docs/language/09-typing-reference.md` section 5.4,
`docs/decisions/0014-scalar-domain-taxonomy.md`) and implemented as a
six-variant enum (`crates/mensura-types/src/expr_check.rs`, `enum Agg`).
Nothing in the surface says *why* those six and not others, which is exactly
the question ADR 0007 left open.  A closed set with no closure principle
cannot answer "may I add `first`?" except by taste.

### `mean` is the tell

`mean` is the one member of the folk "aggregate" set that never became a
builtin.  ADR 0014 rejected it as derivable, `expr_check.rs` records that in a
doc comment on `Agg`, and the docs write it longhand at every real use site
(`docs/language/10-views.md`, `docs/language/07-pipelines.md`).

The reason it resisted is structural, not a matter of taste.  `sum`, `min`,
`max`, `count`, `any`, and `all` are each *one* fold over *one* binary
operator.  `mean` is two folds plus a division.  So the existing set already
*is* "the folds", discovered by accident rather than by design.  Name the
principle out loud and the set closes itself, and the question of what may
join it gets an answer.

### The Lean development already unified the two fiber shapes

`formal/Mensura/Completeness/FiberMap.lean` defines `fiberMap` and proves it
the universal shape of a key-preserving split-invariant operation
(`splitInvariant_keyLocal_iff_fiberMap`), with `flatMap` and `aggregate` as
its two generators (`flatMap_eq_fiberMap`, `aggregate_eq_fiberMap`).  Its
`fiberMap_exhaustive` docstring states that *both* `map_bags` shapes satisfy
the presence-preservation fact: the aggregate shape folds a present fiber to
one row, and the window shape emits one output row per input row.

So what the surface calls "aggregate" and what it calls "window" are one
thing in `formal/` and two unrelated things at the surface.  The claim to be
precise about: what is unified there is the fiber **shape** (each output key
reads only its own input key), not the ordered **content**.  The same file
scopes order-dependent verbs out explicitly, noting that `lag`, `cumsum`, and
`rank` "are not bag operations" and require lifting from the bag monad to the
list monad.  This ADR's position is that the surface should follow the shape
unification, and that the ordered content is new formal work.

### Why the combiner must be closed while the mapper is open

The design rests on an asymmetry of obligations, and it is worth stating
before the decisions rather than after.

The **mapper**'s obligation is "the per-row expression has the combiner's
operand type".  That is a type check, and the existing expression checker
already decides it.  So the mapper can be an arbitrary user lambda at no
cost.

The **combiner**'s obligation is associativity and commutativity.  That is an
algebraic law, undecidable in general, and unverifiable from a user
annotation.  Admitting a user-declared combiner would therefore mean trusting
an unverifiable claim, and the consequence of a false claim is worse than it
first appears.  Compare `assume { complete }`
(`docs/decisions/0017-completeness-establish-consume.md`): a false
completeness claim yields a *wrong* answer, but the answer is still
deterministic and reproducible, and it is auditable against the data.  A
false associativity claim yields a *nondeterministic* answer, because the
result then depends on how the executor shards the bag and on how many
threads it uses.  Same input, different output across runs, and both the
sequential path and the small-input path agree, so testing does not surface
it.

For a language whose thesis is that semantic mistakes become compile errors,
and whose type system exists to make results reproducible, a user-assertable
algebraic law is the worst-shaped hole available.  The combiner set stays
closed.

### What the scan adds, and why it is a sibling rather than a new feature

`cumsum` has been promised as pending sugar in three places
(`docs/language/07-pipelines.md`, `docs/language/09-typing-reference.md`,
`docs/decisions/0013-qualifier-scope-and-the-content-boundary.md`), always
with the same caveat: it needs an ordering from the dependency qualifier.
The dependency qualifier is ADR 0004 machinery, and ADR 0004 itself records
that its document is unwritten.  So an ordering precondition that was never
needed has been a standing blocker on a feature the docs keep promising.

`docs/language/02-stores.md` already resolves this, and commits to the exact
surface this ADR ratifies: a `bag` store carries no row order, and when an
operation needs one, "the order is named at the operator by a `by` clause,
not carried by the store".  Decision 8 is therefore a ratification of a
commitment already in the docs, not a new choice.

## Decision

### 1.  `mean` leaves the language

`mean` is removed, not deferred.  Four reasons, in order of weight: it is not
a fold (two folds plus a division), so it does not belong to the family this
ADR closes; it was never a builtin, so nothing is removed from the
implementation; the docs already spell it out longhand wherever it is really
used; and it has no home yet, because ADR 0027 Decision 2 gives a module
const bindings and type-level names only, so `stdlib/si.mensura` cannot host
a function today.

At the use site it is written `sum b.x / to_real (count b.x)`, which is
honest about being two folds, and which is already dimensionally correct
under ADR 0026 with no special rule.

`mean` and `sd` (used illustratively in `docs/language/03-shapes.md`) are
recorded as future `stats` module content per ADR 0028 Decision 4,
contingent on modules gaining function exports.  This closes ADR 0014's open
question in the negative: not named sugar in the language, a library function
later.

### 2.  `fold(map, combine)` is the primitive: mapper open, combiner closed

The general bag reduction is `fold` over a mapper and a combiner.  The
**mapper** is an arbitrary per-row expression in the bag lambda's row scope.
The **combiner** is a token drawn from the closed table of Decision 3.

No user-supplied combiner is admitted, and no annotation by which a user
asserts that an operator is associative or commutative is admitted.  The
justification is the obligation asymmetry set out in Context: the mapper's
obligation is decidable and the combiner's is not, and a false algebraic
claim costs reproducibility rather than mere accuracy.  See Alternative 1.

The six existing aggregates become **spellings** of `fold` at fixed table
rows, not primitives in their own right.  They keep their current surface
syntax; nothing a user has written changes.

### 3.  The combiner table carries a (mapper, combiner, identity) triple

Each aggregate is a row of one table:

| aggregate | mapper | combiner | identity | algebra |
| --- | --- | --- | --- | --- |
| `sum` | `\|r\| r.x` | `+` | `0` | commutative monoid |
| `count` | `\|r\| 1` | `+` | `0` | commutative monoid |
| `min` / `max` | `\|r\| r.x` | `min` / `max` | none in the domain | commutative semigroup |
| `any` / `all` | `\|r\| p r` | `or` / `and` | `false` / `true` | commutative monoid, with an absorbing element |

Two rows of that table state facts a list of per-aggregate signatures cannot
express, and both are load-bearing.

**`count`'s combiner is not `count`.**  Partial counts are combined by `+`.
An implementation that applies "the same operator" to partial results gets
`count` wrong.  The general shape is a *triple*, not a single operator, and
this is the cheapest possible demonstration of why.

**`min` and `max` have no identity in the domain.**  This is not an oversight
to be patched with an infinity.  `int` has no `+Inf`, and a `+Inf` metre is
not a physical quantity, which is exactly the discipline ADR 0026 imposes on
dimensioned values.  Decision 4 resolves this without inventing one.

### 4.  The identity lives in the accumulator, not in the value domain

`docs/language/07-pipelines.md` guarantees that `map_bags` skips empty bags,
so the lambda always sees a non-empty bag.  That makes a sequential, seedless
fold total over any associative-commutative combiner, with no identity
needed, which is why `min` and `max` work today.

Partial and parallel folding breaks that guarantee at a different level: a
*shard* of a non-empty bag can be empty, so combining partial results does
need a unit.  The resolution is to put the unit in the **accumulator** rather
than in the value domain: a partial result is an optional value, absent for
an empty shard, and combination takes the present side when one is absent.
This is the free monoid completion of the semigroup, and it reuses the
`Cell = Option` structure already in the model
(`docs/language/06-expressions.md`) rather than extending any domain.

**The optionality never reaches a surface type.**  It is internal to the
executor's partial results, so `min` over a bag of `T` still yields `T`, and
ADR 0010's total-versus-optional discipline and its `?` marker are untouched.
A reader should not conclude that `min` became optional-returning.

In Mathlib terms the completion is `WithTop` / `WithBot` shaped, folded by
`Multiset.fold` over the completed monoid; naming it makes the Stage 1 demand
concrete.

### 5.  Short-circuiting is a licensed executor optimization, not surface

Every expression is pure and lazy (`docs/language/06-expressions.md`), so an
`any` that stops at the first `true` and one that scans the whole bag are
observationally identical.  Short-circuiting is therefore an
evaluation-strategy choice that belongs to the executor, and it needs no
surface form and no user declaration.

The absorbing elements recorded in Decision 3 are what license it, and
closing the combiner set is what makes the license safe to grant: an open
combiner would carry no absorbing-element knowledge for the executor to
exploit.

The runtime does not short-circuit today
(`crates/mensura-runtime/src/eval.rs`, the `any` / `all` arm folds the whole
slice) and may begin to with no language change and no further ADR.

### 6.  `scan` is the ordered sibling of `fold`, over the same combiner table

`scan` takes the same combiner table, the same mapper, and additionally a
`by` clause.  The two differ in one respect: `fold` keeps the final
accumulator, and `scan` emits every intermediate.  So `scan` returns one row
per input row, which is *already* the window shape that
`docs/language/07-pipelines.md` and `docs/language/09-typing-reference.md`
derive from a bag-valued `map_bags` return.

`cumsum b.x by |r| r.date` is `scan` at the `+` row with mapper `|r| r.x` and
order key `|r| r.date`, which makes `cumsum` sugar exactly as the docs have
promised.

"Same combiner, two variants" is the organizing idea of this ADR, and it is
not merely a slogan: Stage 2 owes a coherence theorem that a scan's last
element equals the corresponding fold.

### 7.  `by g` is an open key extractor, because its obligation is decidable

The order key `g` is an arbitrary user lambda.  Its obligation is that it
returns a value in an orderable domain (`int`, `real`, `date`, and a
dimensioned `real`), which the existing checker already decides.  This
asymmetry runs the *opposite* way from Decision 2, and the contrast is the
point: the mapper and the order key are open because their obligations are
types, and the combiner is closed because its obligation is a law.

The combiner stays closed here for an additional reason independent of any
type question: parallel prefix scan (Blelloch) and DBSP-style incremental
maintenance both require an associative combiner, so an arbitrary step
function would forfeit both.

Note that `scan` needs only **associativity**, not commutativity, since `by`
supplies the order that a bag lacks.  That is a real difference from `fold`
and Decision 10 exploits it.

One implementation caveat, so this ADR does not assert a capability the
checker lacks: `docs/language/11-physical-units.md` specifies a dimensioned
`real` as orderable, and `ColumnType::is_orderable` in
`crates/mensura-types/src/model.rs` already admits `Quantity`, but the
diagnostic text on `min` / `max` in `expr_check.rs` still names only "int,
real, or date" and will need widening.

### 8.  `by` establishes the ordering locally; it demands no existing fact

The sort happens at the operation.  `by g` does not consume an ordering fact
supplied by a qualifier, and it does not require one to have been
established upstream.

This ratifies a commitment the docs already make:
`docs/language/02-stores.md` states that when an operation needs an order,
"the order is named at the operator by a `by` clause, not carried by the
store".  Two other places call ordering "a dependency-qualifier concern"
(`docs/language/07-pipelines.md`,
`docs/language/09-typing-reference.md`), and that is what has blocked
`cumsum`; but ADR 0004's dependency qualifier is unimplemented and its
document unwritten, so a precondition that was never needed has been
gating a feature that does not need it.

Forward compatibility: when the dependency qualifier lands, a known ordering
fact becomes an **optimization** (skip the sort, because the content is
already arranged), never a precondition.  No program valid under this ADR
becomes invalid later.

### 9.  A sorted map is the third primitive; it hosts `rank`, `lag`, `lead`

`rank` is a positional index function: it cannot be obtained from any monoid
on row values, because it depends on a row's *position* rather than on an
accumulation of values.  `lag` and `lead` are shifts, not accumulations.
Neither is an instance of the combiner table, and introducing `scan` while
saying nothing about them would read as including them, since every document
that mentions `cumsum` mentions `rank` in the same breath
(`docs/language/07-pipelines.md`, `docs/language/09-typing-reference.md`,
`docs/language/02-stores.md`, ADR 0013).

What they share with `scan` is the arranged content plus access to ordered
position.  So the third primitive is a **sorted map**: a row-local map over
the arranged bag whose mapper sees the row, its index, and its neighbours.
`rank` is the index; `lag` and `lead` are the neighbours.

That completes the family by shape:

- `fold` is many-to-one;
- `scan` is one-to-one and accumulating;
- the sorted map is one-to-one and positional.

All three take the same `by` clause and rest on the same Stage 2 arranged
structure.

### 10.  The table may carry associative-only rows for the ordered variants

`fold` requires commutativity because a bag has no order.  Once `by` supplies
an order, `scan` and the sorted map require only associativity (Decision 7).
The table therefore gains an ordered-only column: rows admissible under an
established order but not over a bare bag.

This is what makes the three primitives *sufficient* over the ordered cases
rather than merely broad.  `first` and `last` are many-to-one and
order-dependent, so they are neither commutative folds nor maps; as folds
under a non-commutative associative combiner, admitted only where an order is
established, they become table rows rather than a missing feature.

**Top-k remains outside the family.**  It is neither a fold at a fixed
combiner nor a positional map, and it needs a bounded accumulator whose
algebra this table does not describe.  Saying so is better than stretching
the table to cover it.

### 11.  Ties are made explicit, not made impossible

This is the usability crux of the ADR.  If the order key `g` is not
injective, the induced order is partial, and a scan or a sorted map over tied
rows has no determined result.  The failure mode is the same class as a false
associativity claim: nondeterminism, not inaccuracy.

The framing that resolves it is that **ties are structurally the same problem
as completeness**.  Neither is decidable, so the fact is *established*,
*propagated*, and *demanded* where unsoundness would otherwise hide, which is
the discipline ADR 0017 and ADR 0023 already set.  `by g` therefore requires
a **total** order, established in one of three tiers.  The tiers are what
make the requirement usable rather than merely sound.

**Tier 1, derived.**  When the extractor targets a column set already known
functional, uniqueness follows and no annotation is needed.  This is not
aspirational: `Functional` already exists in the checker
(`crates/mensura-types/src/table.rs`), is backed by `Mensura.Functional` in
`formal/`, and is ratified by
`docs/decisions/0024-key-moves-as-a-true-inverse-pair.md`.  This covers the
common case, an order key drawn from the key, at zero ceremony.

**Tier 2, chained.**  The author chains extractors, in the manner of `by g
then h`, until the order is total in their judgment.  The checker still
cannot prove totality, but the author has stated what to do with ties, which
is the thing they know and the checker does not.

**Tier 3, explicitly arbitrary.**  An `assume`-shaped escape hatch admits a
tiebreak-dependent result.  It is unverified, exactly like
`assume { complete }`, but it is **declared at the site and therefore
greppable**, so a reviewer auditing reproducibility can enumerate every scan
whose result depends on a tiebreak.

The property that matters is that nondeterminism becomes **visible in the
source** rather than silent, and that forgetting about ties produces a
diagnostic pointing at the `by` clause instead of a wrong answer in
production.

This is admitted where the `@associative` annotation of Alternative 1 is
refused, and the distinction is principled rather than a matter of degree: a
false associativity claim is a claim about an algebraic *law*, and it is
invisible and unauditable.  A tie declaration is a claim about *this* data at
*this* site, and it is auditable.

## What this needs from `formal/`

ADR 0021 fixes the rule: a checker propagation rule ships only when a theorem
under `formal/` backs it.  Nothing in the Decision section reaches the checker
before its stage below is proved.  The two stages differ sharply in
difficulty, so they are staged rather than bundled.

### Stage 1: the monoid-parameterized fold (gates `fold`)

- A `foldBag` over a commutative monoid.  Order-independence comes for free
  from `Multiset`'s quotient, and Mathlib's `Multiset.fold` already carries
  the well-definedness, so this is the easy half.
- `aggregate` (`formal/Mensura/Core/Ops.lean`) derived as `fiberMap` at a
  monoid-parameterized fiber action: an analogue of `aggregate_eq_fiberMap`
  that replaces the opaque whole-bag function with a monoid fold.  The honest
  direction here is worth stating: today's `aggregate` takes an *arbitrary*
  whole-bag function and is therefore **more** general than a monoid fold, so
  this carves out the well-behaved subclass rather than generalizing.
- The semigroup case via the optional completion, plus a lemma that on a
  non-empty bag the result is present.  That lemma is what licenses the total
  surface type asserted in Decision 4.
- A shard lemma: folding shard-wise and combining the partial results equals
  folding the whole bag, for arbitrary shards including empty ones.  This is
  the theorem Decision 4 exists to enable, and it is false without the
  accumulator identity.
- A note reconciling the verb set.  The bag-NRC generator table in
  `formal/Mensura/Completeness/Verbs.lean` lists only the additive fold
  `Sigma`, so `min`, `max`, `any`, and `all` already sit outside the literal
  generator.  The verb set's completeness argument therefore needs either a
  widening to any commutative-monoid fold or an explicit conservativity note.
  This is a pre-existing gap that this ADR surfaces rather than creates, and
  it now has an owner.

### Stage 2: the ordered structure (gates `scan` and the sorted map)

The obstacle is structural, not incidental.  A `Table`'s content is a
`Multiset` of rows, and `formal/Mensura/Core/Defs.lean` argues *for*
multisets precisely because order is not something the model should assert
when it is not used.  Neither a scan nor a positional map is expressible over
a `Multiset`.  So this stage needs new structure, and it should claim the node
the blueprint already reserves, `def:arranged` / `Mensura.Arranged` in
`formal/blueprint/src/content.tex`, rather than invent one.

Demanded content:

- an `arrange`-by-key operation taking fiber content from `Multiset` to
  `List`;
- a `scanBag` over a semigroup on arranged content;
- a positional map over arranged content, hosting `rank`, `lag`, and `lead`
  (Decision 9);
- the **coherence theorem** that a scan's last element equals the
  corresponding fold, which is what makes "same combiner, two variants" a
  theorem rather than a slogan;
- a prefix-scan decomposition lemma, if parallel scan is to be licensed.

**On split-safety, the demand is a hypothesis to pin, not a contradiction to
resolve.**  `formal/Mensura/Core/Defs.lean` defines `split` so that an
indicator routes each key's *whole* multiset of rows to one side, and ADR 0023
relies on exactly that when it argues a reducing `map_bags` is Tier A.  A
key's bag is therefore never torn by a split.  So `by g` sorts an intact bag,
and the results are identical whichever side the key lands on: the surface
docs' "split-safety holds regardless"
(`docs/language/07-pipelines.md`, `docs/language/09-typing-reference.md`) is
**correct** for these operations.  The blueprint's note that arranged verbs
are "deliberately not split-invariant" concerns lifting to the list monad in
general, not these operations under this `split`.  What Stage 2 owes is a
lemma pinning the hypothesis, that a scan is split-invariant given a
whole-key-routing split, which `KeyLocal` plus strictness already supplies
for `fiberMap`.

**The tie tiers cost almost nothing formally.**  Only Tier 1 has formal
content: a functional column set induces a total order on the arranged
content, so `arrange` is deterministic.  That should compose with the
existing `Mensura.Functional` rather than restate it.  Tiers 2 and 3 are
surface obligations with nothing to prove, since a chain is a lexicographic
composition of extractors and the escape hatch asserts.  Put another way,
`arrange` needs a determinism hypothesis and the three tiers are three ways
of discharging it.

### Stage 3: blueprint bookkeeping

New nodes for the fold family under the "Safe completeness" chapter, and the
promotion of `def:arranged` out of the "Open problems" chapter when Stage 2
lands.

### Shipping order

`scan` and the sorted map may both ship behind `fold`, because Stage 1's work
is bounded and Stage 2's is not.  Shipping `fold` first is coherent: the
combiner table is shared and the ordered variants are additive to it.  Within
Stage 2, `arrange` plus the Tier 1 determinism lemma gate both ordered
primitives, so those land together or not at all.

## Consequences

**Positive.**  The aggregate set closes by a principle instead of an
enumeration, which answers ADR 0007's open question.  `count`'s distinct
combiner and `min` / `max`'s missing identity become stated facts rather than
latent surprises for an implementer.  `cumsum` stops waiting on ADR 0004, and
`rank`, `lag`, and `lead` gain a home.  The surface catches up to the fiber
shape unification already proved in `formal/`.  Measure semantics gains a
concrete object to gate, the (column, combiner) pair, instead of an
unresolvable taxonomy of columns.  Short-circuiting becomes available with no
surface commitment.

On expressiveness, the open mapper buys real ground rather than only
re-deriving what exists: `count_if` is `fold` at the `+` row with mapper
`|r| if p r then 1 else 0` and needs no new table row, and
`fold` with mapper `|r| r.mass / r.height ^ 2` collapses what is today a
`flat_map` followed by a `map_bags` into one stage.

On usability specifically, three primitives plus the `by` clause cover every
ordered operation the docs currently promise, and ties get a graded answer
whose common case needs no annotation at all.

**Negative.**  `mean` appears in prose across several documents, so the
reconciliation surface is wide even though no code changes.  A user with a
genuinely associative custom combiner has no path, deliberately; the
mitigation is that the table grows by ADR, which is a deliberate speed bump
rather than a wall.  `scan` and the sorted map block on Lean structure that
does not exist and that the blueprint itself flags as hard.  `fold(map,
combine)` is more verbose at the use site than `sum b.x`, so the six sugar
spellings must survive or the surface regresses ergonomically; Decision 2
keeps them.  The ADR surfaces a gap in the verb set's completeness argument
that someone now owns.

**Neutral.**  No runtime behaviour changes today.  The `Agg` enum's six
variants can be reinterpreted as table rows rather than rewritten.  ADR
0023's completeness obligation attaches to the reducing shape unchanged,
since `fold` *is* that shape.

## Alternatives considered

### 1.  A user-assertable `@associative` or `@commutative` combiner

The most tempting and the most dangerous.  Rejected because the claim is
unverifiable and, unlike `assume { complete }`, a false claim costs
reproducibility rather than accuracy: the result then depends on shard
boundaries and thread count, and it is invisible to testing because the
sequential and small-input paths agree.  The contrast with the mapper, which
*is* open because its obligation is decidable, is what makes this a
principled rejection rather than mere conservatism.  Compare Decision 11,
which admits an unverified *data* claim precisely because it is auditable at
the site.

### 2.  Keep six independent signatures (the status quo)

Rejected.  It leaves ADR 0007's question open indefinitely, hides `count`'s
distinct combiner, and offers no place to put `scan` or the sorted map.

### 3.  `fold(init, op)` with a user-supplied seed

Subtler than it looks, and rejected.  The seed is not free: it must be the
combiner's identity, or the Stage 1 shard lemma fails and partial folding
becomes unsound.  So accepting an arbitrary seed either reopens the soundness
hole or requires checking the seed against a table that already carries the
identity.  The seed is therefore redundant where it is safe and unsound where
it is not.  This is also what defeats the natural reading of `min` as
`fold(+Inf, min)`: the seed does not exist in `int`, and inventing one for a
dimensioned value contradicts ADR 0026.

### 4.  A fully general step function with both the operator and the body open

Rejected on the same grounds as Alternative 1, one level up: a general scan's
step is order-dependent by construction, so associativity cannot be required
of it, and without associativity both parallel prefix scan and incremental
maintenance are forfeit.  What it would buy is non-commutative associative
folds over ordered content (string concatenation, `first`, `last`), and
Decision 10 shows those are reachable by adding table rows instead.

### 5.  Making ties impossible instead of explicit

Three variants, all rejected in favour of Decision 11, and each fails the
usability goal differently.  *Require `g` injective*: enforceable neither
statically, since it is a property of the data, nor cheaply at runtime, since
it costs a distinctness pass.  *Manufacture a stable tiebreak from the
storage surrogate row identifier*: `bag` stores have one
(`docs/language/02-stores.md`), but exposing it would leak the storage layer
into the language semantics, which the storage/processing split exists to
prevent, and `formal/Mensura/Core/Defs.lean` deliberately models content as
unordered.  *Make any possibly-tied scan a type error*: sound but unusable,
since the checker cannot derive injectivity for a computed key and the common
case would be rejected.

### 6.  Measure semantics as an optional annotation on this combiner table

Rejected as scope, not as an idea.  The reframing is recorded here as the
interface the later document should use: an annotation declares **which
combiners a column admits**, so a temperature column admits `min` and `max`
but not `+`.  That is a property of the (column, combiner) pair rather than a
taxonomy of columns, and it dissolves the problem of defining
`@semiadditive` before the language has an axis notion
(`ROADMAP.md`, `docs/language/11-physical-units.md`, ADR 0026).  The
annotation family, its grammar, and its per-dimension defaults remain a
separate document.

## Open questions

- **How far Tier 1 reaches.**  Decision 11 derives totality from
  `Functional`, which exists, but the checker must connect an extractor
  lambda to a functional column set.  A bare field access is clearly
  traceable; a computed key (truncating a timestamp to a date) is clearly not
  order-injective even when the source column is.  Where the derivation
  stops, and whether the rule should be syntactic (field access only) or
  something richer, is open.
- **Whether the chained form carries a totality obligation.**  Tier 2 lets an
  author chain extractors without proof.  Whether the checker should also
  *demand* that the final chain be derivably total, collapsing Tier 2 into
  Tier 1, or accept the chain as a better-ergonomics Tier 3, is a judgment
  call worth revisiting once real programs exist.
- **Surface syntax for the three tiers.**  The chaining keyword and the name
  of the arbitrary-tiebreak hatch are placeholders here; the grammar is
  scoped out.
- **Whether a prefix scan over a partial bag needs a completeness fact.**  The
  window shape demands nothing today
  (`docs/language/09-typing-reference.md`) on the grounds that one output row
  per input row is faithful.  That holds for a row-local window, but a
  *prefix* scan reads every earlier row, so a missing early row corrupts
  every later output.  `scan` may therefore sit on the reducing side of ADR
  0023's line despite being window-shaped, which would show that the
  reducing/windowing distinction is not the same distinction as the
  aggregate-shape/window-shape one.  Flagged, not decided.
- **Where the combiner token lives grammatically**: an operator section, a
  reserved word, or a name resolved from the closed table, and how that
  interacts with the keyword-free lexer.
- **Whether the six sugar spellings keep their exact signatures**, in
  particular whether `any` and `all` become predicate-taking.  The docs
  (`docs/language/06-expressions.md`) show a predicate form, while ADR 0014
  and the implementation both give `bag<bool> -> bool`; the open mapper
  resolves the divergence in the docs' favour, and the ADR should confirm
  which spelling survives.
- **Where `mean` lands and when**, contingent on ADR 0027 gaining function
  exports, and whether `stats` is the right module.
- **Whether the dependency qualifier ever demands anything at `scan`**, or is
  only ever the optimization Decision 8 asserts.

## Forward references

- `docs/decisions/0007-single-expression-sublanguage.md` (the builtin-set
  question this answers), `0014-scalar-domain-taxonomy.md` (the signatures
  generalized and the `mean` question closed),
  `0015-map-row-multiset-and-key-first-lambdas.md` (the two `map_bags`
  shapes), `0017-completeness-establish-consume.md` and
  `0023-completeness-consumed-by-the-reducer.md` (the establish/demand
  discipline Decision 11 reuses), `0021-formal-proof-pipeline.md` (the proof
  gate), `0024-key-moves-as-a-true-inverse-pair.md` (`Functional`),
  `0026-dimensional-physical-units.md` (dimensioned combiners and the
  no-invented-infinity discipline), `0027-modules-and-imports.md` and
  `0028-standard-library-si.md` (the future home of `mean`),
  `0004` and `0013` (the deferred dependency qualifier this routes around).
- `formal/Mensura/Completeness/FiberMap.lean`,
  `formal/Mensura/Core/Ops.lean`, `formal/Mensura/Core/Defs.lean`,
  `formal/Mensura/Completeness/Verbs.lean`,
  `formal/blueprint/src/content.tex` (`def:arranged`).
- The measure-semantics document to come (`ROADMAP.md`, M3).

## Follow-ons

None of the following is done in this pull request.

**Reconciliation, `mean` removal.**  `docs/language/06-expressions.md`,
`07-pipelines.md`, and `09-typing-reference.md` (the aggregate list and the
"`mean` is derived" parenthetical in each); `03-shapes.md` (three
illustrative uses); ADR 0026 and ADR 0023 (each lists `mean` among the
dimension-preserving or reducing operations).  `10-views.md` and one site in
`07-pipelines.md` already spell it longhand and need no edit, which is what
makes the removal as cheap as it is.

**Reconciliation, `fold` / `scan` / sorted map.**
`docs/language/06-expressions.md` (bag combinators become fold instances, and
the predicate-form divergence resolves); `07-pipelines.md` (the `map_bags`
shapes, and the named-sugar forward reference where `cumsum` is promised);
`09-typing-reference.md` sections 5.4 and 6.2 plus the forward-reference
list; `04-grammar.md` if a `by` clause or an operator-section form is added;
`02-stores.md` (the `by`-clause promise, now delivered); `ROADMAP.md` at M5
and at the M3 measure-semantics item; and the check-gated book blocks under
`book/src/`.

**Then, separately:** the Stage 1 Lean work, then the `fold` checker rules;
the Stage 2 Lean work, then `scan` and the sorted map; the measure-semantics
document; and the top-k question if a consumer appears.
