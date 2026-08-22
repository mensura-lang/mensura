# Pipelines

A pipeline transforms one table into another.  In Mensura a pipeline is not a
separate construct: it is an ordinary expression of table type, built from the
one expression sublanguage of `06-expressions.md`.  Stages are function
applications composed left to right with the `|>` pipe, intermediate tables are
named with `let`, and several tables are combined by tupling them into a merge.
There is no special pipeline grammar; there is a set of table-valued
operations, catalogued here.

This document specifies those operations.  Each one is a pure function over
`Table<Qs, C>` (a table's qualifiers and content, see
`docs/decisions/0004-qualifier-mechanism.md`), and each is backed by a theorem
in the Lean formalization (`formal/Mensura/`), cited inline.  This round
specifies the **primitives** only; the familiar named forms (`filter`,
`mutate`, `select`, `aggregate`, `group_by`-style forms, window functions,
`tagged_union`/`tagged_split`) are sugar over these and are deferred to a
follow-up, as are the streaming operations and the hosting of pipelines in
`transform`/`view` declarations.

The syntax shown is preliminary, like the rest of the language docs at this
stage; the design content is not.

## What a pipeline tracks

The point of typing pipelines is that a table carries more than its rows.  An
operation is read by what it does to the table's **content** and its
**qualifiers** (the two parts of `Table<Qs, C>`, ADR 0013), and the type
checker rejects a pipeline that would violate one of them:

- **Content** (`C`): the structure, namely the key columns and the
  non-key columns with their domains.  Reindexing moves columns between the
  key and the non-key part.
- **Cardinality** (table-scoped qualifier): how many nested rows share a key,
  **singletons** (`card <= 1`) or **bag** (`card 0..*`).  Operations
  transform it predictably, and some operations *demand* a particular
  cardinality (`pivot` wants `singletons`, at most one row per key).  The
  evidence behind the scalar is a set of **gradings**, column sets over
  which the table is known **functional** (at most one row per combination
  of values); `singletons` is derived as "some grading fits inside the
  current key" (ADR 0024), which is what makes the key moves invertible.
- **Totality** (column-scoped qualifier): whether each non-key value is
  known or may be missing (`Cell = Option`).  A value is total unless its
  type is marked `?` (ADR 0010); `lookup` makes its right columns
  optional, and the `??` discharge makes a column total again (ADR 0039).
- **Completeness** (table-scoped qualifier): whether a partition is fully
  present, that is, whether every key's bag has all of its rows.
  Completeness is what makes a per-bag fold faithful.  It is established
  (by a check, a source annotation, or a `registry` mechanism), cleared
  by a genuinely coarsening `demote` (the fact is about the current key;
  ADR 0035, whose decision 6 spares a coarsening along `exhaustive`
  axes), and consumed (by a reducing `map_bags`; ADR 0023).
- **Lineage** (table-scoped qualifier): the split ancestry that carries
  disjointness, specified in `08-lineage.md`.  Sampling and dependency, the
  two `std` qualifiers of ADR 0004 with no rules yet written, are deferred;
  this document notes where an operation imposes a qualifier-level
  precondition but does not re-specify propagation rules.

The qualifiers this document makes first-class are **cardinality**,
**totality**, and **completeness**: every operation below states how it
affects each one it changes.

## Composition

Three forms thread operations together, all from the expression sublanguage:

- **`|>`**, the pipe: `data |> op` applies `op` to `data`.  An op is an
  ordinary curried function, so a partially applied stage such as
  `lookup machines (|k, r| r.machine)` is the `Table -> Table` value the pipe
  feeds.  The pipe is reversed application, `x |> g` means `g x`, so a stage
  may always be written either way; the equivalence and its two-class
  consequence are spelled out in `06-expressions.md` and recorded in
  `docs/decisions/0018-application-piping-equivalence.md`.  How the checker
  realizes that equivalence is `docs/toolkit/01-application-checking.md`.
- **`let`**, to name an intermediate table and reuse it (forking a pipeline is
  binding a table once and using it twice).
- **tuples**, to bring several tables together for a merge:
  `(train, test) |> union`.

The central guarantee is **split-safety**.  Every Tier A operation is
`SplitSafe` (`PreservesDisjoint` and `SplitInvariant`), and split-safe
operations are closed under composition (`SplitSafe.comp`, `Core/Defs.lean`).  So a
pipeline built only from Tier A operations commutes with a split: running it on
the whole table and running it on each side of a split and re-binding give the
same result.  That is the formal content of "no leakage between train and
test."  A Tier B operation breaks this: it drops the lineage fact, which must
then be re-established or assumed downstream.

## The primitives

Each entry gives the surface form, the parameters, the effect on **content**,
on **cardinality**, on **completeness** (and on **totality** where the
operation changes it), the Tier, and the backing theorem.
Throughout, lambdas are **key-first** (ADR 0015): `|k, r| ...` binds the key
`k` and a single value row `r`, `|k, b| ...` binds the key and the bag `b`
(a row whose cells are bags), `split`'s `|k|` binds the key alone, and
`|_, r|` ignores the key.  A bare column name (`machine`) is a reference to a
column of the current schema.

### `flat_map` - per-row transform

```
data |> flat_map |k, r| (.bmi = r.mass / r.height ^ 2.0)
```

The key-first lambda receives the key and one value row and returns a
**collection of value rows** (ADR 0015): a bare row or record keeps one,
`()` drops the row, and `(a, b, ...)` expands to several.  Content: the
output columns are those of the returned rows; the key is preserved.
Cardinality: the maximum collection size, so a body that returns at most one
row preserves per-key cardinality and a body that may return two or more
yields `bag`.  Completeness: preserved.  Tier A (`flatMap_splitSafe`).

Because the body is a collection, dropping a row (a filter) and emitting
several rows (an expansion) are the same primitive: a filter is
`flat_map |k, r| if c then r else ()`, using the conditionals and collection
literals of `06-expressions.md`.  There is no `filter` primitive; a named
`filter` may later be sugar for this form (ADR 0015).

### `map_bags` - per-key whole-bag transform

```
data |> map_bags |k, b| (.total = bag.sum b.credits)
```

The key-first lambda receives the key and the **fiber** `b`, the bag of rows
at that key (ADR 0031).  Member access on it is projection, defined by
`b.x == map (|r| r.x) b`, so `b.credits` is the bag of `credits` across the
rows at the key, reduced here by `bag.sum`; `#b` is the group's row count, and
a reduction's mapper over `b` itself sees a whole row.  Empty
bags are skipped, so the lambda always sees a non-empty bag.  Content: the
output columns are those of
the return.  Cardinality: **inferred from the return** - returning a single
record yields `singletons` (one row per key, the `aggregate` shape, and it is
what later lets `pivot` satisfy its `singletons` precondition); returning a
bag yields `bag` (the window shape: one output row per input row).
Completeness: **demanded by the reducing shape**
(`docs/decisions/0023-completeness-consumed-by-the-reducer.md`): a body that
folds each key's bag to a single record is silently wrong on a partial bag,
so it consumes the completeness obligation.  The fact itself is carried
through the operation: every body expressible today emits at least one
output row per present fiber (the aggregate shape folds a non-empty bag to
one record, the window shape emits one row per input row), so a key present
before the operation is present after it.  That carry-through is a property
of the current body language, not of `map_bags` in general (see the open
questions).  Over a `singletons` input the
obligation discharges trivially, since a present key's single row is the
identity's whole fiber (`fiberCompleteWrt_of_functional`); the demand bites
only where a present key's bag can be partial, that is, on a `bag` input.  A
window-shaped return demands the same fact when its scan's combiner is
fold-admitting, because such a scan contains its reduction: its last row is
the fold, and every row folds a prefix (ADR 0037 decision 5).  The keep
combiners demand nothing: one output row per input row relating *present*
rows is faithful on a partial bag.  Tier A (`fiberMap_splitSafe`).

Window-style returns (a bag, one row per input row, such as a running total or
a rank) additionally require an **ordering** within the bag.  That ordering is
named at the operator, by a `scan`'s key argument, and needs no qualifier fact:
what a window orders is a single fiber, and a `split` routes a key's whole bag
to one side, so the order is established locally
(`Mensura.scanFiber_splitSafe`, ADR 0029 Stage 2).  Split-safety therefore
holds regardless, and `rank`/`cumsum` are well-defined because the call site
supplies the order rather than because the store carries one.

### `promote` / `demote` - rekeying

Reindexing is one idea with two directions: move a column into the key, or move
one out.  The direction fixes the Tier.

```
data |> promote machine      // move the `machine` column into the key
data |> demote course       // move `course` out of the key
```

Cardinality at the key moves is **key-graded** (ADR 0024): the gradings are
facts about the flat table, indifferent to which columns currently form the
key, so a key move changes the key, leaves the gradings untouched, and
re-derives the scalar from the subset check.  That is what makes the pair
truly inverse: `promote c |> demote c` and `demote c |>
promote c` both restore the source cardinality (`demote_promote`,
`promote_demote`).  The content-identity stages (`assume`,
`completeness_check`) carry the gradings; every other operation resets them
to match its own output cardinality until its transport rule is mechanized.

**`promote cols`** promotes non-key column(s) into the key.  Content: the
named columns join the key.  Cardinality: derived from the gradings; a
`singletons` input stays `singletons` (`promote_functional`), and a `bag`
whose grading fits inside the grown key promotes to `singletons`.
Completeness: re-derived from the graded cardinality
(`docs/decisions/0035-completeness-cleared-by-demote.md`): `Complete` at
a graded `singletons` result, preserved at a `bag` result (a whole fiber
partitions into whole sub-fibers).  Tier A (`promote_splitSafe`).

**`demote cols`** drops key component(s) into the non-key part.
Content: the named key columns become ordinary columns.  Cardinality: derived
from the gradings; on a genuine coarsening no grading fits the retained key,
so per-key cardinality **grows** (the result is `bag` over the coarser key
unless a following `map_bags` reduces it), while an exact round trip
re-derives `singletons`.  Completeness: re-derived from the graded
cardinality, never demanded
(`docs/decisions/0023-completeness-consumed-by-the-reducer.md`,
`docs/decisions/0035-completeness-cleared-by-demote.md`): an exact round
trip is graded `singletons` and stays `Complete`, while a genuine
coarsening **clears** the fact, since merging fibers turns an absent fine
key into a gap inside a coarse fiber (ADR 0035's fiber-gap
counterexample).  The one exception is a **rectangular** coarsening: when
every demoted column is an `exhaustive` axis (section 6.6, ADR 0020) the
gaps it would create are ruled out by construction, so the fact survives
(`Mensura.demote_fiberCompleteWrt_of_exhaustive`, ADR 0035 decision 6).
A
`demote` with no downstream reducer is admitted on its own (a possibly
partial bag is an honest rekey); a reducer over the coarsened bag
establishes the fact after the `demote`.  Tier B on the lineage break
alone (`demote_not_preservesDisjoint`).

### `lookup` / `lookup_total` - join a fixed table

```
readings |> lookup machines (|k, r| r.machine)
```

Joins the current table against a fixed right table; the key-first lambda
maps the current row (key `k`, value `r`) to the right table's key.  Content: the
right table's columns are added.
Cardinality: preserved when the right table is functional (`singletons`); a
right table with several rows per key multiplies them in, raising the bound
to `bag`.  Totality:
`lookup` makes the added right columns **optional**, since an unmatched left
row is kept with them missing; `lookup_total` drops unmatched rows, so it adds no
optionality.  Completeness: preserved on the left.  Tier A
(`lookup_splitSafe`, `lookupTotal_splitSafe`).

### `split` / `union` - partition and merge

```
let (train, test) = data |> split |k| hash k < threshold
let full          = (train, test) |> union
```

**`split |k| pred`** routes each *entity* (each key) wholly to one side of a
pair according to a predicate over the key, never cutting a key's rows apart.
The two halves are disjoint by construction.  Content: unchanged on both sides.
Cardinality: unchanged.  Completeness: each side is complete over the keys it
keeps.  Tier A (`split_disjoint`; `union_split` shows `union` undoes it).

**`(a, b) |> union`** is the multiset union of two tables of the same schema at
each key.  It is **total**: it has no disjointness precondition, and it is
always split-safe and associative/commutative (`bind_comm`, `bind_assoc`).
Content: unchanged.  Cardinality: binding **disjoint** inputs preserves
`singletons`; binding **overlapping** inputs may push an entity above one row,
raising the bound to `bag`.  That lost guarantee is the only thing disjointness
buys; it is not required for the operation to be defined or safe.  Completeness:
the union is complete over a key iff both inputs are.  Tier A.

Disjointness itself (the precondition for *not* leaking across a split) is a
lineage-qualifier matter, tracked in `Qs`, not an algebra precondition on
`union`.  How that fact is established, propagated, demanded, and assumed is
specified in `08-lineage.md`.

### `unpivot` / `pivot` - reshape long and wide

```
wide |> unpivot metric reading    // long, keyed by (..., metric)
long |> pivot metric reading      // wide form
```

The ratified surface is `docs/decisions/0016-reshape-surface.md` as amended
by `docs/decisions/0020-reshape-as-a-true-inverse-pair.md`: `unpivot name
value` names the new key and value columns explicitly and folds **all**
attribute columns (excluding a column is upstream projection), and
`pivot name value` spreads an enum **key** column.  The pair is designed
to be truly inverse; feature coverage comes from composing with the other
primitives.

**`unpivot name value`** turns the value columns (which must share one
domain) into rows, spreading the column *name* into the key.  Content: the
names move into a new `enum` key column, the values into a single column.
**A missing cell yields no row**, so the value column is total by
construction.  Cardinality: preserved.  Completeness: establishes
`exhaustive(name)` exactly when every folded column is total.  Tier A
(`unpivotDrop_splitSafe`).

**`pivot name value`** is the inverse: it gathers, for each residual key,
the values indexed by the `name` key column into one wide row.  `name` in
attribute position is rejected (promote it with `promote` first).  It is
admissible exactly when the input is `singletons` with `value` as its only
attribute, and it consumes **no completeness fact**: an absent
(key, variant) row becomes a missing cell, and the spread columns are total
iff `exhaustive(name)` holds and the value column is total.  Not
split-invariant, so lineage is dropped.  Tier B (`pivot_not_splitInvariant`,
`pivot_total_of_exhaustive`).

The pair is mutually inverse on functional, minimal tables
(`pivot_unpivotDrop`, `unpivotDrop_pivot`): value-missing in the wide table
and row-absent in the long table carry the same information, so a sparse
long table round-trips as it is, with no discharge and no `assume`.

So `pivot` is where cardinality tracking pays off directly: it type-checks
only when each cell it spreads is known to hold at most one value, which
the long form's key discipline provides.

### `window` - replicate rows onto a time grid

```
readings |> window w taken_at (15.0 * si.minute) (5.0 * si.minute)
```

`window w p size stride` replicates each row into every window that
contains its point `p`, adding the window's start as a fresh key column
`w` carrying `p`'s domain (ADR 0037 decision 1).  Unlike every other
column argument in the algebra, `w` names a column the operation
*creates*, so it must not already exist; `p` must, and it stays wherever
it was, key or attribute.

**Window starts lie on the stride grid anchored at the domain's zero**
(the epoch for an `instant`, ADR 0036 decision 5), so placement is
deterministic with no declaration and no data dependence: a row with
point `p` lands in every window `w` with `w <= p < w + size`.  Tumbling
windows are not a second operation, they are `stride == size`.  When
`stride` divides `size` a row lands in exactly `size / stride` windows;
`stride > size` leaves gaps, and a row whose point falls in one lands in
no window at all, which is legal and occasionally wanted.

**A window containing no rows does not appear in the output.**  `window`
replicates rows, and with no row there is nothing to replicate, so an
empty window is represented by absence.  Materializing one is
rectangularization, not windowing (ADR 0038).

`size` and `stride` are const expressions of type `diff(domain(p))`
(ADR 0037 decision 3): `time[real]` for an `instant` point, `int` for an
`int` point, both positive and, for an instant, a whole number of
milliseconds.  Note `15.0 * si.minute` rather than `15 * si.minute`:
`int` and `real` do not mix (ADR 0014).  `date` points wait on
`diff(date)` (ADR 0036 decision 4), and count-based windows over a rank
need no special case, being windows over an `int` point.

Properties.  **Content**: `w` joins the key with `p`'s domain.
**Cardinality and gradings**: extended, not reset.  The replication is
injective on (input identity, `w`), so a `singletons` table at key `K`
is `singletons` at `K + {w}`, a `bag` stays a `bag`, and each grading `G`
becomes `G + {w}` (`window_functional`).  That extension is the reason
the fact is tracked: it keeps a downstream scan's tie-freedom derivable
inside a window fiber, so `window` then `demote p` then a scan needs no
`assume { arranged }`, exactly as it needs none without the window.
**Completeness and lineage**: no new rules, since the operation is
specified as a replicating `flat_map` followed by `promote w` and
transports facts as those two do.  Tier A by that same construction
(`window_splitSafe`).

The checker also records a **windowing fact**: `w` windows `p` at this
extent and stride, over a source whose intake contract it inherits.  It
is the sibling of `unpivot`'s `exhaustive(axis)`, established by the
operation's construction and consumed downstream by `closed`
(`13-registries.md`, ADR 0037 decision 4).

### `closed` - keep the windows that can no longer change

```
readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute)
         |> demote taken_at
         |> closed
         |> map_bags |k, b| (.peak = bag.max b.temperature)
```

`closed` drops every window that is still open and **establishes
completeness** at the current key on the survivors (ADR 0037 decision 4).
It is not a new qualifier: it is a new *establishment mechanism* for the
existing completeness fact, joining `completeness_check`/`assume`, the
registry rule, and the exhaustive-axis rule.

**Why the stage exists: two completenesses, not one.**  A `lateness`
contract gives a *bounded* fact, that nothing will ever arrive below
`watermark - lateness` in this grain.  A reducing `map_bags` demands an
*absolute* one, that this fiber is everything there will ever be
(ADR 0023).  The bounded fact becomes the absolute one only on a fiber
whose whole span lies below the bound, and the only fibers with a finite
span are windows, so `closed` is where the conversion happens.  There is
no such point without a window: `readings |> demote taken_at` is one bag
per machine whose `bag.max` tomorrow's reading can still raise, forever,
which is why no mechanism establishes completeness there and no contract,
however strict, would change that.  A zero bound (`13-registries.md`,
ADR 0041 decision 5) narrows the frontier to its minimum, one open window
per grain under tumbling windows, and does not remove it.

The establishment is mechanism-grade rather than a claim.  The windowing
fact supplies `size` and the source's `lateness` declaration supplies the
bound, both enforced at the intake, so "no row of this window can still
arrive" is a theorem about the intake rather than something the author
asserts.  A window survives when

```
w + size + lateness <= watermark
```

against the watermark of **that row's grain** (the declared key minus the
contracted column, ADR 0041), so a machine whose gateway is behind does
not have its windows closed by a faster machine's traffic.

Like `completeness_check`, `closed` is a checked stage rather than a new
algebra primitive; unlike it, `closed` **drops** rows instead of
asserting over them, because an open window is not an error, it is a
window whose answer does not exist yet.  Its absence from the output is
the honest representation, and it is what makes the design refresh-ready:
**closed windows are final** (`closedWindow_stable`), so rerunning after
further ingestion adds newly closed windows and never changes one already
emitted.

**`closed` against `assume { complete }`.**  The two spellings differ by
one line, and both type-check:

```mensura
view final_peaks {
  readings |> window w taken_at (1.0 * si.day) (1.0 * si.day)
           |> demote taken_at
           |> closed
           |> map_bags |k, b| (.peak = bag.max b.temperature)
}

view peaks_so_far {
  readings |> window w taken_at (1.0 * si.day) (1.0 * si.day)
           |> demote taken_at
           |> assume { complete }
           |> map_bags |k, b| (.peak = bag.max b.temperature)
}
```

With a machine's watermark at today 14:30, `final_peaks` omits today's row
and `peaks_so_far` emits it holding the peak of a half-finished day.  Run
both again at 18:00 over the same program and a grown registry:
`final_peaks` is byte-identical (today is still open, every earlier day
was already final), while `peaks_so_far` reports a different `.peak` for
the same key, with nothing in the row saying that it moved or that it will
move again.  Note what is and is not wrong there: every past row of
`peaks_so_far` is right, and only its newest row is a number the data does
not yet fix.  The claim is a claim about the *whole table*, so one open
window is enough to make it false, and a view row that changes between
runs is the temporal form of reading a value that is not determined yet.

So the two are not a strong mechanism and a weak one.  They are different
statements, one of which is false here, which is why the fallback named
below is for a source with no `lateness` contract to close against rather
than a way to keep the frontier row.

Four demands, each with a diagnostic naming its fix:

- a window column in the current key, carrying a live windowing fact;
- its point demoted **into the fiber**, since a window is only whole once
  its points are grouped into it;
- a `lateness` contract on that point.  Without one there is no
  mechanism, and the author falls back to `assume { complete }`, locally
  and visibly, as ever;
- the contract's **watermark grain** still in the key, since a row can
  only be measured against the producer it came from.

What the fact does and does not say: `closed` re-establishes exactly the
*arrival*-completeness a registry gives at its own key, transported to
the window key.  Whether a device's silence was a genuinely absent
reading or a reading lost before the intake is a deployment property
outside the type system, the same boundary `13-registries.md` draws.

What clears it: nothing new.  Tier A preserves it, and a further genuine
coarsening (`demote w`) clears it, ADR 0035 unchanged.

A machine that has stopped reporting never advances its own watermark, so
its last windows would never close on the data alone.  That is what the
declared **closure floor** is for (`13-registries.md`): it raises every
grain's effective watermark, so silence becomes observable rather than
indefinite.

**Reporting the frontier is a gap, not a policy.**  The dashboard that
wants "the peak so far today" is asking something legitimate that neither
spelling above answers: `closed` withholds the row and
`assume { complete }` misreports it.  What that query needs is `closed`'s
dual, a reduction over the *open* windows that carries the bound it was
computed over, so the row is visibly provisional and a consumer can see
how provisional.  That is establishable rather than claimed, since the
effective watermark is already read once per run and could be exposed as a
column, which is why the direction is recorded as an open question on
ADR 0037 rather than left to each program to improvise with an `assume`.
Until it lands, a provisional aggregate is spelled with a visible claim
and read as provisional by the human, not by the checker.

### `latest` - the newest row per group

```
vibrations |> assume { arranged } |> latest sampled_at
```

`latest p` keeps, per fiber, the row whose point `p` is maximal
(ADR 0037 decision 7).  It is a **reduction**, fiber-to-row, so the result
is `singletons` at the current key with `p` an ordinary total attribute,
and it sits on the reducing side of the line: it demands both facts the
ordered vocabulary demands, with no special case.

- **Tie-freedom** of `p`, because the argmax of a tied key is not
  determined.  Discharged from a grading where the shape allows it, or
  claimed with `assume { arranged }`, exactly as for a scan.
- **Completeness** at the current key, because a partial bag's "latest" is
  silently wrong, the same demand a reducing `map_bags` makes.

**`p` must already be an attribute.**  A key-borne point is rejected, with
the fix named: write the coarsening out, and the claim lands where it
bites.

```mensura
readings |> demote taken_at |> assume { complete } |> latest taken_at
```

Fusing the `demote` into the operation would leave the completeness demand
with nowhere to stand: a claim before it would sit at the fine key, which
a coarsening forfeits, and a claim after it would come too late for the
fold that needed it.

The two source shapes give the two honest cases.  Over an entity-keyed bag
registry nothing coarsens, so completeness holds by mechanism and only
tie-freedom is claimed; over a history keyed by `(entity, time)` the
grading discharges tie-freedom and the coarsening's completeness is
claimed.  After `closed`, both discharge by mechanism and neither claim is
needed.

## Completeness: establish, clear, consume

Two operations are Tier B: **`demote`** and **`pivot`**.  Both change
the key and drop the lineage fact, and that lineage break is all their Tier
means.  Neither demands completeness: `pivot`'s former obligation is
dissolved by ADR 0020 (an absent row becomes a missing cell, and
`exhaustive` decides the spread columns' totality instead), and
`demote`'s is moved by
`docs/decisions/0023-completeness-consumed-by-the-reducer.md` to the
operation whose result is silently wrong without it: a **reducing
`map_bags`** (the aggregate shape) consumes the fact.  Here *consume*
names the demand: the consuming operation is the one that is rejected
when the fact is absent, the one whose obligation an establishment step
discharges.  It does not by itself say what happens to the fact past the
operation; that is stated per operation (for the reducer, see the
`map_bags` entry and the open questions).  The fact itself
does not survive the coarsening
(`docs/decisions/0035-completeness-cleared-by-demote.md`): completeness
is about the *current* key, an absent fine key becomes a gap inside a
coarse fiber, so a genuine `demote` clears the qualifier and the
establishment step belongs after it.  A coarsening along `exhaustive`
axes is not genuine in that sense and keeps the fact (ADR 0035 decision
6), which is the one case where a reducer below the establishing key
still needs no discharge.  Over a `singletons` input the
reducer's obligation discharges trivially, so the ordinary aggregation
over a plain store stays ceremony-free.  The M1 surface for establishing
and consuming the fact is ratified in
`docs/decisions/0017-completeness-establish-consume.md` (as amended by
ADR 0023): M1 ships the `completeness_check` and `assume { complete }`
stages (with key-context asserts).  `registry`-by-mechanism completeness
landed with M4 (`13-registries.md`, ADR 0033 as amended by ADR 0035);
the `@complete_over` annotation stays deferred with its family.
Completeness is established in one of three ways:

- **mechanism**: a `registry` source is complete by construction at its
  **own declared key** (overview pillar 7, `13-registries.md`), so a
  reducer at that key needs no discharge at all.  The fact is established
  at the source, whatever the registry's cardinality: trivially on a
  `singletons` registry, contentfully on an `attr*` one, where it pins
  the full set of observations per entity.  It does not survive a
  coarsening: recording every observation received is not receiving
  every observation that happened, so a reduction below the registry's
  key discharges its own obligation there.

  ```
  events                            // an attr* registry keyed by Machine
  |> map_bags |k, b| (.n = #b.note)
  ```

- **`completeness_check { assert ... }`**, a pipe stage that *establishes* the
  fact locally.  It is an ordinary stage (`completeness_check` applied to a
  block of `assert` statements); conceptually it is an operation that
  guarantees completeness, and a later round may let a combination of asserting
  operations stand in for it.  Each `assert` is a boolean expression; together
  they witness that the partition is complete over the current key.  The fact
  must hold where the reducer runs, so the check is placed on the pipeline
  ahead of it and after any intervening `demote`, whose coarsening would
  forfeit it.

  ```
  enrollments
  |> demote course
  |> completeness_check { assert row_count open_offerings == 0 }
  |> map_bags |k, b| (.total_credits = bag.sum b.credits)
  ```

- **`@complete_over(col)`** on a source store, establishing the fact globally so
  no per-use check is needed.  This is an annotation; its surface lands with
  the annotation family (`@audited`, `@versioned`, ...), so this document names
  it but does not fix its grammar.

`assume { ... }` remains the escape hatch: it admits the reducer by fiat,
locally and visibly, when the obligation cannot be discharged.

## Cardinality, totality, and the type

Two orthogonal qualifiers are threaded by every operation above:
**cardinality** (table-scoped: how many rows share a key, `singletons` or
`bag`) and **totality** (column-scoped: whether a value is known or may be
missing, `Cell = Option`).
The rules that consume them are stated in `06-expressions.md`: a scalar
operator requires a **single known value** (`card 1` and not missing);
reduction brings a many-row bag down to one value (`fold` is the primitive,
`scan` is its ordered sibling, `#` counts, `in` tests membership, and the
named reductions and windows are `bag` and `series` module
bindings such as `bag.sum` and `series.cumsum`, with `mean` derived as
`bag.sum b.x / to_real (#b.x)`; ADR 0031); and a missing value propagates
through the scalar operators and is made known by the `??` discharge
(ADR 0039).  At the pipeline
level `pivot` is
admitted only at `singletons` (at most one row per key).
ADR 0010 settles the total/optional axis and its `?` marker; how
`singletons` / `bag` (and the derived `exhaustive`) are written in a type
stays the content/types document's job.  This document specifies how each operation
*changes* these properties and where one is *demanded*, and leans on inference
for the rest.

## Qualifiers and purity

Sampling, dependency, and lineage propagate through every operation by the rule
combinators of ADR 0004; this document does not re-state those rules per
operation.  Two qualifier-level preconditions are worth flagging because they
sit next to operations here: window-shaped `map_bags` returns need an ordering
from the dependency qualifier, and leak-free use of `union` is governed by the
lineage qualifier (disjointness, specified in `08-lineage.md`), not by the
algebra.

Every operation is pure and lazy, as everything in the expression sublanguage
is.  A pipeline is a description of a table; the hosting site
(`view`/`registry`/`store`/endpoint) decides when it runs.

## Worked examples

**Summarize by an attribute (Tier A throughout).**

```
readings
|> promote machine
|> map_bags |k, b| (.temp_mean = bag.sum b.temperature / to_real (#b.temperature), .temp_max = bag.max b.temperature)
```

`promote` adds `machine` to the key (content: key grows; cardinality and
completeness preserved); `map_bags` reduces each bag to one record, so the
result is **singletons** per `(…, machine)` key.  All Tier A, so it composes
safely; it type-checks.

**Coarsen the key, then reduce (cleared, established, consumed).**

```
enrollments
|> demote course
|> completeness_check { assert row_count open_offerings == 0 }
|> map_bags |k, b| (.total_credits = bag.sum b.credits)
```

`demote course` coarsens the key (dropping `course` makes the table `bag`
over `student`) and **forfeits** any completeness fact along the way
(ADR 0035); the check **establishes** the fact at the key the fold runs
over; the reducing `map_bags` **consumes** it and brings the table back
to **singletons** per student.  It type-checks because the obligation was
discharged where it bites.  Remove the check (and `@complete_over`, and
`assume`), or move it ahead of the `demote`, and the `map_bags` is
rejected; the `demote` alone would still be admitted (ADR 0023).

**Reindex round trip (key-graded cardinality).**

```
readings |> promote ts |> demote ts
readings |> demote channel |> promote channel
```

The gradings survive both moves, so either order restores the source
cardinality (ADR 0024): a `singletons` source comes back `singletons`
(no spurious bag, no vacuous completeness demand downstream) and a `bag`
store comes back a `bag`.  It type-checks against the source's own shape.

**Train/test split and re-merge (cardinality under `union`).**

```
let (train, test) = data |> split |k| hash k < threshold
let full          = (train, test) |> union
```

`split` yields a disjoint pair, each complete over the keys it keeps; binding
the disjoint pair preserves `singletons` and reconstructs `data` (`union_split`).
Binding two *overlapping* tables would instead yield `bag`, the documented
cost of dropping disjointness.  It type-checks.

## Forward references and open questions

- **Consolidated rules.**  The per-primitive rules above are collected, with the
  expression and completeness rules, in `09-typing-reference.md` (the M0
  freeze), which makes the four tracked properties (cardinality, totality,
  completeness, and disjointness via a lineage hierarchy) explicit in
  `Table<Qs, C>`.
- **Named sugar.**  `filter`, `mutate`, `select`, `reduce`,
  and `tagged_union`/`tagged_split` are sugar over the primitives above and get
  their own round.  The window functions have **landed** instead: `rank` and
  `cumsum` are bindings in the bundled `series` module over the `scan`
  primitive (ADR 0031 Decision 8).
- **Expression features the fuller surfaces need.**  Row-dropping and
  row-expanding `flat_map` are covered by the conditionals and collection literals
  of `06-expressions.md` (ADR 0015); bag-returning `map_bags` (windows) now
  has its ordering, named at the operator by a `scan`'s key argument rather
  than drawn from the dependency qualifier, since a fiber's order is
  established locally.
- **The cardinality-type notation.**  How `singletons` / `bag` (and the
  derived `exhaustive`) are written in a `Type` is the content/types
  document.  (The orthogonal total/optional axis and its `?` marker are
  settled in ADR 0010.)
- **Which `map_bags` bodies preserve completeness.**  The checker carries
  the completeness qualifier through `map_bags` (what the reducer consumes
  is the obligation, not the fact).  This is justified today because every
  expressible body emits at least one output row per present fiber, the
  non-emptying hypothesis of `fiberMap_exhaustive`, but no named lemma
  states the preservation.  A future body whose fiber action can empty a
  present fiber (a bag filter, a `top_k`, a conditional yielding an empty
  bag) would forfeit coverage, so when the body language grows such
  combinators the carry-through must either be restricted to non-emptying
  bodies and backed by a lemma, or replaced by the conservative
  alternative: drop the qualifier at `map_bags`, at the cost of a
  redundant `assume` where the fact is still live downstream (a reducer
  after a window-shaped `map_bags`, a `union` that needs both sides
  complete); a genuinely coarsening `demote` clears it anyway (ADR 0035).

  **`exhaustive` rides the same hypothesis and must be revisited in the
  same breath.**  `map_bags` carries it on exactly the non-emptying
  grounds above (`fiberMap_exhaustive`), so an emptying body forfeits
  both facts at once, not completeness alone.  The two differ only in
  their fallback: `exhaustive` has no `assume` form, so dropping it at
  `map_bags` costs a `pivot` its totality upgrade (section 6.6, ADR 0020)
  with no escape hatch, where completeness can be re-established by fiat.
  Whichever way the carry-through is resolved, resolve it for both.
- **`@complete_over` and other annotations.**  The annotation surface
  (`@audited`, `@versioned`, `@auto`, `@complete_over`) is its own document.
- **Hosting and streaming.**  `view` declarations that host pipelines are
  specified in `10-views.md`, and `registry` declarations in
  `13-registries.md`.  The remaining hosting sites (`transform`, endpoints)
  and the streaming operations (`sliding_window`, `latest`, reactive `on`
  blocks) extend this grammar and get their own sections.
