# Mensura roadmap

A phased plan for building the Mensura language and its tooling.  The goal is a
language that encodes data-handling, sampling, dependency, and lineage
properties into the type system so that semantic mistakes (data leakage, the
wrong CV strategy for temporal data, biased training sets, broken
split-invariance, unit mismatches) become compile errors.

The plan is aimed at one driving application: a **streaming
predictive-maintenance service over a fleet of devices**.  Reaching it end to
end (dimensional sensor units, device ingestion over the wire, windowed
features, incrementally refreshed views, and a leak-free temporal/grouped
validation pipeline served behind endpoints) is the North Star that orders the
milestones below.

## Implementation choices (decided)

- **Host language:** Rust.
- **Parser:** hand-written recursive descent over an LL(1) grammar.  No parser
  generator, no backtracking, one token of lookahead.  The grammar in
  `docs/language/04-grammar.md` must be LL(1); any construct that cannot be
  expressed in LL(1) is reworked at the syntax level rather than handled by the
  parser.
- **Backend: storage and processing are split.**  Stores are persisted in
  SQLite (`rusqlite`, in `mensura-runtime`); pipelines and views are evaluated
  by an incremental, DBSP-style processing layer, which is what the streaming
  and `on_change`-refresh targets need.  The split is specified in
  `docs/toolkit/00-storage-backend.md`.
- **CLI shape:** a single `mensura` binary with subcommands, added
  milestone-by-milestone:
  - `mensura check <file>`: typecheck only.
  - `mensura run <file>`: typecheck and execute.
  - `mensura test [<filter>]`: run language and endpoint tests.
  - `mensura fmt <file>`: format.
  - `mensura repl`: interactive REPL.
  - `mensura lsp`: language server (LSP over stdio).
  - `mensura serve <file>`: run a program as a web service (store and
    `registry` endpoints).
  - `mensura migrate <from> <to>`: generate a migration plan between two schema
    revisions.
- **Specs first.**  Every language- or tooling-level feature lands as a design
  document under `docs/` before code.  Code is the encoding of an agreed-upon
  spec, not the place where decisions get made.

## Repository layout

```
mensura/
  ROADMAP.md            -- this file
  docs/
    language/           -- language design documents (one per concept)
    toolkit/            -- design docs for the subcommands and the backend
    examples/           -- worked examples that must compile (validate design)
    decisions/          -- ADR-style notes for non-obvious choices
  book/                 -- the Mensura book (mdBook, hosted on GitHub Pages);
                           ```mensura examples are highlighted and check-gated
  crates/
    mensura-syntax/     -- lexer, parser, AST
    mensura-types/      -- name resolution, the resolved Schema, the hooks;
                           bundles the stdlib modules (stdlib/si.mensura)
    mensura-runtime/    -- SQLite storage backend and the processing layer
    mensura-highlight/  -- source classification shared by the LSP and the book
    mensura-lsp/        -- the `mensura lsp` backend
    mensura-mdbook/     -- the `mdbook-mensura` preprocessor (book highlighting)
    mensura-cli/        -- the `mensura` binary
  formal/               -- Lean 4 formalization of the algebra (Mathlib-backed);
                           see decisions/0008
```

## Status: where we are

- **Design.**  `docs/language/00-overview` through `09-typing-reference`, ADRs
  0001-0011, and `docs/toolkit/00-storage-backend.md` exist.  The core is
  specified: units, stores, shapes, the LL(1) grammar, naming, the expression
  sublanguage, the pipeline primitives, and lineage/disjointness.  The table
  type is `Table<Qs, C>` (a row of qualifiers plus content); sampling,
  dependency, and lineage are standard-library qualifiers, not language slots
  (ADR 0004).
- **Calculus.**  The data-handling algebra is mechanized in Lean 4 under
  `formal/`: split-safety and its composition, completeness, the split-safe
  `pivotAttr` with its reversibility, the `union` disjointness lemma, and
  (ADR 0029's Stage 1) the monoid-parameterized bag fold with its shard and
  presence lemmas, which is the gate `fold` ships behind.  Stage 2 (an
  arranged structure over the row multiset, gating `scan`) is the next
  formal item and is open-ended.
- **Implementation.**  The pipeline `source -> tokens -> AST -> resolved Schema
  -> SQLite` is built for the "basic" subset: scalar-key units, stores with
  primitive and `enum` attributes, shapes, and named enums.  The expression
  sublanguage and the full Tier A / Tier B pipeline algebra (the eight
  primitives, with cardinality, completeness, and tag-based disjointness) are
  implemented and checked by `mensura check` (M1).  Batch view materialization
  runs end to end (M2).  **M3 is complete**: dimensioned types (`D[real]` over
  the seven SI base dimensions), top-level `let` bindings, bundled imports,
  and the `si` standard library are implemented (ADRs 0026-0028).  Compound
  (multi-entity) units and foreign-key (`domain`) resolution are scheduled for
  M4 (where devices and cross-store ingestion first need them); precision is
  deferred to a future library extension of `real`.  **Const functions** (ADR
  0030) and the **fold half of the aggregate family** (ADR 0031) are
  implemented on top of M3's module machinery: lambdas and partial application
  as compile-time values, `fold` and `map` as curried builtins over a closed
  combiner table, the fiber (`b`) as a bag of rows with `b.x` as projection
  sugar, `#` as cardinality, and the six reductions as const bindings in a
  bundled `bag` module rather than intrinsics.  `scan` and the `series` module
  wait on ADR 0029's Stage 2 (M5).
- **Design docs still to write** (each ahead of its milestone, per specs
  first): devices and `registry`; ingestion endpoints; streaming windows and
  refresh; measure semantics (M5, where the window rollups it gates are the
  first consumer); precision; ML signatures and validation; the
  serving/transport integration; and the toolkit docs for the CLI,
  diagnostics, and LSP.

The original design-only phase is essentially complete for the core; what
remains is captured per milestone below.

## M0 - Calculus and spec freeze

Output: a versioned typing-rule reference collecting the rules from the design
docs into one place, detailed enough that two people implementing independently
would build compatible compilers.

Status: a first freeze candidate exists at `docs/language/09-typing-reference.md`.
It makes the four tracked properties explicit in `Table<Qs, C>` (cardinality and
totality in the content `C`; completeness and disjointness, the latter via a
lineage tag hierarchy, in a concrete closed `Qs`) and freezes the pipeline
algebra with Tier A / Tier B and split-invariance.  It defers the extensible
qualifier meta-calculus (ADR 0004), the sampling and dependency qualifiers, and
`08-lineage.md`'s predicate-region elaboration of disjointness.  This narrows the
meta-calculus scope ADR 0004 anticipated for M0; reconciling that ADR is a
follow-up.  The companion LL(1) grammar proof remains the open M0 item.

- Core grammar in EBNF, proven LL(1) (no left recursion, disjoint FIRST sets,
  FIRST/FOLLOW disjoint at every nullable production), including the expression
  productions.  The freeze is contingent on this proof.
- The `Table<Qs, C>` type and the qualifier framework (ADR 0004): the
  propagation combinators and the constraint-hook interface.
- Typing rules for the pipeline primitives (`flat_map`, `map_bags`,
  `promote`/`demote`, `lookup`/`lookup_total`, `split`/`union`,
  `unpivot`/`pivot`), with their cardinality and completeness effects.
- The disjointness constraint hook over the lineage qualifier
  (`docs/language/08-lineage.md`).
- A must-accept / must-reject test suite derived from the book's Chapter 5 and
  the worked examples.

The algebra underpinning the freeze is mechanized in Lean 4 (done; see
`docs/decisions/0008-formalize-algebra-in-lean.md`); the split-safety results
are proved there before the calculus is declared stable.

## M1 - Frontend for the core language

Output: `mensura check file.mensura` accepts or rejects programs over the whole
core language, with span-based diagnostics.

- `mensura-syntax`: extend the parser past the declaration subset to the
  expression sublanguage and the pipeline primitives (record literals,
  statement blocks, `|>`, `|x|` lambdas), per `04-grammar.md`.
- `mensura-types`: type-check expressions and pipelines over `Table<Qs, C>`,
  including cardinality, completeness, and disjointness.  Disjointness is
  tracked by the **tag-based lineage hierarchy** (`09` section 9), with
  `completeness_check` / `assume` discharging the Tier B completeness
  obligation.  The symbolic predicate-region fragment (linear arithmetic over
  numeric key fields, `08-lineage.md`) is **deferred** to M6, where the
  learning operations first consume disjointness.
- `mensura-cli`: the `check` subcommand.
- Diagnostics with source spans and suggested fixes where possible.

Validation: the M0 suite classifies every example correctly.

## M2 - Processing runtime and the first pipeline

Output: `mensura run` materializes a Tier A view from stores, end to end
(non-streaming first).

- `mensura-runtime`: the DBSP-style processing layer over the SQLite storage
  backend (`docs/toolkit/00-storage-backend.md`).
- Implement the Tier A primitives at runtime (`flat_map`/`filter`/`map_bags`/
  `lookup`/...), reading from and writing to stores.
- Disjointness and completeness proven at compile time, then trusted at
  runtime.

This is the "first working language" milestone; narrow on purpose.

## M3 - Physical units and modules (complete)

Output: dimensional quantities are first-class, and unit mismatch is a compile
error.  Delivered.

- Design docs first (ADR 0026 dimensions, 0027 modules, 0028 the `si` stdlib;
  the language docs `11-physical-units.md` and `12-modules-and-imports.md`
  on top).
- Dimensions as the free abelian group over the seven SI base dimensions,
  formally backed (ADR 0026); a dimensioned type is `D[real]`; unit checking
  and automatic (linear) conversion, with affine units (Celsius) handled at
  ingestion.
- Units are ordinary dimensioned constants (`9.8 * meter / second^2`),
  shipped by a small module system (ADR 0027: top-level const bindings +
  imports with bundled resolution) and a bundled `si` standard library
  consistent with the mechanized group (ADR 0028, as amended: hand-written
  with an oracle test until the table grows enough to earn a generator).
  The third-party package layer (manifest, hashes, `pin`) is provisional
  and deferred until it has a consumer; finalizing it (the marked import
  form, the manifest schema, the check artifact) is a later,
  post-M3 roadmap item of its own (ADR 0027, Decision 7).
- Precision (the `NxE` significand/exponent idea) is deferred to a future
  library extension of `real`, not M3 core.
- Measure semantics was originally scoped here and has **moved to M5**, next
  to the window rollups it gates.  Nothing in M3 consumes it, and ADR 0029
  changed what it should be: an annotation over the combiner table rather
  than a taxonomy of columns.

Status: complete.  Dimensions, modules, and the `si` stdlib are implemented
and checked (ADRs 0026-0028, `11-physical-units.md`,
`12-modules-and-imports.md`).

## M4 - Devices, collect, and ingestion

Output: device readings land in stores under a typed ingestion path.

- Design docs first: devices and `registry`; ingestion (the `insert`/`update`/
  `set`/`where`/`case` forms).
- `device` and `registry` declarations; `registry` is complete by mechanism
  (overview pillar 7).
- Store ingestion via the CLI or as a library; the over-the-wire transport is
  wired in M7.
- Compound (multi-entity) units and foreign-key (`domain`) resolution: a
  reading keyed by several entities, and `domain` blocks resolving a column to
  another store's key.  Deferred from M1 (the frontend rejects them today with
  "not yet supported"); needed here because cross-store ingestion is the first
  consumer.

## M5 - Streaming and reactive

Output: windowed, incrementally refreshed views over device streams.

- Design docs first: streaming windows and refresh; measure semantics (below).
- `sliding_window` and tumbling windows, `latest`, window-closedness, and
  `on_change` / incremental refresh through the processing layer.
- **Measure semantics**, moved here from M3 because the window rollups it
  gates are the first consumer.  ADR 0029 reframed it: the annotation
  declares which **combiners** a column admits (a temperature column admits
  `min` and `max` but not `+`), a property of the (column, combiner) pair
  rather than a taxonomy of columns.  That dissolves the problem of defining
  `@semiadditive` before the language has an axis notion, and it means the
  doc is best written against ADR 0029's combiner table, which `fold` now
  realizes (ADR 0031; the table lives in `09-typing-reference.md` section
  5.4).
- Per-window sampling inference (Exhaustive when the fleet is fully covered,
  Biased or Representative otherwise).
- The temporal and dependency typing rules, and temporal referential integrity
  (the "outlives" constraint), extending `docs/language/08-lineage.md`.
- The ordered primitives that window functions need: **`scan` and `prescan`**,
  gated behind ADR 0029's Stage 2 `formal/` work (an arranged structure over
  what is today a `Multiset`).  ADR 0031 revised the shape: the sorted map is
  dissolved, since `rank`, `lag`, and `lead` are all scan-derived, so the
  primitive set is `fold` and `scan` plus `map`.  Landing Stage 2 also brings
  the `desc` order marker and the bundled `series` module (`cumsum`, `rank`,
  `running_min`, `running_max`, `first_value`, `lag`, `lead`).

## M6 - ML strategies and validation

Output: the type system catches the full bug-class catalogue the project
promises to prevent, and the leak-free predictive-maintenance pipeline
computes.

- Design doc first: ML signatures and validation.
- Model signatures (`fit`, `predict`, `evaluate`) as typed primitives
  (`random_forest`, `arima`, `mixed_effects`, ...), each with its structural
  input requirements.
- Validation strategies (k-fold, stratified, temporal, grouped), each with a
  disjointness proof; feature/label separation via shapes and lineage;
  censoring via `is known`.
- A showcase suite in which leakage, the wrong CV on temporal data, unit
  mismatch, and group leak are each a compile error.

## M7 - Serving, transport, and auth (the North Star)

Output: the streaming predictive-maintenance service runs end to end.

Design settled ahead of implementation:
`docs/decisions/0005-identity-and-authorization.md` (federated identity, a
unified `auth {}`, RBAC plus bounded ABAC) and
`docs/decisions/0006-transport-agnostic-surface.md` (the core stays
wire-agnostic; deploy config owns transport selection).  Naming and wire
translation are in `docs/language/05-naming-and-casing.md`.

- Auto-generated REST and MQTT endpoints for stores, `registry`, and views.
- Device identity, RBAC, and compile-time permission-flow analysis.
- Change-control annotations (`@audited`, `@versioned`, `@auto`,
  `@allowcreate`).
- Live views served with `on_change` refresh.
- `mensura serve`, and `mensura test` over language and endpoint tests.

## Cross-cutting (continuous)

- **Tooling.**  `mensura lsp` (hover reveals the full `Table<Qs, C>` type:
  every qualifier and the content), `mensura fmt`, `mensura repl`, and
  `mensura migrate` (schema diffs first, data-migration scaffolding later).
  The LSP runs in parallel from about M1; typed feedback in an editor is the
  language's main user-facing claim, not a polish item.
- **Examples discipline.**  Worked examples live in `docs/examples/`, grow
  incrementally (one milestone's features at a time), and are kept compiling:
  each is exercised by a resolve/run test (as `committed_example_resolves` does
  for `college-stores.mensura`), so a milestone that breaks an example fails
  CI.  A college case study and a streaming fleet example are the running
  integration tests.
- **Diagnostics** are a feature, not polish; the error model gets its own
  toolkit design doc.
- **Benchmarks** against pandas/Polars/tidyverse on equivalent workloads, to
  feed the eventual paper.
- **Decision log.**  ADRs under `docs/decisions/` for any non-obvious choice
  made during implementation.

## Suggested execution order

```
M0 ──► M1 ──► M2 ──► M3 ──► M4 ──► M5 ──► M6 ──► M7
              │
              └──► tooling (LSP, fmt, repl) in parallel from ~M1
```

Units (M3) precede streaming (M5) because a window rollup is a rollup of
dimensioned quantities, and because measure semantics (now an M5 item) is
stated over them; streaming precedes ML validation (M6) because the features
are windowed; serving (M7) is last because it puts the whole typed pipeline
behind endpoints.

## Validation criterion

Every example in the book's Chapter 5 and the worked case studies in
`docs/examples/` transcribes to Mensura, and the docs say unambiguously whether
each type-checks.  The per-milestone must-accept / must-reject suite classifies
them correctly, and every `docs/examples/` file compiles.

## Open questions

1. **`assume` only, or also an `exploratory` mode?**  Recommendation: only
   `assume`, no mode.  Every relaxation is local, visible, and auditable.
2. **How much of the serving surface forks into a companion project?**  The
   core language is usable without the web surface; whether `mensura serve` and
   the transport layer live here or in a `mensura-server` companion is settled
   when M7 is scoped.
3. **Decidability bounds of the qualifier hooks.**  The disjointness hook has a
   decidable fragment with an `assume` fallback (`08-lineage.md`); whether
   other `std` qualifiers stay inside a decidable fragment is open (ADR 0004).

The earlier open question on split-invariance for binary operations is closed
by the Lean formalization: `union` is total and split-safe, and the Tier A / Tier
B boundary is proved.
