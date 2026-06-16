# 0005: Identity and authorization

## Status

Proposed.

## Context

Two source documents sketch how a Mensura system controls access, and they
do not agree.  `proposal.md` (the college case study) models human users:
an external OAuth 2.0 / OIDC issuer authenticates them, roles are derived
from JWT claims with `when:` predicates, and authorization is permission
strings (`read:people`, `write:registrations`, with `read:all` / `write:all`
wildcards).  `iiot.md` (the industrial example) models field devices: a
two-tier X.509 PKI authenticates them over MQTTS with mutual TLS, identity
is the certificate subject (a `device_id`), and authorization is a
device-type gate (`allow: TemperatureSensor`) plus an ad-hoc `rbac: "admin"`.

The result is two authentication subjects (user, device), two trust models
(external IdP, Mensura-owned PKI), and three authorization spellings
(`auth { permissions }`, `allow:`, `rbac:`).  Before any of the M4
web-service work, these must collapse into one model, or the language grows
two parallel access-control systems that drift apart.

A second, sharper problem surfaced while reconciling them: if users come
from an IdP and devices from a CA, their subject identifiers can collide
(both issuers can mint `"12345"`), and that identifier flows into lineage as
provenance.  A collision silently conflates two principals.

This ADR settles the identity model and the authorization model together,
because they form one chain: a credential establishes an identity, the
identity maps to roles, and roles carry permissions.  Transport selection
(which wire each surface is exposed over) is a separate concern and is
settled in ADR 0006.  This ADR is design for M4; nothing here is implemented
yet.  Syntax shown is illustrative, in the spirit of `proposal.md`, not a
frozen grammar.

## Decision

### Identity: uniform and externally federated

Mensura is an identity *verifier*, not an identity *authority*.  Every
principal, human or machine, presents a signed credential from a *trusted
issuer*; Mensura verifies the signature and the validity window, then
extracts claims.  There is no user/device asymmetry: the two differ only in
which issuer signed the credential and which claims it carries.

A credential is one of two shapes, both already in the source documents:

- a **JWT** (OIDC for humans, OAuth client-credentials for services, or a
  JWT-SVID); or
- an **X.509 certificate** (mutual-TLS for devices and connectors, or an
  X.509-SVID).

Both are structurally the same thing: a signed assertion of identity plus
claims, carrying a validity window, from a trusted issuer.

The former "Mensura-managed PKI" of `iiot.md` and the `development { users }`
block of `proposal.md` are not separate mechanisms.  They are a **bundled
issuer**: a trusted issuer that happens to run in-process, for
self-contained deployments and local development.  Federating with an
external IdP or CA and bundling an issuer are two deployment flavours of one
abstraction.

The one thing Mensura cannot externalize is the **trust anchor**: the set of
trusted issuers and their public keys / CA roots must be configured locally.
This is irreducible (you cannot externalize the root of trust itself) and is
small and static.

### Canonical identity: SPIFFE-style

A principal's identity is the triple **`(issuer, kind, subject)`**, never
the bare subject.  This is how federation normally disambiguates (OIDC keys
on `iss` + `sub`), and it makes cross-issuer collisions impossible by
construction.

The triple is expressed as a SPIFFE-style URI:

```
spiffe://<trust-domain>/<kind>[/<type>]/<subject>
```

- **`<trust-domain>`** is the issuer.  For workloads it is a SPIRE trust
  domain; for humans it is the OIDC issuer's domain.  It must appear in the
  local trust-anchor configuration.
- **`<kind>`** is a closed set: `device | connector | service | user`.  It
  carries the provenance class: `device` attests a primary measurement,
  `connector` attests a relayed (secondary) record.  This preserves the
  primary-vs-relayed distinction `iiot.md` attached to device types, now as
  a parseable segment rather than a construct.
- **`<type>`** is a kebab-case role label, present for `device` and
  `connector`.  It is issuer-asserted and matched by a role predicate as a
  plain string; it does not reverse-map to any Mensura declaration.
- **`<subject>`** is a stable identifier.  For a device it is a UUID, chosen
  so it survives key rotation (the credential rotates; the SPIFFE ID does
  not).  For a human it is a segment-safe encoding (lowercase base32) of the
  OIDC `sub`, since raw subjects routinely contain characters SPIFFE path
  segments forbid.

Examples:

```
spiffe://plant.acme/device/temperature-sensor/550e8400-e29b-41d4-a716-446655440000
spiffe://plant.acme/connector/maintenance-event-logger/cmms-bridge-01
spiffe://backend.acme/service/rul-dashboard
spiffe://college.br/user/mfzwizlsmuqq
```

Mensura adopts the SPIFFE *syntax* universally as its canonical principal-ID
scheme.  It adopts SPIFFE *credentials* (SVIDs) where they fit natively, on
the device and service side, and bridges OIDC humans by mapping their
`(issuer, sub)` into a SPIFFE-shaped URI under a trust domain that stands for
the IdP.  Using the URI scheme costs nothing and is implementation
independent; running SPIRE is operationally heavy and stays opt-in.

The **only** identity-derived fact Mensura persists is the full SPIFFE ID,
stamped into lineage (one string per row).  `kind` and `type` are read by
role predicates and never stored.  This keeps the persisted footprint
minimal and pushes everything else into the credential and the role rules.

### The `device` construct is eliminated

`device TemperatureSensor { temperature: temperature; machine: Machine }`
was doing two unrelated jobs:

1. an **access gate** (`allow: TemperatureSensor`); and
2. a **payload contract** (the fields and their units).

Job 1 collapses into roles plus `auth {}` (see below): the SPIFFE path maps
to a role, the role carries `write:temperature-readings`.  Job 2 is already
the ingestion surface's own columns, which today duplicate the `device`
block.  So `device` is removed:

- Sensor-reading stores become `collect` (append-only, auto-stamped with the
  principal and a timestamp, auto-generated ingestion endpoint).  Their
  columns are the payload contract; their `auth {}` is the gate.
- The pure `endpoint receive_temperature` / `receive_vibration` wrappers
  disappear, subsumed by the `collect` ingestion endpoint.
- `endpoint` survives only for ingestion with side effects
  (`register_machine`, `receive_maintenance_event` with its status update),
  which is the RPC / function-call surface.

A device that measures several quantities is no longer "implements multiple
device types"; it is one role holding several permissions.  This converges
`iiot.md` onto the `store` / `collect` / `endpoint` / `auth` vocabulary of
`proposal.md`.

### Authorization: one `auth {}` block, RBAC core plus bounded ABAC

`allow:`, `rbac:`, and `auth { permissions }` collapse into a single
`auth {}` block.

The core is **RBAC**.  A SPIFFE path maps to one or more roles through
`when:` predicates; each role carries permissions:

```mensura
roles {
  temperature-sensor {
    when: principal.kind == "device"
          and principal.type == "temperature-sensor"
    permissions: ["write:temperature-readings"]
  }
}
```

Device-type gating is therefore just a permission a role holds.  "A
vibration sensor cannot publish temperature" means the vibration role lacks
`write:temperature-readings`.  Permissions are **auto-derived from
resources**: `collect temperature_readings` yields `read:temperature-readings`
and `write:temperature-readings`, so there is no separate permission
vocabulary to keep in sync.

The resource half of a permission scope is the **wire form** of the store
name, not the bare identifier: a scope appears in IdP-issued JWT claims and
OAuth scope strings, the same external surface that sees the REST path
`/temperature-readings`.  So scopes use kebab-case, and Mensura maps a scope
back to its store by the same translation
(`docs/language/05-naming-and-casing.md`).  The mapping is `-` to `_`, which
is bijective because identifiers never contain `-` and a snake_case store name
uses `_` only as its separator, so no scope is ambiguous.

A bounded **ABAC** extension covers instance-level scoping that RBAC cannot
express, such as "a device may only write readings for its own machine".  An
optional `where:` predicate ranges over `principal.*` claims and `row.*`
fields:

```mensura
auth {
  permissions: ["write:temperature-readings"]
  where: row.machine == lookup(principal).machine
}
```

This is the mirror of `@auto(auth.id)` (which writes a field *from* the
principal): the predicate checks a field *against* the principal.  The
predicate language stays small and declarative (comparisons over `principal`
and `row`), not a general policy engine.  Operational, mutable bindings such
as device-to-machine stay in a Mensura-owned store, not in the identity:
identity encodes intrinsic, slowly changing, issuer-attestable facts; Mensura
owns operational state.

## Consequences

Positive:

- One access-control model for every principal.  Humans and machines differ
  only by issuer and claims; roles, permissions, `@auto(auth.id)`, and
  lineage attribution work identically across them.
- Cross-issuer identifier collisions are impossible: the canonical ID is born
  namespaced.  Lineage records "device 12345 as vouched by `plant.acme`",
  which is stronger provenance than a bare ID.
- The temporal `outlives` obligation from `iiot.md` (a certificate must be
  valid at the timestamp of every reading it signs) is unaffected by
  externalizing issuance, because the validity window rides inside the
  credential (`notAfter` / `exp`).  Short-lived SVIDs make the obligation
  tighter and rotation routine.
- The language shrinks: `device`, `allow:`, and `rbac:` are gone, and
  `iiot.md` reduces to the proposal's existing vocabulary.
- Mensura persists a single identity string per row; everything else lives in
  the credential and the role rules.

Negative:

- A trust-anchor configuration (issuer list / CA roots) is mandatory local
  state.  It cannot be externalized.
- Relying on external revocation (OCSP / CRL) couples accept/deny decisions to
  the issuer's availability and freshness.  Mitigated by short-lived,
  self-contained credentials (stapling, short `exp`) and a local deny-list for
  emergencies.
- Field devices need offline verification, which favours self-contained X.509
  / stapled credentials over online token introspection.
- Externalizing issuance adds operational surface (a CA or IoT identity
  platform, an EST / ACME rotation flow).  The bundled-issuer mode keeps small
  and dev deployments free of this.
- Putting `<type>` in the credential means re-typing a device requires
  re-issuing its credential.  Automatic SVID rotation makes re-issuance
  routine, which largely neutralizes the cost.

Neutral:

- The claims-to-roles mapping (`roles { when: ... }`) and the
  identity-to-device-type expectation remain Mensura-owned domain facts.  They
  cannot be externalized, and that is correct: an issuer knows nothing about
  Mensura permissions.

## Alternatives considered

1. **Keep the user/device asymmetry.**  Humans via OIDC, devices via a
   Mensura-owned PKI, as the two source documents have it.  Rejected: it bakes
   two trust models and two authorization vocabularies into the language and
   guarantees drift.

2. **Bundle all identity in-process to control the namespace.**  Mensura mints
   every identifier from one sequence, so collisions are structurally
   impossible.  Rejected: it makes Mensura a credential authority (key
   custody, rotation, revocation, attack surface), and it does not actually end
   federation, because any real deployment already has an IdP for humans and
   would re-introduce the namespacing problem the moment that IdP is trusted.
   Canonicalization on `(issuer, kind, subject)` buys the same collision-safety
   far more cheaply.  Bundling remains available as a deployment mode for
   operational control, not as the collision mechanism.

3. **Per-kind trust domains** (`spiffe://devices.plant.acme/...`,
   `spiffe://users.college.br/...`).  Rejected: a trust domain is a PKI /
   administrative boundary, not a principal kind.  `kind` belongs in the path.

4. **Device-type as a Mensura-owned identity-to-type binding** instead of in
   the credential.  Considered.  It allows re-typing without re-issuance, but
   it adds owned mutable state and a lookup on every authorization.  Rejected
   in favour of type-in-credential, since SPIFFE makes the path the idiomatic
   place for a workload's nature and rotation makes re-issuance cheap.  (The
   device-to-*machine* binding, which is operational and mutable, does stay
   Mensura-owned; only the type moves into the credential.)

5. **A general policy engine (Rego / Zanzibar-style ReBAC)** for
   authorization.  Rejected as the core: too large for the use cases at hand.
   The bounded `where:` predicate covers instance-level scoping without a
   policy runtime.  A richer model remains a future option (see Open
   questions).

## Open questions

- **Exact `where:` predicate language.**  Which comparisons and which
  `principal` / `row` accessors are in the minimum set?  Is a `lookup(...)`
  into an owned binding store in scope, and if so how is it typed?
- **Relationship traversal.**  Do the use cases need "may read any machine in
  the same plant as me", which is beyond attribute comparison and pushes
  toward a relationship model?  Deferred until a concrete case demands it.
- **Role source.**  Roles are derived in Mensura via `when:` over claims.
  Should Mensura also accept roles asserted directly by the issuer
  (`jwt.roles`) without a local mapping, and how do the two compose?
- **Human subject encoding.**  Lowercase base32 of the OIDC `sub` is the
  working rule.  Confirm it against real `sub` shapes, and decide whether the
  raw `sub` is ever retained (the current stance is no: store only the
  canonical SPIFFE ID).
- **Wildcard permissions.**  `read:all` / `write:all` come from `proposal.md`.
  Confirm they survive as the only wildcards and define their interaction with
  auto-derived `read:X` / `write:X`.
- **Relation to qualifiers (ADR 0004).**  Lineage is a `std` qualifier under
  the qualifier mechanism; the canonical SPIFFE ID is the provenance value it
  accumulates.  The exact interface is left to the lineage qualifier spec.

## Cross-cutting changes if accepted

These follow from the decision but are separate tasks.

- The M4 language surface for `auth {}`, `roles {}`, and trusted-issuer
  configuration gets its own document when M4 begins, written from this ADR.
- `iiot.md` is revised to drop `device`, turn the reading stores into
  `collect`, and move access control into `auth {}`.
- The deploy / serve work (ADR 0006) must carry the trust-anchor configuration
  and the per-issuer credential settings.
