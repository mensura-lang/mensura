# 0019: Attributes as structure, const and var as store annotations

## Status

Accepted.  Refines `docs/decisions/0003-shapes-as-structural-contracts`: a
shape now describes structure only, and the `const`/`var` distinction moves
off shapes onto store attributes.  Amends
`docs/decisions/0015-map-row-multiset-and-key-first-lambdas` by dropping the
record-field role marker it reserved.  To be realized in
`docs/language/02-stores.md`, `docs/language/03-shapes.md`,
`docs/language/04-grammar.md`, and the overview glossary; the surface it
describes supersedes the `const`/`var` attribute blocks of `0002` and `0003`.

## Context

ADR 0003 defined a `shape` as "an optional `unit` clause plus `const` and
`var` attribute blocks" and made structural conformance require, per
attribute, the "same name after interpolation, same type, same `const`/`var`
block".  A shape therefore has a voice on `const`/`var`.

But a shape is a pure structural contract, claimed by stores, by views
(`docs/decisions/0012-view-hosting`), and later by function parameters.
`const` versus `var` is not structure.  It is a change-control property of
persisted data: `const` attributes are facts that should not change, `var`
attributes evolve, and (per the overview glossary) the distinction "governs
which change-control annotations (`@audited`, `@versioned`, `@auto`,
`@allowcreate`) apply".  ADR 0003 already excluded exactly those annotations
from shapes as store-only policy.  So `const`/`var` was misplaced: it is the
root of the store-only policy that a shape otherwise forbids.

The mismatch shows at the boundary.  A view has no `const`/`var` blocks at
all; its schema is whatever its pipeline yields.  A function parameter typed
by a shape is a table value with no persistence, so no mutability.  When a
shape encodes `const`/`var`, it over-constrains the contract with a property
that two of its three claimants cannot honestly carry, and the conformance
check has to compare a field that only stores possess.

This ADR pulls `const`/`var` off shapes and back onto stores, where the rest
of the change-control policy already lives, leaving shapes as structure alone.

## Decision

- **A new contextual keyword `attr`.**  Attributes in both stores and shapes
  are listed in `attr { ... }` blocks, superseding the `const` and `var` block
  introducers of `0002` and `0003`.  The word matches the glossary term
  "attribute" (already the umbrella over the two former categories) and the
  grammar's existing `attr`/`shape_attr` nonterminal names.  Like the
  blocks it replaces, `attr` is contextual: the lexer still emits it as an
  `Ident` and the parser recognizes it only in the block-introducer position.

- **Shapes carry structure only.**  A shape body is an optional `unit` clause
  plus `attr` blocks of `name : type` (with the optional `?` of `0010`).  A
  shape has no notion of `const`/`var`.  A bodyless or attribute-only shape
  means exactly what its structure says and nothing about mutability.

- **Conformance is name and type.**  ADR 0003's rule becomes: a store or view
  conforms to a shape if it has every attribute the shape requires with the
  same name (after interpolation) and the same type, including optionality,
  and may have more.  The "same `const`/`var` block" criterion is removed.
  One shape now spans stores and views uniformly, and a store's mutability
  choices never affect which shapes it satisfies.

- **`@const` and `@var` are store-only per-attribute annotations.**  Inside a
  store's `attr` block each attribute carries its mutability as an annotation
  written after the `name : type` (and any `?`): `admission: date @const`,
  `status: Status @var`.  The structural core stays identical to a shape
  attribute and the annotation trails as a modifier.  `@const`/`@var` are the
  first members of the store-only annotation family that ADR 0003 kept out
  of shapes; `@audited`, `@versioned`, `@auto`, and `@allowcreate` will
  compose in the same trailing slot, deferred to their own document.  Writing
  `@const` or `@var` in a shape or a view is an error.

- **The annotation is required in a store, with no default.**  Every store
  attribute must carry exactly one of `@const` or `@var`; omitting both is a
  compile error naming the attribute.  Mutability is load-bearing intent, so
  it is stated, not inferred.

- **The record-field role marker is dropped.**  The expression-level
  `(.a const = x)` / `(.a var = x)` marker that `0015` reserved is removed.
  Once `const`/`var` is a store-only change-control concern, tagging a field
  of a computed row with it is nonsensical: a pipeline result is not a
  persisted tabulation and has nothing to audit or version.  A record field
  is written `.a = x` (or `.a: type = x`).

- **The boundary IR is unchanged.**  `@const`/`@var` resolve to the same
  `Const`/`Var` column roles as before, and the storage column order (index,
  then const, then var) is unchanged.  Only the surface moves, from
  block-based to annotation-based, and shapes drop the role.

A shape and a store side by side:

```mensura
shape Named {
  attr { name: string }
}

store students {
  unit { Person }
  attr {
    admission: date   @const
    status:    Status @var
  }
}
```

## Consequences

Positive:

- One shape now describes a structure that a store, a view, and a function
  parameter can all claim on equal terms.  The `const`/`var` field that only
  stores could carry no longer sits in the shared contract, so conformance
  compares only what every claimant has.
- `const`/`var` sits with the change-control annotations it governs, so the
  store surface reads as one coherent policy layer (`@const`, later `@audited`
  and friends) attached to a structural core, rather than two block kinds that
  happen to also mean policy.
- The grammar loses a block kind: a store or shape body is `attr` blocks (plus
  `domain` in a store), not two near-identical `const`/`var` variants.  The
  `attr` nonterminal that already named the field gets the matching keyword.

Negative:

- A breaking surface change to every store, shape, and example written so far.
  Because it is a rename of block introducers plus a moved annotation, the
  migration is mechanical, and the project is pre-1.0.
- Requiring `@const`/`@var` on every store attribute is more verbose than the
  old block grouping, which stated the role once for many attributes.
  Accepted as the price of explicit, per-attribute intent.

Neutral:

- The realization is a localized change: the parser's shared attribute-block
  helper and the store/shape declaration loops, the `@` token the lexer
  already emits but never consumed, and the shape half of conformance checking
  in the resolver.  The resolved `Const`/`Var` roles and storage mapping are
  untouched.

## Alternatives considered

1. **Keep `const`/`var` blocks but make conformance ignore the role.**  Least
   churn: shapes still spell `const`/`var`, conformance just stops comparing
   it.  Rejected because it leaves authors writing a distinction the contract
   cannot enforce, which is exactly the confusion this ADR removes; the role
   belongs off the shape, not merely unchecked on it.

2. **Block-level annotation, `@const attr { ... }`.**  Group attributes under
   a role the way the old `const`/`var` blocks did.  Rejected: it does not
   match how the other change-control annotations attach (per attribute), and
   it reintroduces the two-block-kinds shape this ADR collapses.
   Per-attribute annotations let one `attr` block mix `@const` and `@var`
   freely.

3. **Leading annotation, `@const admission: date`.**  Reads like a modifier
   keyword.  Rejected in favor of the trailing form so the `name : type`
   structural core lines up between a store attribute and the shape attribute
   it conforms to, with policy trailing as a modifier.

4. **Default an unannotated store attribute to `@const`.**  Matches the old
   record-field default and is less verbose.  Rejected: mutability is
   load-bearing, and a silent default is the kind of implicit semantic choice
   the language exists to make explicit.

## Open questions

- **The rest of the change-control family.**  How `@const`/`@var` compose with
  `@audited`, `@versioned`, `@auto`, and `@allowcreate` in the trailing
  annotation slot, and which combinations are legal, is deferred to the
  change-control document.
- **Repeated `attr` blocks.**  Whether a store or shape may write several
  `attr` blocks that the resolver merges, as `const`/`var` blocks did, or is
  held to one, is a small call for the grammar update; the merging behavior is
  the conservative carry-over.
- **What the dropped record-field marker was for.**  If a genuine need to tag
  a computed field resurfaces, it is a fresh feature on the pipeline
  sublanguage, not a revival of the store notion of `const`/`var`.
