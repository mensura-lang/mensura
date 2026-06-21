# 0008: Formalize the data-handling algebra in Lean 4

## Status

Accepted.

## Context

Chapter 5 of [1] defines the algebra Mensura's type system is
supposed to encode: indexed tables, split and bind, tagged variants, pivot,
join/left-join, select, filter, mutate, aggregate, ungroup, and project.  It
states several properties as prose proofs, most importantly that the Tier A
operations are *split-invariant* (`def:split-invariance`), and leaves at least
one open question: a definition of split-invariance for binary operations
(the inner-join conjecture at the end of the chapter).

[1] F. A. N. Verri (2026). Data Science Project: An Inductive Learning Approach. Version
v1.0.0. Victoria, British Columbia, Canada: Leanpub. doi: 10.5281/zenodo.14498010. url:
https://leanpub.com/dsp.

These properties are the justification for the type system.  If the algebra is
wrong, or a "split-invariant" operation turns out not to be, the typing rules
in M0 inherit the error.  Prose proofs over a structure as fiddly as cells
holding multisets of possibly-missing values are exactly the kind of argument
that hides off-by-one and edge-case mistakes (empty rows, the minimality
assumption on value matrices, disjointness preconditions).

The M0 milestone freezes the calculus.  We want that freeze backed by machine-
checked proofs rather than by re-reading the chapter.

## Decision

The algebra is formalized in **Lean 4**, in a Lake project under `formal/`,
before it is committed to as the M0 calculus.  Concretely:

- `formal/` is a standalone Lake package (`Mensura` library) that **depends on
  Mathlib**.  The algebra rests on finite sets of columns, multisets and
  cardinality, finite maps, indicator functions, and domains as enumerable
  sets; re-deriving those is not the work we want to do.
- The Mathlib `rev` in `formal/lakefile.toml` is pinned to the tag matching
  `formal/lean-toolchain` (currently `v4.31.0`) so the prebuilt olean cache is
  available and builds are reproducible.  The two are bumped together.
- Each definition box in the chapter maps to a Lean definition that records
  the chapter's `\label` in its doc comment, so the formalization and the
  source stay traceable.
- The first proof obligations are: split and bind are mutual inverses on
  disjoint tables, and the Tier A operations are split-invariant.  The binary
  split-invariance definition is settled here, closing the chapter's open
  question, and that result feeds back into M0.
- `formal/` is **not** built in CI: a Mathlib-backed Lean build is too
  expensive to run on every pull request.  It is built locally with
  `lake build` (which pulls the prebuilt olean cache) before changes land.

This makes Lean a third pillar alongside the design docs and the Rust
toolchain: the docs say what the language is, Lean proves the algebra is sound,
and the crates encode the result.

## Consequences

- M0 cannot freeze the calculus until the split-invariance proofs build.
- Contributors need `elan` to build `formal/`; the Rust workspace is
  unaffected and the two build independently.
- A Mathlib bump is a deliberate, paired change to `lakefile.toml` and
  `lean-toolchain`, not an incidental one.
