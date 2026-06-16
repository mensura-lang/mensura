# Mensura roadmap

A phased plan for building the Mensura language and its tooling. The goal is a
language that encodes data-handling, sampling, dependency, and lineage
properties into the type system so that semantic mistakes (data leakage, wrong
CV strategy for temporal data, biased training sets, broken split-invariance)
become compile errors.

## Implementation choices (decided)

- **Host language:** Rust.
- **Parser:** hand-written recursive descent over an LL(1) grammar. No parser
  generator, no backtracking, one token of lookahead. The grammar in
  `docs/language/01-syntax.md` must be specified as LL(1); any construct that
  cannot be expressed in LL(1) must be reworked at the syntax level rather
  than handled by the parser.
- **Runtime backend:** Apache Arrow + Polars. Mensura is interpreted on top of
  a Polars `LazyFrame`-shaped runtime, at least until the language stabilizes.
- **CLI shape:** a single `mensura` binary with subcommands. Subcommands are
  added milestone-by-milestone (examples only, not decided):
  - `mensura check <file>` — typecheck only
  - `mensura run <file>` — typecheck + execute
  - `mensura test [<filter>]` — run language tests and endpoint tests
  - `mensura fmt <file>` — format
  - `mensura repl` — interactive REPL
  - `mensura lsp` — language server (speaks LSP over stdio)
  - `mensura serve <file>` — run a Mensura program as a web service
    (stores/collect endpoints)
  - `mensura migrate <from> <to>` — generate a migration plan between two
    schema revisions
- **Specs first.** Every language- or tooling-level feature lands as a design
  document under `docs/` before code. Code is the encoding of an agreed-upon
  spec, not the place where decisions get made.

## Repository layout (proposed)

```
mensura/
  ROADMAP.md            -- this file
  docs/
    language/           -- language design documents (one per concept)
    toolkit/            -- design docs for mensura subcommands
    examples/           -- worked examples used to validate the design
    decisions/          -- ADR-style notes for non-obvious choices
  crates/               -- once implementation begins
    mensura-syntax/     -- lexer, parser, AST
    mensura-types/      -- type checker, disjointness solver
    mensura-runtime/    -- Polars-backed interpreter
    mensura-cli/        -- the `mensura` binary
    mensura-lsp/        -- the `mensura lsp` subcommand backend
```

## Pre-M0 — Design docs only

No code yet. The output of this phase is a `docs/` tree thick enough that
implementation becomes mechanical.

Minimum set of documents before M0 ends:

- `docs/language/00-overview.md` — what Mensura is, its design pillars,
  what is in and out of scope.
- `docs/language/01-syntax.md` — surface grammar (informal, EBNF-ish).
- `docs/language/02-types.md` — the type quadruple
  `Table<Sampling, Dependency, Lineage, Content>` and its subtyping.
- `docs/language/03-units-and-tables.md` — units, indexes, measures, targets,
  relates; constants vs. variables; the `is` schema-extension form.
- `docs/language/04-operations.md` — the algebra. Tier A (split-invariant) and
  Tier B (require completeness checks). Each operation gets a typing rule.
- `docs/language/05-lineage.md` — lineage trees and the disjointness solver.
- `docs/language/06-sampling-dependency.md` — the sampling and dependency
  hierarchies, where they propagate, where they require `assume`.
- `docs/language/07-stores-and-collect.md` — `store` vs. `collect`, the
  CRUD/REST mapping, what properties each derives from its mechanism.
- `docs/language/08-streaming.md` — windows, embeddings, reactive `on` blocks,
  snapshots.
- `docs/language/09-flexibility.md` — `assume`, store-derived properties,
  the policy on where the language deliberately stops checking.
- `docs/toolkit/00-cli.md` — the `mensura` binary and its subcommands.
- `docs/toolkit/01-diagnostics.md` — error model and how types show up in
  diagnostics (this is a feature, not a polish item).
- `docs/toolkit/02-lsp.md` — what the LSP exposes; hovering must reveal the
  full type quadruple.
- `docs/decisions/0001-rust-polars-interpreter.md`
- `docs/decisions/0002-no-defaults-policy.md`
- `docs/decisions/0003-cli-subcommand-shape.md`

Validation criterion: every code example in `data-handling.tex`, the proposal,
and the postdoc report can be transcribed to Mensura syntax and the docs say
unambiguously whether each one type-checks.

## M0 — Calculus & spec freeze

Output: a versioned spec document collecting the rules from the design docs
into a single typing-rule reference. No implementation, but the spec is
detailed enough that two people implementing independently would build
compatible compilers.

Specifically:

- Core grammar in EBNF, proven LL(1): no left recursion, disjoint FIRST sets
  at every alternative, and FIRST/FOLLOW disjoint wherever a nullable
  production appears. The freeze is contingent on this proof.
- Type quadruple `Table<S, D, L, C>` with subtyping rules.
- Typing rules for: `partition`, `filter`, `sample`, `with_ordering`,
  `temporal_split`, `bind` (tagged), `split` (tagged), `select`, `mutate`,
  `aggregate`, `ungroup`, `pivot` / `unpivot`, `left_join`,
  `tumbling_window`, `sliding_window`, `embed`.
- Disjointness solver semantics over lineage trees.
- A working definition of split-invariance for binary operations (closes the
  open question in `data-handling.tex`).
- Completeness propagation rules for `collect`-sourced data.
- A test suite of "must accept" and "must reject" programs, derived from the
  examples in the chapter and the postdoc report.

## M1 — Frontend: parse + typecheck (no execution)

Output: `mensura check file.mensura` accepts/rejects programs with span-based
diagnostics.

- `mensura-syntax`: lexer, hand-written recursive-descent LL(1) parser, AST,
  pretty-printer. One token of lookahead; no backtracking.
- `mensura-types`: name resolution (two-pass), type-checker over the
  quadruple, disjointness solver. Predicate disjointness has a decidable
  fragment (linear arithmetic over numeric fields) and falls back to `assume`
  outside it.
- `mensura-cli`: just the `check` subcommand to start.
- Diagnostics with source spans, suggested fixes where possible.

Validation: the M0 test suite classifies every example correctly.

## M2 — Vertical slice: end-to-end ML pipeline

Output: the concise pipeline at the end of `mensura.tex` (load → partition →
random-forest train → evaluate) actually runs end-to-end.

- `mensura-runtime`: Polars-backed runtime for `Table`. `LazyFrame` underneath
  most operations.
- Implement `load`, `partition`, `filter`, `sample`, `evaluate` at runtime.
- Wire one ML algorithm (random forest, via `linfa` or `smartcore`) with its
  typed signature.
- `mensura run` subcommand.
- Disjointness proven at compile time, then trusted at runtime.

This is the "first working language" milestone. Narrow on purpose.

## M3 — Algebra surface (chapter Tier A + Tier B)

Output: every operation in `data-handling.tex` is a language primitive, with
correct typing.

- Tier A: tagged `bind`/`split`, `pivot`/`unpivot`, `select`, `filter`,
  `mutate`, `aggregate`, `ungroup`, `left_join`.
- Tier B: `project`/`group`, inner `join`, grouped/arranged variants of
  `mutate`/`filter`. Each requires `completeness_check { ... }` or a
  `complete_over` annotation on its source.
- `transform` and `view` blocks compile to typed pipelines.
- `is` schema-extension form with its required derivation transform.

## M4 — Stores, endpoints, auth (the proposal.md scope)

Optional for the academic deliverable; required for the language to be
usable as a backend platform. Defer the decision to fork this off as a
separate `mensura-server` project until after M3.

Output: the college case study from `proposal.md` runs as a web service.

Design settled ahead of implementation:
`docs/decisions/0005-identity-and-authorization.md` (federated SPIFFE
identity, unified `auth {}`, RBAC plus bounded ABAC),
`docs/decisions/0006-transport-agnostic-surface.md` (core stays
wire-agnostic, deploy config owns transport selection), and
`docs/language/05-naming-and-casing.md` (canonical names and wire
translation).

- `store` and `collect` with auto-generated REST endpoints.
- Auditing (`@audited`), versioning (`@versioned`), auto-fields (`@auto`),
  creation-control (`@allowcreate`).
- OAuth integration as specified in the proposal.
- `mensura test` runs both language-level tests and endpoint tests.
- Permission-flow analysis to catch missing permissions at compile time
  (resolving the proposal's `XXX`).
- `mensura serve` subcommand.

## M5 — Tooling

Run in parallel with M3, not after — the LSP is what makes the type system
visibly useful and is essential for any demo.

- `mensura lsp`: hover (showing the full type quadruple),
  diagnostics, completion, goto-def, find-references. Hovering on any
  binding must reveal sampling, dependency, lineage, content.
- `mensura fmt`.
- `mensura repl` for exploratory work.
- `mensura migrate`: diff two schema revisions (typically two git commits)
  and emit a migration plan. Initial version targets schema diffs only;
  data-migration scaffolding comes later.

## M6 — Reactive & streaming

Output: the IoT pipeline from `streaming.tex` runs.

- `store` declarations with `endpoint`, `temporal`, `fleet`, `mode
  append_only`, `report_interval`.
- Per-window sampling inference: Exhaustive when the fleet is fully covered,
  Biased / Representative otherwise.
- `tumbling_window`, `sliding_window`, `embed` with their dependency-typing
  rules.
- `on store.every(...)` reactive blocks.
- `snapshot` for consistency under retraining.
- The $\bot$ (not observed) vs. `?` (missing) distinction surfaced in the
  type system.

## M7 — ML strategies & validation

Output: the type system catches the full bug-class catalog the project
promises to prevent.

- Algorithm signatures encoded as typeclasses/traits: `random_forest`,
  `arima`, `mixed_effects`, etc., each with its structural input
  requirements.
- Validation strategies: k-fold, stratified, temporal, grouped — each with a
  disjointness proof.
- Metrics typed by content (regression vs. classification, etc.).
- "Showcase test suite": leakage, wrong-CV-on-temporal, unit mismatch,
  group-leak. Each must be a compile error.

## Cross-cutting (continuous)

- Documentation site generated from `docs/` and code comments.
- Examples repo: college case study (M4), IoT case study (M6), ML validation
  case study (M7).
- Benchmark suite vs. pandas/Polars/tidyverse on equivalent workloads —
  feeds the eventual paper.
- ADR-style decision log under `docs/decisions/` for any non-obvious choice
  made during implementation.

## Suggested execution order

```
Pre-M0  ──►  M0  ──►  M1  ──►  M2  ──┬──►  M3  ──┐
                                     │           ├──►  M6  ──►  M7
                                     └──►  M5  ──┘
                                     M4 (optional, parallel after M2)
```

M5 (LSP and tooling) starts as soon as M2 is demoable — typed feedback in an
editor is the language's main user-facing claim, not a polish item.

## Open questions to resolve before M0 freezes

1. **Is M4 in scope as a Mensura goal, or a separate project?** The postdoc
   report scopes the academic work to ML validation; M4 (web service /
   endpoints / auth) goes well beyond that. Two viable answers: keep M4 as a
   `mensura-server` companion project, or accept that Mensura is a
   general-purpose data-handling language and absorb M4 into the core.
2. **`assume` only, or also an `exploratory` mode?** Recommendation: only
   `assume`, no mode. Every relaxation is local, visible, and auditable.
3. **Scope of split-invariance for binary operations.** The chapter leaves
   this open and conjectures inner join is unsafe. M0 must close this
   formally — without it, the typing rules for `bind` and `join` cannot be
   written precisely.
