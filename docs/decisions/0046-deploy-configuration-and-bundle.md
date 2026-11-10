# 0046: The deploy configuration and bundle

## Status

Accepted.  The second slice of `ROADMAP.md` M7.  Realizes the
deploy-configuration half of
`docs/decisions/0006-transport-agnostic-surface.md` for one target and
settles its "format and home" open question.  Generates the runtime
configuration of `docs/decisions/0045-serving-a-program.md` decision 10,
which that ADR reserved for a future `mensura deploy`, and keeps
`serve` exactly as ignorant of targets as decision 10 wants.  No
language change: no grammar production, no typing rule, no `formal/`
work.

## Context

`mensura serve` runs a program as a service from a flat runtime
configuration, and ADR 0045 is explicit about what that file is not: it
holds what one instance needs to run and nothing about where.  Bringing
the service up on a real host still means writing that file by hand,
choosing a container image, mounting the program and the configuration
into it, giving the database file a volume, publishing a port, and
wiring a readiness probe, and doing all of it again for each instance
and each redeploy.  That is the gap between a subcommand and a
deployment, and it is what the integration demonstration (M7.3) needs
closed before a program can run beside the platform's broker.

The first deployment fixes the target.  It is a small virtual machine
shared with the platform's broker and its worker, both under compose,
with less than a gigabyte of memory to spare and no swap, so the
database is embedded SQLite in one process (ADR 0045 decision 2's
all-in-one case), the Mensura services must carry memory limits so that
pressure stops them before the broker, and the database file must live
on its own volume so that it can never crowd out the broker's queue.
The rehearsal before it is a developer's compose stack fed with replayed
telemetry.  Both are compose, so compose is the first target.  A
managed container runtime with a managed Postgres is the second target,
and it waits for the Postgres backend (ROADMAP M7.2).

Three constraints shape the answer.

- **Two audiences, two files.**  ADR 0045 decision 10 separates the
  compiled runtime configuration (what an instance needs) from the
  user-authored deploy configuration (where it runs), so that `serve`
  is testable with a hand-written file and never learns what a cloud
  is.  This ADR designs the second file and the compiler between them,
  and nothing here reaches into `serve`.
- **The bundle is the deployment, readable as it is.**  What `mensura
  deploy` writes must be complete enough to bring the system up with
  the target's own tooling and plain enough to be reviewed in a pull
  request on the platform's side, because that is where it will live:
  the platform's repository holds the program and the deploy
  configuration, and the Mensura toolchain knows nothing about the
  platform.
- **Nothing secret is generated or stored.**  The deploy configuration
  is committed beside the program, so it carries the same digests and
  secret references as the runtime configuration and never a token or
  a password.

## Decision

1. **`mensura deploy` compiles a deploy configuration and a program into
   a bundle.**

   ```
   mensura deploy <deploy.toml> [--out <dir>]
   ```

   The deploy configuration is a TOML file the user writes.  The
   bundle is a directory `mensura deploy` writes: one runtime
   configuration per instance, a copy of the program, the target's
   artifacts, and a script that brings the system up.  `--out`
   defaults to `bundle` beside the configuration file.  Deploy writes
   every file it owns and removes nothing, so a bundle can be
   regenerated in place and committed, and a file it does not own (a
   `.env`, a note) survives.

   Deploy validates before it writes anything: it typechecks the
   program, refusing on any diagnostic exactly as `check` does; it
   validates the deploy configuration; and it parses every runtime
   configuration it is about to write with `serve`'s own loader, so
   that deploy can never emit a file `serve` would refuse.  What
   `serve` checks at startup against the database (the schema guard)
   stays at startup, since the bundle does not reach the database.

2. **The deploy configuration names the program, the namespace, the
   target, the database, the instances, and the principals, and
   nothing is inferred from the host.**

   ```toml
   program   = "fleet-monitoring.mensura"
   namespace = "fleet"

   [target]
   kind  = "compose"
   name  = "mensura-fleet"
   image = "ghcr.io/mensura-lang/mensura:sha-1a2b3c4"

   [database]
   kind = "sqlite"

   [[instance]]
   name    = "mensura"
   roles   = ["intake", "read", "materializer"]
   publish = "127.0.0.1:8080"
   memory  = "256M"

   [limits]
   max_body_bytes    = 8_388_608
   min_position_wait = "5s"

   [[principal]]
   id           = "spiffe://plant.acme/connector/telemetry-bridge/press"
   token_sha256 = "9f86d081884c7d65..."
   scopes       = ["write:readings"]

   [[principal]]
   id           = "spiffe://plant.acme/service/dashboard"
   token_sha256 = "60303ae22b998861..."
   scopes       = ["read:all"]
   ```

   - `program` is relative to the configuration file and is copied
     into the bundle, so the bundle is self-contained.
   - `namespace` is ADR 0045 decision 2's namespace and names the
     database file on SQLite (`<namespace>.db` on the data volume).
   - `[target]` names the kind (`compose` is the only kind of this
     slice), the project name the target's tooling uses, and the
     container image (decision 4).
   - `[database]` names the kind.  `sqlite` is the default and the
     only kind of this slice; `postgres` arrives with the backend and
     adds a secret reference for the address here.
   - `[[instance]]` is one `serve` process: its name, its roles
     (ADR 0045 decision 1; all three by default), the host address
     and port its listener is published on (loopback by default, so
     that nothing is exposed until the user says so; TLS terminates in
     front, as decision 8 says), and an optional memory limit.  With no
     `[[instance]]` at all, there is one, named `mensura`, running
     every role.  Over SQLite exactly one instance running all three
     roles is admitted; a second instance, or a proper subset of the
     roles, is refused at deploy time with the same diagnostic `serve`
     would give, naming the database reached by address that makes
     roles separable.
   - `[limits]` and `[[principal]]` are ADR 0045 decision 10's and
     pass through unchanged into every instance's runtime
     configuration.  A token appears only as its digest.

   Nothing in the file is the wall clock, a host name, or a path on
   the host: the bundle is the same whoever runs it.

3. **The compose target writes an includable compose project.**  The
   bundle for `kind = "compose"` is:

   ```
   bundle/
     compose.yaml          the project: one service per instance
     mensura/              mounted read-only at /etc/mensura
       fleet-monitoring.mensura
       mensura.toml        the runtime configuration of instance `mensura`
     up.sh                 docker compose up --detach --wait
     down.sh               docker compose down
   ```

   Each instance becomes one service: the image of decision 4; the
   command `serve --config /etc/mensura/<instance>.toml`; the
   `mensura/` directory mounted read-only; the data volume mounted at
   `/var/lib/mensura`; the database address injected as the
   environment variable the runtime configuration references
   (`MENSURA_DATABASE_URL=sqlite:/var/lib/mensura/<namespace>.db`,
   which is a path, not a secret, but goes through the same secret
   reference so that the Postgres case changes nothing in `serve`);
   the listener bound to every interface inside the container and
   published only on the configured host address; the memory limit as
   a compose resource limit when one is given; a restart policy; and a
   readiness check that calls `mensura health` (decision 6) so that a
   dependent service can wait on it.  Inside the container the runtime
   configuration binds a non-loopback address, which ADR 0045 decision
   8 admits only with a principal, so a deployment with no principal
   is refused at deploy time rather than at start.

   The data volume is a named volume of the project, one per
   deployment, so that the database file is never on the bundle's
   directory and never on a bind mount the host might fill.  A
   deployment on a shared host can point the volume at a dedicated
   disk with compose's own volume options, outside the bundle.

   The project is a complete compose file, so the platform's own stack
   can take it up with compose's `include` rather than copying its
   services, and the bring-up script is for a standalone run.

4. **The image is named, not built, by the bundle.**  `target.image`
   is required and copied verbatim into the compose file; a deployment
   pins a digest or a commit-tagged image.  The repository ships a
   `Dockerfile` that builds the `mensura` binary in a Rust stage and
   copies it into a minimal runtime image, and continuous integration
   publishes it as `ghcr.io/mensura-lang/mensura` tagged `main` and
   `sha-<short commit>` on every push to `main`.  Building inside the
   bundle was rejected (alternatives, 3): a Rust build does not fit
   the first host, and an image is what a container target consumes.

5. **Exposure is by scope in this slice.**  ADR 0006 gives the deploy
   configuration the list of surfaces each transport exposes.  With
   HTTP the only transport, every store, registry, and view is
   reachable by name and gated by the principals' scopes (ADR 0045
   decision 8), so a surface no principal may read or write is
   unexposed in effect, and an explicit list would duplicate the
   scopes.  The list arrives with the second transport, where it is
   not redundant: the MQTT projection's topic-to-table mapping (M7.4)
   is exactly a per-transport exposure list.

6. **Two small subcommands close the loop without a shell in the
   image.**

   - `mensura token` generates a high-entropy bearer token from the
     operating system's randomness and prints it with its SHA-256
     digest.  The digest goes into the deploy configuration; the token
     goes to the client.  It exists because the digest must be taken
     over the exact bytes a client will present, and a shell pipeline
     routinely hashes a trailing newline.
   - `mensura health <address>` requests `/_mensura/health` over plain
     HTTP and exits zero when the instance answers ready.  It is the
     readiness check of decision 3, and it is in the binary because the
     runtime image carries no shell, no `curl`, and no `wget`.

7. **A redeploy is a regeneration.**  Editing the program or the
   deploy configuration and running `mensura deploy` again rewrites
   the bundle; the bring-up script recreates the services whose files
   changed; and ADR 0045's schema guard decides at start: a changed
   view is accepted and rebuilt, a changed store or registry is
   refused with a diagnostic naming the table.  That is M7.3's third
   criterion, and typed migrations (the slice after this one) replace
   the refusal with a generated migration without changing the deploy
   step.

## Consequences

Positive:

- The demonstration's deployment is one file the platform reviews and
  one command, and the same file brings up the rehearsal stack and the
  shared host.
- `serve` is unchanged, and the runtime configuration is still
  hand-writable: deploy is a compiler to it, not a replacement for it.
- The bundle is diffable.  A review on the platform's side sees exactly
  the compose services, mounts, limits, and ports that will run.
- The Postgres target slots into the same configuration: `[database]`
  gains a kind and a secret reference, `[[instance]]` may list several
  entries with different roles, and the bundle gains a secret file the
  user fills.

Negative:

- A second target (a local cluster, a managed container runtime) is a
  second generator over the same model.  The model is kept small for
  that reason, but each target is work.
- The image is a release artifact the project must now publish and
  version, and a deployment that pins a commit tag must be re-pinned
  deliberately.
- Memory limits are the user's numbers.  Deploy prints a note when an
  instance has none, but it cannot know the host.

Neutral:

- The deploy configuration is committed beside the program on the
  platform's side, so it is theirs, like the program; the toolchain
  stays generic.
- `mensura deploy` does not run the target's tooling.  The script does,
  and a user who prefers `include`, Tilt, or a hand-run `docker compose`
  uses the files directly.

## Alternatives considered

1. **`serve` reads the deploy configuration directly**, with no bundle.
   Rejected by ADR 0045 decision 10: the runtime configuration would
   then carry targets and topology, and `serve` would learn what a
   cloud is.
2. **A Kubernetes target first.**  Rejected for now: the first host is
   a VM under compose, and the roadmap's cloud target comes with the
   Postgres backend.  The deploy model (instances with roles, a
   database, principals, limits) is what a Kubernetes generator will
   consume unchanged.
3. **Building the image inside the bundle** (a `Dockerfile` in the
   bundle that compiles the toolchain from a pinned source revision).
   Rejected: a Rust build does not fit a two-gigabyte host, a container
   target consumes images, and a pinned image is the stronger
   reproducibility claim.
4. **Deploy generates tokens** into a secrets directory beside the
   bundle on first run.  Rejected: deploy stays a pure function of its
   inputs and writes nothing secret, so a bundle can be regenerated and
   committed freely; `mensura token` gives the user the same
   convenience with the secret in their hands.
5. **A template engine over the bundle** (Helm-style values and
   templates).  Rejected: the bundle is the readable output, and the
   deploy configuration is the only input.  A user who needs a
   variation edits the configuration, not a template.
6. **A per-surface exposure list now.**  Rejected for this slice
   (decision 5): with one transport it duplicates the scopes.

## Open questions

- **The floor's schedule.**  ADR 0041 and ADR 0045 leave who advances
  the closure floor, and on what schedule, to the deployment.  This
  slice does not declare a scheduled job: the floor is deliberately not
  the wall clock, and a declared `now - bound` loop would make the
  deployment the writer that declares its peers' windows final.  The
  demonstration answers it on the platform's side with a small caller
  under a `floor:` principal.  Whether a declared schedule belongs in
  the deploy configuration, under its own principal, is decided when
  the second deployment wants one.
- **The Postgres target's secrets.**  Whether the bundle expects a
  compose secret file or an environment file for the database address,
  decided with the backend.
- **Retention.**  ROADMAP M7.2 records that the first host has months
  of disk; dropping rows below the closure floor is designed beside the
  storage-level copy tool, and the bundle gains nothing until then.
- **A Kubernetes target.**  Same model, second generator; when a host
  wants it.

## Forward references

- `docs/toolkit/07-deploying.md`, the toolkit document written from
  this ADR once it is accepted: the crate `mensura-deploy`, the
  generator, and the bundle's exact contents.
- The Postgres backend and, with it, several instances per deployment
  and the second target.
- Typed schema migrations, replacing decision 7's refusal.
- The MQTT projection (M7.4), which puts the per-transport exposure
  list of decision 5 into this configuration.
