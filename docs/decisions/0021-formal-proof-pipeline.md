# 0021: A spec-driven proof pipeline for the Lean development

## Status

Proposed.  Process-level: it changes how `formal/` is built, organized,
and grown, not any language or checker semantics.  Realized in
`.github/workflows/`, the layout of `formal/`, a blueprint under
`formal/blueprint/`, and the developer tooling configuration at the
repository root.

## Context

`formal/` backs the calculus.  ADR 0008
(`docs/decisions/0008-formalize-algebra-in-lean.md`) froze the algebra
behind machine-checked proofs, and the standing discipline is that a
checker propagation rule ships only when a theorem under `formal/` backs
it; otherwise the checker stays conservative.

Three pressures on the current setup:

1. **CI does not build the proofs.**  `.github/workflows/ci.yml` runs
   cargo only.  A toolchain or Mathlib bump, or a PR touching `formal/`,
   can break the development silently.  For a repository whose typing
   rules cite these theorems, an unbuilt proof is indistinguishable from
   a missing one.
2. **The development outgrew its map.**  `formal/Mensura/Table.lean`
   (1151 lines) and `formal/Mensura/Completeness.lean` (639 lines) are
   single files whose header comments carry the whole inventory of what
   is defined, proved, and open.  The `Table.lean` header is ~85 lines
   of prose and grows with every result; there is no structured record
   of which theorem depends on which, or which ADR each one discharges.
3. **The next results are theorem-DAG shaped.**  The named future
   directions (the graded, key-changing generalization of `fiberMap`
   in `Completeness.lean`; the book's binary-join split-invariance
   conjecture; window operations via the list monad; ADR 0020's
   axis-aware split question) are single hard theorems that decompose
   into lemma DAGs.  We have no way to plan, track, or review that
   decomposition other than prose.

The method adopted here is a spec-driven Lean workflow: author the lemma
DAG as human-readable mathematical statements first, prove from easy to
hard, and derive all completion status from the code, never from
hand-maintained markers.  It is built on three existing tools:

- [leanblueprint](https://github.com/PatrickMassot/leanblueprint): a
  LaTeX "blueprint" whose nodes link to Lean declarations and whose
  dependency graph renders as a live dashboard;
- [lean-lsp-mcp](https://github.com/oOo0oOo/lean-lsp-mcp): an MCP server
  exposing live goal state and Mathlib search, for agent-assisted
  proving;
- [lean4-skills](https://github.com/cameronfreer/lean4-skills): the
  `/lean4:*` Claude Code commands (draft, prove, golf, checkpoint with
  axiom scan).

This mirrors the repository's own docs-before-code rule: a blueprint
node is to a theorem what a design doc is to a feature.

## Decision

Four adoptions and two rejections.

### 1. CI builds `formal/` and gates on sorry-free, axiom-clean proofs

Add a `formal` job (path-filtered on `formal/**`) that:

- builds the development with the pinned toolchain and the Mathlib
  build cache (`lake exe cache get`), e.g. via
  [`leanprover/lean-action`](https://github.com/leanprover/lean-action);
- fails on any `sorry` or `admit` under `formal/Mensura/`;
- fails if any exported theorem depends on axioms beyond the standard
  three (`propext`, `Classical.choice`, `Quot.sound`), via an axiom
  scan.

Policy consequence: **`main` stays sorry-free**.  A planned result lives
in the blueprint as a statement without a Lean declaration (a white
node); an in-progress `sorry` lives only on branches.  The checker and
the docs may cite only proved (green) nodes, which operationalizes the
"backed by a theorem or stay conservative" rule.

### 2. Split `formal/` into themed modules

Adopt the file conventions of the workflow: one area per file, a heavy
theorem in its own file, minimal imports (import the dependency, not
everything), and a root aggregator whose only job is imports and a
stable map of the module DAG.  Indicatively:

```
formal/Mensura.lean            root: imports + module map
formal/Mensura/
  Core/Defs.lean               Table, Row, Cell; split, bind, Disjoint;
                               SplitInvariant, BindHom, SplitSafe
  Core/Ops.lean                map, joins, aggregate, ungroup, project,
                               tagged bind/split
  SplitSafety.lean             per-operation safety + composition
  Reshape.lean                 unpivot, pivot, unpivotDrop, inverse pair
  Rectangle.lean               Exhaustive / Total / Minimal propagation
  Completeness/FiberMap.lean   the fiberwise characterization
  Completeness/Verbs.lean      the derived verb catalogue
```

The exact split is fixed in the migration PR; the rule is the decision,
the listing is illustrative.  Status inventories ("done here: ...")
move out of file headers into the blueprint; code comments state
mathematics, not progress.

### 3. A blueprint is the map of the development

Add `formal/blueprint/` (leanblueprint layout).  Each result is one
node: a `\lean{...}` link to the declaration, a `\label`, `\uses` edges
to the lemmas it builds on, and a mathematical statement in LaTeX (with
a `\proof` sketch for theorems).  Rules:

- **Color is derived, never authored.**  A script (adapted from the
  spec-driven-lean tooling) marks each node from the code: white when
  the declaration is absent (planned), blue when it exists with `sorry`
  (branches only), green when it is sorry-free.  Hand-written `\leanok`
  is forbidden.
- **Nodes cross-link the decision record.**  A node cites the ADR or
  book definition it discharges (e.g. `def:split-invariance`,
  ADR 0020), and language docs cite blueprint labels back.  This gives
  the ADR-to-theorem traceability that today lives only in doc-comment
  prose.
- **Hard results are planned as white nodes first.**  The lemma DAG for
  a target like the binary-join conjecture is authored and reviewed in
  the blueprint before proving starts, exactly as a design doc precedes
  code.

### 4. Repository-level proving tooling

Ship a `.mcp.json` at the repository root registering `lean-lsp-mcp`,
so agent sessions get live goal state and Mathlib search over `formal/`
(the server locates the lake project from the file paths it is given).
The `lean4-skills` plugin is per-user, not per-repository; installing
it is a one-time step recorded here:

```
/plugin marketplace add cameronfreer/lean4-skills
/plugin install lean4@lean4-skills
```

### Rejected

- **The containerized agent image.**  The reference workflow ships a
  Docker image (Lean + Mathlib + agent) for reproducing a standalone
  formalization artifact.  `formal/` lives inside this repository and
  CI is its reproducibility story; an image adds maintenance without a
  consumer.
- **Local git hooks enforcing one result per commit.**  Our unit of
  review is the pull request, and gates belong in CI where they bind
  everyone.  Commit granularity stays a convention, not a hook.

## Consequences

- Proofs can no longer rot silently: a Mathlib bump or a `formal/` edit
  that breaks a theorem fails the PR that introduces it.
- The checker's dependence on the formal development becomes auditable:
  each propagation rule points at a green blueprint node.
- Planning a hard theorem becomes a reviewable artifact (a white-node
  DAG in a PR) instead of a private effort.
- Costs: CI minutes for the Lean job (bounded by the Mathlib cache and
  the path filter); the plasTeX/leanblueprint toolchain for whoever
  renders the dashboard (rendering is optional, the `.tex` source is
  the reviewed artifact); one migration PR of pure moves to split the
  files, with no statement changes.

## Migration order

1. CI job for `formal/` (build + sorry/axiom gate).
2. File split per section 2 (pure moves).
3. `.mcp.json` at the root.
4. Blueprint bootstrap: the current inventory becomes green nodes, the
   named open problems become white ones.
