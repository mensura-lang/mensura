# Mensura: agent notes

Mensura is a statically typed language for data handling whose type system
encodes sampling, dependency, lineage, and content properties so that
semantic mistakes (data leakage, wrong CV strategy on temporal data, biased
training sets, broken split-invariance, physical-unit mismatches) become
compile errors.

The project is in M1.  Creating a store is built end to end, and shapes,
views, and the core pipeline algebra are typechecked in the frontend (see
Implementation below).
`ROADMAP.md` has the phased plan and `docs/language/00-overview.md` says what
the language is.

## Source material

- The book's Chapter 5 defines indexed tables, split-invariance, and the
  core algebra.  Authoritative for the mathematical foundations.  Cite as:
  F. A. N. Verri (2026). Data Science Project: An Inductive Learning Approach.
  Version v1.0.0. Victoria, British Columbia, Canada: Leanpub. doi:
  10.5281/zenodo.14498010. url: https://leanpub.com/dsp.
- `proposal.md`: the language vision and the college case study.
  Authoritative for surface syntax intent and the store/collect/auth model.
- `../../postdoc_relatorio_2025/main.tex`: the postdoc report that scopes
  the academic deliverable to ML-validation correctness.

When a design decision conflicts across these, the roadmap and the
`docs/decisions/` ADRs win; the source documents are evidence, not
specification.

## Implementation

The toolchain is a Rust workspace under `crates/`.  The pipeline is
`source -> tokens -> AST -> resolved Schema -> SQLite`:

- `mensura-syntax`: lexer, hand-written recursive-descent LL(1) parser, AST.
  The lexer is **keyword-free** (every word is an `Ident`; the parser matches
  contextual keywords by position).  Identifiers follow UAX#31.  Spans are
  byte offsets carried on every token and AST node.  Grammar lives in
  `docs/language/04-grammar.md`.
- `mensura-types`: name resolution and the resolved `Schema` model, the
  boundary IR (a store flattened to ordered, typed columns tagged as index
  or attribute), plus the expression and pipeline checkers (`expr_check`,
  `pipe_check`) that type view bodies.  `resolve` collects *all* diagnostics
  rather than failing on the first.
- `mensura-runtime`: the `StorageBackend` trait and a `SqliteBackend`
  (rusqlite, `bundled`).  A `Schema` maps to `CREATE TABLE` (index columns as
  the primary key, `enum` as `TEXT CHECK`).  Storage mapping and the
  storage-versus-processing (DBSP) split are in
  `docs/toolkit/00-storage-backend.md`.
- `mensura-cli`: the `mensura` binary.  `mensura lex <file>` dumps tokens;
  `mensura check <file>` typechecks without touching a database;
  `mensura run <file> [--db <path>]` typechecks and creates the stores
  (`--db` defaults to an in-memory database); `mensura lsp` runs the
  language server over stdio.
- `mensura-lsp`, `mensura-highlight`, `mensura-mdbook`: the language server
  backend, the shared source-classification layer for highlighting, and the
  mdBook preprocessor that highlights and check-gates the `book/` code
  blocks (`docs/toolkit/03-book-highlighting.md`).

Current scope: scalar-index units; stores with primitive attributes
(`string`, `int`, `real`, `bool`, `date`, and named `enum` types whose
variants are string literals); shapes with `Unit`/`string` parameters; and
views hosting the core pipeline algebra over the expression sublanguage,
including the Tier B completeness discharge (ADR 0017).  Compound
units, `domain`/foreign-key resolution, and physical-unit/precision types
are deferred and rejected with "not yet supported" diagnostics.  Worked
examples live in `docs/examples/*.mensura`.

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
- Doc numbering is **per-folder**, not a global sequence.  `docs/language/`
  and `docs/toolkit/` each carry their own 2-digit reading order starting at
  `00`; the same number in two folders is fine because the path
  disambiguates.  `docs/decisions/` is the exception: a 4-digit, append-only
  ADR series that is never renumbered (a superseded ADR keeps its number).
  Cite docs by path or name, never by bare number.
- Do not create new top-level files without a clear place for them in the
  repository layout described in `ROADMAP.md`.
- Before each commit, run `cargo fmt --all` and `cargo test --workspace`.
  CI enforces the same on every pull request (`cargo fmt --all -- --check`
  plus the tests), so a commit that skips them will fail CI.
