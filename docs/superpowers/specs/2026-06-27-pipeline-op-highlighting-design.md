# Highlighting pipeline operations

## Problem

M1 landed the pipeline sublanguage (`view` bodies, the `|>` operator, and the
operations `map`, `group_map`, `split`, `bind`, `left_join`, `inner_join`,
`extend_key`, `shrink_key`, `unpivot`, `pivot`, `assume`,
`completeness_check`).  The shared highlighter (`mensura-highlight`, consumed by
both `mensura-lsp` and `mensura-mdbook`) does not color any of it well:

1. **Operation names are uncolored.**  Each op is a plain `Ident` at the head
   of a curried application on the right of `|>`.  The parser parses `|>`
   generically and never marks the head, so `map`, `pivot`, and friends render
   as default text.
2. **Expression and statement keywords color inconsistently.**  `then`, `else`,
   and `if` go through `bump_keyword` and color, but `or`, `and`, `not`, `is`,
   `known`, `missing` (the predicate operators) and `let`, `assert` (statement
   keywords) consume their token with a bare `self.pos += 1` and are never
   recorded, so they do not color.  This predates the pipeline work; it is a
   latent highlighting bug that the pipeline bodies make visible.

## Goals

- Color pipeline operation names as builtin functions.
- Color the predicate and statement keywords consistently with the rest of the
  contextual-keyword vocabulary.
- Keep the keyword and operation vocabulary in exactly one place (the parser),
  as `docs/toolkit/02-lsp.md` requires.  The highlighter never re-derives a
  word list.

## Non-goals (deferred)

- The `complete` marker inside `assume { complete }`.  It is not a parser
  keyword today; it parses as a bare identifier expression inside a generic
  block.  Coloring it would require either coupling the parser to the
  `assume` operation's semantics or having `resolve` emit classified spans.
  Both are larger than this feature warrants.  Note that the `assert` and `let`
  keywords inside a `completeness_check { ... }` block *do* color, via the
  keyword-consistency fix above; only the `assume` block's `complete` marker is
  left plain.
- Lambda binders (`|k, r|`, `|k, g|`, `|k|`) as parameters.

## Design

### 1. Parser records operation spans (structural recognition)

`parse_pipe` already walks every `|>` operator.  After it parses each
right-hand side, it peels the curried application down to the leftmost atom; if
that atom is an `ExprKind::Name`, its span is recorded in a new `op_spans:
Vec<Span>` on `Parsed`, a sibling of `keyword_spans`.

Recognition is **structural, not vocabulary-based**: any identifier in
operation position (the head of a `|>` right-hand side) is an operation.  No
name list is needed, which keeps the parser the single source of truth and
avoids a second copy of the op vocabulary alongside `pipe_check`'s `apply_op`
dispatch.  Consequences:

- It works at the AST tier even when `resolve` fails, like `keyword_spans`.
- A mistyped op (`t |> mpa (...)`) still colors as a function; `resolve`
  reports the unknown-operation error separately.  This matches how a
  misspelled keyword is still positionally a keyword.

Nested pipelines (inside a lambda body or a parenthesized subexpression) are
covered for free, because those subexpressions are parsed through `parse_pipe`
recursively.

### 2. Consistent keyword recording

Route the predicate and statement keyword sites through `bump_keyword` instead
of a bare `self.pos += 1`: `or`, `and`, `not`, `is`, `known`, `missing` in the
expression grammar, and `let`, `assert` in statement position.  `if`, `then`,
`else` already do this.  No new vocabulary: these words are already recognized
positionally; they were simply not being recorded.

### 3. A `Function` highlight kind carrying the builtin modifier

`mensura-highlight` gains `HighlightKind::Function`.  The highlighter pushes
every `op_spans` entry as `Function`.  Its overlap priority sits just below
`Keyword` (operation names never overlap another classified span, since
identifiers are skipped by the literal pass, so the exact value is low-risk).

Because every `Function` span in this feature is a builtin operation, the
"builtin" fact is carried by the kind rather than a separate data field: each
consumer maps `Function` to its builtin representation.

- **`mensura-lsp`** maps `Function` to the LSP `function` token type and sets
  the `defaultLibrary` token modifier, the standard LSP way to say "builtin".
  This introduces the server's first token modifier: the legend's
  `token_modifiers` gains `defaultLibrary`, and `encode_tokens` sets the
  modifier bitmask for `Function` tokens (0 for all others).
- **`mensura-mdbook`** maps `Function` to a new `mn-function` CSS class, themed
  in `book/mensura-highlight.css` for both light and dark palettes.

## Files touched

- `crates/mensura-syntax/src/parser.rs`: `op_spans` on `Parsed` and the
  `Parser`; record op heads in `parse_pipe`; switch the eight keyword sites to
  `bump_keyword`.
- `crates/mensura-highlight/src/lib.rs`: `HighlightKind::Function`, its
  priority, and pushing `op_spans`.
- `crates/mensura-lsp/src/analysis.rs`: `function` in the legend, a
  `defaultLibrary` modifier legend, and modifier-bitmask encoding.
- `crates/mensura-lsp/src/lib.rs`: advertise the `token_modifiers` legend.
- `crates/mensura-mdbook/src/render.rs`: `mn-function` class.
- `book/mensura-highlight.css`: `.mn-function` color rules.
- `docs/toolkit/02-lsp.md`: legend table row, a pipeline-operators note, the
  first token modifier, and drop "No token modifiers initially".

## Testing

- Parser: a test asserting `or`/`and`/`not`/`is`/`known`/`missing`/`let`/
  `assert` now appear in `keyword_spans`, and a test asserting `op_spans`
  captures the operation heads of a multi-stage pipeline.
- Highlighter: a test asserting a `view` body's op names classify as
  `Function` and a predicate's operators classify as `Keyword`.
- LSP: a test asserting a `Function` token carries the `defaultLibrary`
  modifier bit and the `function` type index.
