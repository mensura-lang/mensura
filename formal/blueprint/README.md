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

## Color is derived, never authored

```
make color
```

scans the Lean sources and sets or removes `\leanok` on every node:
white = declaration absent (planned), blue = declared with `sorry`
(branches only), green = sorry-free.  Do not write `\leanok` by hand;
re-run after proving.

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
Lean name, no declaration), matching the docs-before-code rule.  When a
result lands in Lean, add or update its node in `src/content.tex` and
re-run `make color`.  A checker propagation rule may only cite a green
node.
