# 0009: Lexer trivia channel

## Status

Proposed.

## Context

The lexer (`crates/mensura-syntax/src/lexer.rs`) discards whitespace and
`//` line comments in `skip_trivia`.  There is no `Comment` token, the
documented contract is "whitespace and `//` line comments are skipped", and a
test (`whitespace_and_comments_are_skipped`) pins that behavior.

Two designs depend on comment spans being recoverable:

- The language server (`docs/toolkit/02-lsp.md`) highlights comments as
  semantic tokens.  It needs the byte span of every comment.
- Doc comments are a likely future feature.  The ROADMAP requires hover to
  reveal a binding's full type, and a doc comment attached to a `unit` or
  `store` is the natural thing to show next to that type (the rustdoc /
  Javadoc / JSDoc pattern: comment *content* consumed by the toolchain).

A constraint must be preserved: the lexer is **keyword-free**, and the
parser matches contextual keywords by position over a clean `Ident` stream
(`crates/mensura-syntax/src/token.rs`).  Nothing in the comment design may
disturb that stream or force the parser to step over comment tokens.

## Decision

The lexer records comments on a **trivia side-channel**, separate from the
token stream, tagged with a comment kind:

```rust
pub struct Trivia {
    pub kind: TriviaKind,
    pub span: Span,
}

pub enum TriviaKind {
    /// A `//` line comment.
    LineComment,
    // DocComment (`///`, `//!`) is reserved for a later feature; see below.
}

pub struct Lexed {
    pub tokens: Vec<Token>,   // the existing stream, ending in Eof
    pub trivia: Vec<Trivia>,  // comments, in source order
}

/// Tokenize, returning the token stream and the trivia channel.
pub fn lex(src: &str) -> Result<Lexed, LexError>;
```

`tokenize` stays as the token-stream entry point, defined in terms of `lex`:

```rust
pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    lex(src).map(|l| l.tokens)
}
```

The token stream is unchanged: comments never appear in it, so the parser
and every existing `tokenize` caller are unaffected.  `skip_trivia` keeps
skipping comments out of the token stream, but pushes each one onto the
trivia channel instead of dropping it.

The `TriviaKind` enum exists from the start so the channel is kind-tagged,
even though only `LineComment` is produced now.  When doc comments are
designed, they add a `DocComment` variant on the same channel; the resolver
will attach the nearest preceding `DocComment` to the following declaration
for hover.  That work is deferred and is **not** part of this ADR.

## Consequences

Positive:

- The parser and the keyword-free token stream are untouched; the contextual
  keyword matching that depends on a clean `Ident` stream keeps working.
- Existing `tokenize` callers compile unchanged.
- Comment spans have a single source of truth (the lexer), so the language
  server does not re-implement comment scanning and the two cannot drift.
- Doc comments extend the same channel rather than needing a second
  mechanism later.

Negative:

- The lexer's public surface grows by one type and one function.
- The documented "comments are skipped" contract and the
  `whitespace_and_comments_are_skipped` test must be updated: comments are
  now *recorded on the trivia channel* while still *excluded from the token
  stream*.
- Trivia is collected on every lex, including the parser's hot path, which
  ignores it.  The cost is one `Vec` and a span push per comment.

Neutral:

- The trivia channel is append-only and in source order, so a consumer that
  wants comments interleaved with tokens can merge the two by span without
  the lexer taking a position on interleaving.

## Alternatives considered

1. **`TokenKind::Comment` in the main stream.**  Simplest lexer change, but
   it forces every parser site that scans tokens to skip comments, which
   touches the LL(1) parser and erodes the clean keyword-free stream.  It
   also still needs separate logic later to associate a doc comment with the
   declaration it documents.  Rejected.

2. **Re-scan comments in the language server.**  Leave the lexer discarding
   comments and have the server scan the source for `//` independently.
   Rejected: duplicate lexing logic and two definitions of what a comment is,
   which will drift (block comments, escapes inside strings, doc-comment
   prefixes).

3. **Highlight comments with a separate grammar.**  Ship a tree-sitter
   grammar or a Vim syntax file just for comments.  Rejected: a second
   grammar to maintain, and it cannot share the doc-comment-to-declaration
   path the toolchain will want for hover.

## Open questions

- **Block comments.**  Only `//` exists today.  If `/* ... */` is added, it
  lands on the same channel; the nesting policy is deferred to that feature.
- **Doc comment syntax.**  `///` and `//!` (Rust) versus `/** */` (Javadoc).
  Settled when doc comments are designed, not here.
- **Span boundaries.**  A `LineComment` span covers the `//` through the end
  of the line, excluding the trailing newline.  Confirm this is what the
  semantic-token mapper wants before implementation.
