# Blueprint

The map of the Lean development (ADR 0021,
`docs/decisions/0021-formal-proof-pipeline.md`): one node per result in
`src/content.tex`, with a `\lean{...}` link to the declaration, `\uses`
dependency edges, and the statement in mathematical language.  The
reviewed artifact is the LaTeX source; rendering is optional.

## Color is derived, never authored

Run

```
uv run blueprint/color.py
```

from anywhere (the script is stdlib-only and carries inline uv
metadata, so no environment setup is needed).  It scans the Lean sources and sets or removes `\leanok`
on every node: white = declaration absent (planned), blue = declared
with `sorry` (branches only), green = sorry-free.  Do not write
`\leanok` by hand; re-run the script after proving.

## Rendering (optional)

Install [leanblueprint](https://github.com/PatrickMassot/leanblueprint)
(`uv tool install leanblueprint`; plasTeX and its plugins come with it,
inside the tool's own environment), then from `formal/`:

- `leanblueprint web` renders the dashboard to `blueprint/web/`
  (open `index.html`; the dependency graph is under "Dependency graph");
- `leanblueprint checkdecls` validates that every `\lean{...}` name
  exists in the built library;
- `latexmk` in `blueprint/src/` builds the PDF (`print.tex`).

Build outputs are gitignored.

## Authoring new nodes

Plan a hard result as white nodes first (statement and edges, a planned
Lean name, no declaration), matching the docs-before-code rule.  When a
result lands in Lean, add or update its node in `src/content.tex` and
re-run `color.py`.  A checker propagation rule may only cite a green
node.
