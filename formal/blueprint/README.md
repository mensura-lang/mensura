# Blueprint

The map of the Lean development (ADR 0021,
`docs/decisions/0021-formal-proof-pipeline.md`): one node per result in
`src/content.tex`, with a `\lean{...}` link to the declaration, `\uses`
dependency edges, and the statement in mathematical language.  The
reviewed artifact is the LaTeX source; rendering is optional.

The Python tooling is a uv project (`pyproject.toml` here, locked in
`uv.lock`): plasTeX and the leanblueprint plugins are resolved
automatically on first use, with no install step.  All commands are
`make` targets run from `formal/`.

## Scope: public guarantees and open problems

The blueprint carries only the results the rest of the project relies
on, plus the open problems: the split/bind algebra and its laws, the
split-safety guarantees behind the checker's propagation rules, the
reshape inverse pair and the rectangle fact (ADR 0020), the safe
completeness characterizations, and the planned white nodes.  Internal
lemmas, plan-rewrite laws for the future optimizer, and per-verb
restatements of `map` corollaries live only in Lean; they are guarded
by the same CI gates (build, sorry grep, axiom check) but get no node.

A checker rule or language doc may cite a Lean declaration name
directly; a blueprint node is expected only for a headline guarantee or
an open problem.  Dropped nodes remain in git history and can be
restored (densified) if outside contributors arrive and need the full
map.

## Color is derived, never authored

```
make color
```

scans the Lean sources and sets or removes `\leanok` on every node:
white = declaration absent (planned), blue = declared with `sorry`
(branches only), green = sorry-free.  Do not write `\leanok` by hand;
re-run after proving.

```
make color-check
```

runs the same derivation without writing and fails on drift: `\leanok`
out of sync with the code, a `\lean{...}` name that resolves to no
declaration and is not marked planned, a stale `% planned` marker, or a
`\uses` edge naming a label that does not exist.  CI runs this on every
pull request touching `formal/**`, so drift cannot land silently.

## `\uses` edges are best-effort

The `\uses` edges are narrative: they sketch how the results hang
together for a reader of the rendered graph.  They are mechanically
checked only for target existence (no dangling labels); their
completeness against the real Lean proof dependencies is not checked
and is not a guarantee.  The authoritative dependency structure is the
Lean code itself.

## Rendering (optional)

- `make web` recolors and renders the dashboard to `blueprint/web/`
  (open `index.html`; the graph is under "Dependency graph");
- `make pdf` recolors and builds the PDF in `blueprint/print/`
  (needs a TeX installation with latexmk).

The [leanblueprint](https://github.com/PatrickMassot/leanblueprint) CLI
assumes the lakefile sits at the git root, which does not fit this
nested layout, so the targets drive plastex/latexmk directly with the
same configuration.  Build outputs are gitignored.

## Authoring new nodes

Plan a hard result as white nodes first (statement and edges, a planned
Lean name, no declaration), matching the docs-before-code rule, and put
a `% planned` comment on the line bearing its `\lean{...}` so the check
knows the name is intentionally unresolved.  When the result lands in
Lean, remove the marker and re-run `make color`.  A checker propagation
rule may only cite a green node or a sorry-free Lean declaration.
