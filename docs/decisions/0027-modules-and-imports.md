# 0027: Modules, imports, and top-level bindings

## Status

Accepted.  Companion to `docs/decisions/0026-dimensional-physical-units.md`
(which motivates it) and `docs/decisions/0028-standard-library-si.md` (its first
client), landing in the same pull request.  Design-only: it fixes the model of
top-level bindings and the module system; the parser/resolver/CLI realization is
a follow-up.  Revised in part by
`docs/decisions/0031-fold-and-scan-primitives.md` (Decision 4 carries the
note).

Modules and imports are **not** on the current `ROADMAP.md`; this ADR adds a
roadmap item (a language-core piece needed by M3's units, plus a later
third-party-distribution piece).

The **motivated mechanism** (top-level const bindings, imports, the
intrinsic/library split, bundled resolution) is decided here in full, because
ADR 0026's units model depends on it.  The **third-party distribution layer**
(manifest-as-lockfile, hashes, `mensura pin`, `git`/`path` resolution, tainting
the check) has no consumer yet and is recorded as **provisional** (Decision 7):
its direction is on record, but it is finalized in its own ADR when a real
consumer appears.

## Context

ADR 0026 makes a physical unit an ordinary dimensioned constant: `km` is a
binding `1000.0 * m`, `N` is `kg * m / s^2`, and the SI symbols and prefixes are
a table of such bindings shipped by the `si` library (ADR 0028).  Two pieces of
machinery this needs do not exist today:

- **Top-level bindings.**  `let` exists only inside a block (view bodies today,
  `06-expressions.md`); the five top-level declarations are `unit`, `store`,
  `shape`, `enum`, `view` (`04-grammar.md:64-66`).  There is no file-scope
  binding, so `let km = 1000.0 * m` has nowhere to live.
- **Imports.**  A Mensura program is a single file (`ast.rs`, the CLI reads one
  path); there is no module, import, or manifest anywhere in the language,
  docs, or roadmap.

The resolver already treats name collisions as errors, not shadows (duplicate
`unit`/`store`/`shape`/`enum`/`view` are diagnostics, and stores and views share
one namespace), but it has no reusable environment value: resolution is a single
pass over one file's items into per-namespace local maps (`resolve.rs`).  So the
*policy* (collisions are errors) transfers; the *disjoint-union environment* is
new.

Beyond units, the general need is the ordinary one: name and reuse definitions,
and ship a small, proven standard library (ADR 0028).

## Decision

### 1.  Top-level const bindings

A new top-level item: `let name = expr` binds an immutable, pure value at file
scope, and `let name[T] = <dimension>` binds a type-level dimension alias (ADR
0026, Decision 8).  This is the same `let` as the block statement, lifted to
`item` position; the kind (value vs type-level alias) is determined by the body.
Bindings are order-independent within a module (they name values, not effects)
and non-recursive.  This is the form that lets units be constants.

**Revised during implementation.**  The item-level surface is
**brace-closed**: `let name { <block> }` for a value (with an optional
`: type` ascription before the brace) and `let name[T] { <dimension> }`
for an alias.  The `=` form above assumed the statement `let` could lift
unchanged, but a top-level item has no terminator, so an unbraced
expression body lets the application spine swallow the next item's
leading keyword (`let x = 2.0 * meter let y = ...` reads `meter let` as
juxtaposition), and every future expression-grammar extension would need
re-auditing against the item-level FOLLOW set.  The brace closes the
hazard class structurally and restores the language's item convention
(every item body ends at `}`); `=` remains the binder at expression level
(statement `let`s, record fields), where `;`/`}` provide the terminators.
The value body is the same statement block a `view` hosts, with the const
evaluator enforcing purity and const-ness semantically.  The kind is
still one token of lookahead: `[` after the name selects the alias form.
See `docs/language/12-modules-and-imports.md` and `04-grammar.md`.

### 2.  Modules export const bindings and type-level names only

A module is a file.  It exports **const bindings** (Decision 1) and
**type-level names** (dimension aliases, and later `enum`/`shape` names).  It
does **not** export pipelines, `store`s, or `view`s: those are materialized,
site-specific resources, not reusable definitions.

**Registries are not importable** (relevant from M4): a `registry`'s type-level
completeness guarantee comes from its being the *sole* intake for its
observations (`00-overview.md`), and importing it into another program would
create a second consumer and silently break that guarantee.  Stating this now
keeps the M4 completeness story intact.

### 3.  Imports are qualified, collision-free, and acyclic

- **Qualified by default.**  `import si` brings the module in under its name;
  members are referenced `si.meter`, `si.km`.  There is **no glob import**.
- **Collisions are compile errors, not shadows.**  Importing two modules that
  would bind the same name is an error, matching the resolver's existing policy;
  formally, an import is the **disjoint union** of environments (Decision 6).
- **Acyclic.**  Module imports form a DAG; a cycle is a compile error.
- **Ergonomics note (open).**  Qualified-by-default makes unit-heavy code verbose
  (`9.8 * si.m / si.s^2`).  A selective-unqualified `exposing` form is the
  contemplated refinement; see Open questions.

### 4.  The intrinsic / library split, and no implicit prelude

The language provides an **initial environment** of intrinsics: the seven base
units (ADR 0026, Decision 6) and the existing ambient builtins (aggregate
combinators, pipeline operations).  These are *language*, always in scope.

There is **no implicit prelude** beyond the intrinsics: nothing else is in scope
that you did not import.  In particular `si` (the unit symbols, prefixes, and
named derived units) is an ordinary import, not ambient.  So `9.8 * meter / second^2`
type-checks with no import, while the terse `9.8 * si.m / si.s^2` (or, with an
`exposing` refinement, `9.8 * m / s^2`) requires `import si`.

(*Revised by ADR 0031*: the aggregate combinators **leave the initial
environment**.  They become const bindings in qualified bundled
libraries (`bag`, `series`), imported like `si`, and cardinality becomes
the `#` operator; the intrinsic environment shrinks to the base units,
`fold`/`scan`/`map`, `to_real`, and the pipeline operations.  This
decision's no-implicit-prelude rule thereby holds without the aggregate
exception it used to carry.)

### 5.  Identity/provenance/location split

A module has an **identity** (the name the source imports), a **provenance
class** (bundled with the toolchain, or manifest-resolved), and a
**location** (how the toolchain finds it within its class).  The import form
fixes the identity and the class; only the location is the manifest's
business.  A bare `import si` is the bundled class: it resolves against the
toolchain and never consults the manifest, so it means the same thing in
every project.  A manifest-resolved import uses a marked form (Decision 7)
and is portable in the intended sense: the same marked import resolves to a
different location per project (`mensura.toml`) without editing source.

### 6.  Resolution: a bare import is `bundled`, and only `bundled`

`bundled` resolution finds a module that ships with the toolchain, and a
**bare** import (`import si`, no marker) resolves `bundled` exclusively: it
never consults the manifest, and a manifest entry that names a bundled module
is a compile error.  The consequences are deliberate.  A bare import cannot
be remapped, so no manifest can silently substitute another module for the
standard library; and bundled names need no reservation rule, because there
is no mechanism by which a third-party module could answer a bare import.
Manifest-resolved imports use a marked form whose surface is finalized with
the third-party layer (Decision 7).

This is all the units work needs: `si` is bundled (ADR 0028), resolvable
offline, with no manifest.  A module resolves to an environment, and `import`
unions it disjointly into the importer's environment (Decision 3).

### 7.  Third-party distribution (provisional; no consumer yet)

Recorded as the intended direction, **not finalized**, because nothing outside
the bundled `si` consumes it and `mensura check` emits no artifact to hang it on
today:

- In source, a manifest-resolved module is imported with a **marked** form
  (for instance `import external geo`; the keyword is settled in the
  finalizing ADR), so the trust boundary is visible at the import site: a
  bare import is toolchain-shipped, a marked import is whatever the manifest
  says (Decision 6).
- `mensura.toml` is the manifest **and** the lockfile: a module's hash is
  recorded there (no separate lockfile to forget), and `mensura pin` writes
  hashes.
- Resolution kinds are manifest fields, not URL schemes: `git` (with a hash)
  and `path` (relative only, hash-free, and **taints the check** as
  non-portable, so dev and release differ by exactly that bit).  Bundled
  modules never appear in the manifest (Decision 6).
- A single-file escape hatch allows a URL with an inline hash directly in the
  import, ugly on purpose.

Two things must be settled before this is more than provisional: whether
`mensura check` gains an artifact that records module hashes (so a `path` import
can taint it), and the exact manifest schema.  Sketch:

```toml
[modules]
geo   = { source = "git", url = "github.com/user/geo.git", hash = "abc123" }
mylib = { source = "path", path = "../mylib" }   # taints the check
```

An in-source resolution block (`import si { source { "bundled" } }`) was
considered and rejected in favor of the identity/location split: resolution
belongs in the manifest, not scattered across source files; the only in-source
location form is the escape-hatch URL+hash.

### 8.  Formal note

An import is a disjoint union of finite name-to-entry environments; a collision
is a non-empty domain intersection.  This mechanizes as a small finite-map
disjointness lemma (distinct from the table `Disjoint` of `formal/`, which is
about multisets at keys).  It is low-value ceremony, not a priority; the
substantive formal targets in this area are ADR 0026's dimensional-arithmetic
soundness and conversion correctness.  If proved, it is a planned (white)
blueprint node until then, per ADR 0021.

## Consequences

Positive:

- Units become ordinary library bindings (ADR 0026) with a real home: a
  top-level `let` and a `bundled` `si`.
- The language core stays small: intrinsics plus five declarations plus
  top-level `let` and `import`; everything else is a library.
- Collision-as-error and qualified-by-default make name provenance explicit,
  extending the resolver's existing discipline.
- A bare import cannot be hijacked: `import si` resolves against the
  toolchain in every project, with no manifest consulted, so nothing can
  silently substitute another module for the standard library.

Negative:

- A new top-level item (`let`) and a new item (`import`), plus a first-class
  environment/disjoint-union in the resolver, where today there is only a
  single-file per-namespace pass.
- Qualified-by-default is verbose for unit-heavy code until an `exposing`
  refinement exists (Open questions).
- The provisional third-party layer risks rework once a real consumer and a
  check artifact exist; it is deliberately not committed.

Neutral:

- The collision *policy* already matches the resolver; only the environment
  abstraction is new.
- `bundled` resolution needs no manifest, so single-file programs that import
  only `si` stay manifest-free.

## Alternatives considered

1. **In-source resolution blocks** (`import si { source { "git" } url { ... } }`).
   Rejected: resolution scattered across source files is unportable; the
   identity/provenance/location split puts locations in the manifest.  Only
   the single-file URL+hash escape hatch keeps a location in source.  What is
   rejected is in-source *location*, not in-source *provenance class*: the
   bare/marked import distinction (Decisions 6 and 7) puts the class in
   source deliberately, because it is part of what the program means, while
   the URL, path, and hash stay in the manifest.

2. **Marking the bundled side instead** (`import bundled si`, with the bare
   form manifest-resolved), or marking both sides.  Rejected: every standard
   library import would pay an annotation forever to flag a third-party
   ecosystem that is still provisional, and the trust boundary belongs on the
   untrusted thing.  "A bare import is toolchain-shipped" is one global rule
   a reader learns once.

3. **A glob / unqualified import** (`from si import *`).  Rejected: it
   reintroduces shadowing and hides provenance.  Selective `exposing` is the
   contemplated middle ground (Open questions).

4. **Exporting pipelines/views.**  Rejected: those are materialized,
   site-specific resources, not reusable definitions; modules export values and
   type-level names.

5. **Fully designing the package manager now** (git/path/hash/pin/taint).
   Rejected for this PR: no consumer, and `mensura check` has no artifact to
   taint; recorded as provisional (Decision 7) and finalized later.

## Open questions

- **The check artifact.**  Whether `mensura check` records module hashes from
  day one (recommended), which is the prerequisite for a `path` import to taint
  it.
- **`exposing` lists.**  Whether a selective-unqualified import (bringing chosen
  names, e.g. unit symbols, into scope unqualified) is worth the surface once
  unit-heavy files appear; and its collision rules.
- **Diagnostic wording** for hashless URLs, for a would-be shadow (an import
  that collides with a local or another import), and for a manifest entry
  that names a bundled module (Decision 6).
- **The marked-import keyword** (`external`?  `managed`?) for
  manifest-resolved modules, settled in the ADR that finalizes Decision 7.
- **Manifest schema** and the third-party layer generally (Decision 7),
  finalized in its own ADR with a real consumer.

## Forward references

- The units model that motivates this is
  `docs/decisions/0026-dimensional-physical-units.md`; the first client is
  `docs/decisions/0028-standard-library-si.md`.
- The intrinsic builtins precedent is the aggregate/pipeline entries in the
  checker's typing context (`crates/mensura-types/src/expr_check.rs`); the
  resolver's collision policy is `crates/mensura-types/src/resolve.rs`.
- The formal-proof pipeline is `docs/decisions/0021-formal-proof-pipeline.md`.
- The completeness guarantee that keeps `registry` un-importable is in
  `docs/language/00-overview.md`.
