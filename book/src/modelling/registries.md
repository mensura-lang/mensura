# Registries

A **registry** tabulates a unit exactly as a store does.  Same key, same
attribute blocks, same storage, same reads; one keyword differs, and what it
declares is not structure but *provenance*: this declaration is the **sole
intake** for these observations, and the intake only ever appends.

```mensura
{{#include ../examples/registry-history.mensura}}
```

That is a promise about the world, not about the columns, and it is the kind of
promise only the person writing the program can make.  Nothing in the data
distinguishes a table someone appends to from a table several systems edit.

## What the promise buys

A reducing `map_bags` needs to know that the bag it folds is whole
([Views](../transforming/views.md)): a maximum over some of a machine's
readings is not the maximum, and it looks exactly as plausible.  Over a plain
store you discharge that with a check or a visible `assume`, because a store
accumulates observations that anyone may revise and no one promised are all
there.

A registry discharges it **by mechanism**, at its own declared key.  If this
declaration is the only way a reading gets in, then what the table holds for a
key is everything there is for that key.

The fact reads differently in the two shapes, and both readings are worth
having.  On the history above, keyed `(machine_id, taken_at)`, it is nearly
free: at most one row exists per key, so a present bag holds one row and is
trivially whole.  On a **bag registry**, keyed by the entity with the
observations recurring inside it, the same promise says something with content:

```mensura
{{#include ../examples/registry-bag.mensura}}
```

Here the bag *is* the tabulation, and the intake pins its contents: the samples
this registry holds for a machine are the samples that machine produced.  So
the fold runs with no `assume` at all, and it runs at the registry's own key,
which is the whole difference.

## Where the promise stops

Two one-word edits break the example above, and both should.

Change `registry` to `store` and the fold is rejected: a store accumulates
revisable observations, so its bags may have holes.

Put the sample time into the unit's key and coarsen it back out before the
fold, and the fold is rejected again.  This is the case worth understanding,
because it is the first example on this page:

```mensura,ignore
readings |> demote taken_at
         |> map_bags |k, b| (.peak = bag.max b.temperature)   // rejected
```

The registry's fact holds at `(machine_id, taken_at)`, and `demote` asks for
one at `machine_id`.  Those are different claims.  **Recording every reading
received is not receiving every reading that happened**: a reading the gateway
dropped before it ever reached the intake is an absent *key* at the fine
grain, and the moment the keys merge it becomes a *gap inside* the machine's
bag, which is precisely what makes the fold silently wrong.  The registry
cannot rule that out; only the deployment can.  So the claim is written after
the coarsening, where it bites, and it says something a reader can check
against the world.

There is one thing that can rule it out, and it is not a claim: a **bound on
how late an observation may arrive**, enforced at the intake.  With that, a
window of time can be known finished rather than assumed so, which is what
[Windows over time](../windows/over-time.md) and
[Finality and the grid](../windows/finality.md) are about.

## Registry against store

Neither is the safer choice; they say different things, and the honest one is
whichever matches how the data actually arrives.

|                                     | `store`               | `registry`                    |
|-------------------------------------|-----------------------|-------------------------------|
| Tabulates a unit                    | yes                   | yes                           |
| Rows may be updated in place        | yes                   | no, append only               |
| Several writers                     | yes                   | no, this is the sole intake   |
| Completeness at its own key         | at most one row per key only | by mechanism, at either cardinality |
| Body, storage, reads                | identical             | identical                     |

A machine is commissioned and its status changes, so `machines` is a store.  A
reading is taken once and never revised, so `readings` is a registry.  The
declaration records that difference where the compiler can use it.
