# 0025: Nomenclature consistency sweep

## Status

Accepted.  Renames a set of surface operations, lambda conventions, and
formal identifiers so that every name says what it does to a reader with
no SQL or PL folklore to unlearn.  Realized across the living documents
(`docs/language/`, `docs/toolkit/`), the book (`book/src/`), the examples
and corpus, the checker/runtime/LSP/highlighter crates, and the Lean
formalization (`formal/`).  `00-overview.md` carries the authoritative
glossary, including the new `bag` entry this ADR leans on.

This ADR is also the **translation reference** for earlier ADRs.  ADRs
are append-only history and keep the names they were written with, so
`0009`, `0015`, `0020`, `0023`, and `0024` still say `extend_key`,
`group_map`, `ungroup`, `project`, `bind`, and `collect`; the tables
below map each to its current name.  The same holds for Lean lemma
citations in older ADRs (`project_ungroup`, `ungroup_functional`, and
friends): the definitions still exist under their new names
(`demote_promote`, `promote_functional`).  The book's Chapter 5 is source
material, not specification, and is cited as-is (`def:grouping`,
`def:projection`, `bind`).

## Context

Since `docs/decisions/0022-observations-as-bags-declared-store-cardinality.md`
the per-key multiset of rows is officially a **bag**, and that word now
drives the cardinality axis (`singletons`/`bag`), the aggregate types
(`bag<T>`), and the completeness story.  Three problems followed:

- Three vocabularies coexisted for the key moves: the book's
  `ungroup`/`project`, the surface `extend_key`/`shrink_key`, and the
  "promote/demote" prose of `docs/decisions/0024-key-moves-as-a-true-inverse-pair.md`.
- "Group" survived in prose, code identifiers, and diagnostics even
  though the concept it names is the bag, dragging in GROUP BY intuitions
  that mislead (the bag exists by virtue of the key alone; nothing groups
  it).
- Several operation names borrowed another tradition's word for an
  operation that is not quite that thing (`join`, `bind`).

Where a clearer name exists, the surface departs from the book.

## Surface renames

| Retired | Current | Why |
| --- | --- | --- |
| `map` | `flat_map` | The body returns a collection (`()` drops the row, `(a, b)` expands), flattened into the key's bag.  That is exactly Rust/Scala `flat_map`, not a 1:1 map; the honest name also explains why there is no separate `filter` primitive. |
| `group_map` | `map_bag` | The lambda receives the key's whole bag (`\|k, b\|`) and returns its replacement: one bag in, one bag out, a map over the table's bags.  A record return is a singleton replacement (the aggregate shape, collapsing the bag to one row); a bag return is a many-row replacement (the window shape).  The aggregation, like `flat_map`'s filtering, happens inside the lambda, so the operator's name stays neutral where `reduce` or `agg` would be false for windows.  "Group" speaks SQL in a language whose word is "bag", and it suggests the operation groups something; the bag exists by virtue of the key alone. |
| `extend_key` | `promote` | The verb everyone already reaches for: ADR 0024's prose ("promote, work at the finer key, demote") and the corpus file names (`roundtrip_promote_first*`) used it while the code said `extend_key`.  The verb's object is exactly the argument: `promote ts` moves the column `ts` into the key. |
| `shrink_key` | `demote` | Same, in the other direction: `demote ts` moves the key component out.  Retiring the formal `project` alias also removes the collision with relational-algebra projection (column selection), which this operation is not. |
| `left_join` | `lookup` | The operation is not a general relational join but a directed lookup against a fixed table: a lambda maps each current row to the right table's key and pulls that row's columns in, as optionals.  "Join" promises symmetric relation matching that is not there. |
| `inner_join` | `lookup_total` | The same lookup with the postcondition named in Mensura's own axis: the pulled columns come out **total**, at the price of dropping unmatched rows.  "Inner" is SQL jargon for that trade.  Once value narrowing lands, this may dissolve into sugar for `lookup` plus a row filter on presence. |
| `bind` | `union` | To a PL reader `bind` is monadic `>>=`, which this is not: it is the per-key multiset sum that reassembles split parts.  `union` is what "recombine disjoint parts" is called everywhere; the glossary notes it is SQL `UNION ALL` (duplicates accumulate, nothing deduplicates), and the cardinality qualifier surfaces any overlap at compile time anyway.  The statistical alternative `pool` was considered and set aside as less immediate. |
| `collect` | `registry` | The declaration is the sole intake through which its observations enter (an ingestion endpoint in the proposal), which is why it can promise completeness by mechanism.  `registry` is the noun for exactly that institution: a record that is complete because recording is obligatory (birth registry, land registry).  "A collect" is a verb pressed into noun duty and says intake without saying completeness. |

**`map` stays deliberately vacant.**  It is retired without a successor:
bare `map` carries the strongest prior in programming (per-element, which
a reader takes to mean per-row), the `|k, x|` call site gives no other
cue about what the lambda receives, and pairing a bag-receiving `map`
with a row-receiving `flat_map` would falsely suggest the Rust/Scala
relationship (same input, flattened output).  Since ops are matched by
name in a keyword-free lexer, `map` should remain a recognized-but-rejected
name with a pointed diagnostic ("no `map` in Mensura: `flat_map` receives
a row, `map_bag` receives the bag") instead of a silent misread.  That
targeted diagnostic is **not yet implemented**: `map` currently falls
through to the generic "unsupported operation" path with an edit-distance
suggestion.  The follow-up is flagged with a `TODO(ADR-0025)` at the
operation-dispatch fallback in `mensura-types/src/pipe_check.rs`.

## Conventions and prose

- **Lambda parameters.**  `k` stays the key everywhere; `r` is a single
  row, now including the lookup lambda (retiring the `l` seen in
  `07-pipelines.md`, since the parameter is just the current table's
  row); `b` is the bag, retiring `g` (for "group").
- **"Group" and "grouping" are retired from living prose** in favour of
  "bag" and "the rows at a key", across `00-overview.md`,
  `06-expressions.md`, `07-pipelines.md`, and the book chapters
  (`the-kernel.md`, `what-the-types-track.md`).  Domain-of-art terms that
  are not the bag concept stay (ML "group leak", "grouped CV"; the
  parenthesized-expression sense of "grouping" in the grammar).
- **`00-overview.md` is the authoritative glossary.**  It gains a `bag`
  entry defining the primary sense (the multiset of rows at one key) and
  framing the expression-level `bag<T>` (a column's values across those
  rows) as a projection of it, records the bag/fiber correspondence, and
  notes that `union` means multiset sum, not deduplicating set union.
- **The `flat_map`/`map_bag` asymmetry is deliberate.**  Only the
  row-level operation flattens (each row's returned collection is merged
  into the bag); the bag-level operation maps one bag to one bag, so
  "flat" would be false there.  The names differ where the semantics do.

## Formal (`formal/`) renames

Lean definitions that model a surface operation take the surface name;
theorem names follow their subject:

- `map` becomes `flatMap` (`flatMap_splitSafe`, `flatMap_eq_fiberMap`,
  ...).
- `ungroup` becomes `promote` (`promote_splitSafe`, `promote_unionHom`,
  `promote_preservesDisjoint`, `promote_functional`, ...).
- `project` becomes `demote` (`demote_completeWrt`,
  `demote_not_preservesDisjoint`, `demote_eq_gatherMap`, ...).
- `bind` becomes `union`, and the `*_bindHom` family becomes
  `*_unionHom`.
- The inverse-pair laws become `demote_promote` and `promote_demote`.

Two formal names deliberately stay mathematical.  `fiberMap` (with
`fiberOf`, `fiberCompleteWrt`, `keyLocal`) states results about fibers in
general, of which `flatMap` and `aggregate` are instances; "fiber" is the
formal register for the surface's "bag", and the correspondence is
recorded once in the overview glossary.  `aggregate` stays as the
record-returning specialization, matching the "aggregate shape" prose.

## Code that follows the surface

- Checker identifiers: `Context::group` -> `Context::bag`,
  `group_value_record` -> `bag_value_record`, `op_group_map` ->
  `op_map_bag`, `group_record_content` -> `bag_record_content`,
  `op_bind` -> `op_union`.
- Diagnostics: "a reducing `group_map` needs completeness ..." -> "a
  reducing `map_bag` ...".
- The op-name match in `pipe_check`, the highlighter/LSP completions,
  the grammar doc's op list, `docs/examples/*.mensura`, and the corpus
  (including the file renames `shrink_after_assume` -> `demote_after_assume`,
  `shrink_reindex_bag` -> `demote_reindex_bag`).
- The two environment-binding helpers also named `bind` (a lambda-scope
  binder in `expr_check.rs`, an env binder in `pipe_check.rs`) are **not**
  the pipeline operation and keep their names.
- `collect` is not yet implemented, so `registry` costs only doc edits.

## Checked and deliberately kept

- `singletons` / `bag` on the cardinality axis (ADR 0022).
- `split` (the operation the whole type system is organized around;
  "split-invariant" and "split-safe" reinforce it), now paired with
  `union`.
- `pivot` / `unpivot` (ADR 0020's true inverse pair; universally
  understood, and the `un-` prefix advertises the inversion).
- `assume` (honest about admitting an obligation by fiat), `attr` /
  `attr*`, `store`, `view`, `shape`, `unit`.
- Tier A / Tier B, and the "aggregate" and "window" shapes of `map_bag`.
- Qualifier vocabulary: establish / consume / discharge, completeness,
  totality, lineage, grading, functional.

## Alternatives considered

- **`bind` -> `pool`** (the statistical term for combining samples): no
  wrong PL prior, but less immediate than `union` for "recombine disjoint
  parts".  Set aside.
- **Keep `left_join`/`inner_join`.**  Familiar, and they give the right
  first intuition, but they name a symmetric relational join the
  operation is not; `lookup`/`lookup_total` are accurate and reuse
  Mensura's own totality axis.
- **Rename `map` to bare `map` for the bag operation** (dropping the
  `_bag` suffix): rejected under the vacancy argument above.
