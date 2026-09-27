# 0045: Serving a program

## Status

Proposed.  Pulls the first slice of `ROADMAP.md` M7 ahead of M5's
refresh half and M6, so that a Mensura program can run as a service
before the rest of M7 is designed.  Realizes
`docs/decisions/0006-transport-agnostic-surface.md` for one transport
(HTTP) and settles its "defaults" open question for that transport.
Adopts an interim credential model that is a strict subset of
`docs/decisions/0005-identity-and-authorization.md` (decision 8 says
exactly which part and how it is superseded).  Reuses, unchanged, the
decoder of `docs/decisions/0034-typed-ingestion.md`, the lateness
contract and closure floor of
`docs/decisions/0041-watermark-grain-and-the-closure-floor.md`, and the
batch evaluator of `docs/toolkit/04-processing-layer.md`.  Widens the
storage boundary of `docs/toolkit/00-storage-backend.md` from one
embedded SQLite file to any database that meets decision 2's contract,
Postgres first.  No language change: no grammar production, no typing
rule, no `formal/` work.

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

Four constraints shape the answer.

- **The batch semantics is the specification.**  A view's contents are
  a function of the stores' rows and the effective watermarks, and
  `mensura run` computes that function (`04-processing-layer.md`).
  Serving must not introduce a second meaning: whatever a client reads
  from a served view is what `mensura run` computes over some state the
  database actually passed through, and the client can tell which.
- **The database is a service, not a file.**  A deployment runs
  against a database it reaches by address: a managed Postgres, a
  Postgres inside the cluster, or an existing database server in which
  the program is given its own area.  Mensura always owns the tables
  it uses; it does not read or write tables another system maintains.
  But it cannot assume it owns the server, the host the server runs
  on, or that it is the only process connected.
- **Serving scales out.**  Several `mensura serve` instances against
  one database is the ordinary shape on a container orchestrator, so
  nothing below may depend on there being one process.  The single
  process on a laptop is the degenerate case of the same design, not a
  different one.
- **ADR 0006 keeps the core wire-agnostic.**  The program names
  surfaces; the transport projects them.  So the operations are stated
  first and projected onto HTTP second, and a later MQTT or GraphQL
  projection is a caller of the same operations.

## Decision

1. **`mensura serve` runs roles, not a monolith.**

   | Role | Serves | Instances |
   |---|---|---|
   | `intake` | `append`, `advance_floor` | any number |
   | `read` | `read`, `health` | any number |
   | `materializer` | turns committed writes into views | one active, any number standing by |

   ```
   mensura serve --config <serve.toml>
   ```

   The configuration names the roles an instance runs, and the default
   is all three, which is the laptop and single-machine deployment.  A
   cluster runs the `intake` and `read` roles as one replicated,
   stateless deployment and the `materializer` as another.  An instance
   holds no state that is not in the database: stopping one loses
   nothing, and any instance of a role can serve any request of that
   role.

   At startup an instance typechecks the program, refusing to start on
   any diagnostic exactly as `check` does, and runs the schema guard
   (decision 9) before reporting ready.  It stops on `SIGTERM` or
   `SIGINT` after the transaction in flight commits or rolls back,
   which is the graceful shutdown a container orchestrator expects.

2. **Mensura owns a namespace in a database reached by address.**  The
   configuration gives a database address and a **namespace**, and
   every table Mensura creates, reads, or writes lives in that
   namespace: the program's stores, registries, and views, and the
   reserved tables below.  On Postgres the namespace is a schema, so
   one server can host several programs, or a program beside a
   company's own tables, without either seeing the other.  Mensura
   never touches a table outside its namespace.

   A database is a valid backend when it meets this contract:

   - transactions with **snapshot reads**, so a reader sees one
     committed state;
   - a **serialization point** for write transactions (decision 4);
   - enforcement of primary keys, `NOT NULL`, `CHECK`, and foreign
     keys, which is how ADR 0032's `domain` clauses and ADR 0034's
     closed domains hold;
   - a namespace mechanism.

   Postgres meets all of it natively and is the backend that makes
   several instances meaningful.  Embedded SQLite, today's backend,
   meets it only inside one process on one host, so it serves the
   all-in-one deployment and local work, and is refused when an
   instance is configured with a proper subset of the roles.  A SQLite
   database *behind a server* that speaks for it is a different
   backend, reached by address like Postgres, and is admissible when
   that server meets the contract; whether one is worth writing is left
   to the first deployment that wants it.

3. **Four logical operations, independent of any wire.**

   | Operation | On | Effect |
   |---|---|---|
   | `append` | a store or registry | as `mensura ingest` |
   | `read` | a store, registry, or view | its rows, and their position |
   | `advance_floor` | a contracted column | as `mensura floor` |
   | `health` | the instance | readiness, roles, positions, fingerprint |

   `append` and `advance_floor` are the two ways the program's state
   changes, and they are the same two input events the batch semantics
   already depends on: rows and watermarks.  A view is never written by
   a client, and no operation updates or deletes a row, because the CLI
   exposes neither (ADR 0034 decision 7 defers them to the
   change-control design).  Serving adds no capability the program's
   own tooling lacks.

4. **Every write is serialized through a numbered write log.**  An
   `append` or `advance_floor` decodes and authorizes its request
   outside any transaction, then opens a write transaction whose first
   act is to take the serialization point: a lock on the single row of
   a reserved counter (on Postgres, `SELECT ... FOR UPDATE`; on
   embedded SQLite, an immediate transaction).  Holding it, the write
   takes the next **position**, runs the intake checks (keys, `domain`
   references, the lateness contract against its grain's effective
   watermark), applies the change, records one entry in the reserved
   log table `mensura_log` (position, operation, target, row count,
   principal), and commits.

   Two properties follow, and both are required rather than
   convenient.

   - **The lateness check is race-free.**  It reads a grain's maximum
     point and then inserts.  Two unserialized writers could both pass
     it against the same watermark and land rows that are jointly late,
     which is exactly the failure `closedWindow_stable` rules out.
     Serializing writes closes that gap for every contract at once.
   - **Positions are gapless in every snapshot.**  Because positions
     are taken under the lock and committed in order, any committed
     snapshot contains exactly the writes `1..N` for some `N`, its
     position.  So "the database at position `N`" is a well-defined
     state, which is what decision 5 needs.

   The cost is that writes do not run in parallel.  The transaction
   holds only the checks and the insert, since decoding and
   authorization happen before it, so the serialized section is short.
   A finer serialization point (per table or per grain) is an
   optimization that keeps both properties, and waits for a workload
   that needs it.

   The log is also the changelog incremental refresh will consume: a
   numbered, ordered record of every change to the program's inputs.

5. **Views are materialized at a log position.**  The materializer
   opens a read transaction, reads the latest position `N` in that
   snapshot, reads the stores and watermarks in the same snapshot,
   evaluates every view in dependency order (ADR 0042), and then, in a
   write transaction, replaces each view's table and records `N` as its
   position in the reserved table `mensura_views`.  The replacement is
   a compare-and-set: a view is replaced only if its recorded position
   is below `N`.

   The served invariant is **prefix consistency**:

   > a view at position `N` holds exactly what `mensura run` would
   > materialize over the database after writes `1..N`.

   It holds with no condition on how many materializers run.  Two
   materializers racing compute from snapshots, and the compare-and-set
   keeps a slower one from replacing a newer result with an older one,
   so running one active materializer is an efficiency matter (no
   duplicated work), not a correctness one.  One is kept active by a
   lease row in the namespace, which a standby takes over when it
   expires.

   The materializer wakes when a position is committed.  Polling the
   latest position is the portable mechanism; a backend's notification
   channel (Postgres `LISTEN`/`NOTIFY`, or an in-process signal when the
   roles share an instance) is an optimization of it.

   A view whose evaluation fails at runtime (the checker guarantees
   shape, not, for example, that instant arithmetic stays in range)
   keeps its table and its last position, and `health` reports it
   failed, with the error and the position that caused it.  The write
   that caused it has already committed, so producers are never
   blocked by a view, and a failed view is visible rather than silently
   stale.

   The cost is stated rather than hidden: each materialization
   recomputes from the full history the views scan.  That is the
   measured motivation incremental refresh has lacked, and prefix
   consistency is also its correctness oracle, since an incremental
   engine is correct exactly when it preserves the invariant.
   Replacing the recompute is a change behind the boundary, not to it.

6. **Reads report their position, and can wait for one.**  A `read`
   returns its rows with the position they reflect: a view's recorded
   position, or for a store or registry the position of the snapshot
   it was read in.  An `append` returns the position it took.  A
   client that must see its own write passes that position as a
   minimum on the next read, and the instance waits for the view to
   reach it, up to a configured bound, and then answers that it has
   not, rather than serving an older state as if it were current.
   Read-your-writes is therefore available to the client that wants
   it, and costs nothing to the one that does not.

7. **HTTP is the first projection, with paths from the naming rule.**
   Each declared name projects to one kebab-case path segment, per
   `docs/language/05-naming-and-casing.md`.  Stores, registries, and
   views share one namespace (ADR 0012), so one segment addresses any
   of them without a kind prefix.

   | Request | Operation |
   |---|---|
   | `POST /<name>` with a JSON Lines body | `append` |
   | `GET /<name>`, optionally `?min_position=N` | `read` |
   | `POST /_mensura/floors/<name>/<column>` with `{"point": ...}` | `advance_floor` |
   | `GET /_mensura/health` | `health` |

   The position travels in a `Mensura-Position` response header on
   `append` and `read`.  The `_mensura` segment cannot collide with a
   declared name: the translation maps every `_` to `-`, so no
   projected segment contains an underscore.

   This settles ADR 0006's "defaults" question for HTTP: every store,
   registry, and view of the program is exposed, and every request but
   `health` is gated by decision 8's scopes.  Narrowing what a
   transport exposes is the deploy configuration's job, and arrives
   with it.

   - **The body of an `append` is JSON Lines**, the interchange format
     `mensura ingest` reads.  One request is one batch is one
     transaction is one position.  A device already producing a file
     for the CLI posts the same bytes.
   - **A `read` returns a JSON array of name-keyed records in key
     order.**  The encoder is the decoder's inverse: an instant in the
     normalized UTC form, a dimensioned value as its base-unit
     magnitude, an enum as its variant literal, a compound component
     under its dotted name, and a missing value as `null`.  So a record
     read from a store re-ingests into a store of the same schema,
     which is the property the encoder is tested against.  Key order
     makes a read deterministic: two reads at one position are
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
     | `min_position` not reached within the bound | 503 |
     | an operation of a role this instance does not run | 421 |

     Every error body is a JSON object carrying the same message the
     CLI prints for the same failure.

   Not in this slice: pagination, filtering, and field selection on
   `read`.  A read returns the whole table.  They are the first
   cross-transport semantics ADR 0006 asks to specify once, and they
   are specified once, later.

8. **Principals are static bearer tokens, as an interim subset of
   ADR 0005.**  Each principal in the runtime configuration carries a
   canonical identity in ADR 0005's SPIFFE-style form, the SHA-256
   digest of its token, and a list of permission scopes.  A request
   presents `Authorization: Bearer <token>`; the instance hashes it,
   finds the principal, and checks the operation's scope:

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

   The principal of every write is recorded in its log entry
   (decision 4), which is the first place a canonical identity is
   stored beside the data it produced.

   Transport security is out of the instance.  `serve` speaks plain
   HTTP and expects TLS to terminate in front of it, at the ingress or
   the platform's load balancer.  To keep that from becoming a silent
   exposure, `serve` refuses to bind a non-loopback address unless at
   least one principal is configured, and every non-`health` request
   requires a credential.

9. **A schema guard refuses to serve a program its namespace was not
   created for.**  The first instance to start over an empty namespace
   creates the stores, registries, views, and reserved tables, and
   records a fingerprint of the program's store and registry schemas in
   the reserved table `mensura_program`.  It does so holding decision
   4's serialization point, so instances starting together agree: one
   creates, the others find the fingerprint and verify it.  The
   fingerprint is a hash over a canonical serialization of every stored
   table: its name, kind, columns with type, role, and optionality, its
   `domain` references, and its `lateness` contracts.  An instance
   whose program's fingerprint differs refuses to start, and the
   diagnostic names the tables that differ.

   Views are excluded, because they are derived: a changed view is
   rebuilt by the next materialization and needs no migration.  So
   editing a view and redeploying is always accepted, and editing a
   store is always refused.

   The guard is the interim stand-in for typed migrations.  It exists
   because table creation does not reconcile an existing table whose
   shape differs (`04-processing-layer.md`), so without it a changed
   store would serve against a stale table and fail on the first write,
   or worse, succeed on it.  The same fingerprint is what a future
   `mensura deploy` compares before generating a migration, and what
   `health` reports so that a deploy script can confirm which program
   is running.

10. **The runtime configuration is a flat TOML file that nothing
    infers.**

    ```toml
    program   = "fleet-monitoring.mensura"
    database  = { url_env = "MENSURA_DATABASE_URL" }
    namespace = "fleet"
    roles     = ["intake", "read", "materializer"]
    listen    = "0.0.0.0:8080"

    [limits]
    max_body_bytes    = 8_388_608
    min_position_wait = "5s"

    [[principal]]
    id           = "spiffe://plant.acme/device/temperature-sensor/0b8e2f6a-..."
    token_sha256 = "9f86d081884c7d65..."
    scopes       = ["write:readings"]

    [[principal]]
    id           = "spiffe://plant.acme/service/dashboard"
    token_sha256 = "60303ae22b998861..."
    scopes       = ["read:all"]
    ```

    Every field is explicit, and there is no target, provider, or
    topology in it.  That is deliberate.  This file is the
    **compiled** configuration: what an instance needs to run, and
    nothing about where.  The user-authored deploy configuration of
    ADR 0006, which names a target (a local cluster, a cloud provider)
    and the surfaces each transport exposes, is a different file with
    a different audience, and a future `mensura deploy` generates this
    one from it, one per role group.  Keeping them apart means `serve`
    is testable with a hand-written file and never learns what a cloud
    is.

    The file holds no secret.  The database address, which carries a
    password, is named by the environment variable that holds it, which
    is how an orchestrator injects a secret.  Tokens appear as digests,
    and a digest in a configuration file or a container image reveals
    nothing usable, since tokens are generated high-entropy secrets.

## Consequences

- A program runs as a service with no language change, and the
  fleet-monitoring example becomes a system a device can post to and a
  dashboard can read from.
- The same design runs as one process over an embedded SQLite file and
  as a replicated cluster over Postgres; what changes between them is
  the configuration, not the semantics.
- `mensura-runtime` gains a second backend.  Postgres implements the
  same `StorageBackend` trait, with the same typed errors, including
  ADR 0034's foreign-key diagnostic, and the trait grows the operations
  decisions 4, 5, and 9 need: a serialized write unit that takes a
  position, snapshot reads that report one, a compare-and-set view
  replacement, and the lease.  The one-shot commands move to the same
  write unit, so `mensura ingest` against a served database takes a
  position like any other write and the views catch up.
- A new crate hosts the server, keeping `mensura-runtime` free of an
  HTTP stack.  It runs an asynchronous HTTP server (axum on tokio,
  since subscriptions and an MQTT projection are both naturally
  asynchronous) and calls the synchronous storage backends on blocking
  threads, as `00-storage-backend.md` chose.
- Incremental refresh gains a changelog, a benchmark, and an oracle:
  the write log is its input, a served fleet with enough history is
  its workload, and prefix consistency is what it must preserve.
- `ROADMAP.md`'s execution order changes: the serving slice lands
  before M5's refresh half and M6.  The roadmap is updated when this
  ADR is accepted.
- A suggested implementation order, open to revision: the write log,
  positions, and roles over embedded SQLite in one process first,
  since they exercise every decision; the Postgres backend second,
  which is what makes the roles separable.

## Alternatives considered

**One process as the only writer, recomputing every view inside each
write transaction.**  The first draft of this ADR.  It gives the
strongest possible consistency (a view never lags a store) and needs no
log.  But it ties the design to one process, makes every write wait on
every view, and lets one failing view block every producer.  Prefix
consistency with a reported position is nearly as strong, is honest
about lag rather than forbidding it, and scales out.

**Unserialized concurrent writes with per-constraint locking.**  Keys
and `domain` references are enforced by the database under any
isolation, so only the lateness check needs care, and a per-grain lock
would serve it.  But positions would then commit out of order, a
snapshot could hold write 5 without write 4, and "the database at
position `N`" would stop being a state.  Serializing first and
narrowing later keeps the invariant simple.

**Recompute asynchronously with no position.**  Debounced
rematerialization without a position gives "eventually consistent",
which a client cannot act on: it cannot tell a fresh view from a stale
one, and cannot wait for its own write.

**Evaluate a view on every read, with no materialization.**  It moves
the cost to reads, where a polling dashboard multiplies it without
bound, and loses the materialized tables as plain SQL-queryable state.

**Build incremental refresh first.**  It optimizes a cost nobody has
measured, without a consumer to measure it, and it would have to be
correct against the very invariant this ADR establishes.

**MQTT or GraphQL as the first projection.**  MQTT is the natural wire
for devices, but it needs a broker, which is one more service on every
target.  GraphQL is generated from the resolved schema (ADR 0006) and
is real work.  HTTP runs on every cheap platform with nothing beside
it, and both later projections are callers of decision 3's operations.

**Implement ADR 0005 in full first.**  JWT and X.509 verification, trust
anchors, and the `auth {}` surface are a milestone of their own.
Static tokens have the same shape (a canonical identity with scopes),
so what is built on them survives the replacement.

**Put the grants in the program now.**  It would anticipate the
`auth {}` surface without designing it, freezing a syntax on a
serving slice's schedule.

**Accept any store change and let the first write fail.**  That is the
behavior without decision 9, and a changed column type can make the
first write succeed against the wrong table, which is the silent
failure the language exists to rule out.

## Open questions

- **Idempotent appends.**  A device that times out and retries posts
  its batch twice.  A `singletons` target rejects the retry with a
  409, which the device cannot tell apart from a genuine conflict; a
  `bag` registry accepts it and double-counts.  An idempotency key per
  batch, recorded in its log entry, is the natural answer, and the
  question is how long the host remembers one.
- **Log retention.**  The log grows by one entry per write.  Whether
  old entries are compacted, and what compaction may drop once
  incremental refresh reads the log, is decided with that slice.
- **Reads at scale.**  Pagination, filtering, and field selection,
  specified once for every projection (ADR 0006).  Paging across
  requests will want to pin a position, which decision 6 makes
  possible.
- **Request limits and backpressure.**  A body limit is configured; a
  rate limit and a bound on writers queued behind the serialization
  point are not.
- **The floor's schedule.**  `advance_floor` makes the operation
  reachable over the wire; who calls it, and on what schedule, is
  still ADR 0041's open question, now with a place to be answered (a
  scheduled job in the deployment).

## Forward references

- `docs/toolkit/06-serving.md`, the toolkit specification written from
  this ADR once it is accepted, and an update of
  `docs/toolkit/00-storage-backend.md` for the Postgres backend and the
  widened trait.
- The deploy configuration and `mensura deploy`: the user-authored
  file naming a target, and the generated bundle (the runtime
  configurations of decision 10, container images, and a script) for a
  local cluster first and a cloud target second.
- Typed schema migrations, which replace decision 9's refusal with a
  generated migration classified by what each change does to stored
  observations.
- Subscriptions: the push-out direction of ADR 0006, as the diff
  between a view's consecutive positions, and with it the provisional
  frontier rows of ADR 0037's open question, which finally have a
  consumer.
- The MQTT and GraphQL projections of decision 3's operations.
