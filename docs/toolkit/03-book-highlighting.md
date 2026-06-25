# Book code highlighting

The Mensura book (`book/`, built with mdBook and hosted on GitHub Pages) is
prose interleaved with `.mensura` code.  This document specifies how that code
is **highlighted** and **kept honest**: a single mdBook preprocessor,
`mdbook-mensura`, colors every Mensura code block using the compiler's own
classification and fails the build when a block that claims to compile does
not.

The point is to never highlight Mensura with an approximation.  The lexer is
keyword-free: every word is an `Ident`, and only the parser knows that a given
identifier was acting as `unit`, `store`, or `const` in its position
(`docs/language/04-grammar.md`, `docs/toolkit/02-lsp.md`).  A regex grammar
(highlight.js, the mdBook default) cannot reproduce this; the parser can.  So
the book colors from the same pipeline the editor does, and the two agree by
construction.

## Architecture: a shared classifier crate

Classifying a source string into colored spans is not an LSP concern; the LSP
is one consumer of it.  The classifier therefore lives in its own crate,
`mensura-highlight`, which depends only on `mensura-syntax` and
`mensura-types` and knows nothing about the protocol.  The language server and
the book preprocessor are both thin adapters on top of it.

```
   mensura-syntax ─┐
                   ├─► mensura-highlight   (classify src -> spans + errors)
   mensura-types ──┘          ▲                        ▲
                              │                        │
                       mensura-lsp               mensura-mdbook
                 (spans -> positions,          (spans -> colored HTML,
                  deltas, Diagnostic)           check-gate the build)
```

This keeps the book binary off the LSP server stack (`lsp-server`,
`lsp-types`) and leaves the classifier reusable for terminal coloring in
`mensura fmt` or the REPL later.  Internally the pipeline is unchanged from
`docs/toolkit/02-lsp.md`:

```
source --lex--> tokens (+ trivia) --parse--> AST --resolve--> errors
                    |                  |
                    +-- literals,      +-- keyword spans, types,
                        comments           properties, parameters
                            \             /
                             classified spans (byte offsets)
```

`mensura-highlight` exposes:

```rust
pub fn highlight(src: &str) -> Highlighted;

pub struct Highlighted {
    pub spans: Vec<Highlight>,   // non-overlapping, byte offsets, in order
    pub errors: Vec<CheckError>, // lex, parse, and resolve errors
}

pub struct Highlight { pub start: usize, pub end: usize, pub kind: HighlightKind }
pub struct CheckError { pub start: usize, pub end: usize, pub message: String }

pub enum HighlightKind {
    Keyword, Type, Property, Parameter,
    String, Number, Operator, EnumMember, Comment,
}
```

`mensura-lsp` translates these byte spans into position-encoded, delta-encoded
`SemanticToken`s and `CheckError`s into `Diagnostic`s; `mensura-mdbook`
translates the same byte spans into HTML.  One classifier and one
overlap-resolution pass serve both, so a fix improves the editor and the book
together.  `HighlightKind` mirrors the nine LSP semantic-token types exactly.

## The preprocessor

`mdbook-mensura` is a workspace binary (`crates/mensura-mdbook`) following the
mdBook preprocessor protocol:

- `mdbook-mensura supports <renderer>` exits `0` for `html` and `1` otherwise,
  so the preprocessor runs only for the HTML renderer.
- Otherwise it reads `[context, book]` as JSON on stdin, rewrites the book, and
  writes the book back as JSON on stdout.

It walks every chapter (and sub-chapter), and in each chapter's Markdown it
finds fenced blocks whose info string starts with `mensura`.  Each such block
is replaced with pre-colored HTML: the source is sliced at the `highlight`
spans, each span wrapped in `<span class="mn-<kind>">`, the gaps and the
surrounding `<pre><code>` emitted as HTML-escaped text.  Because the result is
raw HTML, the bundled highlight.js leaves it untouched (see "Bypassing
highlight.js" below).

### Info-string modifiers

The fence info string selects how strictly the block is checked:

| Fence            | Highlighted | Check gate                              |
|------------------|-------------|-----------------------------------------|
| ` ```mensura `   | yes         | must lex, parse, and resolve with no errors |
| ` ```mensura,ignore ` | yes    | none; errors are not reported           |

`mensura` is the default and the common case: a worked example that must
compile.  If `highlight` returns any `CheckError` for such a block, the
preprocessor prints the file, the block, and the messages to stderr and exits
non-zero, which fails `mdbook build` and therefore CI.  This is the book's
"examples cannot rot" guarantee, enforced at build time with no separate
`mensura check` pass.

`mensura,ignore` is the escape hatch for blocks that legitimately do not
compile: a snippet that uses syntax from a later milestone (a "design preview"
ahead of the implementation) or a deliberately rejected program shown to
explain an error.  It is still colored, best-effort, from whatever the lexer
and parser recover.

### Bypassing highlight.js

mdBook ships highlight.js and colors `<code>` blocks in the browser.  The
preprocessor's output must not be re-colored on top of the spans it already
emitted.  The block is therefore emitted as already-highlighted HTML that
highlight.js skips (a non-`language-*` marker class plus the
already-highlighted signal mdBook's bundled build honors).  The exact markers
are an implementation detail of the renderer version and are pinned by a
snapshot test in the crate, not by this document.

### Styling

Colors are CSS, not inline styles: the preprocessor only emits class names
(`mn-keyword`, `mn-type`, `mn-property`, `mn-parameter`, `mn-string`,
`mn-number`, `mn-operator`, `mn-enum-member`, `mn-comment`).  A stylesheet
shipped via the book's `additional-css` maps them to colors and is themed
alongside the rest of the book.  Editors pick their own theme for the same
nine token types, so the book and an editor share structure, not palette.

## What this is not

This preprocessor highlights and check-gates; it does not run programs, create
stores, or render output.  Executable examples (showing what `mensura run`
produces) are a later concern and out of scope here.  It also does not drive
the live `mensura lsp` server: the server is stateful and session-oriented,
the wrong tool for a batch build.  The shared piece is the classifier, reused
as a library, not the protocol.
