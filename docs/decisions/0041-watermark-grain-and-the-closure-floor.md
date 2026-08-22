# 0041: Watermark grain, the closure floor, and monotone intake

## Status

Accepted, and implemented in the pull request that introduced the
`lateness` contract itself, so the contract never ships with a grain it
is about to lose.  Amends
`docs/decisions/0037-streaming-windows-and-closedness.md` decision 4,
which fixes the watermark as "global to the registry" and files a
per-key variant as an open question, and closes ADR 0038's open
question "Interaction with per-key watermarks", which recorded that the
two must be decided together.

Written **before `closed` ships**, which is the point.  The `lateness`
contract and its watermark have landed
(`docs/toolkit/05-ingestion.md`), but nothing consumes the watermark
yet: `closed` is not implemented, so no view depends on which windows
are final.  The grain is therefore still cheap to change, and under an
append-only intake the observed half of the state is recomputable from
the data.  Once `closed` lands and programs are written against it, the
grain becomes semantics people depend on.

Three decisions in one record, because they are one decision: the grain
forces the floor (without it, an entity that stops reporting can never
be reported silent, which kills ADR 0038's motivating query), and the
value of a zero bound depends entirely on the grain.  Deciding any one
of them alone would be deciding the other two by default.

## Context

ADR 0037 decision 4 defines the watermark as "the maximum point value
the intake has ever accepted on the contracted column", global to the
registry, and the intake rejects a batch containing a row older than
`watermark - lateness`.  Implementing it surfaced two things the ADR
did not state.

**The theorem couples admission and closure.**
`Mensura.closedWindow_stable` (`formal/Mensura/Window/Defs.lean`) has
exactly one `watermark` variable, and it appears in both hypotheses:
`hlate`, which says what the intake still admits, and `hclosed`, which
says which windows may be declared final.  That is not an artifact of
how the lemma is written, it is its content.  Finality here is not
bought by waiting, it is bought by *refusing*: the window is final
because the intake will reject anything that would have landed in it.
Any design that reads one watermark to admit rows and a different one
to close windows is therefore not a tuning choice, it is unsound.

**A single global watermark is wrong for most real deployments.**  The
watermark tracks the fastest reporter, so it conflates independent
delivery paths.  The worked example below runs four fleet scenarios; the
short version is that a global watermark rejects the honest traffic of
any producer slower than the fastest one, makes onboarding a device with
history impossible, and lets one skewed clock poison the whole registry.
ADR 0037's own open question already called this "sound but lossy for an
offline entity"; the scenarios show it is the common case, not an edge.

The obvious repair, a watermark per entity, fails in the opposite
direction, and the failure is not obvious until you look at what
consumes closedness.  An entity whose sensor dies never advances its own
watermark, so its last partial window never closes, `dense` never fills
its grid slots, and "how many intervals was this machine silent" becomes
unanswerable by construction.  That query is the reason ADR 0038 exists.

So the two candidate designs answer different questions.  A global
watermark asks "has the system moved past this window?"  A per-entity
watermark asks "has this entity moved past this window?"  Silence is
observable only if the first question can be answered without the
entity's cooperation, and data is preserved only if the second governs
what the intake accepts.  This ADR keeps both by separating *what the
data says* from *what the deployment asserts about time*, while keeping
them combined into the single value the theorem requires.

## Decision

### 1.  One watermark per grain, read by both admission and closure

The intake's admission rule and the `closed` stage read the **same**
watermark value at the same grain.  Mixing them is rejected as unsound,
not merely as inconsistent, and the counterexample is worth recording
because the mixed design is the tempting one (per-entity admission so
nobody's honest data is refused, global closure so silence stays
countable):

> Global watermark 10:31, machine M-19's own watermark 10:07, bound ten
> minutes, fifteen-minute windows.  Closure on the global reading:
> `10:00 + 15 min + 10 min = 10:25 <= 10:31`, so the 10:00 window is
> final.  Admission per entity for M-19 rejects only below
> `10:07 - 10 min = 09:57`.  A row at 10:10 arrives, is admitted, and
> lands in a window already reported final.

Everything below follows from this constraint.  It also means the ADR
0037 open question cannot be resolved "at the intake" alone: whoever
picks the admission grain has picked the closure grain.

### 2.  The grain is the key minus the contracted column

The **watermark grain** of a contract on column `p` is the registry's
declared key with `p` removed.  One watermark exists per distinct value
of that residual key.

This is uniform over the two shapes, which is what makes it the right
rule rather than a convenient one:

- point in the key (`unit Reading { machine_id, taken_at }`, contract on
  `taken_at`): the grain is `{machine_id}`;
- point as an attribute of an entity-keyed bag registry (`attr*` on
  `unit Reading { machine_id }`, contract on `taken_at`): the grain is
  `{machine_id}` again.

So the attribute-versus-key question, which is otherwise a real
distinction (only a key-borne point seeds an ADR 0024 grading, is
duplicate-free by storage, and is total by construction), does **not**
affect the watermark.  A registry whose key is exactly the contracted
column has an empty residual key and therefore exactly one grain, which
degenerates gracefully to ADR 0037's global watermark; that is the
count-based `ticks` shape of the corpus.

**Why not a coarser, declared grain.**  The delivery contract really
belongs to the *path*, not to the entity: twelve gateways serving two
hundred machines means the ten minutes is a property of a gateway, and
per-machine watermarks are then too fine, because machine A's arrival
cannot vouch for machine B even though the shared pipe has demonstrably
advanced.  Too fine is nonetheless the safe direction: it never refuses
honest data, it only closes later, and decision 3's floor is what
recovers the liveness that lateness costs.  A declared coarsening (a
`by { gateway }` clause) is the natural extension and is deferred to a
consumer that actually models the gateway, in the spirit of ADR 0038
decision 3: the policy inputs of a temporal mechanism are supplied,
never inferred.

### 3.  The effective watermark is the maximum of the observed value
and a declared floor

Per grain and contracted column:

```
effective  =  max(observed, floor)
```

- **observed**: the maximum point the intake has accepted in this grain.
  It is what the data says.
- **floor**: a declared point below which the deployment asserts the
  world is closed.  It is what the operation asserts about time.

**The observed half is derived, not stored; only the floor is stored.**
Under an append-only intake the observed watermark *is* a projection of
the table (`max(point)` grouped by the grain), so storing it separately
would buy a drift risk and an encoding problem and pay for them with
nothing.  The encoding problem is the decisive one: a stored per-grain
watermark has to key on a heterogeneous tuple whose columns and types
differ per registry, which degenerates into a serialized blob key or a
table per registry, while a derived one compares the registry's own
columns with their own types.  Derivation also makes the value travel
with a backup or a replica, removes any write contention on a per-grain
row, and is cheap where it matters: for a key-borne point the primary
key already indexes `(grain..., point)`, so the maximum is a range scan
to the end of a key segment.

ADR 0037 decision 4 gave three reasons for keeping the watermark as
metadata, and none survives the regraining: "independent of downstream
reads" is equally true of a derived value, "O(1)" is near-moot given the
index that already exists, and "well-defined for an empty registry" is
*identical*, since a maximum over no rows is absent in the same way a
missing row is, with one fewer state to represent.  That sentence of
ADR 0037 is amended here.

The floor, by contrast, is irreducible: it is an operator assertion, so
nothing derives it.  It is also, conveniently, the half with **no
grain** (it is declared per registry and column and applies to every
grain), so the state that must be stored is exactly the state with no
encoding problem.

**What derivation costs.**  A derived maximum is monotone in what is
*present*, while the contract is about what was ever *accepted*, so
deleting rows could in principle walk it backwards.  Three cases, and
only the third bites: retention deletes the oldest rows, which does not
move a maximum at all; deleting the newest rows is a rollback that the
append-only declaration already forbids and that invalidates published
finality however the watermark is kept; erasing a whole grain drops it
to absent, which reopens admission for that grain but **not** closure,
because closure reads the floor.  The floor is therefore the safety net
for the one real case, which is a second job it does for free.

Admission rejects a batch containing a row with `p < effective -
lateness`; `closed` keeps a window when `w + size + lateness <=
effective`.  Both read one value, so decision 1 is respected by
construction, and **the floor needs no new theorem**: it enters as the
watermark, so `closedWindow_stable` applies unchanged at the effective
value.

The floor applies to **every** grain, including one that has never been
observed.  This is deliberate and it is the one genuinely uncomfortable
consequence.  It is what makes a never-reporting machine's grid fillable
(ADR 0038's `M-19` gets its sixteen silent slots), and the price is that
loading a new device's historical data below the floor is refused.  That
price is the correct one: backfilling below the floor is exactly the
operation that invalidates finality already published, so it belongs to
the same deliberate-override family as relaxing a `lateness` bound
(ADR 0037's evolution open question), not to ordinary ingestion.  Load
history before declaring the world closed past it, or override
explicitly.

### 4.  The floor is stored state, not program text and not the clock

The floor is backend metadata beside the watermark, advanced by an
explicit operator action (`mensura watermark advance <registry>
<column> <point>` in spirit; the surface belongs to the implementation
slice and eventually to `mensura deploy`).  It is **not** a constant in
the source, because advancing time would then require editing and
recompiling a program, and it is **not** the wall clock, which ADR 0037
alternative 6 rejected for making `mensura run` non-reproducible.

Storing it keeps that purity property intact in the form ADR 0037 bought
it: `mensura run` remains a function of the database, and two runs over
the same database agree.  Advancing the floor is a write, exactly like
ingestion is a write, and it lands in the same audit surface.

The floor is therefore the *pure* counterpart of an idleness timeout.
Real systems reach for the wall clock here (Flink combines per-partition
watermarks with a minimum, then rescues stalled sources with a
wall-clock idleness timeout); this is the same mechanism with the clock
made explicit and reproducible.  There is no free lunch available:
distinguishing "silent" from "has not reported yet" requires a clock not
derived from the data, and the only real choice is whether that clock is
declared or read from the operating system.

### 5.  A zero bound is legal and means monotone intake

`lateness { taken_at: 0.0 * si.second }` (and `lateness { seq: 0 }` for
an `int` point) is accepted.  The positivity check of ADR 0037's worked
example relaxes to non-negativity; a negative bound stays rejected,
since it would demand every row be strictly *ahead* of the watermark and
has no consumer.

Zero needs no new machinery because it is the family's endpoint, not a
new feature: the rule "reject when `p < effective - bound`" at `bound =
0` reads "reject when `p < effective`", which is monotone intake.  Two
precisions:

- **Weakly** monotone.  Equal points are admitted, because "older than"
  is strict.  Strictly increasing is not expressible here and should not
  be.
- **Per batch, not per row.**  Every row is checked against the
  effective watermark as of intake, never against its siblings, which is
  what keeps a batch an unordered bag rather than making file order
  semantic.  Two rows inside one batch may be in any order.

The grain is what makes zero worth having.  Globally, a zero bound
demands that the entire registry arrive in one globally sorted stream
across all producers, which is essentially unimplementable; per grain it
says "each machine's readings arrive in order", which many devices
honour and many pipelines can guarantee.

No new claim word and no new syntax: this is the existing bound at its
endpoint, in the spirit of ADR 0037 alternative 4's refusal to multiply
spellings.

### 6.  Formal obligations

Small, and all in `formal/Mensura/Window/Defs.lean`:

1. `closedWindow_stable` generalizes from a fixed `watermark : P` to a
   watermark indexed by the grain (`watermark : G → P`, with both
   hypotheses reading `watermark (grain k)`).  The proof body already
   uses `hlate` only at the conclusion's own key, so this is a signature
   change rather than a new argument.
2. A lift through `demote`: per-key fiber stability transports to the
   coarsened key, since the coarse fiber is a finite union of unchanged
   fibers and `demote` is a union homomorphism (`demote_unionHom`).
   ADR 0037's statement stops at the windowed key; the surface pipeline
   demotes before `closed`, so the lift is what the checker rule
   actually cites.
3. Nothing for the floor (decision 3) and nothing for the zero bound:
   the floor enters as the watermark, and the theorem carries no
   positivity hypothesis on `lateness`, so both are instances of what is
   already proved.  Recorded because "no new theorem" is a claim worth
   checking rather than assuming.

### 7.  Migration

Almost nothing, because of decision 3's split.  The observed watermark
is derived, so there is no metadata to regrain and no backfill to run;
the stored state is one floor per registry and column, which is the
shape the shipped table already has.  The watermark rows written under
the global rule become dead state and are dropped.  The floor starts
absent, which is the identity element of `max`.

Nothing regresses for a user, and it is worth being precise about why:
the admission change is strictly **permissive** (rows a global watermark
rejected may now be accepted, and nothing was built on the rejections),
and the closure change has **no installed base**, because `closed` has
not shipped.  This is the entire argument for deciding the grain now
rather than with the refresh slice.

## Worked example

The plant of ADR 0037: two hundred machines behind twelve gateways, a
ten-minute delivery bound, fifteen-minute windows.  Four scenarios, with
the current design (global), the naive repair (per grain, no floor), and
this ADR (per grain, with floor).

**M-19's sensor dies at 10:07.**  Its 10:00 window is partial and no
further reading will ever come.
- Global: the fleet advances, 10:25 passes, the window closes with seven
  minutes of data, and `dense` fills 10:15, 10:30, ... with `n = 0`.
  Silence is countable.  Correct.
- Per grain, no floor: M-19's watermark is stuck at 10:07, so its
  windows never close and its silence is invisible.  **Broken**, and
  this is the case that rules the naive repair out.
- This ADR: the floor advances to 10:40 with the deployment's ordinary
  cadence, so `effective = max(10:07, 10:40)` closes the window.
  Correct.

**M-19 is alive but its gateway is partitioned from 10:00 to 10:40,
then flushes its buffer.**
- Global: at 10:25 the 10:00 window was already reported final without
  M-19's readings; at 10:40 the flush carries rows from 10:05, older
  than `10:39 - 10 min`, so the batch is rejected whole.  The type
  system's claim survives (those rows were never accepted, so
  arrival-completeness holds at the boundary ADR 0033 draws), but the
  published peak for M-19 is wrong and the data is gone.  **Lossy.**
- Per grain: M-19's effective watermark is its own, so the 10:05 rows
  clear `10:07 - 10 min` and are admitted, and its 10:00 window has not
  been closed, because closure reads the same value.  Correct, provided
  the floor has not been advanced past 10:15, which is exactly the
  deployment saying "I have published these windows as final".

**Onboarding M-42 with a month of history from its SD card.**
- Global: every historical row is older than `now - 10 min`.  Rejected.
  **Onboarding is impossible without dropping the contract.**
- This ADR: M-42 has no observed watermark, so if the floor does not yet
  reach that history the load succeeds.  Where the floor does reach it,
  the refusal is decision 3's deliberate one and wants the explicit
  override, because those windows were reported empty.

**A device's clock is three days fast.**
- Global: it drags the registry's single watermark three days forward,
  and every other machine's honest traffic is then refused.  The blast
  radius is the whole registry.
- Per grain: it advances only its own watermark, so it corrupts only its
  own stream.  The forward-skew hazard is contained rather than solved;
  the symmetric intake-side bound stays open below.

## Consequences

Positive:

- The admission rule finally matches the shape of the promise it
  enforces: a delivery bound is a property of a producer, and the
  watermark is now per producer.
- Silence stays countable, so ADR 0038 keeps its motivating query and
  its worked example, which the naive per-key repair would have taken
  away.
- Backfill and onboarding become expressible, which they were not.
- Clock skew is contained to one grain.
- `mensura run` stays pure and reproducible, with the operational clock
  explicit rather than ambient.
- The attribute-versus-key question stops mattering for the intake,
  leaving it a question about gradings and duplicates, where it belongs.

Negative:

- One more piece of operational state, and one more thing to get wrong:
  a floor never advanced means windows never close, and a floor advanced
  too eagerly means data refused.  The failure is at least loud in both
  directions.
- Backfill below the floor is refused by design, and the override is not
  designed here.
- An attribute-borne point on a bag registry needs an index over
  `(grain..., point)` that the storage mapping does not create today.  A
  key-borne point needs nothing, since its primary key already is that
  index.
- The observed watermark now depends on what is present rather than on
  what was accepted, which is the deletion caveat of decision 3.  It is
  bounded (only whole-grain erasure reaches it, and only for admission),
  but it is a real weakening of an invariant that stored metadata held
  unconditionally.
- Admission reads the table it is about to write.  The read is a keyed
  maximum per touched grain rather than a scan, but it is no longer a
  single metadata lookup.

Implementation:

- `mensura-types`: the resolved `Lateness` carries its grain (the key
  minus the contracted column), computed once where the schema is
  resolved rather than re-derived at every use; the bound's positivity
  check relaxes to non-negativity.
- `mensura-runtime`: admission derives the observed maximum per touched
  grain inside `apply`'s existing transaction, reads the floor beside
  it, and compares against the effective value.  Nothing is written to
  metadata by an append.
- `mensura-cli`: the command that advances a floor.  It refuses to move
  one backwards, since lowering a floor reopens windows and belongs to
  the same deliberate override as relaxing a bound.
- `formal/`: decision 6, items 1 and 2.
- Examples: a companion to the fleet file showing the grain of each
  declaration shape, and corpus cases for the zero bound.

## Alternatives considered

1. **Keep the global watermark** (ADR 0037 decision 4 as written).
   Rejected: the four scenarios show it refuses honest traffic from any
   producer slower than the fastest, makes onboarding impossible, and
   gives one skewed clock registry-wide reach.
2. **Per-entity watermarks, no floor** (the naive repair, and the
   Flink-style minimum without idleness detection).  Rejected: a dead
   entity's windows never close, so `dense` never fills its grid and
   ADR 0038's silence query is unanswerable by construction.
3. **Per-entity admission with global closure.**  Rejected as
   **unsound**, per decision 1's counterexample.  Recorded prominently
   because it is the design everyone reaches for first, it looks like it
   gets the best of both, and its failure is invisible until a late row
   lands in a window already published as final.
4. **A wall-clock watermark.**  Rejected again, on ADR 0037 alternative
   6's grounds: it makes `mensura run` non-reproducible.  Decision 4's
   floor is its reproducible counterpart, and the comparison is the
   argument for the floor rather than against the clock.
5. **The floor as a `mensura run` flag** rather than stored state.
   Rejected: it makes two runs over the same database disagree, which is
   precisely the property ADR 0037 decision 4 bought by refusing the
   wall clock.
6. **The floor as a constant in the registry declaration.**  Rejected:
   advancing time would mean editing and recompiling the program, and
   the value is deployment state rather than program text.
7. **A declared grain (`by { gateway }`) instead of the residual key.**
   Deferred, not rejected: it is the right answer when the delivery path
   is modelled, and the residual key is the right default until it is,
   because being too fine only costs liveness, which decision 3 restores.
8. **A distinct spelling for monotone intake** (`monotonic`, or a
   separate block).  Rejected per decision 5: zero is the existing
   bound's endpoint, and a second spelling for a point already in range
   spends ADR 0035's audit discipline for no new meaning.
9. **Storing the observed watermark per grain** (ADR 0037 decision 4's
   metadata, regrained).  Rejected per decision 3: it stores a
   projection of the data, so it can drift from it, and its key is a
   heterogeneous per-registry tuple that has to be encoded into a blob
   or a table per registry.  Both problems disappear when the value is
   computed from the columns that already hold it.  A backend that
   later wants to cache the maximum may do so, since caching a
   derivable value is an optimization rather than a semantic choice;
   what it may not do is treat the cache as the source of truth.

## Open questions

- **Who advances the floor, and how it is audited.**  The command's
  surface, whether a deployment advances it on a schedule, and how it
  interacts with `mensura deploy`'s migration policy.  Shares an owner
  with ADR 0037's `lateness`-evolution question, since both are
  "retract published finality" operations and should get one override
  story.
- **Backfill below the floor.**  Decision 3 refuses it; the deliberate
  override is not designed.  The first real consumer is onboarding a
  device with history into a registry already reporting.
- **`closed` demands the grain in the key.**  *Settled while
  implementing (owner, 2026-08-16), recorded here because ADR 0037
  predates the grain and says nothing about it:* to filter a row,
  `closed` must know that row's grain, so the contract's grain columns
  have to still be in the current key at that point, and the stage
  rejects otherwise with a diagnostic naming them.  The alternative, a
  minimum over all grains where they are absent, is sound but silently
  conservative: one stalled producer would hold back every window in the
  view with nothing in the program saying so.  Explicit refusal matches
  how every other demand in the language behaves.
- **A per-grain floor.**  The floor here is per registry and column,
  applying to every grain.  A per-grain floor (retiring one machine
  without touching the rest) is expressible in the same state and has no
  consumer yet.
- **Forward clock skew** (inherited from ADR 0037, now scoped).
  Contained to one grain rather than the registry, but a device with a
  fast clock still refuses its own subsequent honest traffic and closes
  its own windows early.  The symmetric intake-side bound remains the
  fix and remains deferred.
- **Coarser declared grains** (alternative 7), which want the delivery
  path in the data model.
- **Rejected-batch disposal** (inherited from ADR 0037, unchanged): the
  mechanism argument needs only "never accepted", and whether a rejected
  batch is destroyed, logged, or quarantined is operational policy.
- **Cost at a large entity population.**  One watermark row per grain is
  fine for a fleet and unexamined for a registry keyed by user; whether
  it wants a different storage strategy is an implementation question
  with no data yet.

## Forward references

- `docs/decisions/0037-streaming-windows-and-closedness.md` (decision 4,
  whose global watermark this amends, whose "maintained by the backend
  as registry metadata" this replaces with derivation, and whose per-key
  open question this closes; the evolution question this ADR's override
  shares an owner with).
- `docs/decisions/0038-rectangularization-over-the-window-grid.md` (the
  silence query decision 3's floor protects, and the open question on
  per-key watermarks this closes).
- `docs/decisions/0033-registry-declarations.md` (the sole-intake
  mechanism the contract rests on, and the arrival-completeness boundary
  the second scenario stays inside).
- `docs/decisions/0036-temporal-domains-and-torsor-arithmetic.md` (the
  point domains and difference types a bound and a floor are written
  in).
- `docs/toolkit/05-ingestion.md` (the enforcement this regrains).
- `docs/language/13-registries.md` (the user-facing contract).
- `formal/Mensura/Window/Defs.lean` (decision 6).
