//! The `mensura` command-line tool.
//!
//! Subcommands are added milestone by milestone (see `ROADMAP.md`).  For now
//! the only one is `lex`, a debug aid that prints the token stream of a
//! source file.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use mensura_syntax::{LexError, tokenize};

#[derive(Parser)]
#[command(name = "mensura", about = "The Mensura toolchain", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the token stream of a source file (a lexer debug aid).
    Lex {
        /// The Mensura source file to tokenize.
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file } => cmd_lex(&file),
    }
}

fn cmd_lex(path: &Path) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    match tokenize(&src) {
        Ok(tokens) => {
            for tok in &tokens {
                let (line, col) = line_col(&src, tok.span.start);
                println!("{line}:{col}\t{:?}", tok.kind);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            report_error(path, &src, &err);
            ExitCode::FAILURE
        }
    }
}

fn report_error(path: &Path, src: &str, err: &LexError) {
    let (line, col) = line_col(src, err.span.start);
    eprintln!("error: {}", err.message);
    eprintln!("  --> {}:{line}:{col}", path.display());
}

/// Translate a byte offset into a 1-based (line, column) pair.  The column is
/// counted in Unicode scalar values, not bytes, so multi-byte characters
/// advance the column by one.
fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (idx, ch) in src.char_indices() {
        if idx >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
