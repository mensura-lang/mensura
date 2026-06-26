# 0012: Views host pipelines

## Status

Accepted.  Specified in `docs/language/10-views.md`, with the grammar addition
in `docs/language/04-grammar.md` and the naming rule in
`docs/language/05-naming-and-casing.md`.

## Context

The pipeline algebra is frozen (`docs/language/07-pipelines.md`,
`docs/language/09-typing-reference.md`) and pipelines are expressions of table
type (`docs/decisions/0009-pipeline-surface.md`, ADR 0007).  What was missing is
a place to *put* a pipeline: a declaration that names it, gives it sources, and
materializes its result so it can be queried.  ADR 0009 left this as an open
question ("Hosting") and `05-naming-and-casing.md` left the `view` naming rule
deferred because views were "underdefined (both type-like and served)".

A view is the first hosting site.  The decision has to settle four things: what
the declaration looks like, how a pipeline's sources bind, what the view's type
is, and whether a view is constrained at its boundary.

## Decision

- **A view hosts a pipeline.**  `view <name> [: Shape, ...] block`.  The body is
  the ordinary statement `block` of `06-expressions.md` (`let` bindings,
  `assert`, and a trailing table-valued result), so a view introduces no
  pipeline-specific grammar.  Sources resolve by name from the site's context
  (a bare `readings` is a store presented as a table value).
- **A view's content and properties are computed, not declared.**  A view has
  no `const`/`var` blocks; its schema, cardinality, totality, completeness, and
  lineage are whatever the hosted pipeline yields.  This is overview pillar 7
  (properties from mechanism) applied to a derived table.
- **A view is a resource, named snake_case.**  It is materialized and queried
  over a wire like a `store`/`collect` (ADR 0006), so it takes the term naming
  convention, not the type convention (`05-naming-and-casing.md`).
- **A view is not a forced unit boundary.**  Its output cardinality may be
  `bag`; it is a general materialized table.  The 0-or-1 unit-boundary rule of
  `docs/decisions/0001-unit-as-identity-discipline.md` binds only a table
  *promising a tabulation of a unit*, and a bare view promises none.
- **Structure constraints are opt-in via a shape; cardinality is not
  constrained.**  The optional `:` conformance clause is the one structural
  check a view may carry, run against the computed output schema with the
  existing store conformance check (`03-shapes.md`).  A unit-fixing shape
  (`Tabular[Machine]`) requires the output to carry that unit's index columns,
  but it checks *content* only: it does **not** impose the ADR 0001 `singletons`
  discipline.  Enforcing 0-or-1 cardinality on a view is left to a dedicated
  future syntax (open questions).
- **Tier A only, this round.**  A view body admits the split-safe Tier A kernel.
  Hosting the Tier B operations and discharging their completeness obligation,
  lineage-demanding sites, streaming/refresh, serving, and runtime
  materialization are deferred (`10-views.md`, "Scope of this round").

## Consequences

Positive:

- Hosting needs almost no new grammar: one `view_decl` production reusing the
  existing `conforms` clause and the expression `block`.  The "pipelines are
  expressions" decision (ADR 0009) pays off directly.
- A view's type is the pipeline's type, so the four tracked properties of
  `09-typing-reference.md` surface on a view for free, and a shape claim becomes
  a check on the pipeline's result (the "conformance machinery carries table
  properties" direction of `03-shapes.md`).
- Dropping the singleton boundary lets a view be a `bag`-shaped intermediate
  (long-form readings, an unaggregated join) without ceremony, while a
  unit-fixing shape still expresses the strict case when wanted.

Negative:

- There is no way, this round, to require a view to be a proper unit tabulation
  (0-or-1 per key): a shape checks the index structure but not the cardinality,
  so a `bag` view always type-checks.  The guarantee returns only when the
  deferred `singletons` syntax lands.
- `00-overview.md` and ADR 0001 describe `view` as a unit boundary; this ADR
  drops that for the bare `view` (cardinality may be `bag`), which those
  documents now cross-reference.

Neutral:

- The dropped cardinality boundary is specific to views as *derived* tables.
  `store` and `collect` are unchanged: they promise a tabulation of a unit, so
  the ADR 0001 0-or-1 rule still binds them (and the storage index->PRIMARY KEY
  mapping still relies on it).  The asymmetry is intentional, not an oversight.
- Whether `collect` and `device` host pipelines the same way is left to their
  own documents; this ADR settles `view` only.

## Alternatives considered

1. **Mandatory `unit { U }` clause and a `singletons` boundary**, mirroring a
   store exactly and enforcing ADR 0001 on every view.  Rejected: it forbids
   the common `bag`-shaped derived table for no algebraic reason.  The
   `singletons` guarantee is not folded into shape conformance either; it is
   deferred to its own syntax so that requiring it stays an explicit, separate
   choice.
2. **`const`/`var` blocks on a view**, declaring its columns like a store.
   Rejected: a view's content is computed by the pipeline (pillar 7); enumerating
   it would duplicate the algebra's output and invite mismatch.
3. **PascalCase view names**, treating a view as a type.  Rejected: a view is a
   materialized resource that is queried and served, so it reads as a value;
   the shape it may claim is the type, not the view.
4. **A dedicated pipeline grammar in the body** (a sequence of stages with their
   own syntax).  Rejected by ADR 0009 already: a pipeline is an expression, and
   the `block` with `|>`, `let`, and tuples expresses it with no new grammar.

## Open questions

- **Enforcing `singletons`.**  The syntax by which a view requires its output to
  be 0-or-1 per key (the ADR 0001 unit discipline), since neither a bare view
  nor a shape claim imposes it.
- **Materialization semantics.**  How `mensura run` computes and refreshes a
  view over the storage backend (the DBSP-style processing layer, M2).
- **Views reading views.**  Acyclicity of a view dependency graph and how a
  view's properties compose when its source is another view.
- **Tier B hosting.**  Where a `completeness_check` sits relative to a view that
  hosts a `shrink_key`, and how `@complete_over`/`assume` are spelled at the
  hosting site.
- **Serving.**  The query and subscription surface of a view and its
  authorization (M7, ADR 0006, ADR 0005).
