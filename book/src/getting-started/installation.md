# Installing the toolchain

Mensura is a Rust workspace.  Until there are published binaries you build the
`mensura` command from source.

## Prerequisites

- A recent stable Rust toolchain (install via [rustup](https://rustup.rs)).
  The workspace pins no exotic features; any current stable release works.
- Git, to clone the repository.

No database server is needed.  At the moment stores are persisted in SQLite,
which is bundled into the binary, so there is nothing else to install.  SQLite
is the only backend today; future releases will add run and deploy
configurations that let a program target other backends without changing its
source.

## Build and install

Clone the repository and build the command:

```console
$ git clone https://github.com/mensura-lang/mensura
$ cd mensura
$ cargo build --release
```

The binary lands at `target/release/mensura`.  Put it on your `PATH`, or
install it into Cargo's bin directory:

```console
$ cargo install --path crates/mensura-cli
```

Check that it runs:

```console
$ mensura --help
The Mensura toolchain

Usage: mensura <COMMAND>

Commands:
  lex    Print the token stream of a source file (a lexer debug aid)
  check  Typecheck a program without creating any stores
  run    Typecheck a program and create its stores in a database
  lsp    Run the language server, speaking LSP over stdio
```

If you only want to try the language without installing anything, you can run
the command straight from the workspace with `cargo run --`:

```console
$ cargo run -- check path/to/program.mensura
```

## Editor support

`mensura lsp` is a language server: it speaks the Language Server Protocol over
stdio, so an editor can show typed feedback and the same highlighting you see
in this book.  Point your editor's generic LSP client at the `mensura lsp`
command for `.mensura` files.  The server's scope is documented in
`docs/toolkit/02-lsp.md`.

With the toolchain in place, write your [first store](first-store.md).
