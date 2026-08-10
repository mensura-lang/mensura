# 0036: Temporal domains and torsor arithmetic

## Status

Accepted.  Answers, for the absolute family, the date-arithmetic
question `docs/decisions/0014-scalar-domain-taxonomy.md` left open,
and defers the civil half of it with a named successor (decision 4).
Gives the affine-timestamp footnote of
`docs/decisions/0026-dimensional-physical-units.md` (decision 1) its
positive half: instants get a scalar domain, and the difference of two
instants gets a type.  This is the prerequisite the M5 windows ADR
(0037) cites for window extents, for its grid origin, and for the
`w + size + lateness` arithmetic of its `closed` stage.

Touches `mensura-types` (`model`, `resolve`, `expr_check`),
`mensura-runtime` (the ADR 0034 decoder, `eval`), `formal/` (decision
9), the language docs (`06-expressions.md`, `09-typing-reference.md`,
`11-physical-units.md`), and the examples.  Not yet implemented; the
rollout lands with the M5 windows slice, alongside ADR 0037 in the
same pull request.

Honours `docs/decisions/0021-formal-proof-pipeline.md` (decision 9
supplies the theorems that gate the checker rules of decision 4) and
`docs/decisions/0034-typed-ingestion.md` decision 6 (decision 7 does
*not* re-open the input-encoding surface that ADR deferred).

## Context

The only temporal scalar domain is `date` (ADR 0014: equatable,
orderable, not numeric), and its arithmetic is an open question there:
"date arithmetic (durations between dates) is also left open."
Meanwhile ADR 0026 settled what a *duration* is (`time[real]`, a
dimensioned quantity) and recorded, as a footnote, what an instant is
not: "Timestamps are affine (offsets from an epoch) and stay outside
the dimension system; only durations are `time[real]`.  An epoch
timestamp is not `time[real]` any more than a calendar date is a
duration."  The language therefore has differences without points:
`90.0 * si.minute` types, but nothing can be subtracted to produce it.

The implementation matches this gap.  A `date` is an opaque string end
to end: `Value::Date(String)` (`crates/mensura-runtime/src/value.rs`),
a `TEXT` column in SQLite, ordered by string comparison
(`crates/mensura-runtime/src/eval.rs`, `compare_values`), and the
ADR 0034 decoder accepts *any* text as a date
(`crates/mensura-runtime/src/ingest.rs`).  Ordering a scan by
`taken_at` is chronologically sound only because every example happens
to write ISO `YYYY-MM-DD`; the convention is load-bearing and nowhere
enforced.

ADR 0037 forces the issue four ways, and its decisions are the
requirements this ADR must meet:

- **Extents** (0037 decision 3).  `size` and `stride` are const
  expressions of type `diff(domain(p))`, so a point domain must have a
  difference type, and the check must be the ordinary quantity check.
- **Translation** (0037 decisions 1 and 4).  Window placement is
  `w <= p < w + size`, and `closed` drops a window while
  `w + size + lateness > watermark`.  Both move a point by a duration.
- **A grid origin** (0037 decision 1).  Window starts are the integer
  multiples of `stride` anchored at "the domain's zero", which this
  ADR must actually define.
- **Key-eligibility** (0037 decision 2).  The window start `w` joins
  the key carrying `p`'s domain, so a temporal point domain must be
  equatable (ADR 0014's key-eligibility rule).

The fleet example keys readings by `date` today only because nothing
finer exists; readings at minute cadence keyed by calendar day
collide.

## Decision

### 1.  Two temporal families: civil and absolute

Temporal point domains divide into two families, and the division
(not resolution) is the primary axis:

- An **absolute** point identifies a moment on the physical timeline.
  It is the same moment for every observer; no zone is needed to
  interpret it, and none may be attached.
- A **civil** point identifies a position on a calendar or clock as a
  human reads one.  It is not a moment until a zone is supplied.
  `2026-10-31` names a different span of the timeline in Tokyo than in
  New York, and no UTC-only convention makes that go away.

Stating the families explicitly is what keeps the rest of this ADR
coherent.  The two have different difference types (decision 4),
different exactness arguments (decision 6), and no total conversion
between them (open questions).  Treating `date` as merely a coarse
instant assumes a zone silently, and produces a conversion story that
does not survive contact with a commissioning date recorded in a local
calendar.

### 2.  The domains: `date` (civil) and `instant` (absolute)

The temporal scalar domains are `date` (a calendar day: no
time-of-day, no zone) and a new `instant` (a moment on the timeline:
UTC, millisecond precision).  Both are equatable, orderable, and not
numeric; both are key-eligible (key-eligibility is equatability,
ADR 0014), which `instant` must be, since ADR 0037's window-start
column `w` carries the point domain into the key.  ADR 0014's
domain-property table extends by one row and gains a family column:

| domain    | family   | equatable | orderable | numeric | finite-enumerable |
| --------- | -------- | --------- | --------- | ------- | ----------------- |
| `date`    | civil    | yes       | yes       | no      | no                |
| `instant` | absolute | yes       | yes       | no      | no                |

`instant` is a lowercase built-in type name, the category ADR 0026
decision 5 fixes for `int`, `real`, `string`, `bool`, `date`, and the
dimension names.  The lexer is keyword-free, so it is a contextual
type name: no new token, no grammar change.

**On the name.**  The domain is the point, not its encoding, and the
name should say so.  Both obvious candidates fail that test in
opposite directions:

- `timestamp` names the affine *encoding* (a count from an epoch)
  that ADR 0026 decision 1 deliberately keeps out of the dimension
  system.
- `datetime` is worse than encoding-flavoured.  Across MySQL,
  SQL Server, Python's `datetime.datetime` in its default naive form,
  and Java's `LocalDateTime`, it overwhelmingly denotes a *zone-naive
  wall clock*: a civil point, the exact opposite of this domain.  It
  is a false friend of precisely the kind ADR 0025 renamed away from:
  "several operation names borrowed another tradition's word for an
  operation that is not quite that thing", and "group" was retired for
  "dragging in GROUP BY intuitions that mislead".  A SQL-literate
  reader meeting `datetime` will assume naive, and the assumption is
  wrong in the direction that silently corrupts event-time data.

`instant` names the concept directly, carries no encoding, does not
collide with the `time` dimension, and has precedent
(`java.time.Instant`, `Temporal.Instant`).  ADR 0025's rule applies as
written: where a clearer name exists, the surface departs from the
borrowed word.

**The rename is cheap now and never again.**  Nothing is implemented,
and the name appeared only in ADR 0037's draft, never in `crates/`,
`formal/`, `docs/language/`, or the examples.  Since 0036 and 0037
ship in the same pull request, the rename is a same-PR edit, already
applied to 0037.  After the M5 slice it would be a schema migration
in every deployed program.

### 3.  Points are not quantities

Neither temporal domain joins `D[real]`.  ADR 0026's footnote stands
and becomes the rule: instants and calendar days do not add, scale, or
divide; only their *differences* are quantities.  An `instant` column
is not `time[real]` any more than a position is a length.

There is no constructor and no cast from a number to a point, for the
reason ADR 0026 decision 6 gives for dimensioned values: such a cast
is the escape hatch the type discipline exists to make unnecessary.
So there are exactly two ways a point comes to exist: ingestion
through a declared column type, or translation of another point
(decision 4).

### 4.  Torsor arithmetic: each point domain has a difference type

Each orderable point domain `P` carries a difference type `diff(P)`,
and exactly two operation families connect them:

- **difference**: `P - P : diff(P)`
- **translation**: `P + diff(P) : P` and `P - diff(P) : P`

This ADR instantiates the rule at one temporal domain:

- `instant - instant : time[real]`, and
  `instant +/- time[real] : instant`.

`diff(date)` is **deferred** (below).  Until it is settled, `date`
supports no arithmetic at all: it remains equatable and orderable, as
in ADR 0014, and `date - date` and `date +/- x` are type errors for
every `x`.

Nothing else: no point plus point, no scaling a point, and no mixed
operands.  The mixed-operand prohibition is now a consequence of
decision 1 rather than a bare stipulation: `date` and `instant` are in
different families, so `instant - date` is not merely a same-domain
violation (ADR 0014) but a category error no conversion can silently
repair.  Ordering and equality are unchanged from ADR 0014.

The numeric domains are the degenerate case where point and difference
coincide: `diff(int) = int`, `diff(real) = real`,
`diff(D[real]) = D[real]`, which is just ADR 0014's and ADR 0026's
existing arithmetic re-read.  Stating the rule once at this generality
is deliberate: ADR 0037 decision 3 types a window extent as `diff` of
the point column's domain, and count-based windows over an `int` order
key then need no special case.

**The difference is not equatable; the point is.**  `diff(instant)` is
`time[real]`, and `real` is the one non-equatable domain (ADR 0014),
so `t1 - t2 == t3 - t4` is a type error while `t1 == t2` is fine.  The
asymmetry is correct and load-bearing.  An `instant` is an exact point
on a millisecond grid, so equality on it is sound and it may key a
table, which ADR 0037 decision 2 requires of `w`; a duration is a
continuous measurement, so equality on it is the float-equality
problem ADR 0014 barred.

**Why `diff(date)` is deferred.**  Two candidate answers are on the
table and neither can be chosen yet, because **nothing subtracts a
date**: not the fleet example, not `college-stores`, not the CLI
corpus.  `commissioned`, `last_service`, and `sampled_at` are never
operands of `-`.  A rule with no consumer should not be frozen, and
the first consumer is what distinguishes the candidates.

*Not `time[real]`.*  A calendar day is discrete, so a day count is
exact where a `real` is not, and a physical-seconds reading of a day
difference smuggles in an 86400 s/day convention that calendars do not
honour: DST days run 23 or 25 hours, so the error is up to an hour
*per day crossed* and grows with the interval.  It would also make
`date + 90 * si.minute` type, and a date translated by a sub-day
duration is not a date.  This candidate is rejected outright, not
deferred.  A caller who wants the physical estimate states the
convention explicitly: `to_real(d2 - d1) * si.day` once a day count
exists, using the `si` binding that already defines a day as
`86400.0 * second`.

*Not `int`, on present evidence.*  A bare `int` day count is exact and
equatable, and it was this ADR's first answer, but it puts the unit
back in the programmer's head, the `kelvin: real` mistake ADR 0026
exists to prevent.  With `warranty_days: int` and `cycle_count: int`
in the same row, `(d2 - d1) > cycle_count` type-checks and means
nothing.

*The expected answer is a civil quantity domain*, provisionally
`period`: an exact whole-day count, equatable and orderable, forming a
group under `+`/`-` and `int` scaling, outside ADR 0026's dimension
group because a calendar day is not a physical dimension.  It is not
introduced here because it needs a construction surface this ADR does
not ship (decision 8), an ADR 0014 row, aggregate signatures, and an
extent spelling in 0037: a domain, a bundled `calendar` module, and
an ADR, for something nothing calls.  Whether it should also carry
months and years is the question the first consumer answers, and it is
a real fork: months are not group elements (`Jan 31 + 1 month - 1
month` is not `Jan 31`), so they belong in that module as functions
over `date`, not as values.

The deferral is cheap.  Decision 4's rule is stated generically, so
adding the instantiation later is additive; decision 9 proves the
torsor laws over an abstract difference group, so no proof is redone.

**The leap-second asymmetry, stated.**  The argument above appeals
partly to UTC's irregularity, and `diff(instant) = time[real]` is
exposed to a version of the same objection: an `instant` is a civil
UTC label (decision 7), so a difference spanning an inserted leap
second understates physical elapsed time by one second.  This is a
real inexactness, not an oversight, and it is accepted because it is
bounded in a way the calendar case is not: at most one second per
insertion, 37 seconds cumulatively since 1972, and non-accumulating in
any interval that crosses none, against a DST error that is unbounded
and proportional to the interval.  A caller requiring TAI semantics
wants a distinct absolute domain, not a reinterpretation of this one.
The irregularity enters only through differences that span an
insertion, never through the encoding: decision 7 rejects the
`23:59:60` label itself.

### 5.  Each point domain has an origin, for grids only

ADR 0037 decision 1 anchors window starts at "the domain's zero".
This ADR fixes it:

- `instant`: `1970-01-01T00:00:00.000Z`.
- `date`: `1970-01-01`.
- `int`: `0`.

The `date` origin is recorded for completeness and is inert until
decision 4's deferral is lifted: a window needs extents of type
`diff(domain(p))`, so `date`-keyed windows are unavailable in this
slice and ADR 0037 decision 3 drops its `date` clause accordingly.

The origin is a **grid anchor and nothing else**.  It is read by
`window`'s placement rule and by no typing rule, no comparison, and no
storage decision; points before it are ordinary points and produce
negative differences.  Naming it here rather than in 0037 keeps the
per-domain facts in one place, and stating its scope explicitly is
what stops it becoming the invisible global epoch that decision 3 and
alternative 1 reject.  An origin only a grid reads is a convention; an
origin the type system reads is an affine encoding.

### 6.  Precision, range, and exactness of translation

Fixed points make range and precision part of the domain, so both are
specified here rather than left to the decoder.

**Precision.**  An `instant` carries exactly millisecond resolution.
Finer input is rejected, not truncated.

Millisecond is not a round-number guess; it is the finest grid whose
differences the chosen difference type can carry exactly.
`diff(instant)` is `time[real]` (decision 4), ADR 0026 backs `real`
with binary64, and binary64 holds integers exactly only up to 2^53.
The representable range below spans about 10^4 years, which is about
3.2 * 10^14 milliseconds, comfortably inside 2^53 (about 9.0 * 10^15,
a 28x margin), but about 3.2 * 10^17 microseconds, well outside it.
At microsecond precision a difference across the range would not be
an exact integer count in the backing, and every exactness argument
in this decision collapses.  The precision therefore follows from the
backing: a finer absolute domain awaits the exact backing ADR 0026
decision 9 defers, and it arrives as a *sibling* domain in decision
1's absolute family with its own `diff` (decision 4 is stated
generically, so the addition is additive); `instant` itself never
changes precision, because that would be a schema migration in every
deployed program.

The limitation this buys is real and is owned rather than discovered:
an event stream sampled above 1 kHz cannot key by `instant` (adjacent
samples collide on the millisecond grid), and a producer emitting
finer timestamps cannot ingest them at all, because the decoder
rejects rather than truncates.  Such a producer either states its own
truncation policy at the source, where it is a visible measurement
decision rather than a silent repair, or waits for the finer domain.
High-frequency waveform capture (vibration spectra in the driving
application) is out of this domain's reach and is recorded as an open
question below.

**Range.**  The canonical encoding is fixed-width with a four-digit
year (decision 7), so the representable range is
`0001-01-01T00:00:00.000Z` through `9999-12-31T23:59:59.999Z`.
Expanded-year and negative-year forms are rejected.  `date` is bounded
identically at day granularity.

**Translation is exact-or-error.**  `instant +/- time[real]` converts
the duration operand to an integer millisecond count; if the duration
is not a whole number of milliseconds, the operation is rejected.  It
does not round.  Three reasons, in ascending order of force:

1. It is the decoder's decode-or-reject contract (ADR 0034 decision 3)
   applied one layer up.  Silently rounding a computed offset
   falsifies a result exactly as silently rounding a wire value does.
2. Rounding would break key identity.  `instant` is key-eligible
   because its grid is exact (decision 4); a translation that rounds
   can send equal points to unequal ones, reintroducing through the
   back door the float-equality problem ADR 0014 barred from keys.
3. It fails invisibly otherwise.  A window stride off by a fraction of
   a millisecond does not error; it drifts ADR 0037's bucket
   boundaries slowly across a long stream.

**Most of this check is compile-time.**  ADR 0037 decision 3 makes
`size` and `stride` const expressions (ADR 0030), and its decision 4
makes `lateness` one too, so the whole-millisecond and positivity
checks on the extents that matter run in the checker rather than at
evaluation.  A runtime check remains only for data-dependent
translation, where the duration comes from a column.  Any translation
whose result leaves the representable range is likewise an error, as
is a millisecond difference that overflows its difference type.

**Two implementation notes**, recorded because they are easy to get
wrong and cheap to state:

- Differences are computed as an exact integer millisecond count and
  converted *once* to the normalized `time[real]` magnitude, never
  accumulated in floating point.  The single round trip is
  unconditionally safe on this range: dividing an integer count of at
  most 3.2 * 10^14 by 1000 and multiplying back accumulates a
  relative error of at most 2^-52, an absolute error under 0.1 ms
  against the 0.5 ms nearest-integer threshold, so a
  difference-produced duration always recovers its exact count.
  `t + (u - t) == u` therefore holds for all representable `t`, `u`;
  decision 9 makes the grid-level statement a theorem, and this bound
  is the implementation obligation that connects the theorem to the
  binary64 backing (decision 9 states the division of labour).
- "Whole number of milliseconds" needs a precise predicate, but only
  for *constructed* durations; difference-produced ones are covered
  by the bound above.  ADR 0026 normalizes magnitudes to the base
  unit, so a `time[real]` is a count of seconds, and `si.millisecond`
  is `0.001 * second`, a value with no exact binary representation.
  The recommended reading is that the nearest-integer millisecond
  value must differ from the converted magnitude by no more than one
  ULP of that magnitude.  The exact predicate is an open question
  below: it is the one place this ADR's exactness story meets the
  backing's inexactness, and it belongs with the `precision`
  library's authors rather than being guessed here.

### 7.  Encoding: normalized fixed-width UTC text

On the wire (the ADR 0034 decoder) and in storage, an `instant` is an
RFC 3339 string; the decoder accepts an explicit UTC offset,
normalizes to UTC, and re-encodes fixed-width
`YYYY-MM-DDTHH:MM:SS.sssZ`.  Storage stays `TEXT`.  With one zone and
one width, lexicographic order *is* chronological order, so the
backend's string comparison and the evaluator's `compare_values`
remain correct unchanged.  This is the same move as ADR 0026's affine
units, where a messy presentation form is converted once at ingestion
and the core only ever sees the normal form.

**Leap-second labels are rejected.**  RFC 3339 admits a seconds field
of `60` for an inserted leap second; the decoder does not.  Decision 4
gives `instant` its arithmetic, and decision 2 its key-eligibility,
via a bijection between labels and the exact millisecond grid, and
`23:59:60` has no slot in that grid: accepting it would break the
torsor laws and the lexicographic-equals-chronological property in
the same stroke.  The seconds field is `00` through `59`, and a
reading stamped inside an inserted leap second is its producer's to
clamp or smear, the same producer-converts rule as epoch intake
below.  The cost is bounded exactly as decision 4's asymmetry
argument bounds it: at most one second per insertion, none inserted
since 2016, and the CGPM has resolved to discontinue insertions by
2035.

As a consequence the `date` decoder tightens to exactly `YYYY-MM-DD`.
Today any text passes, which makes the ordering convention unenforced
(context above); after this ADR a malformed date is an ingestion error
like any other type mismatch.

**Epoch-encoded intake is deferred, to the same place as Celsius.**
The obvious request is that the decoder also accept an integer epoch
count, since sensors, Unix logs, and most JSON payloads emit one.  It
is not accepted here, and the reason is ADR 0034 decision 6 rather
than anything about time: an epoch count is an *input encoding*, and
declaring an input encoding per column needs a surface that ADR 0034
placed in M7's endpoint payload contract, not in the registry
declaration.  Epoch intake is the exact temporal analogue of the
Celsius hook that ADR deferred (same shape, same missing surface,
same wrong home), and adding it here would land a member of the
annotation family ahead of the annotation document, the outcome
ADR 0034 declined by name.

The rule until then mirrors ADR 0034's rule for a dimensioned column,
where an ingested `D[real]` carries a base-unit magnitude and a
Celsius payload is converted by its producer: an ingested `instant`
carries canonical RFC 3339, and an epoch-emitting producer converts.
When M7 designs the payload contract, temporal input encoding
(`epoch_seconds`, `epoch_millis`) belongs in it beside the affine unit
declaration, and the two should be designed together.  Whatever the
surface, the unit must be declared and never inferred: magnitude
sniffing (treating values above a threshold as milliseconds) is the
archetype of the repair ADR 0034 forbids, correct until a value near
the boundary arrives and then wrong by three orders of magnitude.

### 8.  No temporal literals this round

Values arrive through ingestion; expressions compare, difference, and
translate columns; extents are `si` durations or `int` counts, both
already expressible.  A literal or constructor form
(`date "2026-01-01"`) is recorded as an open question, needed the
first time a program hard-codes a calendar boundary.  This is a
*literal* question only: decision 3's ban on constructing a point from
a number is not up for revision by it.

### 9.  Formal backing

Per ADR 0021, a checker propagation rule ships only when a theorem
under `formal/` backs it, and decision 4 adds three typing rules.
`formal/Mensura/Units/Torsor.lean` mechanizes them, extending the
dimension group of `formal/Mensura/Units/Dimension.lean` (ADR 0026
decision 10):

1. **The torsor structure.**  For a point domain `P` with difference
   group `G`, a free and transitive additive action of `G` on `P`.
   The two operation families of decision 4 are the action and its
   inverse, and the laws follow rather than being stipulated:
   `t + (u - t) = u`, `(t - u) + (u - v) = t - v`, `t - t = 0`, and
   uniqueness of the difference.  This is what makes "torsor" the
   right word in the title rather than decoration, and it is the
   round-trip property decision 6 requires.
2. **Instantiation at `instant`.**  `instant` over the `time` subgroup
   of the dimension group, proved to satisfy the axioms on the exact
   millisecond grid.  The structure is stated over an abstract
   difference group and does not assume the backing is `real`, so
   neither a later exact backing (ADR 0026 decision 9) nor the civil
   instantiation deferred in decision 4 reopens the proof.
3. **Order compatibility.**  Translation by a positive difference is
   strictly increasing.

The third item is not bookkeeping.  ADR 0037's interval test
`w <= p < w + size` and its `closedWindow_stable` gate, which reasons
about `w + size + lateness` against a watermark, both need translation
to be monotone in the difference.  Sequencing follows: this module
lands before or with 0037's window lemmas, which depend on it.

Two gaps between the theorems and the implementation are deliberate
and stated here rather than discovered later.  Conversion correctness
for the decoder's normalization (decision 7) is *not* a formal
target, matching ADR 0026's treatment of scale-factor normalization
as an implementation obligation rather than a mechanized one.
Likewise, the torsor is mechanized over the exact millisecond grid,
while the implementation carries a `diff(instant)` through the
binary64 backing of `time[real]`; the bridge is decision 6's
round-trip bound, an implementation obligation of the same kind, and
it is why decision 6 sizes the grid so that every representable
difference is an exact integer in that backing.

## Consequences

Positive:

- The instant/duration split is complete: points subtract to
  dimensioned durations, durations translate points, and the type
  system rejects `t1 + t2` and `date + 90 * si.minute` the way it
  rejects a unit mismatch.
- The civil half ships no rule rather than a rule with no consumer, so
  the eventual answer is chosen by a caller instead of guessed here.
- ADR 0037's four requirements are met by one rule each, with no
  temporal special case: extents are `diff`, placement and `closed`
  are translation, the grid anchor is decision 5, and `w` is
  key-eligible because `instant` is equatable.
- Chronological ordering of temporal columns becomes a checked
  invariant of the encoding rather than an accident of example style.
- The civil/absolute split gives every future temporal question a
  place to land (a wall-clock domain, zoned serving, calendar
  arithmetic) instead of relitigating the taxonomy each time.
- `instant` does not mislead readers arriving from SQL, and leaves
  `datetime` unclaimed for a civil wall-clock domain should one
  appear.

Negative:

- `instant` is unfamiliar to a SQL-first audience in a way `datetime`
  is not.  `09-typing-reference.md` should carry an explicit "coming
  from MySQL or Postgres" note mapping `instant` to
  `TIMESTAMP`/`timestamptz` and recording that Mensura has no
  equivalent of naive `DATETIME`.
- Exact-or-error translation surfaces as errors in programs that would
  otherwise have drifted quietly.  That is the intent, but
  `06-expressions.md` should teach it with a stride example rather
  than leave it to a diagnostic.
- Producers emitting epoch time must convert until M7 (decision 7), a
  real cost paid to keep the input-encoding surface in one place.
- `date` arithmetic and `date`-keyed windows are unavailable until
  decision 4's deferral is lifted.  No program in the repository wants
  either today, but the gap is real and should be stated in
  `09-typing-reference.md` rather than discovered.
- Sub-millisecond event data is out of `instant`'s reach: a stream
  sampled above 1 kHz collides on the millisecond grid, and finer
  timestamps are rejected at the decoder (decision 6).  The gap is
  structural, since the grid is sized to the binary64 backing, and
  its successor is a finer sibling domain named in the open
  questions; until it lands, the driving application's waveform-rate
  captures are inexpressible as `instant`-keyed rows.
- A new formal module is a slice cost, and 0037's window lemmas now
  carry an ordering dependency on it.

Neutral:

- Genuinely civil `date` columns are unaffected: the fleet's
  `commissioned` and `last_service` keep their domain and their
  meaning.

Implementation (with the M5 slice, not this ADR):

- `mensura-types`: a `ColumnType::Instant` variant; `resolve`
  recognizes the type name; `expr_check`'s binary-operator table grows
  the three torsor rows of decision 4 (`instant - instant`,
  `instant + time[real]`, `instant - time[real]`), and its same-domain
  check grows the family distinction of decision 1.
- `mensura-runtime`: decoder validation and normalization (decision
  7); range, precision, and exactness checks (decision 6); `eval`
  gains `instant +/- time[real]` (days-from-civil and civil-from-days
  plus a millisecond offset; small, exactly specified,
  dependency-free).  Comparison code is untouched, and `date` needs no
  arithmetic at all this slice.
- `formal/`: `Mensura/Units/Torsor.lean` and its blueprint node
  (decision 9).
- ADR 0037: six lines rename `datetime` to `instant`; its decision 1
  grid-origin sentence cites decision 5 here; and its decision 3 drops
  "`int` days for a `date` point" from the extent list, since
  `diff(date)` is deferred.
- Docs: `06-expressions.md` and `09-typing-reference.md` gain the
  torsor rules, the family split, and the exactness rule;
  `11-physical-units.md`'s affine paragraph points here; ADR 0014's
  open question is annotated closed.
- Examples: the fleet's `Reading.taken_at`, `Slot.taken_at`, and
  `vibrations.sampled_at` migrate `date -> instant`, which is the
  honest domain for minute-cadence readings and what 0037's windowed
  examples assume.  `Commissioned.commissioned`,
  `machines.commissioned`, and `machines.last_service` stay `date`;
  they are civil data and always were.

## Alternatives considered

1. **Instants as quantities** (a `timestamp` that *is* `time[real]`
   from an epoch).  Rejected: ADR 0026's footnote is correct that the
   encoding is affine; dimension arithmetic would then type `t1 + t2`
   and `2.0 * t`, which are meaningless, and the epoch becomes an
   invisible global convention.  Decision 5 keeps the one origin the
   design does need visibly scoped to the window grid.
2. **The three candidates for `diff(date)`**, all declined this round
   per decision 4:

   - **`time[real]`.**  Rejected outright, not deferred: the one rule
     it would buy (all temporal extents are durations) costs
     exactness, imports an 86400 s/day convention calendars do not
     honour, and admits sub-day translations of a day-granular point.
   - **`int`.**  Not rejected on principle (it is exact and
     equatable, and it was this ADR's first answer) but not adopted:
     it leaves the unit in the programmer's head, and no consumer
     exists whose needs would confirm or refute the choice.  If the
     first one wants plain whole-day counts and nothing more, this is
     the cheap answer and stays available.
   - **A `period` civil quantity domain.**  The expected eventual
     answer, deferred on cost: a scalar domain, a construction
     surface, aggregate signatures, an ADR 0014 row, an extent
     spelling in 0037, and a bundled `calendar` module, for something
     no program calls.  Recorded so the deferral has a named
     successor rather than an open hole.
3. **Retire `date`, keep only `instant`.**  Rejected, and decision 1
   sharpens why.  Day-granular calendar data is real (the fleet's
   `commissioned`), and forcing it to a midnight instant fabricates
   both a precision and a zone: midnight *where?*  The two domains
   are not coarse and fine versions of one thing; they are members of
   different families.  ADR 0014's discipline is that every column
   states which it is.
4. **Zone-carrying instants.**  Rejected, but not by analogy to
   ADR 0026 decision 7.  "A zone is presentation, exactly like
   Celsius" does not hold: the Celsius-to-Kelvin map is fixed and
   lossless, whereas resolving a *future* civil time to an instant
   depends on a tz database that governments change, so normalizing at
   ingestion destroys information that cannot be reconstructed.  The
   rejection that does hold is narrower: Mensura ingests observations,
   which are absolute points already, and it has no scheduling
   consumer needing a civil time to re-resolve under changed rules.
   Zoned *intake* is handled by normalization at the decoder; zoned
   *display* belongs to the serving layer (M7).  If a scheduling
   consumer appears, the answer is not a zone on `instant` but a civil
   domain in the family of decision 1, paired with a zone: a
   different feature with its own ADR.
5. **`INTEGER` epoch-millisecond storage.**  Rejected: `TEXT` keeps
   parity with `date`, keeps database dumps readable, and keeps the
   existing string-comparison ordering valid; the backend already
   stores normalized magnitudes for quantities, so "normal form in a
   plain column" is the established pattern.  This is a *storage*
   question, independent of the *intake* question decision 7 defers.
6. **A dedicated `duration` scalar domain**, replacing `time[real]` on
   the absolute side.  Rejected: `time[real]` already is the duration
   type (ADR 0026), it gets dimensionless ratios and derived
   quantities like speed for free from the group, and a second
   duration vocabulary would split every downstream signature.  This
   is not the `period` of alternative 2, which is a first vocabulary
   for a different concept (civil day counts) rather than a second one
   for physical durations.
7. **Naming the domain `datetime`.**  Rejected per decision 2: it
   denotes a zone-naive wall clock nearly everywhere it appears, which
   is the opposite of this domain's meaning, and it consumes the
   natural name for the civil domain decision 1 anticipates.
   ADR 0025's false-friend precedent governs.
8. **Adding a civil wall-clock domain now**, alongside `date` and
   `instant`.  Deferred, not rejected.  Decision 1 makes room for it,
   but it has no consumer before M7 serving, and specifying it
   properly needs the zone representation alternative 4 defers.
9. **Accepting epoch input now**, with a per-column unit declaration
   on the registry.  Rejected per decision 7: ADR 0034 decision 6
   assigns the per-column input-encoding surface to M7's payload
   contract, and a registry-level declaration today would be in the
   wrong home once endpoints exist.  Recorded because it is the first
   thing to design when M7 opens, together with the affine unit hook.
10. **Truncating rather than rejecting sub-millisecond translation.**
    Rejected: it is the silent falsification the decoder refuses, it
    can break key identity (decision 6), and it fails invisibly as
    grid drift rather than as an error at the first evaluation.
11. **Shipping decision 4's typing rules with no formal module.**
    Rejected: ADR 0021's rule is that a checker propagation rule ships
    with a theorem behind it, and ADR 0037's `closedWindow_stable`
    needs the order-compatibility lemma regardless, so the module is
    owed either way.

## Open questions

- **`diff(date)`, and with it `date` arithmetic and `date`-keyed
  windows** (decision 4).  Deferred until a consumer exists; nothing
  in the repository subtracts a date today.  The expected answer is
  `period`, an exact whole-day civil quantity outside the dimension
  group, shipped with a bundled `calendar` module that provides `day`
  and `week` as periods and `add_months`/`add_years` as functions over
  `date` (months are not group elements, so they cannot be periods).
  The first consumer decides between that and a plain `int` day count,
  and in particular decides whether calendar units are needed at all.
- **The exact float predicate for "whole number of milliseconds"**
  (decision 6), against ADR 0026's normalized-magnitude
  representation.  The predicate matters only for constructed
  durations (`15 * si.minute`); difference-produced durations are
  covered by decision 6's round-trip bound.  The likely resolution is
  a one-ULP tolerance, and it should be settled with the deferred
  `precision` library (ADR 0026 decision 9, ADR 0028 decision 4)
  rather than fixed independently here.
- **A finer absolute domain** (microsecond or nanosecond instants)
  for waveform-rate capture, gated on the exact backing of ADR 0026
  decision 9: binary64 cannot carry a full-range microsecond count
  exactly (decision 6).  It joins decision 1's absolute family as a
  sibling domain with its own `diff`, additive under decision 4's
  generic rule; `instant` never changes precision.
- **A civil wall-clock domain** (alternative 8), if a consumer
  appears, and its name: `datetime` is now available and would read
  correctly to a SQL audience.
- **Temporal literals or a constructor builtin** (decision 8).
- **`date <-> instant` conversion.**  This is *zone-dependent, not
  truncation*: "the calendar day containing this instant" requires a
  zone, and "midnight on this date" fabricates one.  Any future
  conversion must take an explicit zone argument.  The first concrete
  consumer is ADR 0037's deferred rectangularization: giving each
  entity a lower bound on the window grid wants
  `machines.commissioned`, a `date`, compared against a `w` of domain
  `instant`.  It is a good motivating case precisely because it needs
  what the deferral demands: "the machine existed from the start of
  2024-03-15" is a claim about somebody's local midnight, and the
  program should have to say whose.
- **Calendar-unit arithmetic** (months, years): not torsor
  differences, since they do not commute with translation, so they sit
  outside this ADR's algebra and outside decision 9's structure.  Out
  unless a consumer appears.
- **Grid origins other than the domain zero.**  ADR 0037's open
  question on aligned-but-offset grids (business days, fiscal weeks)
  would extend decision 5 with an explicit origin argument; the
  question stays 0037's.
- **Whether `mensura serve` (M7) accepts zoned RFC 3339 on all
  transports** or narrows the profile per endpoint, and the temporal
  input encodings that land with it (decision 7).

## Forward references

- `docs/decisions/0014-scalar-domain-taxonomy.md` (the taxonomy this
  extends; its open date-arithmetic question closes for the absolute
  family and stays open for the civil one, annotated with decision 4's
  deferral rather than marked resolved; its key-eligibility and
  non-equatable-`real` rules are load-bearing in decisions 2 and 4).
- `docs/decisions/0021-formal-proof-pipeline.md` (the rule decision 9
  satisfies).
- `docs/decisions/0025-nomenclature-consistency-sweep.md` (the
  false-friend precedent decision 2 applies).
- `docs/decisions/0026-dimensional-physical-units.md` (durations, the
  affine footnote, the built-in type-name category, the
  normalization-at-ingestion precedent, and the mechanized dimension
  group decision 9 extends).
- `docs/decisions/0028-standard-library-si.md` (`si.minute`,
  `si.day`, `si.millisecond`: the extents and the explicit
  day-conversion idiom).
- `docs/decisions/0030-const-functions.md` (why most of decision 6's
  exactness check is compile-time).
- `docs/decisions/0034-typed-ingestion.md` (the decoder that enforces
  decisions 6 and 7; its decision 6 is why epoch intake is deferred).
- `docs/decisions/0037-streaming-windows-and-closedness.md` (extents
  are `diff` of the order key's domain; the grid anchors at decision
  5's origin; `closed` rests on decision 9's order-compatibility
  lemma).
- `docs/language/06-expressions.md`, `09-typing-reference.md`,
  `11-physical-units.md` (rules and cross-references on
  implementation).
- `formal/Mensura/Units/Dimension.lean` (the group decision 9
  extends); `formal/Mensura/Units/Torsor.lean` (new).
