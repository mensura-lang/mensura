# 0036: Temporal domains and torsor arithmetic

## Status

Accepted.  Closes the date-arithmetic question
`docs/decisions/0014-scalar-domain-taxonomy.md` left open, and gives the
affine-timestamp footnote of
`docs/decisions/0026-dimensional-physical-units.md` (decision 1) its
positive half: instants get a scalar domain, and the difference of two
instants gets a type.  This is the prerequisite the M5 windows ADR
(0037) cites for window extents.  Touches `mensura-types` (`model`,
`resolve`, `expr_check`), `mensura-runtime` (the ADR 0034 decoder,
`eval`), the language docs (`06-expressions.md`,
`09-typing-reference.md`, `11-physical-units.md`), and the examples.
Not yet implemented; the rollout lands with the M5 windows slice.

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

The implementation matches this gap.  A `date` is an opaque string
end to end: `Value::Date(String)` (`crates/mensura-runtime/src/value.rs`),
a `TEXT` column in SQLite, ordered by string comparison
(`crates/mensura-runtime/src/eval.rs`, `compare_values`), and the
ADR 0034 decoder accepts *any* text as a date
(`crates/mensura-runtime/src/ingest.rs`).  Ordering a scan by
`taken_at` is chronologically sound only because every example happens
to write ISO `YYYY-MM-DD`; the convention is load-bearing and nowhere
enforced.

M5 forces the issue three ways.  A window over an event-time column
needs point *differences* (the extent `15 * si.minute` must compare
against `t - w`), point *translation* (a window start is
`t - (t - start) mod stride`, movement of a point by a duration), and
a point domain fine enough for sensor cadence.  The fleet example keys
readings by `date` today only because nothing finer exists; readings
at minute cadence keyed by calendar day collide.

## Decision

### 1.  Two temporal point domains: `date` and `datetime`

The temporal scalar domains are `date` (a calendar day: no
time-of-day, no zone) and a new `datetime` (an instant: UTC,
millisecond precision).  Both are equatable, orderable, and not
numeric; both are key-eligible (key-eligibility is equatability,
ADR 0014), which `datetime` must be, since an event-time key field is
the paradigm case.  ADR 0014's domain-property table extends by one
row:

| domain     | equatable | orderable | numeric | finite-enumerable |
| ---        | ---       | ---       | ---     | ---               |
| `datetime` | yes       | yes       | no      | no                |

A time-of-day domain is deferred: it has no consumer, and its natural
name collides with the `time` dimension.  The lexer is keyword-free,
so `datetime` is a contextual type name like `date` and `int`: no new
token, no grammar change.

The name is `datetime`, not `timestamp`: "timestamp" names the
affine *encoding* (a count from an epoch) that ADR 0026 deliberately
keeps out of the dimension system, and the domain is the point, not
its encoding.

### 2.  Points are not quantities

Neither temporal domain joins `D[real]`.  ADR 0026's footnote stands
and becomes the rule: instants and calendar days do not add, scale,
or divide; only their *differences* are quantities.  A `datetime`
column is not `time[real]` any more than a position is a length.

### 3.  Torsor arithmetic: each point domain has a difference type

Each orderable point domain `P` carries a difference type `diff(P)`,
and exactly two operation families connect them:

- **difference**: `P - P : diff(P)`
- **translation**: `P + diff(P) : P` and `P - diff(P) : P`

For the temporal domains:

- `datetime - datetime : time[real]`, and
  `datetime +/- time[real] : datetime`.
- `date - date : int` (a whole-day count), and
  `date +/- int : date`.

Nothing else: no point plus point, no scaling a point, no mixed
`date`/`datetime` operands (the same-domain rule of ADR 0014 stands;
conversion between the two is deferred until it has a consumer).
Ordering and equality are unchanged from ADR 0014.

The numeric domains are the degenerate case where point and
difference coincide: `diff(int) = int`, `diff(real) = real`,
`diff(D[real]) = D[real]`, which is just ADR 0014's and ADR 0026's
existing arithmetic re-read.  Stating the rule once at this
generality is deliberate: the windows ADR (0037) types a window
extent as `diff` of the point column's domain, and count-based
windows over an `int` order key then need no special case.

Why `diff(date)` is `int` and not `time[real]`: a calendar day is
discrete, so a day count is exact where a `real` is not; a
physical-seconds reading of a day difference smuggles in an
86400 s/day convention that calendars do not honour (DST days run 23
or 25 hours in local calendars, and UTC has leap seconds); and
translation by a sub-day duration (`date + 90 * si.minute`) is not a
date.  A caller who wants the physical estimate states the convention
explicitly: `to_real(d2 - d1) * si.day`.

### 4.  Encoding: normalized fixed-width UTC text

On the wire (the ADR 0034 decoder), a `datetime` is an RFC 3339
string; the decoder accepts an explicit UTC offset, normalizes to
UTC, and re-encodes fixed-width `YYYY-MM-DDTHH:MM:SS.sssZ`.  Storage
stays `TEXT`.  With one zone and one width, lexicographic order *is*
chronological order, so the backend's string comparison and the
evaluator's `compare_values` remain correct unchanged; this is the
same move as ADR 0026's affine units, where the messy presentation
form (a zoned local time, a Celsius reading) is converted once at
ingestion and the core only ever sees the normal form.

Precision is exactly milliseconds.  Finer input is rejected, not
truncated: silently rounding a wire value falsifies it, and the
ingestion layer's contract (ADR 0034) is decode-or-reject, never
repair.  Leap seconds follow the Unix convention (a `datetime` is a
civil UTC label, not a TAI count); out of scope beyond that.

As a consequence the `date` decoder tightens to exactly
`YYYY-MM-DD`.  Today any text passes, which makes the ordering
convention unenforced (context above); after this ADR a malformed
date is an ingestion error like any other type mismatch.

### 5.  No temporal literals this round

Values arrive through ingestion; expressions compare, difference, and
translate columns; extents are `si` durations or `int` counts, both
already expressible.  A literal or constructor form (`date
"2026-01-01"`) is recorded as an open question, needed the first time
a program hard-codes a calendar boundary.

## Consequences

Positive:

- The instant/duration split is complete: points subtract to
  dimensioned durations, durations translate points, and the type
  system rejects `t1 + t2` and `date + 90 * si.minute` the way it
  rejects a unit mismatch.
- Window extents (0037) type uniformly as `diff` of the point domain,
  with `datetime`, `date`, and `int` order keys covered by one rule.
- Chronological ordering of temporal columns becomes a checked
  invariant of the encoding rather than an accident of example style.

Implementation (with the M5 slice, not this ADR):

- `mensura-types`: a `ColumnType::Datetime` variant; `resolve`
  recognizes the type name; `expr_check`'s binary-operator table
  grows the four torsor rows of decision 3.
- `mensura-runtime`: decoder validation and normalization (decision
  4); `eval` gains calendar arithmetic for `date +/- int` and
  `datetime +/- time[real]` (days-from-civil and civil-from-days plus
  a millisecond offset; small, exactly specified, dependency-free).
  Comparison code is untouched.
- Docs: `06-expressions.md` and `09-typing-reference.md` gain the
  torsor rules; `11-physical-units.md`'s affine paragraph points
  here; ADR 0014's open question is annotated closed.
- Examples: the fleet's `taken_at` migrates `date -> datetime`, which
  is the honest domain for minute-cadence readings and what the
  windowed examples of 0037 assume.

## Alternatives considered

1. **Instants as quantities** (a `timestamp` that *is* `time[real]`
   from an epoch).  Rejected: ADR 0026's footnote is correct that the
   encoding is affine; dimension arithmetic would then type `t1 + t2`
   and `2.0 * t`, which are meaningless, and the epoch becomes an
   invisible global convention.
2. **`diff(date) = time[real]`.**  Rejected for the reasons in
   decision 3; the one rule it would buy (all temporal extents are
   durations) costs exactness and admits sub-day translations of a
   day-granular point.
3. **Retire `date`, keep only `datetime`.**  Rejected: day-granular
   calendar data is real (`commissioned` dates), and forcing it to a
   midnight instant fabricates precision.  ADR 0014's discipline is
   that every column states which it is.
4. **Zone-carrying datetimes.**  Rejected: a zone is presentation,
   exactly like Celsius (ADR 0026 decision 7).  The core is UTC-only;
   zoned display belongs to the serving layer (M7) and zoned intake
   is handled by normalization at the decoder.
5. **`INTEGER` epoch-millisecond storage.**  Rejected: `TEXT` keeps
   parity with `date`, keeps database dumps readable, and keeps the
   existing string-comparison ordering valid; the backend already
   stores normalized magnitudes for quantities, so "normal form in a
   plain column" is the established pattern.
6. **A dedicated `duration` scalar domain.**  Rejected: `time[real]`
   already is the duration type (ADR 0026), and a second duration
   vocabulary would split every downstream signature.

## Open questions

- A time-of-day domain, if a consumer appears, and its name (the
  obvious one collides with the `time` dimension).
- Temporal literals or a constructor builtin (decision 5).
- `date <-> datetime` conversions (truncation is well-defined in a
  UTC-only core, but deferred until needed).
- Calendar-unit arithmetic (months, years): not torsor differences
  (they do not commute with translation), so they are outside this
  ADR's algebra; out unless a consumer appears.
- Whether `mensura serve` (M7) accepts zoned RFC 3339 on all
  transports or narrows the profile per endpoint.

## Forward references

- `docs/decisions/0014-scalar-domain-taxonomy.md` (the taxonomy this
  extends; its open date-arithmetic question closes here).
- `docs/decisions/0026-dimensional-physical-units.md` (durations,
  the affine footnote, the normalization-at-ingestion precedent).
- `docs/decisions/0034-typed-ingestion.md` (the decoder that
  enforces decision 4).
- ADR 0037 (streaming windows; extents are `diff` of the order key's
  domain).
- `docs/language/06-expressions.md`, `09-typing-reference.md`,
  `11-physical-units.md` (rules and cross-references on
  implementation).
