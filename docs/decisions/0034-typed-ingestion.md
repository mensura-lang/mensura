# 0034: Typed ingestion

## Status

Accepted.  Specifies the intake that
`docs/decisions/0033-registry-declarations.md` decision 4 rests on, its
companion in the same slice.  Gives the storage backend of
`docs/toolkit/00-storage-backend.md` its write path and settles the
`PRAGMA foreign_keys` question
`docs/decisions/0032-compound-keys-flatten-to-dotted-columns.md` left
open.  Amends `ROADMAP.md`'s M4 line naming `insert`/`update`/`set`/
`where`/`case` as the ingestion forms: decision 1 ships none of them and
says why.  Honours
`docs/decisions/0007-single-expression-sublanguage.md` (no new
expression forms) and `docs/decisions/0006-transport-agnostic-surface.md`
(no wire protocol in the core).

## Context

M4's output is "device readings land in stores under a typed ingestion
path".  Everything upstream of the write is built: the resolved `Schema`
is a flat ordered list of typed columns with dotted names for flattened
compound components (ADR 0032), and the storage backend already creates
the tables and reads them back as typed rows.  Nothing writes.

`ROADMAP.md` names the ingestion surface as "the `insert`/`update`/
`set`/`where`/`case` forms".  That line is the only occurrence of those
words as language constructs anywhere in the repository: no grammar
production, no design document, no test.  Meanwhile three accepted ADRs
bear on them directly.  ADR 0007 fixed a *single* expression
sublanguage and names `case` among the sites where it is used, not as a
construct of its own; ADR 0015 removed `filter` because `flat_map` with
an `if` already expresses it; and ADR 0019 dropped `const`/`var` and
deferred the mutability model that `set` and `update` would presuppose.
The roadmap line predates all three.

The other open question is format.  ADR 0006 decided that the core
language is transport-agnostic, that transport selection lives in a
deploy configuration read by `mensura serve` (M7), and that each
direction of data flow projects onto the transports that suit it:
GraphQL and REST `GET` for query, MQTT and REST `POST` and gRPC
client-stream for ingest.  It also promised that a GraphQL schema for
the read side is *generated* from the resolved schema rather than
hand-written.  None of that is M4 work, but all of it constrains what
M4 may commit to.

What this ADR must settle: whether ingestion is language surface; what
the backend write path looks like given that DBSP arrives at M5; what a
record is decoded *from*; whether foreign keys become enforced; and
where affine (Celsius) conversion happens, since ADR 0026 says "at
ingestion" without saying what provides the hook.

## Decision

1. **Ingestion is a typed API and a CLI subcommand, not language
   surface.**  No new expression forms, no new statement, no new
   declaration.  Taking the roadmap's five forms in turn:

   - **`case` and `where` are already the expression sublanguage.**
     ADR 0007 fixed one expression language used at every site; `case`
     is `if c then a else b`, and a row filter is
     `flat_map |k, r| if p then r else ()`, which is exactly the
     argument ADR 0015 used to remove `filter`.  Adding them would give
     the language two spellings for one meaning, in the one place the
     project has been most careful to have exactly one.
   - **`set` and `update` presuppose a mutability model the language
     deliberately lacks.**  ADR 0019 dropped `const`/`var` and assigned
     per-attribute mutability to the deferred change-control document
     (`@audited`, `@versioned`).  Designing a mutation form here would
     silently pre-empt that document's central question.
   - **`insert` is an effect.**  Pipelines are pure and lazy, and the
     hosting site decides when one runs.  Putting a write into the
     expression sublanguage requires an effect discipline the type
     system does not have and M4 does not need, and the roadmap itself
     already places writes outside the language: "Store ingestion via
     the CLI or as a library; the over-the-wire transport is wired in
     M7."

   The roadmap line is amended rather than implemented.

2. **The backend write path is delta-shaped.**  The storage trait gains
   one method taking a batch of inserts and deletes against a table's
   boundary shape, keyed off the same `TableShape` that `scan` and
   `materialize_view` already take, and reporting how many rows of each
   it applied.

   Insert and delete lists are preferred to DBSP's `(row, weight)`
   Z-set encoding.  Weights earn their keep inside a circuit, and no
   circuit exists until M5; the two representations convert in a few
   lines (an insert is weight `+1`, a delete `-1`), so this is a
   widening at M5 rather than a rewrite.  That is the discipline
   `docs/toolkit/04-processing-layer.md` already committed to: the
   boundary is kept DBSP-shaped so that "replacing the batch evaluator
   with a DBSP circuit at M5 changes the engine behind the boundary, not
   the boundary".

   Views are unaffected.  They are still recomputed in full by
   `mensura run`, so a registry that grew since the last run is picked
   up by the next recompute; incremental refresh is M5's.

3. **The decoder is typed against the `Schema` and independent of any
   wire format.**  A record is a **name-keyed** map of field name to
   scalar value, decoded against the resolved schema: dotted names
   address flattened compound components (ADR 0032), each value is
   decoded per its `ColumnType`, an `enum` value must be a declared
   variant, a value may be absent only where the column is optional, and
   an unknown field is an error, because the column set is closed and a
   typo'd field name is a real bug that silence would hide.  A failure
   names the record and the field.

   This decoder is the substantive deliverable of the slice, and it is
   deliberately format-free: it maps a name-keyed record onto a schema
   and knows nothing about how the record arrived.

   **Why not a wire standard, and specifically not GraphQL.**  The
   question is natural, since ADR 0006 does name standards.  Three
   reasons it is not this slice's:

   - **Wrong direction.**  ADR 0006's projection table places GraphQL
     under *query* and *subscribe*, and ingest under MQTT, REST `POST`,
     and gRPC client-stream.  A GraphQL mutation is request/response
     with a query language wrapped around each call; M4's need is to
     decode and append a batch.
   - **Wrong milestone.**  ADR 0006 decided the core is transport-
     agnostic and that transport selection belongs to a deploy
     configuration read by `mensura serve`.  Adopting a protocol here
     would bake a wire into the toolchain a milestone before the ADR
     that owns wires is implemented, and would pre-empt its open
     question about what `mensura serve` exposes by default.
   - **Already promised, on the other side.**  ADR 0006 commits that a
     GraphQL schema is generated from the resolved schema, with `domain`
     foreign keys as its edges.  That is M7 work, on the read side, and
     duplicating it here would produce a second answer to a settled
     question.

   Because the decoder is format-free, M7 wiring a GraphQL mutation, a
   REST `POST` body, or an MQTT payload onto it adds a caller and
   changes nothing beneath.

4. **`mensura ingest` reads JSON Lines.**

   ```
   mensura ingest <file.mensura> <target> --data <rows.jsonl> [--db <path>]
   ```

   `--data -` reads standard input.  The target names a store or a
   registry in the program; naming a view is an error.  A registry
   accepts appends only (ADR 0033 decision 4); a store accepts appends
   in this slice, with upsert and delete behind later flags.  The batch
   is one transaction: all rows land or none do.  `mensura run` is
   unchanged.

   JSON Lines is the encoding a local batch file uses, chosen because it
   is line-oriented (so one malformed record does not poison the file
   and a large batch streams), it needs no schema negotiation, and it is
   the shape M7's REST and MQTT payloads already carry.  It is an
   interchange encoding for a CLI reading a file, in the same sense that
   `--db path.db` names a local database: not a transport decision, and
   by decision 3 not a load-bearing one.

5. **`PRAGMA foreign_keys` goes on.**  The backend sets it per
   connection when opening a database, so the read and write paths agree
   about whether the `FOREIGN KEY` clauses it already emits mean
   anything.  This settles the question ADR 0032 decision 6 deferred to
   this slice.

   The clauses are in the DDL already, so enforcement costs one
   statement; a `domain` block is a *declared constraint*, and a write
   that violates it is a bug the storage layer catches correctly and for
   free; and the alternative reimplements referential checking in
   Mensura, where it cannot see a concurrent writer.  A violation is
   reported as a typed error naming the `domain` entry, the offending
   child values, and the target table, not as a raw SQLite constraint
   code.

   One consequence to state plainly: turning the pragma on enforces
   constraints on databases created before this slice, which is the
   intent but is a behaviour change for an existing database file.

6. **Affine (Celsius) conversion is deferred, with its reason.**
   ADR 0026 decision 7 and `docs/language/11-physical-units.md` say an
   offset unit is handled "by an explicit value-level conversion at
   ingestion".  That names where the conversion belongs semantically; it
   does not say this slice ships the hook.

   The rule here: an ingested `D[real]` column carries a **base-unit
   magnitude**, matching the storage convention that values normalize to
   base and the dimension is tracked only at the type level.  A payload
   in Celsius is converted by its producer.

   Deferring is right because a conversion hook needs a surface on which
   to declare the input unit per column, and that surface is the
   endpoint payload contract (M7, ADR 0006's transport projection), not
   the registry declaration.  A registry-level `@in(celsius)` today
   would land a member of the annotation family ahead of the annotation
   document and be in the wrong home once endpoints exist.

7. **Deferred, recorded.**  Update and delete on a store through the
   CLI (the trait supports them; the change-control semantics are the
   deferred policy document's).  Per-record best-effort reporting
   (`--continue-on-error`); this slice is all-or-nothing.  Streaming
   ingestion, backpressure, and window closedness (M5 and M7).  Every
   wire protocol, all of which become callers of decision 3's decoder.

## Consequences

Positive:

- M4's stated output is met with no new language surface, so the LL(1)
  grammar and ADR 0007's single sublanguage are untouched by the write
  path.
- The decoder is the reusable part, and it is reusable precisely because
  it is format-free: M7 adds transports as callers.
- Declared referential structure becomes enforced rather than
  documentary, closing ADR 0032's open question in the slice it was
  assigned to.
- The backend boundary reaches M5 already delta-shaped, so adopting a
  circuit widens the representation instead of replacing the interface.

Negative:

- Ingestion is real code with no language-level test harness: the
  must-accept / must-reject corpus exercises the *declaration*, while
  the write path needs runtime tests of its own.
- Turning on foreign-key enforcement changes how the toolchain treats a
  pre-existing database, including one a user only ever ran views over.
- All-or-nothing batches make a single bad record reject a large file,
  which is the safe default but is unhelpful for exploratory loading
  until `--continue-on-error` exists.

Neutral:

- JSON Lines is a choice this ADR could revisit without disturbing
  anything above the reader, since decision 3 puts no format knowledge
  in the decoder.
- A store and a registry share one intake implementation; only the set
  of permitted operations differs.

## Alternatives considered

1. **Building the roadmap's `insert`/`update`/`set`/`where`/`case`
   forms.**  Rejected per decision 1: two of them duplicate the
   expression sublanguage that ADR 0007 unified and ADR 0015 pruned, two
   pre-empt the mutability model ADR 0019 deferred, and the fifth puts
   an effect in a pure lazy pipeline language.  The roadmap line is
   older than the ADRs that decide against it.

2. **GraphQL as the ingestion surface.**  Rejected per decision 3: it is
   the wrong direction in ADR 0006's own projection table, it is M7's
   milestone, and it duplicates the generated read-side schema ADR 0006
   already promises.

3. **Generating a GraphQL SDL artifact now**, read-only, as a by-product
   of resolution.  Genuinely cheap and it would validate ADR 0006's
   "falls out of the read side for free" claim early.  Rejected as scope:
   M4's output is an ingestion path, and the artifact belongs with the
   M7 serving work that consumes it.  Recorded because it is the first
   thing to try when M7 opens.

4. **Ingestion as a pipeline sink** (`rows |> into readings`).  Reads
   idiomatically, but puts an effect in the expression sublanguage,
   needs an effect discipline the type system does not have, and answers
   the "when does it run" question that `07-pipelines.md` assigns to the
   hosting site.  Rejected.

5. **A weight-carrying Z-set delta API now.**  No consumer until M5's
   circuit, and the conversion is a few lines, so the churn buys
   nothing.  Rejected, and recorded in decision 2 so the M5 change is
   understood as a widening.

6. **Positional records (CSV) rather than name-keyed.**  Breaks on
   ADR 0032's dotted compound columns, where the flattened order is a
   resolver detail no payload author should have to track, and cannot
   express an absent optional value distinctly from an empty one.
   Rejected.

7. **Leaving `PRAGMA foreign_keys` off** and pre-checking references in
   the decoder.  Reimplements what SQLite does correctly from clauses
   already emitted, and a pre-check cannot see a concurrent writer.
   Rejected.

## Open questions

- Whether a partially-bad batch should ever be best-effort, and if so
  what the report looks like (decision 7 defers `--continue-on-error`).
- Whether ingestion should refuse a store whose `domain` targets are
  not yet populated, or let the foreign-key error of decision 5 speak.
- The affine input-unit surface of decision 6, which lands with M7's
  payload contract.
- Whether the library API should be public from `mensura-runtime` at
  M4 or wait until M7 has a second caller to shape it.

## Forward references

- `docs/toolkit/05-ingestion.md` (the surface specified here),
  `00-storage-backend.md` (the write path and the pragma),
  `04-processing-layer.md` (recompute semantics, unchanged).
- `docs/decisions/0033-registry-declarations.md` (the append-only intake
  of its decision 4).
- `docs/decisions/0006-transport-agnostic-surface.md` (M7 transports as
  callers of decision 3's decoder).
- `docs/decisions/0026-dimensional-physical-units.md` decision 7 (the
  affine conversion deferred in decision 6).
