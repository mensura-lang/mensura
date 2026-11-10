# Deploying a program

`mensura deploy` compiles a deploy configuration and a program into a
bundle that a target's own tooling brings up.  It is the second slice of
M7 (`ROADMAP.md`) and the realization of
`docs/decisions/0046-deploy-configuration-and-bundle.md`.  The ADR is the
specification; this document is the map of the code and the exact
contents of the bundle.  What one instance does once it runs is
`docs/toolkit/06-serving.md`.

## Scope of the slice

One target, compose, over embedded SQLite, so exactly one instance
running all three roles.  The slice ships:

- the crate `mensura-deploy`: the deploy configuration model and its
  validation (`config.rs`), the compose generator (`compose.rs`), and
  the compiler and bundle writer (`lib.rs`);
- `mensura deploy <deploy.toml> [--out <dir>]`, `mensura token`, and
  `mensura health <address>` in the CLI;
- the token generator and the readiness probe in `mensura-serve`
  (`auth::new_token`, `probe.rs`), since both speak `serve`'s protocol;
- a `Dockerfile` at the repository root and the `Image` workflow that
  publishes `ghcr.io/mensura-lang/mensura`.

Not in the slice: the Postgres target and with it several instances,
typed migrations, retention, a declared floor schedule, a per-surface
exposure list, and a second target kind (ADR 0046, open questions).

## The deploy configuration

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
token_sha256 = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
scopes       = ["write:readings"]
```

Unknown fields are refused, as in the runtime configuration.  The rules
deploy checks, beyond `serve`'s own:

| Field | Rule |
|---|---|
| `program` | names a file; resolved against the configuration's directory |
| `namespace` | ASCII letters, digits, `-`, `_`: it names the database file |
| `target.kind` | `compose` |
| `target.name` | a compose project name: lowercase letters, digits, `-`, `_` |
| `target.image` | non-empty, no whitespace; copied verbatim |
| `database.kind` | `sqlite` (default); `postgres` names the slice it waits for |
| `[[instance]]` | one, named like a compose service, all three roles (default) |
| `publish` | a host address and port; `127.0.0.1:8080` by default |
| `memory` | digits and an optional `b`, `k`, `m`, `g` (or `kb`, `mb`, `gb`) |
| `[[principal]]` | at least one, since the container binds every interface |

`[limits]` and `[[principal]]` are `serve`'s types, deserialized by
`serve`'s code, and passed through unchanged.

## Compilation

`compile` (and `compile_text`, which tests use) runs in this order and
writes nothing until all of it succeeds:

1. parse and validate the deploy configuration;
2. read the program and resolve it with `serve`'s frontend, so a program
   with any diagnostic is refused exactly as `check` and `serve` refuse
   it;
3. render each instance's runtime configuration and parse it with
   `mensura_serve::Config::parse` against `/etc/mensura`, so deploy can
   never emit a file `serve` would refuse;
4. render `compose.yaml`, `up.sh`, and `down.sh`.

The result is a `Bundle`: a list of files with their relative paths and
contents, and the notes deploy prints (an instance with no memory
limit).  `Bundle::write` creates directories as needed, overwrites the
files it owns, marks the scripts executable, and removes nothing, so a
`.env` or a note beside the bundle survives regeneration.  The files
cite the configuration by its file name only, so no host path enters
the bundle and the same inputs give the same bytes on any machine.

## The bundle

```
bundle/
  compose.yaml
  mensura/
    fleet-monitoring.mensura
    mensura.toml
  up.sh
  down.sh
```

Each instance's runtime configuration, `mensura/<instance>.toml`:

```toml
program   = "fleet-monitoring.mensura"
database  = { env = "MENSURA_DATABASE_URL" }
namespace = "fleet"
roles     = ["intake", "read", "materializer"]
listen    = "0.0.0.0:8080"

[limits]
max_body_bytes    = 8388608
min_position_wait = "5s"

[[principal]]
id           = "spiffe://plant.acme/connector/telemetry-bridge/press"
token_sha256 = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
scopes       = ["write:readings"]
```

Each instance becomes one compose service:

| Key | Value |
|---|---|
| `image` | `target.image` |
| `command` | `serve --config /etc/mensura/<instance>.toml` |
| `labels` | `io.mensura.mounted-sha256`: the digest of the files it mounts |
| `environment` | `MENSURA_DATABASE_URL=sqlite:/var/lib/mensura/<namespace>.db` |
| `volumes` | `./mensura` read-only at `/etc/mensura`; the `data` volume at `/var/lib/mensura` |
| `ports` | `<publish>:8080` |
| `deploy.resources.limits.memory` | `memory`, when given |
| `restart` | `unless-stopped` |
| `healthcheck` | `mensura health 127.0.0.1:8080` every 10s, 5s timeout, 3 retries, 30s start period |

The project is named by `target.name` and declares one named volume,
`data`, so the database file lives on a volume of its own and never in
the bundle's directory.  Every string is written double-quoted with
JSON escaping, which is valid YAML and valid TOML, and the files are
rendered as text rather than serialized, so they read as a person would
write them.

`up.sh` runs `docker compose up --detach --wait` from the bundle's
directory, so it returns once every instance reports healthy and fails
if one does not.  `down.sh` runs `docker compose down`; the data volume
survives unless `--volumes` is passed.  Both forward extra arguments.

## Redeploying

A redeploy regenerates the bundle and runs `up.sh` again (ADR 0046
decision 7).  Compose recreates a service only when its definition
changes, and a file the service mounts is not part of its definition,
so an edit to the program alone would leave the old container running.
The `io.mensura.mounted-sha256` label closes that gap: it is a digest
over the program and the instance's runtime configuration, so a changed
file changes the service definition and compose recreates exactly the
instances whose files changed.

The recreated instance then meets the schema guard
(`06-serving.md`): a changed view is accepted and rebuilt at its first
materialization, by its definition digest, even with no write in
between; a changed store or registry is refused, the instance exits
naming the table, and `up.sh` fails on the unhealthy service.

## The image

The `Dockerfile` builds `mensura-cli` in a `rust:1-bookworm` stage with
`--locked`, and copies the binary into `gcr.io/distroless/cc-debian12`
under the unprivileged user `65532`, with `/var/lib/mensura` created and
owned by that user so that a fresh named volume inherits a writable
directory.  The runtime image has no shell, which is why the readiness
check is `mensura health`.  The entrypoint is `mensura`, so a compose
`command` is a subcommand.

The `Image` workflow builds the image on every push to `main` and on a
pull request that touches the `Dockerfile`, `.dockerignore`, or the
workflow, runs `mensura --version` in it, and on `main` pushes it as
`ghcr.io/mensura-lang/mensura:main` and `:sha-<short commit>`.  A
deployment pins the latter.

## `mensura token` and `mensura health`

`mensura token` reads 32 bytes from `/dev/urandom` and prints them in
hexadecimal with the SHA-256 digest of exactly those 64 characters:

```
token        = 3f1c...
token_sha256 = "a0b2..."
```

The second line pastes into a `[[principal]]`; the token goes to the
client and nowhere else.

`mensura health <address> [--timeout <seconds>]` opens a socket to
`host:port`, sends `GET /_mensura/health` with `Connection: close`, and
exits zero when the answer is a 200 whose JSON body says
`"ready": true`, printing the namespace and the position.  Anything
else, including a refused connection, exits non-zero with the reason.

## Validation

- Configuration: defaults give one loopback instance over SQLite;
  unknown fields are refused; only `compose` and `sqlite` are admitted,
  and `postgres` names the slice it waits for; a second instance or a
  proper subset of the roles is refused as `serve` would refuse it; a
  deployment with no principal is refused and names `mensura token`;
  names, addresses, and sizes are checked.
- Compilation: the fleet example compiles to the five files in order;
  the runtime configuration loads in `serve` with the program at
  `/etc/mensura`, the database behind its environment reference, and
  the listener on every interface; the compose file carries the image,
  command, database address, published port, memory limit, health
  check, read-only mount, and data volume; a missing memory limit is a
  note; a missing or ill-typed program is refused; compilation is pure,
  and an edit to the program changes the service definition; writing
  twice keeps a file the bundle does not own and leaves the scripts
  executable.
- Serve: the token is 64 hexadecimal characters and fresh each time;
  the probe reads a listening instance and fails on a closed port or a
  non-ready answer.
- By hand, with the image built locally: the fleet bundle comes up
  healthy, takes a write, keeps it across `down.sh` and `up.sh`,
  serves an edited view's new definition after a redeploy, and refuses
  an edited store with the guard's diagnostic.

## Forward references

- The Postgres target: `[database]` gains a secret reference,
  `[[instance]]` may list several entries with different roles, and the
  bundle gains a secret file the user fills (ADR 0046, open questions).
- Typed schema migrations, replacing the refusal of a changed store.
- Retention of rows below the closure floor, beside the storage-level
  copy tool (`ROADMAP.md` M7.1, M7.2).
- The MQTT projection (M7.4), which adds the per-transport exposure
  list to this configuration.
