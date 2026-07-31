# Registries

A registry is a tabulation of observations of a unit whose data arrives
through an ingestion mechanism rather than through CRUD operations.
Because that mechanism is the sole intake for its observations, a
registry carries a completeness guarantee at the type level that an
ordinary store does not.

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
tables are `Incomplete`: a fold over one key's rows may be folding over
a partial bag.

A registry is written only by appending, and the declaration is the sole
intake for its observations.  Nothing else writes it, and nothing
removes from it.  That is a statement about *mechanism*, and it is what
licenses the type-level fact: what the registry holds for a key is
everything there is.

This is the "properties are derived from mechanism, not declared"
pillar of `00-overview.md` in its clearest form.  The programmer does
not claim completeness; the declaration's intake discipline establishes
it.

## Registry declaration

A registry declaration has the same shape as a store's: a name, a unit
reference, an optional `domain` block, and one or more `attr` blocks.

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
is observed once.  The completeness fact is true but says little on its
own, because a key with at most one row has nothing missing from its
bag.  What it buys is that a reduction over a *coarsened* key stays
free of ceremony: `demote taken_at` propagates the fact to the machine,
and the reducing `map_bags` consumes it with no `assume` in sight.

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

Because the fact holds at the source, everything downstream inherits it
without ceremony.  Tier A operations preserve it, `demote` propagates it
from the fine key to the coarse one, and a reducing `map_bags` consumes
it.  The pipeline that needs an `assume { complete }` over a store needs
nothing at all over a registry:

```
// Over a store: the fold may be folding over a partial bag, so the
// obligation must be discharged before the reducer.
readings |> assume { complete }
         |> demote taken_at
         |> map_bags |k, b| (.max_temperature = bag.max b.temperature)

// Over a registry: established at the source, propagated by `demote`,
// consumed by the reducer.  Nothing to discharge.
readings |> demote taken_at
         |> map_bags |k, b| (.max_temperature = bag.max b.temperature)
```

The uniform rule (complete at either cardinality) is deliberate.  On a
`singletons` registry the fact is the
`Mensura.fiberCompleteWrt_of_functional` corollary and adds nothing a
reducer had not already discharged from cardinality; on a `bag`
registry it is the contentful fact above.  One rule covers both, so the
keyword means one thing wherever it appears.

## Registry versus store

| | `store` | `registry` |
|---|---|---|
| Tabulates a unit | yes | yes |
| `attr` / `attr*` / `domain` / conformance | yes | yes, identically |
| Materializes as a table | yes | yes, identically |
| Readable by a view | yes | yes |
| Written by | create, update, delete | append only |
| Table completeness | `Incomplete` | `Complete` |
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
           |> map_bags |k, b| (.max_temperature = bag.max b.temperature)
}
```

`machines` is a store: a machine is commissioned, its status changes,
and rows are updated in place.  `readings` is a registry: a reading is
appended when it is taken and never revised.  The view reduces each
machine's readings with no completeness ceremony, because the registry
established the fact at the source.

## Forward references and open questions

- **Exposure and `auth {}`.**  Auto-generated ingestion endpoints, the
  permission scopes derived from a registry's name, and the `auth {}`
  block are the web-service work (M7), settled in
  `docs/decisions/0005-identity-and-authorization.md` and
  `docs/decisions/0006-transport-agnostic-surface.md`.
- **Streaming intake.**  Windowed refresh, window closedness, and
  per-window sampling inference arrive with the streaming milestone
  (M5), which is where a registry's observations start feeding
  incrementally refreshed views.
- **A stated relationship between a store and a registry of one unit.**
  Today they are independent tabulations; whether a program ever wants
  to declare that one is the intake for the other is unsettled.
- **Completeness over a coarser key.**  See "What is not in a registry"
  above; shared with `assume { complete }`
  (`docs/decisions/0023-completeness-consumed-by-the-reducer.md`).
