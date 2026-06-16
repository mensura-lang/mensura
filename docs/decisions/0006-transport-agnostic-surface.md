# 0006: Transport-agnostic surface

## Status

Proposed.

## Context

A Mensura system needs several ways to interact with the outside world: a
high-volume ingest path for devices (MQTT), a query path for consumers
(GraphQL, REST), an RPC path for function calls (gRPC), and a way to push
live results out as data streams in.  `proposal.md` bakes one transport into
the language: a `store` or `collect` *means* an auto-generated REST endpoint
(`endpoint: "/people"`).  `iiot.md` bakes in another: device ingestion
*means* MQTTS.

If each construct carries its transport, the language acquires a wire
protocol per construct, and adding GraphQL or gRPC means changing the
language.  This ADR settles where transport lives.  Identity and
authorization are settled separately in ADR 0005.  This is M4 design;
nothing here is implemented yet.

## Decision

### The core language is transport-agnostic

The language declares *what*: the surfaces (`store`, `collect`, `view`,
`endpoint`), their I/O contracts (columns, units), and their access control
(`auth {}`).  It never names a wire protocol.  A `store` does not mean REST;
a `collect` does not mean MQTT.

The deployment configuration decides *how*: which transports are live and
which surfaces each one exposes.  This matches the split already chosen for
storage (the backend is abstracted behind a trait) and keeps the type system
about semantics, not wires.  Transports can be added or swapped without
touching a Mensura program.

### One logical surface, many transport projections

Each surface has a natural direction of data flow, and each direction
projects onto the transports that suit it:

| Direction | Construct | Transports |
|---|---|---|
| Ingest (push in) | `collect`, ingestion `endpoint` | MQTT(S), REST `POST`, gRPC client-stream |
| Query (pull) | `store` `GET`, `view` | GraphQL query, REST `GET` |
| Call (RPC) | `endpoint` with a pipeline | gRPC unary, REST RPC-style |
| Subscribe (push out) | `view { refresh: on_change }` | GraphQL subscription, MQTT publish, gRPC server-stream |

Two observations make this clean:

- **GraphQL falls out of the read side for free.**  The resolved schema is
  already a typed, relational graph; `@domain` foreign keys are GraphQL
  edges.  A GraphQL schema is generated, not hand-written, exactly as the REST
  CRUD is in `proposal.md`.
- **Ingest and subscribe are the same primitive in two directions.**
  "Receiving online data" is a stream in; a `refresh: on_change` view (the
  `LiveRUL` example in `iiot.md`) is a stream out.  Both are subscriptions
  over a streaming transport.

### Transport selection lives in a deploy configuration file

Which transports are enabled, on which addresses, with which credentials and
trust anchors (per ADR 0005), and which surfaces each transport exposes, are
all deploy-configuration concerns, read by `mensura serve`.  The language
program is identical across a REST-only deployment and one that also speaks
MQTT and gRPC.

### Wire names are translated deterministically

A surface or field has one canonical Mensura name; each transport projects it
with that transport's idiom (REST kebab-case paths, GraphQL camelCase fields,
and so on).  The translation is deterministic and total, so the same program
yields stable, idiomatic names on every wire.  The canonical naming
convention and the full translation table are specified in
`docs/language/05-naming-and-casing.md`.

## Consequences

Positive:

- Adding or changing a transport (GraphQL, gRPC, a future protocol) is a
  server / deploy change, not a language change.
- One program serves every deployment shape; transports are a deployment
  decision.
- The four interaction needs (feed, consult, call, stream) map onto existing
  constructs without new language surface.

Negative:

- The deploy configuration becomes load-bearing: it owns transport selection,
  exposure, and (with ADR 0005) trust anchors.  It needs its own specification
  and careful defaults.
- Generating idiomatic schemas per transport (GraphQL types, proto messages)
  is real implementation work, deferred to M4 and beyond.
- A surface exposed over several transports must behave consistently across
  them (pagination, error mapping, auth results), which is a cross-transport
  conformance burden.

Neutral:

- The transport list (MQTT, REST, GraphQL, gRPC) is indicative, not closed.
  The architecture admits more without language impact.

## Alternatives considered

1. **Transport per construct** (the source documents' implicit model: `store`
   means REST, device ingestion means MQTT).  Rejected: it couples the
   language to wire protocols and forces a language change per new transport.

2. **Per-surface transport declaration** (`collect ... { transport: [mqtt,
   rest] }`).  Rejected as the default: it leaks deployment topology into the
   program and re-couples source to environment.  May return as an optional
   override if a surface ever needs a transport pinned in source.

3. **A single transport** (REST only, as `proposal.md` assumes).  Rejected:
   the IIoT case needs MQTT ingest and streaming subscriptions that REST
   serves poorly.

## Open questions

- **Deploy-config format and home.**  Where the file lives, its schema, and
  how it relates to the deployment / migration design in `untracked/DEPLOY.md`
  is a toolkit document still to be written.
- **Defaults.**  With no explicit configuration, what does `mensura serve`
  expose (everything over REST + GraphQL, nothing until configured)?
- **Per-surface override.**  Is alternative 2 ever needed as an escape hatch,
  and if so with what syntax?
- **Cross-transport semantics.**  Pagination, partial failure, and
  subscription lifecycle must be specified once and projected, not redefined
  per transport.
- **Streaming model.**  The push-out direction depends on `refresh: on_change`
  views (the streaming layer, M6) and the deferred model layer; the transport
  projection here assumes them but does not specify them.
