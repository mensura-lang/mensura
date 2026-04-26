# 0004: Qualifier mechanism

## Status

Proposed.

## Context

The original Mensura sketch (see `docs/language/00-overview.md`, pillar
2) gave tables a fixed type quadruple `Table<S, D, L, C>`, where each of
sampling, dependency, lineage, and content is a distinct, language-level
type parameter with its own typing rules baked into the compiler.

This is enough for the academic deliverable, which is scoped to
ML-validation correctness.  It is not enough for the language as a
long-lived artifact: any future invariant we want to track on tables
(differential-privacy budget, data freshness, PII taint, GDPR
jurisdiction, license compatibility, ...) requires a language change.
Each new property involves edits to the core grammar, the type checker,
the inference algorithm, the diagnostics renderer, and the LSP.

The alternative, considered here, is to make the four properties
**the first four members of a standard library**, with the core language
defining only the *meta-calculus* over which any such property can be
encoded.

## Decision

A table type is `Table<Qs, C>`, where:

- `Qs` is a (possibly empty) row of **qualifiers**.  A qualifier is a
  user-definable abstraction with a state space, propagation rules
  attached to each algebra primitive, and (optionally) a constraint
  hook that fires at well-defined points in inference.
- `C` is the content schema (index columns, non-index columns, their
  domains, units, and semantic refinements).  Content stays
  structurally distinct from qualifiers; see Open questions.

Sampling, dependency, and lineage are **not language features**.  They
are qualifiers defined in the standard library `std`, alongside any
future canonical qualifiers.

A qualifier is declared using a small declarative interface:

1. **State space.**  A finite enumeration, a bounded lattice, or a
   structural value (for lineage, an accumulating tree).  The
   framework provides combinators for these shapes; library authors
   do not invent arbitrary state spaces in arbitrary host code.
2. **Propagation rules.**  For each algebra primitive (`bind`,
   `split`, `filter`, `mutate`, `aggregate`, `ungroup`, `pivot`,
   `unpivot`, `select`, `left_join`, ...), the qualifier specifies
   how its value on the input maps to its value on the output.  Rules
   are written using a fixed, sound-by-construction set of **rule
   combinators** (`preserve`, `map`, `demand`, `accumulate`,
   `intersect-on-bind`, ...), not arbitrary code.
3. **Constraint hook.**  A predicate the qualifier may register that
   must hold at the point of use (for example, lineage's disjointness
   obligation when binding two tables).  Hooks may be partially
   decidable; the framework provides a uniform `assume` escape
   hatch.

Library authors do not write typing-rule code in the host language.
They compose rule combinators that the type checker interprets.  This
keeps soundness compositional: if every combinator is sound, every
qualifier built from them is sound.

## Consequences

Positive:

- Future invariants are library code, not language changes.  The
  language stays small as the catalogue of properties grows.
- The core algebra (Tier A and Tier B operations from Chapter 5) is
  decoupled from any specific statistical semantics.  Operations are
  typed once, against the qualifier framework, not once per property.
- The split between language (algebra plus qualifier mechanism) and
  library (the canonical qualifiers) gives a clean axis for future
  research and for third-party extension.
- Soundness arguments shrink to the rule-combinator set, which is
  small and fixed.  Each library qualifier is sound by construction.

Negative:

- M0 must freeze the meta-calculus, not just four properties.  The
  meta-calculus is a more ambitious target and carries more risk of
  needing post-freeze revisions.
- The lineage disjointness solver moves from being a core feature to
  being a `std::lineage` constraint hook.  The hook interface
  becomes load-bearing and must be specified carefully (decidability,
  fallback, error reporting).
- The LSP and the diagnostics renderer must display arbitrary
  qualifiers, not four hard-coded ones.  Hover tooltips and error
  messages need a uniform protocol every qualifier participates in.
- The grammar must accommodate qualifier declarations while remaining
  LL(1) (see ROADMAP "Implementation choices").  Qualifier syntax has
  to be designed against this constraint, not after it.
- Versioning the standard library becomes part of the language
  surface: programs depend on `std::sampling@1.0`, not on a built-in.

Neutral:

- The `assume` escape hatch, already a design pillar, takes on a
  second job: it relaxes library-defined constraints, not just
  language-defined ones.  This is a feature, not a hazard, but it
  means `assume` must be designed against the qualifier interface.

## Alternatives considered

1. **Hard-code `Table<S, D, L, C>`.**  Smallest M0 surface, simplest
   formalization, fastest path to a working type checker.  Rejected
   because every future invariant becomes a language change.

2. **Hybrid: hard-code for M0, refactor to extensible later.**  Defers
   the meta-calculus risk.  Rejected because by M3 the compiler would
   have absorbed assumptions about specific properties (in inference,
   in diagnostics, in the LSP) that are expensive to undo.  The
   project chooses to take the meta-calculus risk up front rather
   than carry it as technical debt.

3. **Type-class / trait dispatch.**  Each property is a Rust-style
   trait; instances supply propagation rules as methods.  Workable,
   but trait-style dispatch over a row of properties does not compose
   as naturally as row-polymorphic effect systems for this problem
   (where every primitive has to know about every property, and the
   row grows over time).

4. **Macro-level metaprogramming.**  Users write macros that rewrite
   typing rules.  Maximum power, minimum soundness, hard to integrate
   with diagnostics.  Rejected as the primary mechanism, though some
   compile-time machinery may still be useful internally.

## Open questions

- **Exact shape of the rule-combinator DSL.**  Which combinators are
  in the minimum set?  How are constraint hooks expressed?  Is there
  a `coerce` combinator for sub-qualifier widening, or is widening
  implicit?
- **Content (C) as qualifier, or as a separate term?**  C is a record
  of columns with domains; the rest of the qualifiers are values in
  lattices.  This ADR keeps C separate.  Should this be revisited if
  the framework turns out to subsume schemas cheaply?
- **Composition of independent qualifiers.**  Are qualifiers fully
  orthogonal (each row entry propagates independently), or are there
  primitives where two qualifiers interact (a privacy budget reduced
  by sampling fidelity, etc.)?  This ADR assumes orthogonality and
  pushes interactions to a follow-up.
- **Decidability bounds.**  The lineage disjointness check is on the
  edge of decidability; arbitrary library qualifiers could push the
  type checker into undecidable fragments.  What bounds does the
  framework impose?  What is the fallback?
- **Versioning.**  How does a Mensura program pin the version of
  `std::sampling` it expects?  How are breaking changes to a
  qualifier surfaced?
- **Naming.**  "Qualifier" is the working term.  Alternatives:
  "facet", "property", "trait", "channel", "aspect".  Settle this
  before any spec text uses it normatively.

## Cross-cutting changes if accepted

These follow from the decision but are not part of this ADR; each is
its own task.

- ROADMAP M0 deliverables grow: the meta-calculus and the four
  standard-library qualifiers must each be specified before M0 ends.
- `docs/language/02-types.md` is replaced (or split) into a document
  on the qualifier framework and a set of documents on the
  standard-library qualifiers.
- `docs/language/00-overview.md` pillar 2 is rewritten: the table
  type is `Table<Qs, C>`, with the four canonical qualifiers in the
  standard library, not in the language.
- `docs/language/05-lineage.md` and
  `docs/language/06-sampling-dependency.md` become documents *about
  specific standard-library qualifiers*, written against the
  framework rather than as language features.
