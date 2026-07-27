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
against a new toolchain.  For stable physics (`si`) that is the right trade.
The boundary with ADR 0027's provisional third-party layer is structural, not
conventional: the pinning machinery (per-module hashes, `mensura pin`)
applies to manifest-resolved modules only, and `si` is imported bare and
never appears in the manifest (ADR 0027, Decision 6), so the monolithically
versioned stdlib and the individually pinned third-party layer cannot be
confused.

### 2.  Generated from the mechanized dimension group

`si`'s content is **generated** from the mechanized group in
`formal/Mensura/Units/Dimension.lean` plus a small units table (each unit's name,
symbol, dimension, and scale factor).  So the Lean module, the generator, and
the emitted prelude land in one implementation PR/CI run, and the shipped units
cannot drift from the proved group: the base-unit symbols, the prefix table
(`let km = 1000.0 * m`, ...), and the named derived units (`let N = kg * m / s^2`,
...) are all emitted from one source of truth.  The generator's output is
ordinary Mensura source (const bindings per ADR 0026/0027), reviewable as such.

**Revised during implementation.**  The generator is dropped for now, and
`si.mensura` is hand-written, for a reason the implementation made plain:
the emitted source carries no group facts a generator could pin.  Each
binding is an ordinary expression over the intrinsic base units
(`let newton = kilogram * meter / second^2`), so the checker *recomputes*
its dimension from the mechanized-group rules on every compile; the only
hand-authored content is the physics (which product of base units, which
scale), and that is a definition wherever it lives, unprovable in Lean or
anywhere else.  The no-drift mechanism is therefore: (a) `si` is compiled
by the frontend in CI like any source, and (b) an **oracle test** asserts
each binding's *resolved* dimension exponent vector and magnitude against
a review table, bidirectionally (no unlisted binding, no stale row),
which checks the checker's output, something generation cannot do.  A
(Rust) generator becomes worthwhile when the table grows toward the full
prefix set, where the content turns combinatorial; that is the recorded
plan, not a Lean emitter.

### 3.  Imported bare: `bundled`, offline-first, un-remappable

A bare `import si` resolves `bundled`, and only `bundled` (ADR 0027, Decision
6): it ships with the toolchain, needs no manifest, no network in CI, and
cannot be remapped, because a bare import never consults the manifest.  A
program that uses only `si` stays manifest-free.

### 4.  The standard library stays small and proven

The library is deliberately minimal: `si` now; later `precision` (the backing
extension of `real`, ADR 0026 Decision 9) and perhaps `stats`.  Each module gets
the ADR treatment and formal backing where applicable (`si` is backed by the
dimension group; `precision` will carry its own).  Breadth is a non-goal, in
keeping with the project's ML-validation scope.  Other candidates under the
same discipline are `math` (mathematical constants and operations) and
`rand` (seeded, reproducible pseudo-random primitives backing sampling and
split strategies).

### 5.  Corpus examples import `si` via bundled resolution

The worked examples in `docs/examples/` import `si` the way a user would
(bundled), not with a `path =` into the repo tree.  This keeps the corpus
exercising the real user experience and resolvable offline in CI, and turns the
fleet example's `kelvin: real` placeholder into a properly dimensioned column
once the implementation lands.

## Consequences

Positive:

- The shipped units are checked against the mechanized group's rules on
  every compile, and the oracle test pins each binding's resolved
  dimension and scale (Decision 2, as revised): no unit table that can
  drift silently.
- Offline-first, manifest-free default: `import si` just works.
- The stdlib discipline (small, proven, ADR-per-module) keeps the surface honest.

Negative:

- One release train couples `si` evolution to toolchain releases (Decision 1);
  fine for stable physics, more limiting for a future `stats`.
- A hand-written table plus its oracle test is two places to edit per unit
  (Decision 2, as revised); acceptable at the common-subset size, and the
  trigger to build the Rust generator when it stops being acceptable.

Neutral:

- `si` is a normal module: nothing about it is special to the language beyond
  being bundled.  It cannot be overridden: a bare import never consults the
  manifest (ADR 0027, Decision 6), so a project that wants a different units
  library imports it under another name.

## Open questions

All three were resolved with the implementation:

- **SI-symbol casing.**  Resolved: one casing rule with no exceptions.
  `si` exports full lowercase names always and short symbols only where
  snake_case-valid (`s`, `m`, `g`, `kg`, `mol`, `cd`, `km`, `ms`, ...);
  uppercase and mixed symbols (`A`, `K`, `N`, `Pa`, `W`, `Hz`) are not
  bound.  A future `exposing`-with-rename can revisit the terse spellings
  without a casing change.  Recorded in `05-naming-and-casing.md`.
- **The units table's authority.**  Resolved by the Decision 2 revision:
  the authority is the hand-written `si.mensura` plus the oracle test's
  review table (resolved dimension vector and magnitude per binding),
  which CI keeps bidirectionally consistent.
- **Prefix scope.**  Resolved: the common engineering subset (nano, micro,
  milli, centi, kilo, mega, giga) on second/meter/gram plus a few
  conventional derived forms, honoring the `kg`/`g` footnote (prefixes
  attach to `gram`; `kilogram` is the base).  The full SI set waits for
  the generator.

## Forward references

- The units model that defines these bindings is
  `docs/decisions/0026-dimensional-physical-units.md`; the module mechanism that
  ships them is `docs/decisions/0027-modules-and-imports.md`.
- The mechanized group the content is generated from is
  `formal/Mensura/Units/Dimension.lean`; the formal-proof pipeline is
  `docs/decisions/0021-formal-proof-pipeline.md`.
- The casing rule the symbol question feeds back into is
  `docs/language/05-naming-and-casing.md`.
