# Serving a program

`mensura serve` runs a program as a service, the first slice of M7
(`ROADMAP.md`) and the realization of
`docs/decisions/0045-serving-a-program.md`.  This document says what the
subcommand does, where the runtime changed to support it, and what the
first slice leaves for the next.  The ADR is the specification; this is
the map of the code.

## Scope of the slice

One process over an embedded SQLite file, running all three roles.  That
is the all-in-one deployment of ADR 0045 decision 1, and the degenerate
case of the same design that later runs replicated over Postgres.  The
slice ships:

- positioned writes and the write log (decision 4), the identity check
  for re-sent rows (decision 11), and the schema guard (decision 9), in
  `mensura-runtime`;
- views materialized at a log position by compare-and-set (decision 5),
  which `mensura run` now also records;
- the runtime configuration (decision 10), the interim credential model
  (decision 8), and the HTTP projection of the four operations
  (decisions 3, 6, 7), in a new crate `mensura-serve`;
- `mensura serve --config <file>` in the CLI.

Not in the slice: the Postgres backend (which is what makes the roles
separable), the materializer lease (trivial in one process), typed
migrations, subscriptions, and pagination or filtering on reads.

## The runtime: positions and the log

Every change to a store or registry is a **positioned write**.  The
backend opens an immediate transaction (SQLite's serialization point),
takes the next position as one past the log's maximum, runs the intake
checks, applies the change, records one entry in the reserved table
`mensura_log` (position, operation, target, rows appended, deleted, and
skipped, principal), and commits.  Positions are therefore gapless in
every committed snapshot, which is what gives "the database at position
`N`" a meaning.

The intake checks run in this order: the identity check, the lateness
contract, the inserts.  On a keyed table, a row equal column for column to
the row stored at its key is dropped from the batch and counted as
skipped; a row whose key is stored with different content rejects the
batch with a key conflict naming the key.  The check runs ahead of the
lateness contract so that a re-sent old reading is a no-op rather than a
late rejection.  A bag table has no key to compare at and gets no check.

`StorageBackend` grows accordingly:

| Method | Role |
|---|---|
| `apply_as(table, delta, principal)` | the positioned write; `apply` delegates with the local principal |
| `advance_floor_as(store, column, point, principal)` | a positioned write too; `advance_floor` delegates |
| `position()` | the latest committed position |
| `materialize_view(view, rows, position)` | compare-and-set: replaces only if the recorded position is lower |
| `view_position(view)` | the position a view reflects, if ever materialized |
| `guard_program(program)` | create the tables and record fingerprints, or verify them |

`LOCAL_PRINCIPAL` (`spiffe://local/service/mensura`) is the identity a
CLI command or a test writes under; a served write carries the caller's.
The raw SQL scaffold tests seed with is a positioned write as well,
because nothing may change a store without taking a position, or a view
materialized afterwards would be refused by the compare-and-set as
already current.

`materialize_views` reads the position first, then every source and
watermark in the same snapshot, and records that position on each view
it writes.  That is prefix consistency: a served view at position `N`
holds exactly what `mensura run` computes over the database after writes
`1..N`.  The serve integration test checks it against an independent
database fed the same writes through the batch path.

## The schema guard

`guard_program` fingerprints each store and registry (SHA-256 over a
canonical rendering of name, kind, cardinality, unit, columns with type,
role, and optionality, `domain` references, and `lateness` contracts; see
`fingerprint.rs`) and keeps them in the reserved table
`mensura_program`.  Over an empty namespace it creates the tables and
records the fingerprints; afterwards it compares, and a mismatch names
every table that differs, is missing from the database, or is missing
from the program.  Views are not fingerprinted: a changed view is rebuilt
by the next materialization, so editing a view and redeploying is always
accepted, and editing a store is always refused.

What makes the rebuild happen is the view's **definition digest**
(`fingerprint::view_definitions`): a SHA-256 over the view's resolved
plan, chained through the digest of each source it reads, so that
editing a view also invalidates every view downstream of it.  Each
materialization records the digest beside the view's position, in the
reserved table `mensura_view_definitions`, and the compare-and-set
replaces a view whose recorded digest differs at any position,
recreating its table so that a changed set of columns applies.  Without
it, a redeploy with no write in between would serve the old
definition's rows, since the position alone says they are current.  The
plan is rendered with `Debug`, which includes source spans, so moving a
view's text invalidates it too; that is conservative, since a spurious
invalidation costs one replacement and never a stale read.  This holds
for `mensura run` over a persistent database as well.

A database created by
`mensura run` before this slice carries no fingerprints; the guard
records them on first start without verifying the existing tables'
shape, which is the one case it cannot check.

## The configuration

A flat TOML file, nothing inferred, no secret:

```toml
program   = "fleet-monitoring.mensura"
database  = { env = "MENSURA_DATABASE_URL" }
namespace = "fleet"
roles     = ["intake", "read", "materializer"]
listen    = "127.0.0.1:8080"

[limits]
max_body_bytes    = 8_388_608
min_position_wait = "5s"

[[principal]]
id           = "spiffe://local/device/sensor/0b8e2f6a"
token_sha256 = "9f86d081884c7d65..."
scopes       = ["write:readings"]
```

`program` is relative to the configuration file.  `database` is a secret
reference, `{ env = ... }` or `{ file = ... }`, resolved once at startup;
a literal is a load error.  The resolved address is `sqlite:<path>` or a
bare path (`:memory:` is allowed); a Postgres address is refused with a
diagnostic naming the next slice.  `roles` defaults to all three, and a
proper subset over SQLite is refused (decision 2).  A non-loopback
`listen` without a principal is refused (decision 8).  A principal's
scopes are `read:`, `write:`, or `floor:` over a kebab-case resource, and
`<op>:all` is the wildcard for each.

## The HTTP projection

| Request | Operation | Scope |
|---|---|---|
| `POST /<name>` with a JSON Lines body | `append` | `write:<name>` |
| `GET /<name>`, optionally `?min_position=N` | `read` | `read:<name>` |
| `POST /_mensura/floors/<name>/<column>` with `{"point": ...}` | `advance_floor` | `floor:<name>` |
| `GET /_mensura/health` | `health` | none |

A declared name projects to one kebab-case segment
(`docs/language/05-naming-and-casing.md`).  An `append` answers
`{"position", "appended", "skipped"}`; a `read` answers a JSON array of
name-keyed records in key order, the encoder being the decoder's inverse
(an instant in normalized UTC, a quantity as its base-unit magnitude, an
enum as its literal, a missing value as `null`); both carry the position
in the `Mensura-Position` header.  A `read` of a view never materialized
is the view at position zero, which is empty.  With `min_position`, the
read waits up to `limits.min_position_wait` for the table to reach it
and answers 503 if it does not.  Status codes follow the ADR's table:
400 for malformed JSON Lines, 422 for a record that does not decode, 409
for a conflict, a violated reference, a late row, or a lowered floor,
405 for an append to a view, 404 for an unknown name, 401 and 403 for
credentials, 413 for a body over the limit, 503 for an unreached
position, and 421 for a role this instance does not run.  Every error
body is `{"error": "<the message the CLI prints>"}`.

`health` is open and reports readiness, the roles, the namespace, the
fingerprint, the log position, each view's position, and the views whose
last materialization failed, with the error and the position.

## The materializer

A task that materializes every view, then waits for the next committed
write (an in-process wake after each append or floor advance) or for its
two-second poll, whichever comes first.  A view that fails at runtime
keeps its table and its last position, the failure is recorded for
`health`, and the write that caused it has already committed, so
producers are never blocked by a view.  In this slice the views after
the failing one in dependency order are not attempted in that run; they
keep their last position too and catch up when the failure is gone.

Startup typechecks the program, refusing on any diagnostic exactly as
`check` does, runs the schema guard, binds, prints one line with the
roles, the position, and the fingerprint, and then serves until
`SIGTERM` or `SIGINT`, after which the listener drains and a
materialization in flight runs to completion.

## Validation

- Runtime: positions are gapless and a rejected batch takes none; a
  re-sent row is a no-op and a conflicting one names its key; a re-sent
  old row is a no-op rather than late; a view write is a compare-and-set,
  and a redefined view is replaced at any position and reshaped; a view
  definition digest reaches the views that read it;
  the guard creates, verifies, accepts a view change, and refuses a store
  change naming the table; a record encodes and re-ingests byte for byte.
- Serve: health is open; an append takes position 1 and the served view
  at position 1 equals the batch materialization over the same writes on
  an independent database; credentials and scopes gate every operation;
  each refusal carries its status; a body over the limit is a 413 in the
  same error shape; the floor is a positioned write under its own scope
  that refuses to lower and closes the intake; the guard survives a
  restart over a file; a restart with an edited view serves the new
  definition, columns included, with no write in between; the readiness
  probe of `mensura health` reads a listening instance.

## Forward references

- The Postgres backend behind the same trait, which makes several
  instances meaningful, and the materializer lease with it.
- The deploy configuration and bundle (`ROADMAP.md` M7.2,
  `docs/toolkit/07-deploying.md`), which generates this file per
  instance.
- The MQTT projection of `append` (M7.4) as another caller of the same
  four operations.
- Idempotent appends to a `bag` registry, pagination and filtering on
  reads, and log retention (ADR 0045, open questions).
