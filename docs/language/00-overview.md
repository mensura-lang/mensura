# Mensura: overview

Mensura is a statically typed language for **data handling**, in which the
type system encodes properties that data manipulation libraries (pandas,
tidyverse, Polars, dplyr, …) leave to runtime, convention, or the
programmer's discipline.  The compiler rejects programs whose data-handling
operations are *syntactically valid but semantically wrong*: data leakage
between training and test sets, the wrong cross-validation strategy on
temporal data, biased sub-sampling, broken split-invariance, unit
mismatches, and so on.

The language is small. The novelty is not in the surface syntax but in the
typing rules attached to each operation.  Those rules are collected, for the
settled core, in `docs/language/09-typing-reference.md`.

## Motto

*Measure twice, run once.*

## Design pillars

1. **Tables are the central object.** A Mensura table is the indexed table
   of Chapter 5 of Data Science Project: An Inductive Learning Approach
   (F. A. N. Verri, 2026; doi: 10.5281/zenodo.14498010): a tuple
   `(K, H, c)` of index columns, non-index columns, and a cell function.
   Rows are entities, identified by their index tuple; a key may carry
   several rows (cardinality), and individual values may be missing.
   Values are total by default, with optional ones marked `?` (see
   `docs/decisions/0010-attribute-totality.md`).

2. **The type of a table is `Table<Qs, C>`.** A table binding carries a row
   of **qualifiers** `Qs` and a **content** schema `C`, both checked at
   compile time. The qualifiers are defined in the standard library rather
   than baked into the language (see
   `docs/decisions/0004-qualifier-mechanism.md`); the canonical ones are:

   - **sampling**: how the rows came to be in this table (Exhaustive,
     Representative, Biased, …). Determines what statistical claims are
     defensible.
   - **dependency**: whether rows are independent, grouped, ordered,
     temporal, or otherwise structured. Determines which split and CV
     strategies are sound.
   - **lineage**: the provenance a table carries through a pipeline; its
     *disjointness* constraint hook proves that two tables share no entities,
     which is what licenses leak-free train/test validation (see
     `docs/language/08-lineage.md`).

   `C`, the **content**, is the schema: index columns, non-index columns,
   their domains, units, cardinality, and semantic types. Operations are
   typed as transformations on `Qs` and `C`; every primitive carries rules
   for how each qualifier propagates and how the content changes.

3. **Split-invariance is the default.** Chapter 5's Tier A operations
   (`bind`, `split`, `unpivot`, the attribute form of `pivot`, `select`,
   `filter`, `mutate`, `aggregate`, `ungroup`, and `left_join`/`inner_join`
   against a fixed table) are split-invariant by construction and require no
   extra ceremony. The Tier B operations (`project`/`group`, which shrinks the
   key, and the index form of `pivot`) break split-invariance and require an
   explicit `completeness_check { … }` stage, or a `complete_over` annotation
   on their source, to be admissible. See `docs/language/07-pipelines.md`.

4. **Indexes and units are part of the type.** Each table declares its
   index columns, and each column declares its domain, including physical
   units and semantic refinements (CPF, email, regex-constrained strings).
   Unit and semantic mismatches are compile errors, not runtime
   conversions.

5. **Constants vs. variables.** Non-index columns are split into `const`
   (facts that should not change), `var` (data that evolves), and the
   annotations that govern this evolution: `@audited`, `@versioned`,
   `@auto`, `@allowcreate`. The distinction between immutable facts and
   evolving state is encoded in the type, not left to convention.

6. **No defaults that hide assumptions.** Where existing tools silently
   pick a row order, a join key, an imputation strategy, or a CV scheme,
   Mensura requires the user to state it. Where the user wants to bypass a
   check, they write `assume`, locally and visibly.

7. **Properties are derived from mechanism, not declared.** When data
   enters Mensura through a `store` or `collect` declaration, the sampling,
   dependency, and (initial) lineage of the resulting table are fixed by
   the declaration's mechanism, not chosen by the programmer.

## In scope

- A core algebra for data handling (the operations of Chapter 5, with
  their typing rules).
- The `Table<Qs, C>` type (qualifiers plus content), with sampling,
  dependency, and lineage as standard-library qualifiers, and the
  disjointness constraint hook over lineage.
- A Polars-backed interpreter sufficient to run typed pipelines
  end-to-end.
- Compile-time prevention of the specific bug classes the postdoc report
  promises to address: leakage, wrong-CV-on-temporal-data, unit mismatch,
  group-leak, broken split-invariance.
- Validation strategies and ML algorithm signatures as typed primitives
  (random forest, ARIMA, mixed-effects, k-fold, stratified, temporal,
  grouped), each with its disjointness obligations.
- A `mensura` toolchain: `check`, `run`, `test`, `fmt`, `repl`, `lsp`.

## Out of scope (for the academic deliverable)

- A general-purpose data-transformation language competing with pandas,
  Polars, or tidyverse on coverage. The postdoc report explicitly scopes
  the work to ML-validation correctness; breadth is a non-goal.
- Storage engines, query planning beyond what Polars provides, distributed
  execution.
- The web-service surface (`store`/`collect` endpoints, OAuth, REST,
  auditing/versioning at the HTTP layer) is **deferred** to M4 and may be
  spun off as a companion `mensura-server` project. The core language is
  usable without it. Its design is settled in
  `docs/decisions/0005-identity-and-authorization.md` and
  `docs/decisions/0006-transport-agnostic-surface.md`.
- A new query language for analytics. Mensura is a transformation
  language; it does not aim to replace SQL.

## Non-goals

- Turing-completeness as a goal in itself. Mensura is expressive enough
  for data-handling pipelines and ML validation; it deliberately stops
  short of becoming a general-purpose programming language.
- Performance parity with hand-tuned Polars. The interpreter exists to
  prove the type system carries through to execution; speed is M2-and-after
  work.

## Where this fits in the literature

Existing formalizations of data-handling algebras (LaraDB by Hutchison
et al., 2017; Modin by Petersohn et al., 2020; SDTA/SDTL by Song et al.,
2021/2022) answer *can we express this operation?* and *can we execute
it efficiently?* They do not answer *should this operation be allowed?*
Mensura is the answer to the third question, built on top of the
indexed-table model and the split-invariance property developed in
Chapter 5.

## What this document is not

This is an orientation, not a specification. The typing rules, the grammar,
the disjointness solver, the algebra, and the toolchain each have their own
document under `docs/language/` and `docs/toolkit/`. The roadmap
(`ROADMAP.md`) lists them and the order in which they are written.
