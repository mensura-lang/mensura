# Language server

`mensura lsp` is the language server: it speaks the Language Server Protocol
(LSP) over stdio so editors can show typed feedback as the user writes
(`ROADMAP.md`, M5).  This document specifies the **basic** server: the two
features that make `.mensura` files legible in an editor today, **semantic
token highlighting** and **diagnostics**.  Hover, completion, goto-definition,
and find-references are forward references (see the end of this document).

Scope follows the rest of the toolchain: units and scalar-index stores with
primitive attributes (`docs/language/01-units.md`,
`docs/language/02-stores.md`).  Compound units, `domain` resolution, and
physical-unit types are out of scope and surface as diagnostics, not crashes.

## What the basic server exposes

| Capability                    | Notes                                       |
|-------------------------------|---------------------------------------------|
| Text document sync (full)     | Whole-document text on open and change.     |
| Semantic tokens (full)        | Highlighting for an entire document.        |
| Diagnostics (push)            | `publishDiagnostics` after each change.     |

Nothing else is advertised.  An editor that asks for hover or completion gets
the protocol's empty response until those features land.

## Architecture: reuse the pipeline

The server is the backend of the `mensura lsp` subcommand, living in
`crates/mensura-lsp` and wired into `mensura-cli` next to `lex` and `run`
(`ROADMAP.md`, repository layout).  It adds no new analysis: it drives the
existing pipeline and translates the results into protocol messages.

```
source --lex--> tokens (+ trivia) --parse--> AST --resolve--> Schema
                    |                  |                  |
                    +-- comments       +-- keyword spans  +-- diagnostics
                        (highlight)         (highlight)        (publish)
```

Every stage already carries byte-offset spans: `Token`, `Trivia` (ADR 0005),
each AST node, `LexError`, `ParseError`, and `ResolveError`.  The server's job
is span translation and protocol plumbing, not language analysis.

### Protocol library

The transport and message types use `lsp-server` together with `lsp-types`
(the synchronous pair from the rust-analyzer project).  This is deliberate:

- The compiler pipeline is synchronous, so a synchronous request loop matches
  it without an async runtime.  `tower-lsp` would pull in `tokio` and an
  async boundary for no benefit at this scope.
- `lsp-types` provides the wire structs (capabilities, semantic tokens,
  diagnostics), so the server only owns the mapping logic.

### Document state

The server keeps, per open document URI, the latest full text and its version.
On `didOpen` and `didChange` it replaces the stored text, re-runs the pipeline,
and republishes.  Incremental sync is a forward reference; full sync keeps the
basic server simple and is correct for files of the size Mensura programs have
now.

## Position encoding: byte spans to LSP positions

Internally Mensura uses byte offsets (`Span` is a half-open byte range).  LSP
positions are `(line, character)` pairs, where `character` is measured in
UTF-16 code units by default.  The two disagree for any non-ASCII source, and
Mensura allows Unicode identifiers (`máquina`, `温度`,
`docs/language/04-grammar.md`), so the mapping must be exact.

The server builds a **line index** per document version: the byte offset of
each line start.  Translating a byte offset is then a binary search for the
line, plus a count of code units in that line's prefix up to the offset.

During `initialize` the server negotiates a position encoding via
`general.positionEncodings`: it prefers **UTF-8** when the client supports it
(Neovim does), which makes the prefix count a byte count and avoids UTF-16
re-counting.  It falls back to UTF-16 otherwise.  The line index is built for
whichever encoding was negotiated.

## Semantic tokens

The server reports semantic tokens for the whole document
(`textDocument/semanticTokens/full`).  The editor maps each token type to a
theme color, so this is where highlighting comes from; no Vim syntax file or
tree-sitter grammar is required.

### Legend

Token types (the legend advertised at `initialize`):

| Type         | Source                                                |
|--------------|-------------------------------------------------------|
| `keyword`    | Contextual keywords: `unit`, `store`, `const`, `var`, `domain`, `enum`. |
| `type`       | Declaration and reference names of units (`UnitDecl.name`, `StoreDecl.unit`, type-position idents). |
| `property`   | Field and attribute names (`Field.name`, `DomainEntry.field`). |
| `string`     | String literals.                                      |
| `number`     | Integer and float literals.                           |
| `operator`   | Operators and punctuation.                            |
| `enumMember` | `enum(...)` variants.                                 |
| `comment`    | Line comments, from the trivia channel.               |

No token modifiers initially.  A `var`-versus-`const` modifier and a
primitive-versus-unit-reference split are forward references.

### Two tiers, and why keywords need the parser

The lexer is keyword-free: `unit`, `store`, and friends are all `Ident`
tokens, and only the parser knows, from position, that a given `Ident` is
acting as a keyword (`docs/language/04-grammar.md`).  A token-stream
highlighter therefore cannot color keywords correctly: a unit a user named
`store` would be miscolored.  So highlighting runs over the **parse**, not the
raw tokens.

**AST tier (primary).**  When the document parses, the server emits tokens
from the AST and a small companion table:

- *Keyword spans* come from the parser.  As it recognizes each contextual
  keyword (it already calls `at_keyword("unit")`, `"store"`, `"const"`,
  `"var"`, `"domain"`, `"enum"`), the parser records the matched span in a
  classified-span table returned alongside the AST.  This keeps the keyword
  vocabulary in exactly one place (the parser) and covers the clause-header
  keywords (`unit {`, `const {`, `var {`, `domain {`) whose spans the AST does
  not otherwise store.  The highlighter never re-derives the keyword set.
- *Types* are `UnitDecl.name`, `StoreDecl.name`, `StoreDecl.unit`,
  `DomainEntry.store`, and every `TypeExpr::Named` ident.
- *Properties* are `Field.name` and `DomainEntry.field`.
- *enumMembers* are the `StrLit` variants inside `TypeExpr::Enum`.
- *strings*, *numbers*, and *operators* come from the token stream, since
  those token kinds never conflict with the AST classification above.

**Lex tier (fallback).**  When parsing fails, the AST is absent or partial, so
the server cannot classify keywords, types, or properties.  It falls back to
the token and trivia streams: string literals, numbers, and operators still
color, comments still color (see below), and bare `Ident`s are left
unclassified (default text).  This guarantees a file mid-edit, with a syntax
error, still gets reasonable highlighting instead of going blank.

**Comments (both tiers).**  Comments come from the lexer's trivia channel
(ADR 0005), independent of whether the parse succeeded, so they always color.
Every `Trivia` span becomes a `comment` token.

This split is the load-bearing reason ADR 0005 records comments on a side
channel rather than discarding them: it is the only source of comment spans,
and it is available even when the parser is not.

## Diagnostics

After each change the server runs the pipeline and pushes every diagnostic via
`textDocument/publishDiagnostics`.  The error model itself belongs to
`docs/toolkit/01-diagnostics.md`; this section only specifies the mapping onto
the wire.

- A `LexError` halts the pipeline (the lexer stops at the first malformed
  token), so it yields a single diagnostic at its span and no semantic tokens
  beyond what the partial token stream allows.  Lexer error recovery is a
  forward reference.
- A `ParseError` yields one diagnostic at its span; the server still emits
  lex-tier highlighting for the document.
- `resolve` collects *all* errors (`crates/mensura-types/src/resolve.rs`), so
  every `ResolveError`, including the "not yet supported" rejections for
  out-of-scope constructs, becomes its own diagnostic.

Each diagnostic carries the span (translated through the line index), the
message, and `source: "mensura"`.  All are reported at error severity for now;
distinguishing "not yet supported" as a separate severity is a forward
reference.

## Editor integration

The server is editor-agnostic, but Neovim is the reference client.  Two pieces
are needed on the editor side:

- **Filetype detection** for the `.mensura` extension, so the client knows
  when to attach.
- **A client config** that launches the server and scopes it to the filetype:

```lua
vim.filetype.add({ extension = { mensura = 'mensura' } })

vim.lsp.config('mensura', {
  cmd = { 'mensura', 'lsp' },
  filetypes = { 'mensura' },
  root_markers = { '.git', 'Cargo.toml' },
})
vim.lsp.enable('mensura')
```

Neovim applies semantic tokens to highlight groups automatically once the
server advertises them, so no extra plugin is required for color.  Diagnostics
flow through the editor's standard diagnostic UI.

## Forward references

- Hover revealing a binding's full type (`ROADMAP.md`, M5: the type quadruple).
  Doc comments (ADR 0005) attach to the declaration they precede and show in
  hover, the rustdoc / Javadoc pattern.
- Completion, goto-definition, and find-references.
- Incremental document sync and semantic-token deltas / range requests.
- Token modifiers: `readonly` for `const` versus `var` attributes, and a
  primitive-versus-unit-reference distinction once resolution annotates types.
- Lexer error recovery, so highlighting and diagnostics survive a malformed
  token instead of stopping at it.
- Pull diagnostics (`textDocument/diagnostic`) if a client prefers them to the
  push model.
- Shared infrastructure with `mensura fmt`, which needs the same trivia
  channel to preserve comments across formatting.
