# 0019: Attributes as structure, const and var dropped

## Status

Accepted.  Refines `docs/decisions/0003-shapes-as-structural-contracts`: a
shape describes structure only, and the `const`/`var` distinction leaves the
language entirely.  Amends
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
persisted data, and (per the overview glossary) the distinction "governs
which change-control annotations (`@audited`, `@versioned`, `@auto`,
`@allowcreate`) apply".  ADR 0003 already excluded exactly those annotations
from shapes as store-only policy.  So `const`/`var` was misplaced on shapes.

Pulling it off shapes raises the next question: where does it go?  An
earlier draft of this ADR moved it onto store attributes as a required
per-attribute annotation (`admission: date @const`).  But the distinction
has no consumer today.  Nothing in the compiler or the runtime behaves
differently for a `const` column versus a `var` column beyond storage
column order, and the change-control annotations the distinction would
govern are themselves deferred to a future document.  A required annotation
that nothing consumes is ceremony: the compiler cannot verify it, tools
cannot act on it, and authors must decide it before the language gives the
decision any meaning.

There is also a live design direction that a required annotation would
pre-empt.  When change control arrives, mutability may well be assigned by
default per tabulation kind, for instance store attributes mutable by
default and `collect` results immutable, with an annotation only on the
exceptional attribute.  Committing every store author to a mandatory
`@const`/`@var` today would bake in the opposite convention before the
family it belongs to is designed.

This ADR therefore drops `const`/`var` from the language for now, surface
and boundary IR both, the same treatment `@audited` and `@versioned`
already receive: named as future work, absent from the implementation.

## Decision

- **A new contextual keyword `attr`.**  Attributes in both stores and shapes
  are listed in `attr { ... }` blocks, superseding the `const` and `var` block
  introducers of `0002` and `0003`.  The word matches the glossary term
  "attribute" (already the umbrella over the two former categories) and the
  grammar's existing `attr`/`shape_attr` nonterminal names.  Like the
  blocks it replaces, `attr` is contextual: the lexer still emits it as an
  `Ident` and the parser recognizes it only in the block-introducer position.

- **Stores and shapes carry structure only.**  A store or shape attribute is
  `name : type` (with the optional `?` of `0010`) and nothing else.  There is
  no mutability marker in either body; the store and shape attribute grammars
  are identical.

- **Conformance is name and type.**  ADR 0003's rule becomes: a store or view
  conforms to a shape if it has every attribute the shape requires with the
  same name (after interpolation) and the same type, including optionality,
  and may have more.  The "same `const`/`var` block" criterion is removed.

- **Repeated `attr` blocks merge.**  A store or shape body may write several
  `attr` blocks; the resolver merges them into one attribute list, exactly as
  the old `const`/`var` blocks merged.  `attr { a: T } attr { b: U }` is
  equivalent to one block listing both.  Duplicate names are checked after
  merging, so a name repeated across blocks is the same error as a name
  repeated within one block.

- **The record-field role marker is dropped.**  The expression-level
  `(.a const = x)` / `(.a var = x)` marker that `0015` reserved is removed.
  A record field is written `.a = x` (or `.a: type = x`).

- **The boundary IR drops the `Const`/`Var` roles.**  A resolved column is
  an index column or an attribute column; the former three-way
  index/const/var role collapses to that two-way distinction.  Storage
  column order becomes index columns, then attributes in declaration order.

- **`const`/`var` is deferred, not redesigned here.**  Mutability and change
  control (`@audited`, `@versioned`, `@auto`, `@allowcreate`, and whatever
  replaces `const`/`var`) return together in a future change-control
  document.  The default-per-tabulation-kind direction sketched in the
  context is that document's to evaluate, not a commitment made here.

A shape and a store side by side:

```mensura
shape Named {
  attr { name: string }
}

store students {
  unit { Person }
  attr {
    admission: date
    status:    Status
  }
}
```

## Consequences

Positive:

- One shape now describes a structure that a store, a view, and a function
  parameter can all claim on equal terms, and a store body is written in
  exactly the same attribute language as the shape it claims.
- Authors no longer decide a property the language does not yet give meaning
  to.  When change control is designed, mutability arrives with its
  consumers, and possibly as a per-tabulation-kind default rather than a
  per-attribute obligation.
- The grammar loses a block kind and an annotation position: a store or
  shape body is `attr` blocks (plus `domain` in a store), and the compiler
  loses the `Const`/`Var` role plumbing.

Negative:

- A breaking surface change to every store, shape, and example written so
  far.  Because it is a rename of block introducers, the migration is
  mechanical, and the project is pre-1.0.
- Mutability intent that authors used to record in the old blocks is not
  recorded anywhere for now.  Accepted: an intent the compiler neither
  checks nor consumes is documentation, and prose can carry it until change
  control gives it teeth.

Neutral:

- The realization is a localized change: the parser's block introducers and
  the store/shape declaration loops, the merged attribute list in the AST,
  the role enum in the resolver's `Schema` model, and the shape half of
  conformance checking.  The storage mapping keeps its index-first column
  order.

## Alternatives considered

1. **Keep `const`/`var` blocks but make conformance ignore the role.**  Least
   churn: shapes still spell `const`/`var`, conformance just stops comparing
   it.  Rejected because it leaves authors writing a distinction the contract
   cannot enforce; the role belongs off the shape, not merely unchecked on it.

2. **Store-only per-attribute annotations, `admission: date @const`, required
   on every store attribute.**  The earlier draft of this ADR.  Keeps the
   distinction expressible while freeing shapes.  Rejected: nothing consumes
   the distinction yet, so the annotation is mandatory ceremony, and it
   commits to per-attribute opt-in before the change-control design decides
   whether mutability should instead default per tabulation kind.

3. **Store-only annotations with a default for the unannotated case.**  Less
   verbose than 2, but a silent default is an implicit semantic choice, and
   choosing the default now is exactly the decision this ADR defers.

4. **Keep `Const`/`Var` in the boundary IR while dropping the surface.**
   Would spare the runtime a column-order change.  Rejected: an IR
   distinction no surface can express is dead weight, and the storage order
   change is trivial while the project is pre-1.0.

## Open questions

- **The change-control family.**  How mutability returns, per-attribute
  annotations, per-tabulation-kind defaults (for instance store attributes
  mutable by default, `collect` results immutable, annotating only
  exceptions), or something else, and how it composes with `@audited`,
  `@versioned`, `@auto`, and `@allowcreate`, is deferred to the
  change-control document.
- **What the dropped record-field marker was for.**  If a genuine need to
  tag a computed field resurfaces, it is a fresh feature on the pipeline
  sublanguage, not a revival of the store notion of `const`/`var`.
