# 0045: Serving a program

## Status

Proposed.  Pulls the first slice of `ROADMAP.md` M7 ahead of M5's
refresh half and M6, so that a Mensura program can run as a service
before the rest of M7 is designed.  Realizes
`docs/decisions/0006-transport-agnostic-surface.md` for one transport
(HTTP) and settles its "defaults" open question for that transport.
Adopts an interim credential model that is a strict subset of
`docs/decisions/0005-identity-and-authorization.md` (decision 5 says
exactly which part and how it is superseded).  Reuses, unchanged, the
decoder and write path of `docs/decisions/0034-typed-ingestion.md`,
the lateness contract and closure floor of
`docs/decisions/0041-watermark-grain-and-the-closure-floor.md`, and the
batch materialization of `docs/toolkit/04-processing-layer.md`.  No
language change: no grammar production, no typing rule, no `formal/`
work.

## Context

Everything a running system needs exists as a one-shot CLI command
over a SQLite file.  `mensura run` creates the stores and materializes
the views, `mensura ingest` appends a batch, `mensura floor` advances a
closure floor.  Each opens the database, does one thing, and exits.
What is missing is the process that stays up between those things, so
that a device can post a batch and a dashboard can read a view without
anyone at a shell.

That process is the precondition of every deployment story.  The
direction the project is taking, a `mensura deploy` that compiles a
program and a deploy configuration into a self-contained folder whose
script brings the system up on a local cluster or a cheap cloud
target, has nothing to deploy until there is a `mensura serve`.  So
this ADR fixes what `mensura serve` is, and only that.  The deploy
configuration, the generated bundle, typed schema migrations, and
subscriptions are each their own decision (forward references).

Three constraints shape the answer.

- **The batch semantics is the specification.**  A view's contents are
  a function of the stores' rows and the effective watermarks, and
  `mensura run` computes that function (`04-processing-layer.md`).
  Serving must not introduce a second meaning.  Whatever a client reads
  from a served view is what `mensura run` would materialize over the
  same database.
- **ADR 0006 keeps the core wire-agnostic.**  The program names
  surfaces; the transport projects them.  So the decisions below are
  stated as logical operations first and projected onto HTTP second,
  and a later MQTT or GraphQL projection is a caller of the same
  operations.
- **The target is small.**  One program, one SQLite database, one
  process, at a scale of tens of devices and months of readings.  That
  is the scale a demonstration runs at, and the scale at which a full
  recompute per write costs milliseconds.  Incremental refresh is the
  answer to a slowness nobody has measured yet.

## Decision

1. **`mensura serve` is a long-running host over one program and one
   database, and it is the database's only writer.**

   ```
   mensura serve --config <serve.toml>
   ```

   At startup it typechecks the program, refusing to start on any
   diagnostic exactly as `check` does; ensures every store and
   registry table; runs the schema guard (decision 6); materializes
   every view; and only then reports ready.  It stops on `SIGTERM` or
   `SIGINT` after the write in flight commits or rolls back, which is
   the graceful shutdown a container orchestrator expects.

   While `serve` runs, every write goes through it.  The one-shot
   commands are not forbidden from opening the file (SQLite would
   serialize them), but a write that bypasses the host leaves its
   materialized views stale, which breaks decision 3.  They remain the
   tools for local work and for a stopped deployment.

   One process, one replica.  SQLite has one writer, and a second
   replica would need a shared backend that does not exist.  A
   deployment runs `serve` as a single instance with its database on
   durable storage.

2. **Four logical operations, independent of any wire.**

   | Operation | On | Effect |
   |---|---|---|
   | `append` | a store or registry | as `mensura ingest` |
   | `read` | a store, registry, or view | its current rows |
   | `advance_floor` | a contracted column | as `mensura floor` |
   | `health` | the host | readiness and the program fingerprint |

   `append` and `advance_floor` are the two ways the host's state
   changes, and they are the same two input events the batch semantics
   already depends on: rows and watermarks.  A view is never written,
   and no operation updates or deletes a row, because the CLI exposes
   neither (ADR 0034 decision 7 defers them to the change-control
   design).  Serving adds no capability the program's own tooling
   lacks.

3. **Every committed write is followed by a full rematerialization in
   the same transaction.**  An `append` or `advance_floor` opens one
   transaction, applies the change, recomputes every view in
   dependency order (ADR 0042), replaces their tables, and commits.  A
   reader therefore never observes a store state and a view state that
   disagree, and the served invariant is exact:

   > at every committed state, each view table holds what `mensura run`
   > would materialize over that state.

   Reads are snapshot reads (the database runs in WAL mode), so they
   neither block the writer nor see a half-applied write.

   The cost is stated rather than hidden: a write's latency grows with
   the history the views scan.  That is the measured motivation
   incremental refresh has lacked, and it is also the correctness
   oracle incremental refresh will need, since an incremental engine is
   correct exactly when it preserves this invariant.  Replacing the
   recompute is a change behind the boundary, not to it.

   A view whose evaluation fails at runtime (the checker guarantees
   shape, not, for example, that instant arithmetic stays in range)
   rolls the whole write back and reports the view.  The database never
   holds a state the program cannot evaluate.  The availability cost of
   that choice (one such row blocks its batch) is recorded as an open
   question.

4. **HTTP is the first projection, with paths from the naming rule.**
   Each declared name projects to one kebab-case path segment, per
   `docs/language/05-naming-and-casing.md`.  Stores, registries, and
   views share one namespace (ADR 0012), so one segment addresses any
   of them without a kind prefix.

   | Request | Operation |
   |---|---|
   | `POST /<name>` with a JSON Lines body | `append` |
   | `GET /<name>` | `read` |
   | `POST /_mensura/floors/<name>/<column>` with `{"point": ...}` | `advance_floor` |
   | `GET /_mensura/health` | `health` |

   The `_mensura` segment cannot collide with a declared name: the
   translation maps every `_` to `-`, so no projected segment contains
   an underscore.

   This settles ADR 0006's "defaults" question for HTTP: every store,
   registry, and view of the program is exposed, and every request but
   `health` is gated by decision 5's scopes.  Narrowing what a
   transport exposes is the deploy configuration's job, and arrives
   with it.

   - **The body of an `append` is JSON Lines**, the interchange format
     `mensura ingest` reads.  One request is one batch is one
     transaction.  A device already producing a file for the CLI posts
     the same bytes.
   - **A `read` returns a JSON array of name-keyed records in key
     order.**  The encoder is the decoder's inverse: an instant in the
     normalized UTC form, a dimensioned value as its base-unit
     magnitude, an enum as its variant literal, a compound component
     under its dotted name, and a missing value as `null`.  So a record
     read from a store re-ingests into a store of the same schema,
     which is the property the encoder is tested against.  Key order
     makes a read deterministic, and two reads of an unchanged table
     byte-identical.  A `bag` table orders ties by its attribute
     columns.
   - **Errors map to status codes by cause.**

     | Cause | Status |
     |---|---|
     | malformed JSON or JSON Lines | 400 |
     | a record that does not decode (with record index and field) | 422 |
     | a duplicate key, a violated `domain` reference, or a late row (with its grain) | 409 |
     | an `append` to a view | 405 |
     | an unknown name | 404 |
     | no credential, or an unknown one | 401 |
     | a credential without the permission | 403 |
     | a body over the configured limit | 413 |
     | a view evaluation failure (decision 3) | 500 |

     Every error body is a JSON object carrying the same message the
     CLI prints for the same failure.

   Not in this slice: pagination, filtering, and field selection on
   `read`.  A read returns the whole table.  They are the first
   cross-transport semantics ADR 0006 asks to specify once, and they
   are specified once, later.

5. **Principals are static bearer tokens, as an interim subset of
   ADR 0005.**  Each principal in the runtime configuration carries a
   canonical identity in ADR 0005's SPIFFE-style form, the SHA-256
   digest of its token, and a list of permission scopes.  A request
   presents `Authorization: Bearer <token>`; the host hashes it, finds
   the principal, and checks the operation's scope:

   | Operation | Scope |
   |---|---|
   | `append` to `readings` | `write:readings` |
   | `read` of `readings` | `read:readings` |
   | `advance_floor` on `readings` | `floor:readings` |
   | `health` | none |

   Scopes use ADR 0005's `read:` and `write:` strings over the
   kebab-case resource (`05-naming-and-casing.md`), and its `read:all`
   and `write:all` wildcards.  `floor:` is new: advancing a floor is an
   operator's assertion that the world is closed through a point,
   distinct from writing a row, and a device that may append must not
   thereby be able to declare its peers' windows final.

   How this relates to ADR 0005, precisely:

   - A static token is the degenerate case of ADR 0005's **bundled
     issuer**: a credential Mensura itself vouches for, suitable for
     development and small self-contained deployments.  JWT and X.509
     verification against configured trust anchors, which is ADR
     0005's general case, replaces it without changing the identity or
     the scopes.
   - **The grants are in the runtime configuration, not the program.**
     ADR 0005 puts roles and permissions in the program's `auth {}`,
     which is not designed yet.  When it lands, the grants move into
     the program and the configuration keeps only credentials and trust
     anchors.  Until then the grants sit where the program cannot see
     them, which is a known and temporary inversion.

   Transport security is out of the host.  `serve` speaks plain HTTP
   and expects TLS to terminate in front of it, at the ingress or the
   platform's load balancer.  To keep that from becoming a silent
   exposure, `serve` refuses to bind a non-loopback address unless at
   least one principal is configured, and every non-`health` request
   requires a credential.

6. **A schema guard refuses to serve a program the database was not
   created for.**  At its first start over an empty database, `serve`
   records a fingerprint of the program's store and registry schemas in
   a reserved table, `mensura_program`.  The fingerprint is a hash over
   a canonical serialization of every stored table: its name, kind,
   columns with type, role, and optionality, its `domain` references,
   and its `lateness` contracts.  At every later start, a program whose
   fingerprint differs is refused, and the diagnostic names the tables
   that differ.

   Views are excluded, because they are derived: a changed view is
   rebuilt by the startup materialization and needs no migration.  So
   editing a view and restarting is always accepted, and editing a
   store is always refused.

   The guard is the interim stand-in for typed migrations.  It exists
   because `ensure_store` does not reconcile an existing table whose
   shape differs (`04-processing-layer.md`), so without it a changed
   store would serve against a stale table and fail on the first write,
   or worse, succeed on it.  The same fingerprint is what a future
   `mensura deploy` compares before generating a migration, and what
   `health` reports so that a deploy script can confirm which program
   is running.

7. **The runtime configuration is a flat TOML file that nothing
   infers.**

   ```toml
   program  = "fleet-monitoring.mensura"
   database = "/var/lib/mensura/fleet.db"
   listen   = "0.0.0.0:8080"

   [limits]
   max_body_bytes = 8_388_608

   [[principal]]
   id     = "spiffe://plant.acme/device/temperature-sensor/0b8e2f6a-..."
   token_sha256 = "9f86d081884c7d65..."
   scopes = ["write:readings"]

   [[principal]]
   id     = "spiffe://plant.acme/service/dashboard"
   token_sha256 = "60303ae22b998861..."
   scopes = ["read:all"]
   ```

   Every field is explicit, no field has an environment-dependent
   default beyond `listen` (loopback when absent), and there is no
   target, provider, or topology in it.  That is deliberate.  This file
   is the **compiled** configuration: what the host needs to run, and
   nothing about where.  The user-authored deploy configuration of ADR
   0006, which names a target (a local cluster, a cloud provider) and
   the surfaces each transport exposes, is a different file with a
   different audience, and a future `mensura deploy` generates this one
   from it.  Keeping them apart means `serve` is testable with a
   hand-written file and never learns what a cloud is.

   Token digests rather than tokens: a digest in a configuration file
   or a container image reveals nothing usable, since tokens are
   generated high-entropy secrets, so the file needs no secret store of
   its own.

## Consequences

- A program runs as a service with no language change, and the
  fleet-monitoring example becomes a system a device can post to and a
  dashboard can read from.
- `mensura ingest`, `mensura floor`, and `mensura run` stay the local
  and offline tools, and each now has a served counterpart with
  identical semantics, because both call the same decoder, write path,
  and materializer.
- The storage boundary needs a unit of work that spans an `apply` or a
  floor advance and the view materializations.  Today each of those
  opens its own transaction.  This is a trait change in
  `mensura-runtime`, and it is the one piece of the slice that touches
  existing code rather than adding to it.
- A new crate hosts the server, keeping `mensura-runtime` free of an
  HTTP stack.  The host runs an asynchronous HTTP server (axum on
  tokio, since subscriptions and an MQTT projection are both naturally
  asynchronous) with the storage backend owned by one dedicated writer
  thread and reads served from read-only connections; `rusqlite` stays
  synchronous, as `00-storage-backend.md` chose.
- Incremental refresh gains a benchmark and an oracle.  A served
  fleet with enough history to make decision 3's recompute slow is the
  workload the refresh slice is measured against, and decision 3's
  invariant is what it must preserve.
- `ROADMAP.md`'s execution order changes: the serving slice lands
  before M5's refresh half and M6.  The roadmap is updated when this
  ADR is accepted.

## Alternatives considered

**Recompute asynchronously after the write commits.**  Debouncing
rematerialization keeps write latency flat.  But it opens a window in
which a store read and a view read disagree, and the invariant of
decision 3 weakens to "eventually", which is a harder thing to test
and to explain.  At the target scale the recompute is cheap enough to
keep inside the write.  If it stops being cheap, incremental refresh is
the answer, not a weaker consistency statement.

**Evaluate a view on every read, with no materialization.**  It makes
writes cheap and moves the cost to reads, where a polling dashboard
multiplies it without bound.  It also loses the materialized tables as
plain SQL-queryable state, which is how `mensura run` has always
presented a view.

**Build incremental refresh first.**  It optimizes a cost nobody has
measured, without a consumer to measure it, and it would have to be
correct against the very invariant this ADR establishes.

**MQTT or GraphQL as the first projection.**  MQTT is the natural wire
for devices, but it needs a broker, which is one more service on every
target.  GraphQL is generated from the resolved schema (ADR 0006) and
is real work.  HTTP runs on every cheap platform with nothing beside
it, and both later projections are callers of decision 2's operations.

**Implement ADR 0005 in full first.**  JWT and X.509 verification, trust
anchors, and the `auth {}` surface are a milestone of their own.
Static tokens have the same shape (a canonical identity with scopes),
so what is built on them survives the replacement.

**Put the grants in the program now.**  It would anticipate the
`auth {}` surface without designing it, freezing a syntax on a
serving slice's schedule.

**Accept any store change and let the first write fail.**  That is the
behavior without decision 6, and a changed column type can make the
first write succeed against the wrong table, which is the silent
failure the language exists to rule out.

## Open questions

- **Idempotent appends.**  A device that times out and retries posts
  its batch twice.  A `singletons` target rejects the retry with a
  409, which the device cannot tell apart from a genuine conflict; a
  `bag` registry accepts it and double-counts.  An idempotency key per
  batch, remembered by the host, is the usual answer, and the question
  is where it is remembered and for how long.
- **Evaluation failure and availability.**  Decision 3 rolls a write
  back when a view cannot evaluate over its result.  Whether a view
  should instead be marked failed while the write commits depends on
  whether a blocked producer or a stale view is the worse outcome, and
  that is best decided against a real failure.
- **Reads at scale.**  Pagination, filtering, and field selection,
  specified once for every projection (ADR 0006).
- **Request limits and backpressure.**  A body limit is configured; a
  rate limit and a bound on concurrent writers queued behind the one
  writer are not.
- **The floor's schedule.**  `advance_floor` makes the operation
  reachable over the wire; who calls it, and on what schedule, is
  still ADR 0041's open question, now with a place to be answered (a
  scheduled job in the deployment).
- **One-shot writers while serving.**  Whether `ingest` and `floor`
  should detect a running host and refuse, rather than relying on the
  operator.

## Forward references

- `docs/toolkit/06-serving.md`, the toolkit specification written from
  this ADR once it is accepted.
- The deploy configuration and `mensura deploy`: the user-authored
  file naming a target, and the generated bundle (the runtime
  configuration of decision 7, a container image, and a script) for a
  local cluster first and a cloud target second.
- Typed schema migrations, which replace decision 6's refusal with a
  generated migration classified by what each change does to stored
  observations.
- Subscriptions: the push-out direction of ADR 0006, as the diff
  between consecutive materializations, and with it the provisional
  frontier rows of ADR 0037's open question, which finally have a
  consumer.
- The MQTT and GraphQL projections of decision 2's operations.
