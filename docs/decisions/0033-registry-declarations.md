# 0033: Registry declarations

## Status

Accepted.  Extends `docs/decisions/0002-stores-tabulate-units.md` with a
second tabulation kind and discharges the `registry` forward reference
that `docs/language/02-stores.md` leaves open.  Delivers the
by-mechanism establishment that
`docs/decisions/0017-completeness-establish-consume.md` named and
deferred to M4, at the consumer placement
`docs/decisions/0023-completeness-consumed-by-the-reducer.md` fixed and
with the contentful bag reading
`docs/decisions/0022-observations-as-bags-declared-store-cardinality.md`
predicted.  Implements the name
`docs/decisions/0025-nomenclature-consistency-sweep.md` chose.  The
intake this declaration promises is specified in
`docs/decisions/0034-typed-ingestion.md`, its companion in the same
slice.  Endpoint exposure and `auth {}` stay with M7
(`docs/decisions/0005-identity-and-authorization.md`,
`docs/decisions/0006-transport-agnostic-surface.md`).

## Context

Five documents already promise that a `registry` source is complete by
construction: `docs/language/00-overview.md` (pillar 7 and the
establishment list), `07-pipelines.md`, `08-lineage.md` ("`split` is to
disjointness what `registry` is to completeness"),
`09-typing-reference.md` section 8, and the book's *What the types
track*.  The checker implements none of it:
`crates/mensura-types/src/table.rs` constructs `Completeness::Incomplete`
for every store, under a doc comment that already describes the
registry behaviour it does not perform.

ADR 0025 renamed `collect` to `registry` on the explicit grounds that
"`collect` is not yet implemented, so `registry` costs only doc edits".
That is no longer true of the milestone: M4 is the slice where the
construct becomes real, and the parser does not yet know the word (a
`registry` declaration dies on the generic item fallback that lists the
seven keywords it does know).

The cost is visible in the driving example.
`docs/examples/fleet-monitoring.mensura` carries `assume { complete }`
under a comment reading "Until ingestion (M4) establishes the fact at
the source by mechanism, the view assumes it there".  The assumption
exists only because the mechanism does not.

What this ADR must settle, none of it fixed by the documents above:

- whether a registry is a new kind of table or a store with a different
  intake, and what that choice costs the resolved model and the runtime;
- what completeness fact a registry establishes, given that ADR 0022
  makes "complete over the key" contentful only on a `bag`;
- what append-only means statically, when the language has no write
  path and ADR 0019 deferred the mutability model;
- whether a registry shares the store and view table namespace, and
  whether a `domain` entry may target one, given that ADR 0027 forbids
  importing one.

## Decision

1. **A registry is a store with a different intake discipline, not a
   new kind of table.**  The grammar mirrors `store_decl` exactly:

   ```ebnf
   registry_decl = "registry" ident [ conforms ] "{" unit_clause { store_block } "}" ;
   ```

   Same mandatory `unit { U }` clause, same `attr` and `attr*` blocks,
   same `domain` blocks, same conformance clause.  A registry of a
   compound unit needs `domain` resolution exactly as a store does, and
   the `attr*` bag registry is the case decision 2 makes contentful, so
   neither is withheld.  The declarations differ in their introducer
   word and in nothing else the parser can see.

2. **A registry's table type is `Complete` at its declared boundary,
   whatever its cardinality.**  One rule, no cardinality condition.  The
   fact reads differently at the two cardinalities, and both readings
   are wanted:

   - On a `singletons` registry it is **trivially true**, and it is the
     `Mensura.fiberCompleteWrt_of_functional` corollary: at `card <= 1`
     a present key's single row is its whole fiber, which is already
     ADR 0023's base case for discharging a reducer over a `singletons`
     input.
   - On an `attr*` **bag** registry it is **contentful**, and it is
     exactly what ADR 0022 anticipated: "on a `bag` store, 'complete
     over the key' is contentful and establishable at the source (an
     annotation, or a `collect` mechanism): the store pins the full set
     of observations per entity, so it is where the *reference*
     population of `0023`'s `CompleteWrt` lives".  A bag registry is
     that reference population, because the declaration is the sole
     intake.

   The fact is established at the source, so every Tier A operation
   preserves it and `demote` propagates it to the coarser key
   (`Mensura.demote_completeWrt`) with no discharge step anywhere in
   the pipeline.

3. **The resolved `Schema` gains a `kind`; nothing else in the runtime
   changes.**  `StoreKind::{Store, Registry}` sits on the declaration
   and on the resolved `Schema`, and the registry declaration reuses the
   store's AST payload rather than introducing a parallel one.

   Stated as a consequence audit, because the audit is the argument:
   `TableShape`, `CREATE TABLE` generation, the key index, `scan`,
   `materialize_view`, the evaluator's source tables, shape
   conformance, the `domain` resolution pass, and the store-graph
   acyclicity check are all **untouched**, because a registry
   materializes as exactly the same table with exactly the same key
   discipline.  The only consumer of the new field is the lift from
   `Schema` to the pipeline's table type, where it selects the
   completeness qualifier.  A parallel `RegistryDecl` and a parallel
   resolved type would fork six resolver functions and the boundary IR
   for a construct that differs by one boolean.

4. **Append-only is a property of the intake, not a static property of
   the declaration.**  The language has no write path: ADR 0019 dropped
   `const`/`var` and left per-attribute mutability to the deferred
   change-control document, so "immutable" cannot be a column-level type
   fact today, and this ADR does not invent one.

   What is real is the intake surface.  ADR 0034 gives a registry
   *append* and nothing else: no update, no delete, no upsert.  That is
   the mechanism the completeness fact rests on, and it is why the fact
   is sound: the declaration is the sole intake for its observations,
   and the intake only ever adds.  It is also what keeps ADR 0027
   Decision 2 coherent, since importing a registry would create a second
   consumer of that sole intake.

5. **A registry joins the one table namespace, and a `singletons`
   registry is a legal `domain` target.**  Stores and views already
   share a single namespace because both name a queryable table; a
   registry does too, and a collision with either is the same error it
   is today.  A view body reads a registry by name exactly as it reads a
   store, which is the whole point.

   A `domain` entry may resolve into a **`singletons` registry**.
   ADR 0032 Decision 5 restricts targets by *cardinality*, not by kind,
   and its three reasons (a `bag`'s entity presence is incidental, the
   companion-store guidance, a `bag` has no primary key to reference)
   are all satisfied by a singletons registry, which has a primary key
   and a well-defined set of observed values.  A `bag` registry is
   rejected as a target for the same reasons a `bag` store is.

   This does not weaken ADR 0027 Decision 2.  "Not importable" is about
   a **module** boundary, where a second program becomes a second
   consumer; a `domain` edge lives inside one program, consumes no
   observations, and creates no intake.  The store `domain` graph
   acyclicity check covers registries unchanged.

6. **Deferred, recorded.**  Each of the following is left to the slice
   that owns it rather than designed here:

   - `auth {}` on a registry, endpoint exposure, and the auto-derived
     permission scopes.  ADR 0006 keeps the core wire-agnostic and says
     the exposure surface is settled with M7's serving work; ADR 0005
     owns the identity model.
   - The `@complete_over(col)` annotation.  A registry establishes the
     same fact by mechanism, so the annotation loses its near-term
     consumer and stays with the annotation family
     (`09-typing-reference.md` section 13).
   - Whether the completeness fact should carry the key it is complete
     *over*.  ADR 0023's open question on giving `assume` a key argument
     applies verbatim to the by-mechanism fact.

## Consequences

Positive:

- The claim five documents and one doc comment already make becomes true
  in code, at a single line in the lift from `Schema` to the table type.
- `docs/examples/fleet-monitoring.mensura` loses its `assume { complete
  }` with nothing replacing it, which is the slice's concrete
  validation.  The example's `readings` becomes a **`singletons`**
  registry keyed by `(machine_id, taken_at)`, unchanged otherwise, so
  the ADR 0024 grading that discharges `scan`'s tie-freedom obligation
  in `reading_trend` survives intact.  Under the rejected
  bag-only reading of decision 2 (alternative 2) the driving example
  would have gained nothing, which is the strongest argument for the
  uniform rule.
- The bag registry is the first construct in the language where
  completeness is contentful *at the source*, closing the loop ADR 0022
  opened and ADR 0023 fixed the consumer for.
- The whole runtime, evaluator, storage backend, highlighter, and
  language server support registries with no work beyond one match arm,
  because a registry resolves to a `Schema`.

Negative:

- `Complete` on a `singletons` registry discharges nothing the reducer
  had not already discharged from cardinality, so a reader may find the
  uniform rule over-general.  Alternative 2 records why the
  cardinality-conditional version is worse.
- A second declaration keyword whose body is byte-for-byte a store's
  invites the question of why they are two declarations; the answer
  (completeness by mechanism, and an intake that only appends) is a
  type-level distinction with no syntactic trace inside the braces.

Neutral:

- A registry is un-importable across a module boundary yet a legal
  `domain` target within a program.  The two rules constrain different
  boundaries and neither weakens the other.
- Nothing here exposes a registry over a wire.  Until M7, its intake is
  the CLI and the library of ADR 0034.

## Alternatives considered

1. **A separate `Item::Registry(RegistryDecl)` with its own resolved
   type.**  Forks the declaration collection pass, `resolve_store`, the
   `domain` resolution pass, the store-graph check, conformance
   checking, and the `mensura run` loop, and adds a second collection to
   the resolved program, for a construct that differs from a store by
   one boolean and one intake method.  The `kind` field localizes the
   entire difference to the one place that consumes it.  Rejected.

2. **`Complete` only on a `bag` registry**, leaving a `singletons`
   registry `Incomplete` because the fact is vacuous there.  Rejected on
   three counts: the rule grows a cardinality condition and stops being
   one sentence; `registry` would mean different things at different
   cardinalities, so a reader could not predict the type from the
   keyword; and the "vacuous" case is not a wart but a corollary already
   proved (`fiberCompleteWrt_of_functional`) and already relied on.
   Decisively, it is the *singletons* case the driving example needs:
   under this alternative `fleet-monitoring.mensura` would keep its
   `assume { complete }` and the slice would deliver nothing visible.

3. **An `append_only` (or `mutable`) modifier on `store` rather than a
   new keyword.**  Puts change-control vocabulary into the language
   ahead of the change-control document (ADR 0019), and hides a
   *type-level completeness guarantee* behind what reads as a storage
   flag.  ADR 0025 chose `registry` precisely because the noun names an
   institution whose records are complete because recording is
   obligatory; the declaration should say that.  Rejected.

4. **Letting a registry omit the `unit` clause** (a free-form event log).
   Rejected: ADR 0001's identity discipline binds every tabulation, and
   a registry with no unit has no key for completeness to be *over*.

## Open questions

- Whether `mensura run` should say anything about a registry that has
  never been ingested.  An empty registry is complete, vacuously, and
  every reduction over it is faithful, so this is a usability question
  rather than a soundness one.
- Whether a future `registry` of a unit already tabulated by a store
  wants a stated relationship between the two (today they are
  independent tabulations that happen to share a unit).
- The key-carrying completeness fact of decision 6, if a consumer
  appears that needs completeness over a key coarser than the
  declaration's.

## Forward references

- `docs/language/13-registries.md` (the surface), `02-stores.md` (the
  discharged forward reference), `04-grammar.md` (the production and
  its LL(1) justification).
- `docs/decisions/0034-typed-ingestion.md` (the intake decision 4 rests
  on).
- `docs/decisions/0005-identity-and-authorization.md`,
  `0006-transport-agnostic-surface.md` (M7 exposure and `auth {}`).
- `formal/Mensura/Completeness/CompleteOver.lean`
  (`fiberCompleteWrt_of_functional` for decision 2's trivial reading,
  `demote_completeWrt` for its propagation).
