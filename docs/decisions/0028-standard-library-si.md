# 0028: The standard library and the `si` module

## Status

Accepted.  The first client of `docs/decisions/0027-modules-and-imports.md` and
the shipping vehicle for the unit symbols, prefixes, and named derived units
that `docs/decisions/0026-dimensional-physical-units.md` defines as library
bindings.  Lands in the same pull request as 0026 and 0027, design-only: the
`si` content, its generator, and the prelude wiring are a follow-up
implementation.

## Context

ADR 0026 makes units ordinary dimensioned constants and puts the SI symbols
(`m`, `kg`, ...), prefixes (`km`, `ms`, ...), and named derived units
(`N`, `J`, `Pa`, ...) in a library rather than the language.  ADR 0027 provides
top-level const bindings, imports, and `bundled` resolution.  `si` is that
library.  This ADR decides where it lives, how it is versioned, how its content
is produced, and the discipline the standard library follows.

## Decision

### 1.  Embedded in the repository, versioned with the toolchain

`si` lives in the main repository and is versioned with the toolchain: one
release train, no independent `si` version.  This is a deliberate closed door:
you cannot update `si` without a toolchain release, and cannot pin an old `si`
against a new toolchain.  For stable physics (`si`) that is the right trade.  It
does sit in mild tension with ADR 0027's provisional third-party layer
(per-module hashes and pinning): the stdlib is monolithically versioned, so the
pinning machinery exists for *third-party* modules, not for `si`.  Recorded so
the boundary is explicit.

### 2.  Generated from the mechanized dimension group

`si`'s content is **generated** from the mechanized group in
`formal/Mensura/Units/Dimension.lean` plus a small units table (each unit's name,
symbol, dimension, and scale factor).  So the Lean module, the generator, and
the emitted prelude land in one implementation PR/CI run, and the shipped units
cannot drift from the proved group: the base-unit symbols, the prefix table
(`let km = 1000.0 * m`, ...), and the named derived units (`let N = kg * m / s^2`,
...) are all emitted from one source of truth.  The generator's output is
ordinary Mensura source (const bindings per ADR 0026/0027), reviewable as such.

### 3.  Default resolution is `bundled`, offline-first

`import si` resolves `bundled` by default (ADR 0027): it ships with the
toolchain, needs no manifest, no network in CI, and is overridable like any
manifest entry.  A program that uses only `si` stays manifest-free.

### 4.  The standard library stays small and proven

The library is deliberately minimal: `si` now; later `precision` (the backing
extension of `real`, ADR 0026 Decision 9) and perhaps `stats`.  Each module gets
the ADR treatment and formal backing where applicable (`si` is backed by the
dimension group; `precision` will carry its own).  Breadth is a non-goal, in
keeping with the project's ML-validation scope.

### 5.  Corpus examples import `si` via bundled resolution

The worked examples in `docs/examples/` import `si` the way a user would
(bundled), not with a `path =` into the repo tree.  This keeps the corpus
exercising the real user experience and resolvable offline in CI, and turns the
fleet example's `kelvin: real` placeholder into a properly dimensioned column
once the implementation lands.

## Consequences

Positive:

- The shipped units are provably consistent with the mechanized group (Decision
  2): no hand-maintained unit table that can drift.
- Offline-first, manifest-free default: `import si` just works.
- The stdlib discipline (small, proven, ADR-per-module) keeps the surface honest.

Negative:

- One release train couples `si` evolution to toolchain releases (Decision 1);
  fine for stable physics, more limiting for a future `stats`.
- A generator plus a prelude is real implementation surface (the follow-up),
  and the SI-symbol casing question (below) must be resolved before `si` can
  bind its short symbols.

Neutral:

- `si` is a normal module: nothing about it is special to the language beyond
  being the default `bundled` target; it could in principle be overridden.

## Open questions

- **SI-symbol casing.**  Unit *names* are snake_case terms and lowercase names
  are fine (`ampere`, `kelvin`, `newton`).  But many SI *symbols* are uppercase
  or mixed (`A`, `K`, `N`, `Pa`, `W`, `Hz`, `MPa`), which snake_case forbids for
  a term binding.  Options: `si` exports full lowercase names always and short
  symbols only where snake_case-valid (`s`, `m`, `kg`, `mol`, `cd`, `km`, `ms`);
  or a designated unit-symbol namespace relaxes casing; or symbols come only via
  an `exposing`-style alias (ADR 0027).  To resolve before the `si`
  implementation; it also feeds the `05-naming-and-casing.md` reconciliation.
- **The units table's authority.**  Whether the name/symbol/scale table that
  feeds the generator lives beside the Lean module or is itself derived, and how
  it is reviewed.
- **Prefix scope.**  Which prefixes ship (full SI set vs a common subset) and
  for which units, given the `kg`/`g` footnote (ADR 0026 Decision 1).

## Forward references

- The units model that defines these bindings is
  `docs/decisions/0026-dimensional-physical-units.md`; the module mechanism that
  ships them is `docs/decisions/0027-modules-and-imports.md`.
- The mechanized group the content is generated from is
  `formal/Mensura/Units/Dimension.lean`; the formal-proof pipeline is
  `docs/decisions/0021-formal-proof-pipeline.md`.
- The casing rule the symbol question feeds back into is
  `docs/language/05-naming-and-casing.md`.
