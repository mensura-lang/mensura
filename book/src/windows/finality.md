# Finality and the grid

Every windowed view in the [previous chapter](over-time.md) ended with the same
line:

```mensura,ignore
|> assume { complete }
```

The fold needs to know its bag is whole, the coarsening forfeited that, and
nothing so far could supply it.  Look at what that claim actually says, though,
and it is *false*: it says every window in the table is finished, and the
window covering right now is still filling.  Every earlier row is fine.  The
newest one is a number the data does not yet fix, and re-running the program an
hour later will change it, with nothing in the row saying that it moved or that
it will move again.

This chapter is about earning that fact instead of claiming it, and then about
the windows the data never produced at all.

## Two completenesses

The reason the claim was unavoidable is worth stating precisely, because it is
not a gap in the implementation.

A reducing fold needs an **absolute** fact: this bag is everything there will
ever be.  Nothing about a stream can supply that in general.  Readings arrive
late, gateways buffer through outages, and a machine's history is never
finished while the machine runs.

What a deployment *can* supply is a **bounded** fact: nothing will ever arrive
more than ten minutes late.  That is an operational property of the pipe, the
kind of thing written in a runbook, and on its own it is not what the fold
needs.

The bounded fact becomes the absolute one on a bag whose **whole span lies
below the bound**, and the only bags with a finite span are windows.  That is
why time windows are where this conversion happens, and why no amount of
declaring will let you fold a machine's entire history without a claim:
`readings |> demote taken_at` is one bag per machine whose maximum tomorrow's
reading can still raise, forever.

## Declaring the bound

The bound goes on the registry, beside its attributes, because it is a property
of the intake:

```mensura
{{#include ../examples/closed-final.mensura}}
```

`lateness { taken_at: 10.0 * si.minute }` says: a reading may arrive up to ten
minutes after its time, and no later.  It is not documentation.  The intake
**enforces** it: a batch containing a reading older than the bound allows is
rejected whole, at the boundary, rather than quietly corrupting a window that
was already reported as finished.  A gateway that buffers through an outage and
then flushes an over-age buffer surfaces here, as a rejected batch, which is
where you want to find out.

Only a registry may declare it, and the reason is the same one that makes a
registry's completeness mean anything: a bound is worth nothing without a sole
intake to enforce it.  On a store, where anyone may write arbitrarily late
rows, the block is a compile error rather than a comfortable lie.

### The watermark, per producer

What the bound is measured against is the **watermark**: the newest point the
intake has ever accepted.  A window is finished once

```text
w + size + lateness <= watermark
```

and one detail in that inequality is a design decision rather than an
implementation choice: *whose* watermark.  It is the watermark of the row's own
**grain**, the declared key minus the contracted column, which for `readings`
is one watermark per machine.

So a machine reporting every minute cannot close a slower machine's windows,
and cannot refuse its buffered flush either.  Under a single fleet-wide
watermark it would do both, and the failure would be invisible: the fast
machine's traffic would publish the slow machine's windows as final while its
readings were still in flight.

### The floor, for a machine that stops

A grain's watermark advances only when that grain accepts a reading, so a
machine that stops reporting never advances its own, and its last windows never
close.  Its silence becomes indefinite rather than reported.

The other half of the watermark fixes that: a **closure floor**, a point
through which the deployment asserts the world is closed.  It is not written in
the program, because advancing time would then mean editing and recompiling,
and it is not read from the system clock, because `mensura run` would stop
being reproducible.  It is stored beside the data and advanced by an explicit
action:

```console
$ mensura floor fleet.mensura readings taken_at 2025-01-02T00:10:00Z
readings.taken_at is closed through 2025-01-02T00:10:00.000Z
```

The effective watermark is then `max(observed, floor)`, per grain.  Advancing
the floor is a write, exactly as ingestion is, and it lands in the same audit
surface.  This is the honest version of what streaming systems usually spell as
an idleness timeout against the wall clock: the same mechanism, with the clock
made explicit and the run still reproducible.

## `closed`

With the bound declared, `closed` takes no arguments and needs none: the extent
comes from the `window` stage and the bound from the declaration.  It **drops
every window that can still receive a row** and **establishes completeness** on
the survivors, which is exactly the fact the fold demands.  That is the whole
of the `closed-final.mensura` example above, and its notable feature is the
line that is no longer there.

Three things follow.

**An open window is not an error.**  It is a window whose answer does not exist
yet, and its absence from the output is the honest representation of that.
Nothing is rejected, and nothing is reported early.

**Closed windows are final.**  Rerun after further ingestion and previously
emitted rows are byte-identical; only newly closed windows appear.  This is a
theorem about the intake, not a hope: the bound is enforced, so no row that
could change a closed window will ever be accepted.  It is what lets alerting
treat each row as settled, and what will let the view be maintained
incrementally rather than recomputed.

**Delete the `lateness` block and the program stops compiling.**  Not
"produces worse numbers": stops compiling, because the mechanism is gone and
the diagnostic names the fallback, `assume { complete }`, which you then write
visibly and own.

### `closed` against `assume { complete }`

The two spellings differ by one line and both type-check, so it is worth being
exact about how they differ.  With a machine's watermark at 14:30 today and
daily windows, the `closed` version omits today's row, and the `assume` version
emits it holding the peak of a half-finished day.  Run both again at 18:00 over
the same program and a grown registry: the first is byte-identical, while the
second reports a different peak for the same key.

So these are not a strong mechanism and a weak one.  They are different
statements, one of which is false, and the false one is false at exactly one
row.  That is what makes it dangerous: every past row of the `assume` version
is right.

## The windows that never happened

One question is still unanswerable, and it is the one an operator actually
asks: *in how many quarter-hours did this machine report nothing?*

Those quarter-hours are not rows.  `window` replicates rows, so a window with
no readings never appears, and `#b` therefore never yields zero in a windowed
view.  A filter looking for sparse windows omits the sparsest ones, silently.

`dense` materializes them:

```mensura
{{#include ../examples/dense-grid.mensura}}
```

`dense w machines activated` reads: complete the `w` grid, one row per machine
in `machines` per slot, from that machine's `activated` bound up to the closed
bound.

**Why those two arguments cannot be inferred.**  The population cannot come
from the windowed bag, which by construction knows nothing of a machine that
never sent a row, and that machine is the entire point.  The lower bound cannot
be the earliest window observed, because that would make a sensor that was
offline on day one indistinguishable from a sensor that was not yet installed.
Both are policy, both are stated, and the stage is wordier than its neighbours
for exactly that reason.

**Why it runs after the reduction.**  The obvious design materializes the empty
bags first and lets the fold handle them, and it would cost the guarantee that
a reducing lambda never sees an empty bag: every fold would have to answer for
nothing, `bag.min` and `bag.max` would stop working, and missing values would
appear in types that have no business carrying them.  Filling *reduced rows*
keeps all of that, and computes the same table.  The conclusion that matters
survives either way: a filled row is a **reduced** row, so `n = 0` says zero
readings were reduced, not that a placeholder reading was invented and counted.

### Where the information goes

The filled columns split, and the split is the useful part:

- `n` is `#b`, a fold at `+`, and `+` has an identity, so a filled row's `n` is
  a true `0` and the column stays total.
- `peak` is `bag.max`, and there is no maximum of nothing, so the column
  becomes **optional** and is absent on a filled row.

That absence is the answer to a question the old row shape could not even ask.
An absent `peak` says *no readings*; a present low `peak` says *cold readings*;
before the fill, both were the same row.  Consume the absence with `??` where
a default is a true statement about the domain, or serve it and let the reader
see it.

One trap, because it is easy to write and quiet when it breaks.  Materialize a
sum and a count and divide them downstream, and every filled row divides zero
by zero, which `real` division answers with `NaN` rather than an error.  Both
filled components are *total* zeros, so nothing is optional and nothing
propagates to warn you.  Compute the ratio upstream of `dense`, where the rule
above sends it honestly optional, or let the consumer read `.n = 0` and know
the mean does not exist.

### The count, with no claims left

Now the operator's question, and the reason `dense` records what it built:

```mensura
{{#include ../examples/silence-count.mensura}}
```

Read the last two stages carefully, because their interaction is the point of
the whole chapter.  `demote w` coarsens the key, which normally forfeits
completeness: a window absent at the fine key becomes a gap inside the
machine's bag, so the fold below would be silently wrong.  Here there is no
absent window.  `dense` materialized every slot between the machine's
activation and the closed bound, so the coarse bag is the whole grid, and the
compiler re-derives the fact from that mechanism instead of asking you to
promise it.

This is the only place a genuinely coarsening `demote` gives completeness back,
and it is not a special case bolted on: it holds because over a window grid,
and only there, "are these values a contiguous run" is a question with an
answer.  The stride is a constant the program wrote down, the origin is fixed,
and closedness bounds the run above.  Over a raw order key none of that is
true, which is why there is no general "resample this series" stage: the step
would have to be asserted, and an asserted step is exactly the sort of claim
this design refuses.

The count is honest about time in the same way `closed` is.  It counts silent
intervals **as of the closed bound**, and because filled rows are final, a
rerun only ever grows it.

## What this chapter bought

Two views, one number each, and no `assume` anywhere in either:

- the peak per finished window, which will not change under you;
- the count of intervals in which a machine said nothing, including for a
  machine that has said nothing since it was installed.

Neither is expressible in a schema, and both are the sort of thing a dashboard
usually gets wrong quietly.
