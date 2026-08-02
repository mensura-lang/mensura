# 0037: Streaming windows and window-closedness

## Status

Accepted.  The design half of M5's first slice ("Streaming and
reactive", `ROADMAP.md`), on top of the ordered primitives that landed
early with ADR 0029 Stage 2.  Rests on
`docs/decisions/0036-temporal-domains-and-torsor-arithmetic.md` for
extents.  Settles two questions flagged in earlier ADRs: whether a
prefix scan over a partial bag needs a completeness fact
(`docs/decisions/0029-fold-and-scan.md`, open questions) and the
contiguity question
(`docs/decisions/0035-completeness-cleared-by-demote.md`, "Tie-freedom
is not gap-freedom").  Amends the intake of
`docs/decisions/0034-typed-ingestion.md` with the `lateness` contract.
Registers one deliberate breakage: the fleet example's
`reading_trend` gains an `assume { complete }` (decision 5).

Out of scope, each with its destination: incremental refresh,
`refresh: on_change`, the changelog, the plan IR, and the DBSP
lowering (the M5 refresh slice, `docs/toolkit/04-processing-layer.md`;
the mergeability analysis sketched in `untracked/iiot.md` belongs
there); per-window sampling inference (the sampling-qualifier ADR,
slot held by `docs/language/09-typing-reference.md` section 13);
temporal referential integrity and the dependency qualifier (a later
slice extending `docs/language/08-lineage.md`); parameterized views;
and the `resample` surface (decision 6 settles the model only).
Throughout, `untracked/iiot.md` is cited as evidence of demand, not
as specification: its `sliding_window(size: size, stride: stride) by
machine` spelling uses named arguments and a `by` clause the LL(1)
grammar does not have, the same surface family ADR 0031 already
declined for `scan`.

## Context

The ordered half of the aggregate family is built: `scan` and
`prescan` over the closed combiner table, the `desc` marker, and the
`series` module (ADR 0031), gated by the arrangement structure in
`formal/Mensura/Arranged.lean`.  What M5 still owes is the window
half: a *window extent* (the shipped vocabulary is whole-prefix only),
and an answer to when a windowed reduction may trust its fiber.

Two recorded hand-offs converge exactly here.  ADR 0029 flagged that
a prefix scan reads every earlier row, so a missing early row
corrupts every later output, and left open whether `scan` sits on the
reducing side of ADR 0023's line despite being window-shaped.  ADR
0035 recorded that tie-freedom is not gap-freedom: `series.lag` over
readings at 10:00, 10:01, 10:04 reports 10:01's value at 10:04, well
typed and misleading, and named three candidate fixes (a dense marker
on the order key, a gap-aware vocabulary, an explicit resampling
stage).  The fleet example's `reading_trend` carries the same
hand-off in its comments.

Nothing window-related exists in the implementation: no tokens, no
operators, no scaffolding.  The processing layer is a batch
recompute, and this ADR's semantics must be coherent in that world;
the refresh slice swaps the engine, not the meaning.

## Decision

### 1.  One window primitive, not a sliding/tumbling pair

A single rekeying operation:

```mensura
data |> window w taken_at (15 * si.minute) (5 * si.minute)
```

`window w p size stride` replicates each row into every window that
contains its point `p`, adding the window's start as a fresh key
column `w` with `p`'s domain.  Window starts lie on the stride grid
anchored at the domain's zero (the Unix epoch for `datetime`, day
zero for `date`, zero for `int`): the starts are the integer
multiples of `stride`, and a row with point `p` lands in every window
`w` with `w <= p < w + size`.  The grid makes placement deterministic
with no declaration and no data dependence.

Tumbling windows are not a second operation: they are `stride ==
size`.  When `stride` divides `size` a row lands in exactly
`size / stride` windows; `stride > size` leaves gaps between windows
and a row whose point falls in a gap lands in none, which is legal
and occasionally wanted (periodic sampling).

The semantics are *specified as a derived form*: a replicating
`flat_map` (one output row per containing window, `w` computed from
`p`) followed by `promote w`.  Split-safety and the disjointness
rules therefore come from the composition of two operations the
calculus already proves, and `window` is Tier A by construction.  It
is nonetheless a builtin rather than a library binding, twice over:
the replication arity is data-dependent and ADR 0015's collection
literals are statically sized, so the body language cannot express
the expansion; and `w` and `p` are column names, which are not values
(the same reason `demote` is not a function).

### 2.  Typing through `Table<Qs, C>`

- **Content.**  `w` joins the key; its domain is `p`'s.  `p` itself
  is untouched (it stays wherever it was, key or attribute).
- **Cardinality.**  The replication is injective on (input identity,
  `w`): the same input row never lands twice in one window.  A
  `singletons` table at key `K` is `singletons` at `K + {w}`; a `bag`
  stays a `bag`.
- **Gradings.**  Extended, not reset: every tracked grading `G`
  extends to `G + {w}` by the same injectivity.  This is what keeps a
  downstream scan's tie-freedom derivable inside a window fiber: the
  fleet's `{machine_id, taken_at}` grading becomes
  `{machine_id, taken_at, w}`, so after `demote taken_at` the times
  are still unique within one `(machine, window)` bag and the
  obligation discharges with no ceremony, exactly as it does today
  without the window.
- **Completeness and lineage.**  No new rules.  As a derived form
  (`flat_map` then `promote w`) the operation transports facts
  exactly as those two operations already do under ADR 0024 and ADR
  0035; in the canonical program the completeness fact at the window
  key comes from `closed` (decision 4), not from transport.
- **The windowing fact.**  `window` records a checker-side fact: `w`
  windows `p` at extent `size`, stride `stride`, over a source whose
  intake contract (decision 4) it inherits.  It is the sibling of
  `unpivot`'s `exhaustive(axis)` (ADR 0020): established by the
  operation's construction, consumed downstream (`closed`), and reset
  conservatively by any operation that touches `w` or `p` or is not
  content-identity in ADR 0024's sense.

### 3.  Extents are torsor differences, known at compile time

`size` and `stride` are const expressions (compile-time values, ADR
0030) of type `diff(domain(p))` per ADR 0036: `time[real]` for a
`datetime` point (`15 * si.minute`), `int` days for a `date` point,
`int` for an `int` point.  Both must be positive.  The dimension
check is the ordinary quantity check, so a `size` in kelvin against a
`datetime` point is the same compile error as any other unit
mismatch, and count-based windows ("the last N rows" over a
`series.rank` key) need no special case: they are windows over an
`int` point with `int` extents.  Extents are const because the
checker must carry them in the windowing fact and `closed` must
compute `w + size` at a key it can reason about; a data-dependent
extent is a different feature (and nothing in the driving
application asks for it).

### 4.  Closedness: a mechanism that establishes completeness

Window-closedness is **not a new qualifier**.  It is a new
*establishment mechanism* for the existing completeness fact, at the
window-extended key, joining `completeness_check`/`assume` (ADR
0017), the registry-by-mechanism rule (ADR 0033), and the
exhaustive-axis rule (ADR 0035 decision 6).  Three parts:

**The `lateness` contract.**  A registry may declare a lateness bound
on an orderable temporal point column:

```mensura
registry readings {
  unit { Reading }
  attr { temperature: temperature[real] }
  lateness { taken_at: 10 * si.minute }
}
```

The bound is a const expression of type `diff(domain(column))`.  The
contract it states: once the intake's watermark has passed `t +
lateness`, no row with point `t` will ever be accepted.  The intake
*enforces* it, the way ADR 0033's fact is enforced by the append-only
sole intake rather than assumed: `mensura ingest` (and any future
transport, which per ADR 0034 is another caller of the same decoder)
rejects a batch containing a row whose point is older than
`watermark - lateness`, transactionally, like any other decode
failure.

**The watermark.**  The registry's high-water mark: the maximum point
value the intake has ever accepted on the contracted column,
maintained by the backend as registry metadata at `apply` time.
Under an append-only intake it equals the maximum value present, but
as metadata it is independent of downstream reads, O(1), and
well-defined for an empty registry (no watermark, every window
open).  At `mensura run` time the watermark is read once; a batch run
is therefore deterministic and pure (two runs over the same database
agree), and wall-clock time never enters the semantics.  The
watermark is global to the registry; a per-key variant is an open
question.

**The `closed` stage.**  Applied where the windowed table has been
demoted to its window key (the key contains a `w` carrying a live
windowing fact over `p`, and `p` is in the fiber):

```mensura
readings |> window w taken_at (15 * si.minute) (5 * si.minute)
         |> demote taken_at
         |> closed
         |> map_bags |k, b| (.peak = bag.max b.temperature)
```

`closed` drops every window that is still open (`w + size + lateness
> watermark`) and establishes `Complete` at the current key on the
survivors.  The establishment is mechanism-grade: the windowing fact
supplies `size` and the source contract supplies `lateness`, both
enforced, so "no row of this window can still arrive" is a theorem
about the intake, not a claim.  Like `completeness_check`, `closed`
is a checked stage, not a new algebra primitive; unlike
`completeness_check` it *drops* rows rather than asserting over
them, because an open window is not an error, it is a window whose
answer does not exist yet.  Its absence from the output is the
honest representation, and it is what makes the design refresh-ready
(below).  `closed` demands the windowing fact and the source
contract; without a `lateness` declaration there is no mechanism and
the stage is rejected (the author falls back to
`assume { complete }`, locally and visibly, as ever).

What the fact does and does not say: `closed` re-establishes exactly
the *arrival*-completeness the registry mechanism gives at its own
key (ADR 0033 decision 2), transported to the window key: every row
the intake will ever accept for this window is present.  Whether the
device's silence was a genuinely absent reading or a reading lost
before the intake is a deployment property outside the type system,
the same boundary ADR 0033 draws, and the docs must not oversell it.

**Finality.**  The invariant this buys, stated now and proved as this
slice's formal gate: *closed windows are final*.  Rerunning after
further ingestion adds newly closed windows and never changes a
previously emitted one (`closedWindow_stable`, decision 8).  The
refresh slice will lean on exactly this: maintaining a closed-window
view under appends is retraction-free.

What clears the fact: nothing new.  Tier A preserves it, a further
genuine coarsening (`demote w`) clears it, ADR 0035 unchanged.  No
new `assume` claim word is added: the fact being claimed is
completeness, and `assume { complete }` already says that.

### 5.  Scans demand completeness per combiner (settles ADR 0029)

The completeness demand of `scan` and `prescan` is a property of the
**combiner row**, not of the operation's output shape:

- Under the six fold-admitting combiners (`` `+` ``, `` `*` ``,
  `` `or` ``, `` `and` ``, `` `<<` ``, `` `>>` ``) a scan **demands**
  the same fiber-completeness fact a reducing `map_bags` demands
  (ADR 0023).
- Under the keep combiners (`` `<:` ``, `` `:>` ``) it demands
  nothing, as today.

The justification is already a theorem:
`Mensura.scanl_getLast_eq_foldBag` (`formal/Mensura/Arranged.lean`)
proves the inclusive scan's last entry *is* the fold.  A
fold-admitting scan provably contains the reduction, and every one of
its output rows is a fold over a prefix, silently wrong on a partial
bag in precisely ADR 0023's sense; exempting it would make
`` scan `+` `` a loophole that recomputes `` fold `+` `` without the
obligation, three lines from a `bag.max` that carries it.  The keep
combiners' outputs, by contrast, are claims about adjacency among
*present* rows ("the previous reading in this bag"), which a partial
bag represents honestly, the reading ADR 0035 and the fleet comments
already give `lag`.  So ADR 0029's flag resolves to **yes**: the
reducing/windowing distinction is not the aggregate-shape/
window-shape distinction.  The demand axis is "does any output row's
claim quantify over absent rows", and the closed combiner table makes
the line a column lookup.

Ripple, applied uniformly with no per-binding exemption:
`series.cumsum`, `series.running_min`, `series.running_max`, and
`series.rank` (scans at `+`, `<<`, `>>`, `+`) now demand the fact;
`series.lag`, `series.lead`, and `series.first_value` (prescan/scan
at `:>`, `<:`) do not.  A presence-relative `rank` remains
expressible, deliberately, under an `assume`.  This **breaks the
fleet example's `reading_trend`** by design: `running_max` over
`readings |> demote taken_at`, a bag whose completeness ADR 0035
cleared, is rejected until the view states the claim
(`assume { complete }`, mirroring `machine_temperature` three
declarations down, which demands the identical fact for the identical
number) or the view splits so `lag`/`lead` stay ceremony-free.  It
was incoherent for `bag.max` to carry the obligation while
`running_max`'s last row computed the same value without it; the
break is the fix.  The book's "a scan demands no completeness" prose
and the checker test named for it are reconciled in the
implementation slice.

### 6.  Contiguity (settles ADR 0035): gap-aware by construction

Of ADR 0035's three candidate fixes, this ADR adopts the
explicit-mechanism family and rejects the marker:

- **Interval windows are the gap-aware vocabulary.**  `window`
  selects rows by key interval, not by position: a 3-minute window
  over readings at 10:00, 10:01, 10:04 contains what it contains, and
  a rate computed per window is a rate over the window's stated
  extent.  The 10:01-versus-10:03 hazard is a *positional*-vocabulary
  hazard, and the idiomatic fix is to stop writing rates positionally:
  difference per window, not `lag` per row.
- **Positional vocabulary keeps present-rows semantics, with no
  contiguity obligation.**  Densely-indexed and irregular series are
  both legal and no checker demand can tell them apart (ADR 0035's
  point); `lag` keeps meaning "the previous reading in this bag".
- **The model for positional-as-temporal is an explicit `resample`
  stage**, deferred: a rectangularizing mechanism that establishes
  contiguity by construction the way `completeness_check` establishes
  completeness, fixing row presence (one row per `(entity, step)`,
  the cardinality axis) and pushing missingness into the value axis
  (unfilled columns become optional per ADR 0010; a fill policy
  narrows them).  This ADR settles that contiguity is *established by
  a mechanism, never claimed*; the stage's surface and fill policies
  are a follow-up with its own consumer.
- **The dense/regular marker is rejected**: it is a claim about the
  data with no establishing mechanism, the exact shape ADR 0035's
  audit refused, and nothing in this slice would consume it that
  `resample` will not discharge by construction.

### 7.  `latest` is a reduction, shipped as a builtin

`latest p` keeps, per fiber at the key without `p`, the row with the
maximal point `p`:

```mensura
view newest_reading {
  readings |> latest taken_at |> assume { complete }   // see below
}
```

Specification: if `p` is a key column, `latest p` is `demote p`
followed by the argmax reduction; if `p` is already an attribute over
a bag, the reduction applies to the existing fibers.  Formally the
kept row is `getLast (arrange p fiber)`, deterministic by
`Mensura.IsArrangement.unique` given tie-freedom.  The result is
`singletons` at the reduced key, with `p` an ordinary total
attribute.

`latest` sits on the *reducing* side and demands both facts, with no
special case: **tie-freedom** of `p` in the fiber (tier 1 from a
grading, tier 3 `assume { arranged }`, exactly as `scan`), because
the argmax of a tied key is not determined; and **completeness** at
the reduced key (ADR 0023), because a partial bag's "latest" is
silently wrong.  The demands discharge or not by the existing rules,
and the two source declarations of the fleet make the two honest
cases:

- `readings |> latest taken_at`: tie-freedom derives from the
  surviving grading, but the internal `demote` clears the registry's
  fine-key completeness (ADR 0035), so the view states
  `assume { complete }`, the same coverage claim
  `machine_temperature` makes for the same coarsening.
- `vibrations |> assume { arranged } |> latest sampled_at`:
  completeness holds by mechanism at the registry's own key (ADR
  0033, contentful on an `attr*` registry; nothing coarsens), but a
  bag registry seeds no grading (ADR 0022), so tie-freedom of
  `sampled_at` is the claimed fact.
- After `closed`, both discharge from decision 4's establishment
  inside each window fiber.

`latest` is a builtin for the reasons `window` is: the self-referring
argmax is not expressible as a library binding until the value-tuple
type ADR 0031 left open exists, and column names are not values.  ADR
0031's suspicion that "`latest` looks scan-derivable" resolves
negative: a scan is fiber-to-bag and `latest` is fiber-to-row, and
the keep-combiner route cannot return the *row*, only one column of
it.

### 8.  Grammar and formal gates

**Grammar.**  The three operations add zero productions: `window`,
`closed`, and `latest` are pipeline operations in the existing
application grammar, with bare column names as juxtaposed arguments
(`demote course`, `unpivot sensor reading` precedent) and
parenthesized const expressions for extents.  The one real grammar
change is declaration-level and LL(1)-trivial: `store_block` gains a
third alternative,

```
store_block    = attr_block | domain_block | lateness_block ;
lateness_block = "lateness" "{" { lateness_entry } "}" ;
lateness_entry = ident ":" expression ;
```

with `lateness` a contextual word like `attr` and `domain` (the lexer
stays keyword-free).  `04-grammar.md`'s promised "streaming
operations" section reduces to the op names in its operation list.

**Formal gates** (ADR 0021 idiom; lemmas land before the
implementation ships the ops):

1. `window` is defined in `formal/` as the composition of the
   replicating fiber map and the key extension, so split-safety and
   disjointness come from the existing composition lemmas; one new
   statement, the grading extension (injectivity on
   (identity, `w`)), gates decision 2's "extended, not reset".
2. `closedWindow_stable`: for an append-only extension in which every
   added row's point exceeds `watermark - lateness`, the restriction
   to any window with `w + size + lateness <= watermark` is
   unchanged.  A small multiset-filter lemma; it is the soundness of
   decision 4's establishment given the contract, and the finality
   invariant the refresh slice inherits.
3. No new propagation claims anywhere else: decision 5 cites the
   existing `scanl_getLast_eq_foldBag` family, and decision 7 is a
   definition plus the existing `IsArrangement.unique`.  Demands are
   conservative and need no proof.

## Consequences

Positive:

- The two flagged questions close with less machinery than either
  anticipated: no contiguity qualifier, no closedness qualifier, no
  new `assume` claim, no new grammar beyond one declaration block.
  The frozen `Qs` row of `09-typing-reference.md` is untouched.
- The canonical M5 program (registry with `lateness`, `window`,
  `demote`, `closed`, reducing `map_bags`) carries **no `assume` at
  all**: every fact is established by a mechanism and consumed by a
  demand, which is the language's whole pitch.
- The combiner table earns its keep again: the scan demand is a
  column lookup, as ADR 0031 predicted gated semantics would be.
- Batch semantics are deterministic and refresh-ready: closed
  windows are final, so `on_change` maintenance of these views is
  retraction-free by construction.

Deliberate breakage:

- `reading_trend` (decision 5).  The migration is one `assume` line
  or a view split, made in the implementation slice with the
  example's comments rewritten to teach the new line.

Implementation (the M5 windows slice, not this ADR):

- `mensura-syntax`: the `lateness` block.
- `mensura-types`: `window`/`closed`/`latest` in `pipe_check`; the
  windowing fact; the grading extension; the per-combiner demand in
  `expr_check` (`type_scan`); the contract on the resolved schema.
- `mensura-runtime`: evaluation of the three ops; the high-water-mark
  metadata and its maintenance in `apply`; `lateness` rejection in
  the decoder path; ADR 0036's `datetime` alongside.
- Corpus and examples: accept/reject cases per the sketches above;
  the fleet example gains the windowed view and the `latest` views,
  migrates `taken_at` to `datetime` (ADR 0036), and takes the
  `reading_trend` fix; the book's scan prose and the
  `a_scan_is_the_window_shape_and_demands_no_completeness` test are
  reconciled.
- Docs: forward-reference edits to `04-grammar.md`,
  `07-pipelines.md`, `09-typing-reference.md`, `10-views.md`,
  `13-registries.md`, `docs/toolkit/05-ingestion.md`, and the
  annotations closing the flagged questions in ADRs 0029 and 0035;
  `ROADMAP.md`'s M5 entry updates.

## Alternatives considered

1. **`sliding_window` and `tumbling` as two intrinsics** (the iiot
   sketch's shape).  Rejected: one is the other at `stride == size`,
   and the sketch's named-argument `by`-clause surface is not LL(1)
   and was already declined for `scan` by ADR 0031.
2. **Windows as a pure library over `flat_map`.**  Rejected as
   surface (the expansion needs data-dependent collection
   construction the body language lacks, a bigger addition than one
   op) but adopted as the semantic model, which is what makes the
   formal gate compositional.
3. **Bounded scans** (extent arguments on `scan`/`prescan`).
   Rejected: conflates ordering with extent, forces an extent slot
   onto every `series` binding, and windows per column where the
   language's unit of grouping is the key.
4. **`assume { closed }` as a new claim word.**  Rejected: the fact
   is completeness; multiplying claim spellings spends ADR 0035's
   audit discipline for no new meaning.
5. **`lateness` as an argument of `closed`.**  Rejected (owner
   decision): a stage-local bound is unverified, so the establishment
   would be claim-grade; declared on the registry it is enforced by
   the sole intake and the mechanism argument goes through.
6. **A wall-clock watermark.**  Rejected for this slice: it makes
   `mensura run` non-reproducible.  Revisit with refresh, where a
   clock is native.
7. **On the scan demand**: *no demand* (status quo) leaves the
   fold-via-scan loophole and lets `cumsum` total a sample silently;
   *all scans demand* charges `lag`/`lead` for a fact their
   present-rows claim does not rest on.  Both rejected for the
   per-combiner line (owner decision).
8. **A `dense`/`regular` marker on order keys.**  Rejected, decision
   6.
9. **Gap-aware `lag`** (step-based rather than positional).
   Rejected: it rewrites the shipped vocabulary's semantics; interval
   windows already give the temporal reading, and `resample` will
   give the positional one honestly.
10. **`completeness_check { assert window_closed(...) }`** (the iiot
    sketch's spelling).  Rejected: an open window is not an
    assertion failure, it is a row that must not exist yet; the
    filter and the establishment are inseparable, so closedness is a
    stage, not an assert.

## Open questions

- **Per-key watermarks.**  The global watermark plus ingest
  enforcement is sound but lossy for an offline entity: its buffered
  data older than `watermark - lateness` is rejected at the intake.
  Whether a per-key watermark (never closing an absent entity's
  windows) is an opt-in, and where the loss policy belongs, is
  deferred to the refresh/serving slices where the operational story
  lives.
- **Grid origin.**  Window starts anchor at the domain zero.  If a
  consumer needs aligned-but-offset grids (business days, fiscal
  weeks), an explicit origin argument is the natural extension.
- **`resample`.**  Decision 6 settles the model; the stage's surface,
  its fill policies, and its interaction with optionality narrowing
  await a consumer.
- **Parameterized views.**  The iiot sketch's
  `view RULTrainingWindows(size, stride)` wants extents as view
  parameters; no parameter surface exists on views, and the const
  machinery (ADR 0030) is the likely substrate when it is designed.
- **Windows over `attr*` sources.**  A bag registry seeds no grading,
  so ordered vocabulary inside its window fibers rides on
  `assume { arranged }`; whether the declaration should be able to
  state a per-entity event-time uniqueness fact (giving bags a
  grading) is ADR 0024's `ReadingBack` question, sharpened.

## Forward references

- `docs/decisions/0036-temporal-domains-and-torsor-arithmetic.md`
  (extents and the `datetime` point domain).
- `docs/decisions/0029-fold-and-scan.md` and
  `docs/decisions/0031-fold-and-scan-primitives.md` (the combiner
  table; the prefix-scan flag closes per decision 5).
- `docs/decisions/0035-completeness-cleared-by-demote.md` (the
  clearing model this ADR leaves untouched; the contiguity flag
  closes per decision 6).
- `docs/decisions/0033-registry-declarations.md` and
  `docs/decisions/0034-typed-ingestion.md` (the mechanism argument
  and the intake the `lateness` contract extends).
- `docs/decisions/0017-completeness-establish-consume.md` and
  `docs/decisions/0023-completeness-consumed-by-the-reducer.md` (the
  establish/consume model `closed` joins).
- `docs/decisions/0024-key-moves-as-a-true-inverse-pair.md`
  (gradings; decision 2 extends them through `window`).
- `docs/toolkit/04-processing-layer.md` (the refresh slice that
  inherits the finality invariant).
- `formal/Mensura/Arranged.lean` (`scanl_getLast_eq_foldBag`,
  `IsArrangement.unique`; home of the new `closedWindow_stable`).
