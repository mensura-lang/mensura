# Windows over time

Two different things get called windows, and the difference matters enough to
name up front.

The [Views](../transforming/views.md#windows-the-other-shape-of-map_bags)
chapter walked a bag **in order**: a running maximum, the previous reading, a
rank.  Those windows are about *rows* and their positions, and they answer
"what came before this one".

This chapter cuts **time** into intervals.  These windows are about the clock,
and they answer "what happened between ten and quarter past", including the
answer no positional vocabulary can give: *nothing happened*.  A reading's
predecessor always exists somewhere in the bag; a quarter-hour with no
readings is a quarter-hour with no readings, and after this chapter and the
[next](finality.md) you can say so.

## The operation

`window w p size stride` replicates each row into every window that contains
its point `p`, and adds the window's start as a fresh key column `w`.

```mensura
{{#include ../examples/window-tumbling.mensura}}
```

`w` is a column the operation *creates*, so it must not already exist; `p` must
exist, and it stays where it was.  A row with point `p` lands in every window
`w` where `w <= p < w + size`.

The extents are compile-time constants of the point's difference type: for an
`instant`, durations like `15.0 * si.minute` from the bundled `si` module.
Note `15.0` and not `15`: `int` and `real` do not mix, and a duration is a
real quantity.  (Dimensioned attribute types get their own chapter; here the
durations are only arguments.)

## Where the windows are

**Window starts lie on the stride grid anchored at the domain's zero**, which
for an `instant` is the epoch.  Nothing is declared and nothing depends on the
data: a fifteen-minute grid lands on `:00`, `:15`, `:30`, and `:45` past each
UTC hour, so a reading at `10:07:31.221Z` gains exactly one window start,
`10:00:00.000Z`.

Two consequences worth knowing before you are surprised by them.  The alignment
is to UTC rather than to plant-local time, because an `instant` is a moment on
the physical timeline and carries no zone.  And because placement is arithmetic
rather than a decision, two runs of the same program over the same data agree,
which is what lets a window's result be published.

## Tumbling, sliding, and gaps

There is one operation, not a family.  What the arguments say:

- `stride == size`: **tumbling**.  The windows tile the timeline, so each row
  lands in exactly one, as in the example above.
- `stride < size`: **sliding**.  The windows overlap and each row lands in
  `size / stride` of them.

```mensura
{{#include ../examples/window-sliding.mensura}}
```

- `stride > size`: gaps between the windows, and a row whose point falls in
  one lands in no window at all.  This is legal, and occasionally exactly what
  is wanted: sampling one minute in each hour.

## The key grows, and so does what the compiler knows

`w` joins the key, so the table is now keyed `(machine_id, taken_at, w)`, and
the uniqueness fact grows with it.  That is not bookkeeping; it is what keeps
the *other* kind of window free of ceremony.

Recall from [Views](../transforming/views.md) that a scan over a bag demands an
unambiguous order, discharged either by construction or by a visible
`assume { arranged }`.  Because the reading time was part of the identity and
`window` extended that fact rather than resetting it, ordering inside a
window's bag is still provable:

```mensura
{{#include ../examples/window-order.mensura}}
```

No `assume { arranged }`, exactly as there would be none without the window.
The two kinds of window compose, and the time grid does not cost you the row
order.

`lag` was chosen there for a second reason worth noticing: it relates rows
that are present, so it demands no completeness, and the view carries no
claims whatsoever.  A `series.running_max` in the same position would demand
completeness, because its last row *is* the maximum and every row folds the
readings so far.  That demand is the subject of the next chapter, which
answers it with a mechanism rather than a claim.

## An empty window is not a row

`window` **replicates rows**.  Where there is no row, there is nothing to
replicate, so a window that contains no readings does not appear in the output
at all.  Absence is the representation.

This is correct for the operation and it is a trap for a reader, because it
makes `#b` never yield zero in a windowed view: the quarter-hours in which a
machine reported nothing are not rows, so no filter over these rows can find
them.  Materializing them is a different operation, `dense`, and it is in the
[next chapter](finality.md), because it needs to know where the grid *ends*
before it can fill it in.

## The newest row per group

One reduction belongs here rather than with the folds, because it is about
time: `latest p` keeps, per bag, the row whose point `p` is maximal.

```mensura
{{#include ../examples/latest-newest.mensura}}
```

It is a reduction, fiber to row, so it demands both facts the ordered
vocabulary demands: the order must be unambiguous (a tie has no single
argmax), and the bag must be whole (the latest of some of the rows is not the
latest).  Here the first is free, because the time is part of the reading's
identity, and the second is the claim the coarsening forced, as in
[Registries](../modelling/registries.md).

**`p` must already be an attribute.**  Writing `readings |> latest taken_at`
with the time still in the key is rejected, and the diagnostic names the fix:
write the coarsening out, `demote taken_at`, and the completeness claim then
has somewhere to stand.  Fusing the two would leave that demand with no place
to go, since a claim before the coarsening is a claim at the wrong key and a
claim after it comes too late for the fold.  After `closed`, in the next
chapter, both facts hold by mechanism and neither claim is written at all.
