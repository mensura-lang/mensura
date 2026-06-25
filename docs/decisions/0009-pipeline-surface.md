# 0009: Pipeline surface

## Status

Accepted.  Specified in `docs/language/07-pipelines.md`, with the lineage
follow-up in `docs/language/08-lineage.md`.

## Context

The data-handling algebra is formalized and stable in Lean
(`formal/Mensura/`), and the expression sublanguage is specified
(`06-expressions.md`, ADR 0007).  What was missing is the surface for the
data-transformation layer: how a user writes a pipeline of operations over a
table.  The design has to satisfy three constraints at once:

- It must be the **one expression sublanguage**, not a second grammar (ADR
  0007): a pipeline is an expression of table type.
- It must make the **tracked properties first-class**.  The whole point of
  the type system is that a table carries more than rows; the surface should
  read in terms of what each operation does to **cardinality** and
  **completeness** (and the ADR-0004 qualifiers), so semantic mistakes are
  compile errors.
- It must follow the project's discipline of **few general primitives, sugar
  later**.

This ADR records the decisions; `07-pipelines.md` is the full specification
and `04-grammar.md` carries the small grammar additions.

## Decision

- **Pipelines are expressions.**  Operations are ordinary curried builtins
  applied by juxtaposition; `|>`, `let`, and tuples compose them.  No separate
  pipeline grammar.
- **Primitives only.**  This round specifies a small kernel; the familiar
  named forms (`filter`, `mutate`, `select`, `aggregate`,
  `group`/`ungroup`/`project`, window functions, `tagged_bind`/`tagged_split`)
  are sugar over the kernel and are deferred.
- **The kernel.**  `map` (per-row); `group_map` (per-key whole-group);
  `extend_key`/`shrink_key` (reindexing); `left_join`/`inner_join`; `split`
  and `bind`; `unpivot`/`pivot`.  Each is backed by a named Lean theorem.
- **Reindexing is one idea with two directions, and the direction fixes the
  Tier.**  Moving a column *into* the key (`extend_key`) is split-safe
  (`ungroup_splitSafe`, Tier A); moving one *out* (`shrink_key`) is not
  (`project_not_preservesDisjoint`, Tier B).  `group`/`ungroup`/`project` are
  sugar over this.
- **`group_map` is one operation; result cardinality is inferred.**  Returning
  a single record yields card 1 (the `aggregate` shape, and what lets `pivot`
  satisfy `card <= 1`); returning a bag yields card many (the window shape).
  This is the `fiberMap` form (`fiberMap_splitSafe`), with `map` and
  `aggregate` as named cases.
- **`bind` is total.**  It is the multiset union and is always split-safe
  (`bind_comm`, `bind_assoc`, `bind_split`); it has *no* disjointness
  precondition.  Binding non-disjoint inputs only loses the `card <= 1`
  guarantee.  Disjointness (the no-leak property) is a lineage-qualifier
  concern, not an algebra precondition.
- **Completeness is a tracked property that Tier B consumes.**  `shrink_key`
  and the index form of `pivot` are sound only over a complete partition.
  `completeness_check { assert ... }` is its **own pipe stage** that
  *establishes* the completeness fact for the type checker; `@complete_over`
  establishes it globally on a source; a `collect` source is complete by
  mechanism; `assume` is the escape hatch.
- **Cardinality and completeness are first-class.**  Every operation states
  how it transforms cardinality and how it propagates or demands completeness,
  alongside its content and key effects.
- **Brackets and binders.**  Records are `( )` values written with leading-dot
  fields, `(.a [: Type] = value, ...)`; `{ }` is reserved for blocks and
  declaration bodies; `[ ]` is type/shape parameters.  `:` means typing only,
  `=` means value-binding only, uniformly across schema fields, `let`, record
  fields, and lambda return ascriptions.

## Consequences

Positive:

- One sublanguage carries the whole surface; pipelines need almost no new
  grammar (record literals and statement blocks, both small).
- Split-safety composes (`SplitSafe.comp`), so an all-Tier-A pipeline is
  leak-free by construction; the only ceremony is at the two Tier B
  operations.
- The kernel is small and each operation maps to a proof, so soundness is
  argued once and the sugar inherits it.
- Cardinality tracking pays off concretely: `pivot`'s attribute form
  type-checks exactly when the spread cell is `card <= 1`, which an upstream
  `group_map` establishes.

Negative:

- The fuller surfaces of two primitives depend on expression features not yet
  specified: row-dropping/expanding `map` and bag-returning (window)
  `group_map` need conditionals, collection literals, and ordering.  The doc
  specifies the signatures and defers those surfaces.
- Reindexing by bare column names, and aggregation as a cardinality-inferred
  `group_map`, are less familiar than `group by` / `aggregate`; the familiar
  spellings arrive only with the sugar round.

Neutral:

- `@complete_over` is named as the global completeness mechanism but its
  annotation surface lands with the annotation family, not here.

## Alternatives considered

1. **Named `group`/`ungroup`/`project`/`aggregate` as primitives.**  Familiar,
   but they are all reindexing or bag-reduction; making them primitives
   multiplies the kernel.  Rejected in favour of one reindex primitive and one
   `group_map`, with the names as sugar.
2. **A singleton-only `aggregate`.**  Gives the card-1 guarantee but forbids
   windows (cumulative sum, rank).  Rejected: one `group_map` with inferred
   cardinality gives both, and the card-1 case still feeds `pivot`.
3. **`bind` restricted to disjoint tables.**  Rejected: the theory shows the
   multiset union is total and safe; disjointness only governs the `card <= 1`
   guarantee, so restricting `bind` would forbid sound programs for no reason.
4. **Records in `{ }`.**  Forces a record-versus-block disambiguation on every
   brace.  Rejected: records move to `( )` (with leading-dot fields), leaving
   `{ }` to mean only a block.
5. **`completeness_check` as a clause bound to its Tier B op.**  Ties the
   obligation to the op syntactically but needs `completeness_check` to be a
   reserved word that ends an application spine.  Rejected in favour of an
   ordinary pipe stage that establishes the fact by position.

## Open questions

- **Cardinality-type notation.**  How card 1 / 0-or-1 / many is written in a
  `Type` (the content/types document).
- **Naming the partition** a completeness fact is "complete over", and how a
  `completeness_check`'s asserts are tied to it.
- **`@complete_over` and the annotation family** surface.
- **Window ordering.**  How the dependency qualifier supplies the order that
  `rank`/`cumsum` need on a group.
- **Hosting.**  `transform`/`view` declarations and the streaming operations
  that consume these pipelines.
