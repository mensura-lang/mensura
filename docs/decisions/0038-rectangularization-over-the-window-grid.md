# 0038: Rectangularization over the window grid

## Status

Accepted.  Takes the window-grid case out of the `resample` stage that
`docs/decisions/0037-streaming-windows-and-closedness.md` decision 6
defers, and ships it as a narrow mechanism with a consumer.  Leaves
the general case (an arbitrary step over a raw order key) deferred
where 0037 left it, for the reason decision 5 below gives.

Rests on ADR 0037 (the grid, `closed`, and the contiguity model),
ADR 0036 (the grid origin and point arithmetic), ADR 0031 (the
combiner table's identity column, which supplies every fill value),
ADR 0029 decision 4 (the guarantee decision 1 preserves), and
ADR 0010 (optional columns).  Honours ADR 0021: decision 7 supplies
the theorems for the typing rules of decisions 2 and 4.

**Implemented** (2026-08-22), as the last unimplemented ADR of M5's
window slice.  This was the follow-up 0037 decision 6 promised "with its
own consumer"; the consumer arrived as soon as anyone asked how many
intervals a machine went silent, and the two things that had been
missing arrived with it: ADR 0039's lifted operators, without which a
`T?` fill column would have been serve-only, and ADR 0041's closure
floor, without which the never-reporting machine of the worked example
could not have had closed windows at all.

Three things the implementation had to add, none of them changing a
decision:

- **the combiner each column reduced at** is recorded by the reducing
  `map_bags`, where the field's defining expression is in hand.  The
  ADR says the recognition needs no provenance model, which is true, and
  true only because it happens there: a stage downstream sees columns and
  cannot ask what produced one;
- **a closedness flag on the windowing fact**, since decision 3 makes
  closedness the upper bound and completeness alone is too loose a proxy
  for it (an `assume { complete }` would pass);
- **a sharpening of `closed`'s own demand.**  Its rule was "the point is
  not in the key"; because the grid facts now cross a reducing
  `map_bags`, the rule is "the point is still a column", which is what it
  always meant.  Closing a grid after the reduction that consumed its
  points is now rejected rather than vacuously accepted.

## Context

ADR 0037 decision 1 states that a window containing no rows does not
appear in the output: `window` replicates rows, and with no row there
is nothing to replicate.  Absence is the representation.  That is
correct for the primitive and wrong for three questions users
actually have:

- **Counting silence.**  "How many 15-minute intervals did machine M
  report nothing?" is unanswerable, because the intervals in question
  are not rows.  Worse, it fails quietly: `count` never yields zero
  in a windowed view, so a filter meant to find sparse windows omits
  the sparsest ones.
- **Positional-as-temporal, one level up.**  ADR 0037 decision 6
  recommends "difference per window, not `lag` per row" as the fix
  for ADR 0035's positional hazard.  But a scan *across* windows
  meets the same hazard again: `lag` over a bag ordered by `w` means
  "the previous non-empty window", not "the previous `stride`".  A
  machine silent from 10:15 to 11:00 puts `w = 10:00` and `w = 11:00`
  adjacent in the bag.
- **Serving a rectangle.**  Anything that renders a series (a chart,
  an export, a fixed-shape feature vector for the iiot sketch's
  training windows) wants one row per grid slot, with the gaps
  visible as gaps rather than as missing rows.

ADR 0037 decision 6 already fixes the *model*: contiguity is
established by a mechanism, never claimed, and the mechanism
rectangularizes: fixing row presence on the cardinality axis and
pushing missingness onto the value axis.  What it defers is the
stage.  This ADR ships the stage for the one grid where the deferral's
hard part is already solved.

**Why the window grid is the tractable case.**  ADR 0035 found that a
densely-indexed and an irregular series are indistinguishable in
principle, which is why no checker demand can tell them apart and why
ADR 0037 decision 6 imposes no contiguity obligation on `lag`.  That
finding is about a raw order key.  It does not hold of `w`: the
checker knows `stride`, ADR 0036 decision 5 fixes the origin, and
closedness bounds the grid above.  "Are these `w` values a contiguous
run?" is a finite question with an answer.  The window grid is the
only place in the language today where contiguity is decidable, which
is what makes a mechanism cheap here and expensive everywhere else.

## Decision

### 1.  Rectangularization completes rows after the reduction, never bags before it

The tempting design is to materialize the empty fibers (one bag per
grid slot, empty where nothing arrived) and let the reduction handle
them.  **Rejected**, because ADR 0029 decision 4 guarantees the
opposite and the guarantee is load-bearing:

> `docs/language/07-pipelines.md` guarantees that `map_bags` skips
> empty bags, so the lambda always sees a non-empty bag.

That guarantee is why a seedless fold is total, why `min` and `max`
work at all, and why 0029's accumulator `Option` never reaches a
surface type.  Manufacturing empty bags would break all three at
once: every reducing lambda would have to be total on the empty bag,
and the optionality 0029 deliberately confined to the executor would
surface everywhere.

So the stage runs **after** the reduction, on reduced rows, and adds
the grid slots that produced none:

```mensura
readings |> window w taken_at (15 * si.minute) (15 * si.minute)
         |> demote taken_at
         |> closed
         |> map_bags |k, b| (.n = count b.temperature,
                             .peak = bag.max b.temperature)
         |> dense w machines activated
```

There is never an empty bag anywhere in the pipeline.  `map_bags`
sees exactly the fibers it sees today, and `dense` adds rows to its
output.

This inverts the mechanism from the obvious one while keeping its
conclusion.  The conclusion that matters is that **a filled row is a
reduced row, not a fabricated observation**: `n` is `0` because zero
readings were reduced, not because a placeholder reading was invented
and counted.  Emitting a synthetic input row with unfilled columns
would make `count` report `1` for an interval in which nothing
happened, which is a lie about the cardinality axis dressed up as
missingness on the value axis.

### 2.  Fill values come from the combiner's identity; the rest go optional

ADR 0031 decision 6's identity column is not merely available for
this, it was written for it:

> An **identity is written**: the machinery fabricates it into
> results, as the empty window's answer, `prescan`'s first output,
> and the parallel shard's base, so an identity must be the
> *genuinely true answer of the empty case*, storable and
> arithmetic-safe as data.

So the fill is a column-by-column lookup in a table that already
exists, read down the identity column instead of the admission
column:

| combiner       | identity        | filled row gets | column type |
| -------------- | --------------- | --------------- | ----------- |
| `+`            | `0`             | `0`             | total       |
| `*`            | `1`             | `1`             | total       |
| `or`, `and`    | `false`, `true` | that value      | total       |
| `<<`, `>>`     | none            | absent          | optional    |
| `<:`, `:>`     | none            | absent          | optional    |

`count` is a fold at `+` over ones (ADR 0031 decision 3), so it fills
with `0`, which is the answer users are asking for when they ask this
question at all.

**The typing consequence is real and is the point.**  A column
produced by a no-identity combiner becomes optional (ADR 0010) in a
rectangularized view: `bag.max b.temperature` is
`temperature[real]` today and `temperature[real]?` downstream of
`dense`.  This is ADR 0037 decision 6's "pushing missingness into the
value axis" made concrete, and it is what distinguishes an interval
with no readings from an interval whose maximum happened to be low.
0031's reasoning transfers unchanged: there is no true minimum of
nothing, so absence is the honest answer and a sentinel would not be.

**The lookup requires a column the table can answer for.**  A value
column fills from the identity exactly when its defining expression
is a single fold application at an identity-carrying combiner,
written directly or through the bundled bindings that resolve to one
(`bag.sum`, `count`, `#b`).  Every other column goes optional,
compound expressions included.  The fleet's own `sensor_avg` is the
worked non-example: `.average = (bag.sum b.reading) / to_real(#b)`
is two folds joined by a division, no single combiner produced it,
and identity-filling its components would compute `0 / 0.0`.
Absence is the true answer: an empty interval has no mean.

**The decomposed form is legal and has a trap worth naming.**  The
mergeable components travel honestly through `dense`: a view that
materializes `.sum = bag.sum b.reading` and `.n = #b` fills both
with `0`, and both filled values are true.  What must not follow is
recombining them downstream, a `flat_map` computing
`.avg = r.sum / to_real(r.n)`: the expression is well-typed, and on
every filled row it divides zero by zero, which `real` division
answers with a silent `NaN` rather than an error.  Note ADR 0039's
lifted operators do not save this form: both filled components are
*total* zeros, so nothing is optional and nothing propagates.  A
ratio over the dense grid is computed upstream of `dense`, where
this rule sends it honestly optional, absent exactly on the empty
windows and consumable through ADR 0039's `??`, or by the consumer
of the served rectangle, which sees `.n = 0` and knows the mean does
not exist.  Recorded so the idiom is taught rather than discovered.

A fill policy that narrows an optional column back to total
(carry-forward being the obvious one) is deferred to the open
questions.  It is a strictly additive surface and it should be
designed against a consumer, not guessed.

### 3.  The upper bound is closedness; the other two inputs are given

Four things determine which rows exist after `dense`.  Two are
already fixed, and two are policy that must be supplied and must
never be inferred:

- **Stride and origin: fixed.**  From the `window` declaration and
  ADR 0036 decision 5.
- **Upper bound: fixed, by closedness.**  The grid is completed only
  as far as `closed` reaches.  Filling past the watermark would
  fabricate confirmed-empty intervals for a future that has not
  happened, which is the one error this stage must not make.  It
  therefore runs after `closed` and inherits its bound, and `closed`
  needs no change.
- **Population: given.**  Which entities should have windows (every
  row in the store, only commissioned ones, only those that ever
  reported) is policy.  It cannot come from the windowed bag, which
  by construction knows nothing of an entity that never sent a row.
  It comes from the store the registry's unit references, named in
  the stage (the `machines` argument above).
- **Per-entity lower bound: given.**  The grid runs back to the
  domain origin.  Something must say where each entity's history
  starts (the `activated` argument above), and the natural answer is
  a column on the population store.  The bound aligns to the grid as
  the **first full window**: the entity's first slot is the smallest
  grid multiple at or above the bound.  The slot containing the
  bound is excluded deliberately, because part of it precedes the
  entity's history: a `0` filled into it would read as silence where
  the truth is that the entity did not yet exist for part of the
  slot, which is decision 1's fabrication error returning on the
  time axis.

Inferring either of the last two is the ADR 0034 repair pattern: a
heuristic that is right until it is silently, structurally wrong.
Taking the earliest observed `w` as the lower bound, in particular,
would make a sensor that was offline on day one look like a sensor
that did not exist yet.

### 4.  `dense` establishes completeness, and the fact survives `demote w`

The motivating query is one step past the fill.  "How many intervals
was machine M silent" reduces the dense grid at the machine key (the
`sensor_health` view is in the worked example below):

```mensura
view silence_per_machine {
  sensor_health |> demote w
                |> map_bags |k, b|
                     (.silent = fold `+` (|r| if r.n == 0 then 1 else 0) b)
}
```

The reducing fold demands completeness at `{machine_id}` (ADR 0023),
and `demote w` is a genuine coarsening, which clears the fact
(ADR 0035).  Without an establishment, the view would carry an
`assume { complete }` that is true *by construction of `dense`*: the
mechanism would do the work and the program would still state a
claim, which is the outcome the establish/consume model (ADR 0017)
exists to avoid.  So the stage records what it built:

- **At its own key**, `dense` establishes `Complete`: the grid
  between the bounds of decision 3 is materialized, so every row the
  ideal rectangle has is present.  The establishment is
  mechanism-grade; the mechanism is the grid enumeration itself.
- **A rectangularity fact** is recorded beside it, the sibling of
  ADR 0037's windowing fact: per residual key, the `w` values are
  exactly the grid between that entity's lower bound and the closed
  upper bound.  One rule consumes it: a subsequent `demote w`
  re-derives `Complete` at the coarsened key, because the coarse
  fiber is the whole grid as of the closed bound.  This is the one
  place a genuinely coarsening demote re-establishes completeness
  from a checked fact rather than clearing it, and it is sound only
  where ADR 0037 decision 6 says contiguity is decidable: stride and
  origin are known, and closedness bounds the grid above.  Like the
  windowing fact, it is reset conservatively by any operation that
  touches `w` or is not content-identity in ADR 0024's sense.

An earlier draft established a separate `Contiguous` fact and struck
it for lacking a consumer.  The consumer is the query above, and what
it demands turns out to be completeness itself, so no new fact name
is needed: rectangularity is bookkeeping the checker keeps so the
existing fact survives the one key move the idiom requires.

The count is honest about time in the same way `closed` is: it counts
silent intervals *as of the closed bound*, and because filled rows
are final (decision 7, item 3), reruns only ever grow it.

A filtered spelling (a `flat_map` keeping the `r.n == 0` rows, then
`demote w`, then `#b`) computes the same number but threads the fact
through a row filter, which needs a completeness-transport rule for
per-row filters that no ADR states; it is recorded as an open
question rather than smuggled in here, and the in-fold spelling above
does not need it.

### 5.  Scope: the window grid only

`dense` applies to a column produced by `window`, where stride and
origin are known statically.  It is not the general `resample`, and
ADR 0037 decision 6's deferral of that stage stands.

The reason is decision-theoretic, not incremental.  Over a raw order
key, ADR 0035's finding applies: no mechanism can distinguish a
series that is dense from one that is irregular, so "complete the
grid" has no well-defined meaning without a step the user asserts,
and an asserted step is a claim, which is the thing ADR 0035's audit
refused.  Over a window grid the step is a compile-time constant that
the program already wrote down.  The two cases differ in kind.

If general `resample` ever lands, it should be a different stage
with an explicit step argument, and this one should remain as its
specialization rather than being folded in.

### 6.  Grammar

Illustrative, and the naming is an open question:

```
dense <window-column> <store> <column>
```

Three juxtaposed arguments and no keywords, matching the application
grammar of every other pipeline operation (`demote course`, `unpivot
sensor reading`, `window w taken_at (...) (...)`): the arity is
fixed, so bare juxtaposition is unambiguous, and an earlier draft's
`dense w over machines from activated` paid two prose words no other
operation pays.

The `dense` spelling is chosen deliberately against ADR 0037 decision
6's rejection of "the dense/regular marker", on the grounds that the
intuition was always right and only the epistemic status was wrong:
this is the same idea, earned by a mechanism instead of asserted.
Reviewers who find that too cute have a fair point, and ADR 0025's
discipline would probably prefer two distinct words for two distinct
things.  Alternatives are recorded below.

### 7.  Formal backing

Per ADR 0021, decisions 2 and 4 are checker-visible rules (the
optionality narrowing and the completeness re-derivation across
`demote w`), so they ship with theorems.
`formal/Mensura/Window/Dense.lean`:

1. **Agreement with the ideal, per column.**  For a column that is a
   single fold at an identity-carrying combiner, filling after
   reduction equals reducing over the ideal completed grid.  This is
   the theorem that licenses decision 1's inversion: the cheap order
   of operations computes the expensive one's answer.  For a
   no-identity fold the column is absent, which is the same statement
   in `Option`.  Compound columns sit outside the statement: their
   ideal-grid reading does not exist, because ADR 0029 decision 4 is
   precisely the guarantee that the lambda never faces the empty
   bag.
2. **Idempotence.**  `dense` twice is `dense` once, given the same
   population and bounds.
3. **Stability under append.**  Extending an append-only bag adds
   rows for newly closed slots and changes no existing filled row.
   This is ADR 0037's `closedWindow_stable` carried across the fill,
   and it is what makes a rectangularized view safe to serve
   incrementally.
4. **Re-derivation across the demote.**  After `dense`, the fiber at
   the coarsened key (the window column demoted) is exactly the grid
   between the entity's bound and the closed bound, so the coarse
   bag is complete with respect to the ideal rectangle.  This is the
   theorem behind decision 4's one exception to ADR 0035's clearing
   rule, and it rests on the same grid decidability as item 1.

Item 3 depends on ADR 0036 decision 9's order-compatibility lemma by
way of `closedWindow_stable`, so the dependency chain is
`Torsor.lean` → 0037's window lemmas → this module.

## Worked example

The plant of ADR 0037's worked example.  The `machines` store gains
one column:

```mensura
store machines : Commissioned {
  unit { Machine }
  attr {
    commissioned: date       // civil: the paperwork date
    activated: instant       // absolute: telemetry came online
    status: MachineStatus
    last_service: date?
  }
}
```

`activated` is the moment the machine's sensor first came online, a
machine-generated absolute event and honestly an `instant`.  It is
not a converted `commissioned`: the commissioning date is civil
paperwork, its conversion to an instant is zone-dependent and
deferred (ADR 0036), and "when should telemetry exist" was never a
question about paperwork.

The sensor-health view completes the grid:

```mensura
view sensor_health {
  readings |> window w taken_at (15 * si.minute) (15 * si.minute)
           |> demote taken_at
           |> closed
           |> map_bags |k, b| (.n    = count b.temperature,
                               .peak = bag.max b.temperature)
           |> dense w machines activated
}
```

The pipeline is ADR 0037's canonical program plus the final stage.
`dense w machines activated` reads: complete the `w` grid, one row
per machine in `machines` per slot, from that machine's `activated`
bound up to the closed bound.  Machine `M-19`, whose sensor died
before it ever reported, is why the population cannot be inferred:
no windowed bag knows it exists.  With `activated = 06:03:22Z` and
the watermark at `10:31:12` (so closed windows run through
`w = 10:00`, ADR 0037's numbers), `M-19` gets sixteen rows,
`w = 06:15` through `w = 10:00`: the first *full* window, not the
`06:00` slot that partly precedes activation (decision 3).  Every
row is a reduced row: `.n` fills with `0` from `count`'s identity,
true because zero readings were reduced, and `.peak` has no identity
to fill from, so the column is `temperature[real]?` view-wide and
absent on every filled row.  An interval with no readings has no
peak, and absence now distinguishes it from a cold one.

The silence count of decision 4 then reduces the rectangle with no
`assume` anywhere: `dense` established completeness, `demote w`
re-derives it at the machine key from the rectangularity fact, and
the fold's demand discharges.  Sixteen silent slots for `M-19` is
the row this ADR exists to make expressible.

## Consequences

Positive:

- "How many intervals were silent" becomes expressible with no
  `assume`: decision 4's establishment survives the one key move the
  query needs, and the `count`-never-zero trap closes.
- Cross-window positional vocabulary becomes safe to write, because
  after `dense` the previous present row *is* the previous grid slot.
- The optionality lands where the information actually is: an absent
  `peak` says no readings, a present low `peak` says cold readings,
  and today those are the same row shape.
- The identity column of ADR 0031 gets the use it was specified for,
  which is some evidence that table's axes were chosen well.
- ADR 0029 decision 4's empty-bag guarantee survives intact, so no
  reducing lambda changes and no surface type gains optionality it
  did not already have.

Negative:

- A rectangularized view's no-identity columns change type.  This is
  correct and it is a breaking change for any view that adds `dense`
  downstream of an existing reduction.
- Two policy arguments make the stage wordier than the other
  pipeline stages, and there is no shorter honest form: both inferred
  defaults are wrong in the field.
- Row count grows to the full grid, which for a sparse entity over a
  long history is a large multiple of the input.  Anyone
  rectangularizing a year of 15-minute windows should know they asked
  for 35,040 rows per entity.

Implementation:

- `mensura-types`: the stage's typing rule, which recognizes the
  value columns that are a single fold application (directly or
  through the bundled bindings, which already resolve to fold shapes
  under the modules oracle), reads that combiner's identity cell,
  and narrows every other column to optional.  The recognition is
  syntactic plus const resolution, machinery the checker has; no
  provenance model is needed.  Also decision 4's facts: `Complete`
  and rectangularity established by the stage, and the `demote w`
  re-derivation rule that consumes the latter.
- `mensura-runtime`: grid enumeration between the per-entity lower
  bound and the closed upper bound, an anti-join against the reduced
  rows, and the identity fill.
- `formal/`: `Mensura/Window/Dense.lean` (decision 7).
- Docs: a section in `07-pipelines.md`; ADR 0037 decision 6 already
  cross-references this ADR as taking its window-grid case.
- Examples: the fleet's windowed view gains a silence count, which is
  the motivating query and should appear as the worked example.

## Alternatives considered

1. **Materialize empty bags, reduce them with the identity.**  The
   obvious design.  Rejected per decision 1: it breaks ADR 0029
   decision 4's guarantee that `map_bags` never sees an empty bag,
   which is load-bearing for seedless folds, and it would surface the
   accumulator optionality that 0029 confined to the executor.
2. **Fabricate placeholder input rows before windowing.**  Rejected:
   a synthetic reading is counted like a real one, so `count` reports
   1 for an empty interval.  It puts a cardinality-axis lie on the
   value axis, which is the exact inversion of ADR 0037 decision 6's
   model.
3. **Sentinels instead of optionality** (`+Inf` for an empty maximum).
   Rejected on ADR 0031 decision 6's grounds, which already litigated
   this: the extension mints `NaN`, ADR 0026 bans dimensioned
   infinities, and a fabricated `+Inf` is a sentinel where `0`-for-sum
   is an honest value.
4. **Infer the population from the windowed bag.**  Rejected per
   decision 3: an entity that never reported has no rows, so it would
   never get windows, which is precisely the case the feature exists
   to expose.
5. **Infer the lower bound as the earliest observed `w`.**  Rejected
   per decision 3: it makes "offline since before we started
   watching" indistinguishable from "not yet installed".
6. **Fill past the watermark**, to the current wall clock.  Rejected:
   it would assert that future intervals are confirmed empty.
   Closedness is the bound precisely because it is the point past
   which the answer is not yet known.
7. **Ship the general `resample` instead.**  Rejected per decision 5:
   over a raw order key the step must be asserted, and an asserted
   step is the claim ADR 0035's audit refused.  The window grid is
   the case where the step is already written down.
8. **Do nothing; let users left-join against a generated grid.**
   Rejected as a design position, though it is what people will do in
   the meantime: it requires a grid-generating source the language
   does not have, it reintroduces the identity fill by hand and
   inconsistently, and it discards the closedness bound, which is the
   part most likely to be got wrong.

## Open questions

- **Naming** (decision 6).  *Settled (owner, 2026-08-22): shipped as
  `dense`, as decision 6 wrote it.*  The alternatives on the table were
  `rectangular`, `grid`, and `complete_grid` (`fill` collides with the
  fill policy below), and the argument for keeping `dense` is the one
  decision 6 makes: the intuition ADR 0037 decision 6 rejected was
  always right, and only its epistemic status was wrong.  The word now
  names something a mechanism earns.
- **Fill policies** (decision 2).  Carry-forward, carry-backward,
  interpolation, and a user-supplied constant all narrow an optional
  column back to total.  Additive, and each wants a consumer.
- **Total wrappers around a fold.**  The single-fold rule excludes
  `.n = to_real(#b)` even though `to_real` is total and preserves
  the identity (`to_real(0)` is `0.0`).  Admitting
  identity-preserving wrappers is a compositional analysis in
  miniature, the slippery slope decision 2 declines; store `#b` as
  `int` and convert where it is consumed.  Revisit only if the idiom
  proves common.
- **Interaction with per-key watermarks** (ADR 0037's open question).
  If an absent entity's windows never close, `dense` never fills
  them, which is either exactly right or exactly wrong depending on
  how that question resolves.  The two should be decided together.
  *Settled together by
  `docs/decisions/0041-watermark-grain-and-the-closure-floor.md`*: the
  watermark is grained by the residual key, which alone would indeed
  stop `dense` from ever filling an absent entity's grid, so the same
  ADR adds a declared closure floor whose whole job is to close the
  windows of an entity that has stopped reporting.  This ADR's worked
  example (`M-19`'s sixteen silent slots) is the case that forced it.
- **`date`-keyed grids.**  Blocked twice over: ADR 0036 defers
  `diff(date)`, and a calendar lower bound (`machines.commissioned`,
  a `date`) needs the zone-dependent `date <-> instant` conversion
  ADR 0036 also defers.  The worked example waits for neither: "when
  should telemetry exist" is answered by `activated`, an absolute
  event the store records directly.  A bound that is genuinely
  calendar data ("since the contract started", somebody's local
  midnight) remains the first real consumer of both deferrals.
- **Completeness transport through per-row filters.**  The filtered
  spelling of decision 4's query (a `flat_map` keeping the silent
  rows, then `demote w`, then `#b`) needs a rule that a
  deterministic per-row filter preserves completeness relative to
  the filtered ideal.  The lemma is small (filtering commutes with
  fiber restriction), but it is a general fact-transport rule in the
  ADR 0024 family, not a window fact, so it is not shipped here;
  decision 4's in-fold spelling does not need it.

## Forward references

- `docs/decisions/0010-attribute-totality.md` (the optionality
  decision 2 introduces).
- `docs/decisions/0017-completeness-establish-consume.md` and
  `docs/decisions/0023-completeness-consumed-by-the-reducer.md` (the
  establish/consume model decision 4 joins, and the demand its query
  discharges).
- `docs/decisions/0021-formal-proof-pipeline.md` (the rule decision 7
  satisfies).
- `docs/decisions/0029-fold-and-scan.md` (decision 4's empty-bag
  guarantee, which decision 1 preserves, and the accumulator
  `Option`).
- `docs/decisions/0031-fold-and-scan-primitives.md` (the combiner
  table; its identity column supplies every fill value, and its
  decision 6 already names "the empty window's answer" as the
  identity's purpose).
- `docs/decisions/0034-typed-ingestion.md` (the repair pattern
  decision 3 declines to imitate).
- `docs/decisions/0024-key-moves-as-a-true-inverse-pair.md` (the
  fact-transport family decision 4's rectangularity fact joins, and
  the home of the filter-transport open question).
- `docs/decisions/0035-completeness-cleared-by-demote.md` (the
  clearing rule decision 4 excepts, and the indistinguishability
  finding that scopes this ADR to the window grid, decision 5).
- `docs/decisions/0036-temporal-domains-and-torsor-arithmetic.md`
  (the grid origin; the deferred `date` cases in the open questions).
- `docs/decisions/0037-streaming-windows-and-closedness.md` (the
  grid, `closed` as the upper bound, and decision 6, whose
  window-grid case this ADR takes).
- `formal/Mensura/Window/Dense.lean` (new).
