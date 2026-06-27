# 0015: Map as a row-multiset, key-first lambdas, filtering without filter

## Status

Accepted.  Generalizes the surface of `map` to the formal row-multiset (so
filtering and row-expansion fall out, with no `filter` primitive), splits every
pipeline lambda into a key-first form, adds `if`/`then`/`else` and a homogeneous
collection literal `( ... )`, and reserves a `const`/`var` record-field marker.
Touches `mensura-syntax` (grammar, AST, parser), `mensura-types` (`expr_check`,
`pipe_check`), and the examples/corpus/tests.  The implementation lands on the
same branch/PR as this acceptance.

## Context

The formal `map` (`formal/Mensura/Table.lean`, `def map`) is

```
map (φ : K → Row H σ → Multiset (Row H' σ')) (T : Table K H σ) : Table K H' σ'
  := ⟨fun k => (T.rows k).bind (φ k)⟩
```

The lambda takes the **key `k` and the value-row separately**, and returns a
**multiset of output rows**: "`0` drops it, a singleton keeps or transforms it,
and several rows expand it."  So `filter` (keep where a predicate holds) and
row-expansion are already *inside* `map`; `filter` was only ever going to be
sugar.  `map_splitSafe` / `map_bindHom` prove the general φ is split-safe.

The surface, however, forces a `map` lambda body to be a single record literal
(exactly one output row), conflates the key and the value in one parameter `|r|`
(so "output the value row" and `|r| r` are ambiguous, since `r` includes the
index columns the operation preserves), and offers no conditional or
multi-row literal.  `09 §13` parked "expression-level conditionals" and
"collection literals" as exactly the missing prerequisites.

## Decision

### 1.  Key-first lambdas

Every pipeline lambda binds the key first, mirroring `φ(k, row)`:

| operation | lambda | binds |
| --- | --- | --- |
| `split` | `\|k\|` | the key only (it routes whole keys; no value payload) |
| `map`, the join key | `\|k, r\|` | key `k`, value row `r` |
| `group_map` | `\|k, g\|` | key `k`, value columns as bags `g` |

`k` is a record of the index columns as single values; `r` is a record of the
non-index columns as single values; `g` is a record of the non-index columns as
bags.  Read the key with `k.id`, a value with `r.x`.  `|_, r|` ignores the key.

This corrects `group_map`: the key is constant within a group, so it is a single
value (`k`), not a bag.  `split` staying one-parameter fits the rule (it has no
value payload).

### 2.  `map` returns a collection of value rows

A `map` body denotes the formal `Multiset` of output value rows:

- `( ... )` is a **homogeneous collection**: `()` is empty (**drop**),
  `(a, b, ...)` is several rows (**expand**).  `(e)` is grouping.  A bare value
  row -- `r`, or a record `(.x = ...)` -- **auto-lifts to a one-element
  collection**.
- So `map \|k, r\| r` keeps, `map \|k, r\| ()` drops, `map \|k, r\| (a, b)`
  expands, and `map \|k, r\| (.degraded = r.status == "degraded")` transforms.
  Filtering is `map \|k, r\| if r.degraded then r else ()`.

The output content's non-index columns are the collection's row schema (all rows
share one schema).  The **index is preserved**, so an output record may not name
an index column.  **Cardinality is the maximum collection size**: `<= 1`
preserves the input bound (filtering keeps `singletons`); `>= 2` (a literal of
two or more, or a branch of size `>= 2`) yields `bag`.  Sizes are statically
known because a row lambda's body is literal/conditional-shaped.

This is exactly the formal map's bind-homomorphism, so there is **no new proof
obligation** and **no `filter` primitive**; a named `filter` may later be sugar
for `if c then r else ()`.

### 3.  Conditional expressions

`if c then a else b`: `c` is a known `bool`; the two branches unify to one type.
General (also valid in a field value, e.g. `.flag = if r.hot then 1 else 0`).
It is the introduction site for the deferred `is known` narrowing.

### 4.  Collection literal and the bracket budget

`( ... )` is the homogeneous collection (above).  `ExprKind::Tuple` is
reinterpreted as this collection; its only prior use, the `bind` pair
`(train, test)`, is a two-element collection of tables, so nothing is lost.
`[ ... ]` stays the **shape-argument** syntax and is not a collection literal.
A heterogeneous sequence `([ ... ])` is reserved for the future.  A record
`(.a = ..., .b = ...)` remains the heterogeneous form, disambiguated by the
leading `.`.

### 5.  Reserved record-field role marker

```
record_field = "." ident [ "const" | "var" ] "=" expr      // default: const
```

Parsed and carried in the AST now, so the surface is fixed; the type-level
meaning of `const`/`var` (a **column-scoped qualifier**, in the ADR 0013 sense,
parallel to totality) and its use in view shape-conformance are settled with the
view-conformance work.  There is no `index` field role: `map` preserves the
key, so a record never sets the index.

## Consequences

Positive:

- The surface matches the formal `map` exactly; filtering, identity (`|k, r| r`),
  transform, and expansion are all the one operation, consistently.
- The kernel stays small: no `filter` primitive (overview pillar of few
  primitives, sugar later).
- Conditionals unblock the deferred `is known` narrowing.
- `group_map` exposes the key as a single value, fixing a latent modelling bug.
- No new brackets: `( )` reused, `[ ]` left to shapes, `([ ])` reserved, records
  unchanged.
- The `const`/`var` field syntax is settled and forward-compatible with view
  conformance.

Migration (one-time, like the `number` split of ADR 0014):

- The three lambda contexts (`Context::row`/`group`/`key`) become key + payload;
  `pipe_check`'s lambda extraction handles two parameters for `map`/`group_map`.
- `expr_check` gains conditional and collection typing; `map` typing yields
  `0..n` rows with the cardinality-from-size rule.
- Every existing lambda migrates: `\|r\| ...` -> `\|k, r\| ...` (or `\|_, r\|`),
  `\|g\| ...` -> `\|k, g\| ...`, and index reads `r.x` -> `k.x`.  The examples,
  corpus, and `expr_check`/`pipe_check` tests move with it.
- AST gains `ExprKind::If`; `Tuple` is reinterpreted (and may be renamed) as a
  homogeneous collection; record fields gain the optional role.  The grammar
  adds `if`/`then`/`else` (reserved in expressions) and the field-role marker,
  kept LL(1).

Deferred:

- `const`/`var` as a column-scoped qualifier, and the `: Shape` conformance
  check on a view's output (the next design).
- `group_map` bag/window returns (a separate axis) and the named sugar
  (`filter`/`select`/`mutate`).
- The heterogeneous sequence `([ ... ])`.

## Alternatives considered

1. **A dedicated `filter` primitive.**  Rejected: the formal `map` already
   subsumes it (`filterRows_splitSafe` is derived), so a separate op duplicates
   it and grows the kernel.
2. **`[ ... ]` collection literal.**  Rejected: `[ ]` is the shape-argument
   syntax, so it would clash with `Tabular[Machine]`.
3. **Single `\|r\|` with `r` the whole row, and "a row in output position means
   its value columns"** (auto-project the index away).  Rejected: implicit, and
   the key/value conflation is exactly what `\|k, r\|` removes at the binder.
4. **`r.values` / `r.keys` projections** with a single `\|r\|`.  Rejected in
   favor of `\|k, r\|`, which separates them at the binder and needs no
   projection sugar.
5. **Postfix conditional `x if c else y`.**  Rejected for the conventional,
   LL(1)-trivial prefix `if c then a else b`.
6. **An `index` record-field role.**  Rejected: `map` preserves the key, so a
   record never sets the index; reindexing is `extend_key`/`shrink_key`.

## Forward references

- `docs/decisions/0013-qualifier-scope-and-the-content-boundary.md` (const/var
  fits as a column-scoped qualifier), `docs/decisions/0014-scalar-domain-taxonomy.md`.
- `docs/language/09-typing-reference.md` (section 5 expression rules and the new
  conditional; section 6.1 `map`; section 6.2 `group_map`; section 6.5 `split`).
- `formal/Mensura/Table.lean`: `map`, `map_splitSafe`, `map_bindHom`;
  `filterRows_splitSafe` (filter is derived, not primitive).
- The view shape-conformance design (the `const`/`var` qualifier plus the
  `: Shape` check on a view's computed output).
