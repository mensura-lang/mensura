# 0038: Rectangularization over the window grid

## Status

Accepted.  Takes the window-grid case out of the `resample` stage that
`docs/decisions/0037-streaming-windows-and-closedness.md` decision 6
defers, and ships it as a narrow mechanism with a consumer.  Leaves
the general case (an arbitrary step over a raw order key) deferred
where 0037 left it, for the reason decision 4 below gives.

Rests on ADR 0037 (the grid, `closed`, and the contiguity model),
ADR 0036 (the grid origin and point arithmetic), ADR 0031 (the
combiner table's identity column, which supplies every fill value),
ADR 0029 decision 4 (the guarantee decision 1 preserves), and
ADR 0010 (optional columns).  Honours ADR 0021: decision 6 supplies
the theorems for decision 2's typing rule.

Not scheduled.  This is the follow-up 0037 decision 6 promised "with
its own consumer"; the consumer arrived as soon as anyone asked how
many intervals a machine went silent.

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
         |> dense w over machines from commissioned_at
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
answers with a silent `NaN` rather than an error.  Until value
narrowing lands (ADR 0010's optional columns have no consuming
surface yet; ADR 0014 defers the missing-aware machinery), a ratio
over the dense grid is computed either upstream of `dense`, where
this rule sends it honestly optional, absent exactly on the empty
windows, or by the consumer of the served rectangle, which sees
`.n = 0` and knows the mean does not exist.  Recorded so the idiom
is taught rather than discovered.

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
  the stage (`over machines` above).
- **Per-entity lower bound: given.**  The grid runs back to the
  domain origin.  Something must say where each entity's history
  starts (`from commissioned_at` above), and the natural answer is a
  column on the population store.

Inferring either of the last two is the ADR 0034 repair pattern: a
heuristic that is right until it is silently, structurally wrong.
Taking the earliest observed `w` as the lower bound, in particular,
would make a sensor that was offline on day one look like a sensor
that did not exist yet.

### 4.  Scope: the window grid only

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

### 5.  Grammar

Illustrative, and the naming is an open question:

```
dense <window-column> over <store> from <column>
```

The `dense` spelling is chosen deliberately against ADR 0037 decision
6's rejection of "the dense/regular marker", on the grounds that the
intuition was always right and only the epistemic status was wrong:
this is the same idea, earned by a mechanism instead of asserted.
Reviewers who find that too cute have a fair point, and ADR 0025's
discipline would probably prefer two distinct words for two distinct
things.  Alternatives are recorded below.

### 6.  Formal backing

Per ADR 0021, decision 2 is a checker-visible typing rule
(optionality narrowing), so it ships with theorems.
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

Item 3 depends on ADR 0036 decision 9's order-compatibility lemma by
way of `closedWindow_stable`, so the dependency chain is
`Torsor.lean` → 0037's window lemmas → this module.

## Consequences

Positive:

- "How many intervals were silent" becomes expressible, and the
  `count`-never-zero trap closes.
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
  provenance model is needed.
- `mensura-runtime`: grid enumeration between the per-entity lower
  bound and the closed upper bound, an anti-join against the reduced
  rows, and the identity fill.
- `formal/`: `Mensura/Window/Dense.lean` (decision 6).
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
7. **Ship the general `resample` instead.**  Rejected per decision 4:
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

- **Naming** (decision 5).  `dense` reuses a word ADR 0037 decision 6
  rejected in a different role.  `rectangular`, `grid`, `fill`, and
  `complete_grid` are the alternatives; `fill` collides with the fill
  policy below.
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
- **`date`-keyed grids.**  Blocked twice over: ADR 0036 defers
  `diff(date)`, and the natural fleet lower bound
  (`machines.commissioned`, a `date`) needs the zone-dependent
  `date <-> instant` conversion ADR 0036 also defers.  This ADR's
  worked example is written against an `instant`-typed commissioning
  column for that reason, and the `date` form is the first real
  consumer of both deferrals.
- **A `Contiguous` fact, with its first consumer.**  An earlier draft
  had `dense` establish a `Contiguous` fact in the ADR 0017 idiom.
  Struck, on the no-consumer discipline ADR 0036 decision 4 applies
  to `diff(date)`: nothing demands the fact yet, and pre-establishing
  it buys nothing, since the visible consumer (a grid-positional
  vocabulary where `lag` means "the previous grid slot" rather than
  "the previous present row") will define what it demands in its own
  ADR, where the establishment is one line on `dense`.  The fact
  ships with that vocabulary, not ahead of it.

## Forward references

- `docs/decisions/0010-attribute-totality.md` (the optionality
  decision 2 introduces).
- `docs/decisions/0021-formal-proof-pipeline.md` (the rule decision 6
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
- `docs/decisions/0035-completeness-cleared-by-demote.md` (the
  indistinguishability finding that scopes this ADR to the window
  grid, decision 4).
- `docs/decisions/0036-temporal-domains-and-torsor-arithmetic.md`
  (the grid origin; the deferred `date` cases in the open questions).
- `docs/decisions/0037-streaming-windows-and-closedness.md` (the
  grid, `closed` as the upper bound, and decision 6, whose
  window-grid case this ADR takes).
- `formal/Mensura/Window/Dense.lean` (new).
