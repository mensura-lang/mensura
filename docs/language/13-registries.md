# Registries

A registry is a tabulation of observations of a unit whose data arrives
through an ingestion mechanism rather than through CRUD operations.
Because that mechanism is the sole intake for its observations, a
registry carries a completeness guarantee at the type level, at its own
declared key, that an ordinary `attr*` store does not.

This document defines what a registry is and how it is declared.  It is
the companion of `02-stores.md`, which defines the store it varies, and
of `01-units.md`, which defines the unit both tabulate.  The intake
itself (the typed decoder, the `mensura ingest` subcommand, the write
path) is a toolkit concern, specified in `docs/toolkit/05-ingestion.md`.
The API surface a registry may expose (endpoints, authentication,
permissions) is part of the web-service work and is out of scope here.

Decided in `docs/decisions/0033-registry-declarations.md`.

## What a registry is

A store and a registry both tabulate observations of a unit, and both
materialize as the same kind of table.  What differs is how
observations get in, and what the type system may therefore conclude.

A store is written through ordinary create, update, and delete
operations.  Rows may be added at any time by anyone with permission, so
at any moment a store holds *some* of the observations that exist.  Its
`attr*` tables are `Incomplete`: a fold over one key's rows may be
folding over a partial bag.  (An `attr` table is trivially `Complete` at
its own key whatever the declaration kind: a present key's single row is
its whole bag.)

A registry is written only by appending, and the declaration is the sole
intake for its observations.  Nothing else writes it, and nothing
removes from it.  That is a statement about *mechanism*, and it is what
licenses the type-level fact: what the registry holds for a key of its
**own declared boundary** is everything there is.  The fact stops at
that boundary: recording every observation received is not receiving
every observation that happened, so it does not survive a coarsening of
the key ("Completeness by mechanism" below,
`docs/decisions/0035-completeness-cleared-by-demote.md`).

This is the "properties are derived from mechanism, not declared"
pillar of `00-overview.md` in its clearest form.  The programmer does
not claim completeness; the declaration's intake discipline establishes
it.

## Registry declaration

A registry declaration has the same shape as a store's: a name, a unit
reference, an optional `domain` block, and one or more `attr` blocks.
One block is registry-only: the `lateness` intake contract, below.

```
registry temperature_readings {
  unit { Reading }
  attr {
    kelvin: real
  }
}
```

Every part behaves exactly as it does in a store (`02-stores.md`).  The
name is what pipelines use to refer to the registry, the `unit { U }`
line says which unit is tabulated, `attr` blocks list the attributes and
may repeat, and a conformance clause (`registry r : SomeShape { ... }`)
claims shapes in the usual way.

The two declarations differ in their introducer word and in nothing the
parser can see inside the braces.  The difference is entirely in what
the intake permits and what the checker may conclude.

## Basic and compound registries

A registry of a compound unit resolves its unit-reference fields in a
`domain` block, exactly as a store does:

```
registry maintenance_events {
  unit { Event }
  domain {
    machine: machines
  }
  attr {
    kind: EventKind
  }
}
```

The rules are `02-stores.md`'s, unchanged: one entry per unit-reference
field, resolution one level deep, transitivity through the store graph,
and the graph must be acyclic.  Registries participate in that graph on
the same terms as stores.

A `domain` entry may target a **`singletons` registry** as readily as a
`singletons` store.  The restriction is on *cardinality*, not on kind: a
singletons tabulation has a per-key row for a reference to land on,
while a `bag` has none.  A `domain` entry naming a `bag` registry is
rejected for the same reason a `bag` store is
(`docs/decisions/0032-compound-keys-flatten-to-dotted-columns.md`).

### References do not affect completeness

A reference in either direction leaves the completeness fact intact, and
it is worth seeing why.  Completeness is *key coverage over the
registry's own population*: every key that should be there is there.  A
`domain` entry constrains **which values a column may hold**, not
**which rows exist**.  The two say different things, so a reference
neither strengthens nor weakens the fact.

Read the guarantee precisely, though.  A registry is complete over **its
own key**, never over its `domain` target's.  The example above is a
complete census of *events*; it does not assert that every machine in
`machines` has one.  A machine with no events is an absent key, not a
partial bag, so a fold over the registry stays sound and simply produces
no row for that machine.  "Every entity in the target store appears
here" is a different claim, and nothing in a registry declaration makes
it.

## Cardinality: `attr` versus `attr*`

A registry declares its cardinality through the attribute block form,
exactly as a store does
(`docs/decisions/0022-observations-as-bags-declared-store-cardinality.md`).
Both forms are available, and the completeness fact reads differently
under each.

A **`singletons`** registry holds at most one observation per key:

```
unit Reading {
  machine_id: string
  taken_at: date
}

registry readings {
  unit { Reading }
  attr {
    temperature: temperature[real]
  }
}
```

Here time is part of the identity, so each `(machine_id, taken_at)` pair
is observed once.  The completeness fact is true but trivial, because a
key with at most one row has nothing missing from its bag.  It does
**not** extend below the reading's own key: an unrecorded reading is an
absent key, and `demote taken_at` merges that absence into the machine's
bag as a gap, so the fact is cleared at the coarsening and a reduction
per machine discharges its own obligation there
(`docs/decisions/0035-completeness-cleared-by-demote.md`).  What a
`singletons` registry buys is the intake discipline itself and, through
the ADR 0024 grading its key seeds, derived tie-freedom for windows.

An **`attr*`** registry is a `bag`, keyed by the entity the observations
are about:

```
registry readings {
  unit { Machine }
  attr* {
    temperature: temperature[real]
    taken_at:    date
  }
}
```

Here the completeness fact is contentful: the registry pins the full set
of observations per machine, so a fold over a machine's readings is
folding over all of them.  This is the reference population that
`docs/decisions/0023-completeness-consumed-by-the-reducer.md` reduces
against, established at the source rather than claimed mid-pipeline.

The `attr` / `attr*` mixing restriction of `02-stores.md` applies
unchanged, as do the ordering and storage consequences recorded there.

## Completeness by mechanism

A registry's table is `Complete` at its declared boundary, whatever its
cardinality.  This is the mechanism arm of the three ways completeness
is established (`07-pipelines.md`, `09-typing-reference.md` section 8):

- **mechanism**: a registry source is complete by construction;
- **check**: `completeness_check { assert ... }` establishes it locally;
- **fiat**: `assume { complete }` admits it, visibly.

The fact holds at the source, **at the registry's own key**.  Row-wise
Tier A operations preserve it (they map whole fibers to whole fibers),
and a reducing `map_bags` at that key consumes it.  It does not survive
a genuine coarsening: `demote` clears it, because an absent fine key
becomes a gap inside a coarse fiber
(`docs/decisions/0035-completeness-cleared-by-demote.md`).  The pipeline
that needs an `assume { complete }` over a `bag` store needs nothing at
all over a `bag` registry:

```
// Over an attr* store keyed by Machine: the fold may be folding over a
// partial bag, so the obligation must be discharged before the reducer.
readings |> assume { complete }
         |> map_bags |k, b| (.max_temperature = bag.max b.temperature)

// Over an attr* registry keyed by Machine: established at the source,
// consumed by the reducer.  Nothing to discharge.
readings |> map_bags |k, b| (.max_temperature = bag.max b.temperature)
```

Below the declared key the two kinds are on the same footing.  A
`singletons` registry of readings demoted to the machine is a possibly
partial bag exactly as a store's would be, and the reduction states its
own claim:

```
readings |> demote taken_at
         |> assume { complete }
         |> map_bags |k, b| (.max_temperature = bag.max b.temperature)
```

The uniform rule (complete at either cardinality) is deliberate.  On a
`singletons` registry the fact is the
`Mensura.fiberCompleteWrt_of_functional` corollary and adds nothing a
reducer had not already discharged from cardinality; on a `bag`
registry it is the contentful fact above, and the one the mechanism arm
exists for.  One rule covers both, so the keyword means one thing
wherever it appears.

## The `lateness` contract

A registry may bound how late a row can arrive
(`docs/decisions/0037-streaming-windows-and-closedness.md` decision 4):

```
registry readings {
  unit { Reading }
  attr { temperature: temperature[real] }
  lateness { taken_at: 10.0 * si.minute }
}
```

The entry names an orderable point column whose domain has a difference
type (`instant` or `int`, ADR 0036 decision 4; `date` waits on
`diff(date)`), and its bound is a const expression of that difference
type: `time[real]` for an `instant` point, checked positive and a whole
number of milliseconds at compile time, `int` for an `int` point.  One
entry per block; a second contracted column is a second block, merged in
source order like repeated `attr` blocks.

The contract it states: once the intake's watermark, the maximum point
value ever accepted on the contracted column, has passed
`t + lateness`, no row with point `t` will ever be accepted.  Like the
completeness fact above, it is **enforced, not trusted**: the intake
rejects a batch containing a row older than `watermark - lateness`,
whole and transactionally, like any other decode failure.  A producer
that breaks its declared delivery bound surfaces at the boundary
instead of corrupting a window already reported as final, which is
exactly what the M5 `closed` stage builds on.

The block is rejected on a plain `store`, deliberately: a store's rows
are created, updated, and deleted by anyone, so a watermark over it
bounds nothing and the contract would be claim-grade.  A store's intake
accepts arbitrarily late rows, and that is the honest behaviour for a
tabulation that accumulates revisable observations.

The watermark is currently one value per contracted column, shared by
every entity in the registry, so a slow reporter is measured against
the fastest one.  That is the part of the design most likely to change
under you: `docs/decisions/0041-watermark-grain-and-the-closure-floor.md`
proposes one watermark per entity (the declared key minus the
contracted column) plus a declared floor, which is what makes a
bound of zero, "this entity's rows arrive in order", a useful thing to
write.

**Changing a declared bound later is not symmetric.**  Tightening one
(a smaller `lateness`) only ever closes windows earlier, so every
result already emitted as final stays final, and it is an ordinary
edit.  Relaxing one (a larger `lateness`) reopens windows that were
already reported as final and retracts their results, so it will
require an explicit annotation at the redeclaration saying what
becomes of the rows the change invalidates.  Neither is enforced yet:
nothing persists a program's previous text to compare against, so today
a redeclaration simply takes effect.  The annotation and the check
arrive with `mensura deploy` and its migration policy
(ADR 0037, open questions).

## Registry versus store

| | `store` | `registry` |
|---|---|---|
| Tabulates a unit | yes | yes |
| `attr` / `attr*` / `domain` / conformance | yes | yes, identically |
| Materializes as a table | yes | yes, identically |
| Readable by a view | yes | yes |
| Written by | create, update, delete | append only |
| Table completeness at `attr*` | `Incomplete` | `Complete` |
| Table completeness at `attr` | `Complete` (trivially) | `Complete` (trivially) |
| May declare `lateness` | no | yes |
| Valid `domain` target | when `singletons` | when `singletons` |
| Importable across modules | yes | **no** |

The last row is the one asymmetry that is not about the intake
directly.  A registry is never importable
(`12-modules-and-imports.md`, `docs/decisions/0027-modules-and-imports.md`
Decision 2): its completeness guarantee comes from being the *sole*
intake for its observations, and importing it into another program would
create a second consumer and silently break that guarantee.  Note that
this is about a **module** boundary; a `domain` edge inside one program
consumes no observations and is unaffected.

A unit may be tabulated by a store and a registry at the same time, on
the same terms as two stores of one unit (`02-stores.md`, "Multiple
stores of the same unit").  They are independent tabulations that happen
to share an identity discipline.

## What is not in a registry

Everything `02-stores.md` excludes from a store, plus:

- **The intake configuration.**  How records reach the registry, in what
  format, over which transport, and under whose authority is not part of
  the declaration.  The local intake is `docs/toolkit/05-ingestion.md`;
  the wire is deployment configuration (M7,
  `docs/decisions/0006-transport-agnostic-surface.md`).
- **A mutability annotation.**  Append-only is a property of the intake,
  not a static property of the declaration.  The language records no
  mutability at all today
  (`docs/decisions/0019-attr-blocks-and-dropped-const-var.md`), and a
  registry does not change that; what makes it append-only is that its
  intake exposes nothing else.
- **A completeness key.**  A registry is complete at its declared
  boundary.  Stating completeness over some *other* key is not
  expressible, and is an open question shared with
  `assume { complete }`.

## Worked example

The fleet's readings, as `docs/examples/fleet-monitoring.mensura` has
them.  Time is part of the reading's identity, so the registry is
`singletons`:

```
unit Machine {
  machine_id: string
}

unit Reading {
  machine_id: string
  taken_at: date
}

store machines {
  unit { Machine }
  attr {
    commissioned: date
    status: MachineStatus
  }
}

registry readings {
  unit { Reading }
  attr {
    temperature: temperature[real]
  }
}

view machine_temperature : Tabular[Machine] {
  readings |> demote taken_at
           |> assume { complete }
           |> map_bags |k, b| (.max_temperature = bag.max b.temperature)
}
```

`machines` is a store: a machine is commissioned, its status changes,
and rows are updated in place.  `readings` is a registry: a reading is
appended when it is taken and never revised.  At the reading's own key
the registry's fact is trivially true and nothing more is needed.  The
view folds *below* that key: `demote taken_at` coarsens to the machine
and forfeits the fact, so the `assume { complete }` states the claim the
fold actually rests on, that no reading a machine produced went
unrecorded.  The registry cannot supply that claim; only the deployment
can.

## Forward references and open questions

- **Exposure and `auth {}`.**  Auto-generated ingestion endpoints, the
  permission scopes derived from a registry's name, and the `auth {}`
  block are the web-service work (M7), settled in
  `docs/decisions/0005-identity-and-authorization.md` and
  `docs/decisions/0006-transport-agnostic-surface.md`.
- **Streaming intake.**  The `lateness` contract and its watermark have
  landed (above); the `closed` stage that consumes them, windowed
  refresh, and per-window sampling inference arrive with the rest of
  the streaming milestone (M5), which is where a registry's
  observations start feeding incrementally refreshed views.
- **A stated relationship between a store and a registry of one unit.**
  Today they are independent tabulations; whether a program ever wants
  to declare that one is the intake for the other is unsettled.
- **Completeness over a coarser key.**  See "What is not in a registry"
  above; shared with `assume { complete }`
  (`docs/decisions/0023-completeness-consumed-by-the-reducer.md`,
  `docs/decisions/0035-completeness-cleared-by-demote.md` alternative 3).
