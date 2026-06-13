//! The `mensura` command-line tool.
//!
//! Subcommands are added milestone by milestone (see `ROADMAP.md`):
//!
//! - `lex`  -- print the token stream of a source file (a lexer debug aid).
//! - `run`  -- typecheck a program and create its stores in a database.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use mensura_runtime::{EnsureOutcome, SqliteBackend, StorageBackend};
use mensura_syntax::{Span, parse, tokenize};

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
    /// Typecheck a program and create its stores in a database.
    Run {
        /// The Mensura source file to run.
        file: PathBuf,
        /// The SQLite database to create the stores in.  Defaults to an
        /// ephemeral in-memory database; pass a path to persist.
        #[arg(long, default_value = ":memory:")]
        db: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file } => cmd_lex(&file),
        Command::Run { file, db } => cmd_run(&file, &db),
    }
}

fn cmd_lex(path: &Path) -> ExitCode {
    let Some(src) = read_source(path) else {
        return ExitCode::FAILURE;
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
            report(path, &src, &err.message, err.span);
            ExitCode::FAILURE
        }
    }
}

fn cmd_run(path: &Path, db_path: &Path) -> ExitCode {
    let Some(src) = read_source(path) else {
        return ExitCode::FAILURE;
    };

    let tokens = match tokenize(&src) {
        Ok(tokens) => tokens,
        Err(err) => {
            report(path, &src, &err.message, err.span);
            return ExitCode::FAILURE;
        }
    };
    let program = match parse(&tokens) {
        Ok(program) => program,
        Err(err) => {
            report(path, &src, &err.message, err.span);
            return ExitCode::FAILURE;
        }
    };
    let schemas = match mensura_types::resolve(&program) {
        Ok(schemas) => schemas,
        Err(errors) => {
            for err in &errors {
                report(path, &src, &err.message, err.span);
            }
            return ExitCode::FAILURE;
        }
    };

    let in_memory = db_path.as_os_str() == ":memory:";
    let opened = if in_memory {
        SqliteBackend::open_in_memory()
    } else {
        SqliteBackend::open(db_path)
    };
    let mut backend = match opened {
        Ok(backend) => backend,
        Err(e) => {
            eprintln!("error: cannot open database {}: {e}", db_path.display());
            return ExitCode::FAILURE;
        }
    };
    if in_memory {
        eprintln!("note: using an in-memory database; pass --db <path> to persist");
    }
    for schema in &schemas {
        match backend.ensure_store(schema) {
            Ok(EnsureOutcome::Created) => {
                println!(
                    "created store {} ({} columns)",
                    schema.store,
                    schema.columns.len()
                );
            }
            Ok(EnsureOutcome::AlreadyExists) => {
                println!("store {} already exists", schema.store);
            }
            Err(e) => {
                eprintln!("error: store {}: {e}", schema.store);
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn read_source(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(src) => Some(src),
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            None
        }
    }
}

/// Print a span-located diagnostic in `error: ...` / `--> file:line:col` form.
fn report(path: &Path, src: &str, message: &str, span: Span) {
    let (line, col) = line_col(src, span.start);
    eprintln!("error: {message}");
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
