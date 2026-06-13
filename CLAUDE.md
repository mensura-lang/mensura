# Mensura: agent notes

Mensura is a statically typed language for data handling whose type system
encodes sampling, dependency, lineage, and content properties so that
semantic mistakes (data leakage, wrong CV strategy on temporal data, biased
training sets, broken split-invariance, unit mismatches) become compile
errors.

The project is in **Pre-M0**: design docs only, no code yet.  See
`ROADMAP.md` for the phased plan and `docs/language/00-overview.md` for
what the language is.

## Source material

- Chapter 5 of <https://zenodo.org/records/18815798>: defines indexed
  tables, split-invariance, and the core algebra.  Authoritative for the
  mathematical foundations.
- `proposal.md`: the language vision and the college case study.
  Authoritative for surface syntax intent and the store/collect/auth model.
- `../../postdoc_relatorio_2025/main.tex`: the postdoc report that scopes
  the academic deliverable to ML-validation correctness.

When a design decision conflicts across these, the roadmap and the
`docs/decisions/` ADRs win; the source documents are evidence, not
specification.

## Style guide

- **Double spaces after a period in documentation files** (`.md`, `.tex`).
  This matches the existing prose in the source material.
  Single space inside code, identifiers, and inline code spans.
- **Avoid em-dashes** (`—` in Markdown, `---` in LaTeX).  Use a comma,
  colon, parentheses, or a new sentence instead.
- Wrap prose at ~78 columns in `.md` and `.tex` files.
- One concept per design doc.  Cross-link rather than duplicate.
- No emojis in docs or code.

## Working on this repo

- Every language- or tooling-level feature lands as a design document under
  `docs/` *before* any code.  Code is the encoding of an agreed-upon spec.
- Do not create new top-level files without a clear place for them in the
  repository layout described in `ROADMAP.md`.
- Before each commit, run `cargo fmt --all` and `cargo test --workspace`.
  CI enforces the same on every pull request (`cargo fmt --all -- --check`
  plus the tests), so a commit that skips them will fail CI.
