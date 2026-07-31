//! The `mensura` command-line tool.
//!
//! Subcommands are added milestone by milestone (see `ROADMAP.md`):
//!
//! - `lex`   -- print the token stream of a source file (a lexer debug aid).
//! - `check` -- typecheck a program without touching a database.
//! - `run`   -- typecheck a program, create its stores in a database, and
//!   materialize its views (`docs/toolkit/04-processing-layer.md`).
//! - `ingest` -- decode a batch of records and append it to a store or
//!   registry (`docs/toolkit/05-ingestion.md`).
//! - `lsp`   -- run the language server over stdio.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use mensura_runtime::{
    Delta, EnsureOutcome, SqliteBackend, StorageBackend, decode_jsonl, materialize_views,
};
use mensura_syntax::{Span, StoreKind, parse, tokenize};

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
    /// Typecheck a program without creating any stores.
    Check {
        /// The Mensura source file to check.
        file: PathBuf,
    },
    /// Typecheck a program, create its stores in a database, and
    /// materialize its views.
    Run {
        /// The Mensura source file to run.
        file: PathBuf,
        /// The SQLite database to create the stores in.  Defaults to an
        /// ephemeral in-memory database; pass a path to persist.
        #[arg(long, default_value = ":memory:")]
        db: PathBuf,
    },
    /// Decode a batch of records and append it to a store or registry.
    Ingest {
        /// The Mensura source file declaring the target.
        file: PathBuf,
        /// The store or registry to append to.
        target: String,
        /// A JSON Lines file of records, or `-` for standard input.
        #[arg(long)]
        data: PathBuf,
        /// The SQLite database to write to.  Defaults to an ephemeral
        /// in-memory database, which makes a dry run cheap.
        #[arg(long, default_value = ":memory:")]
        db: PathBuf,
    },
    /// Run the language server, speaking LSP over stdio.
    Lsp,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file } => cmd_lex(&file),
        Command::Check { file } => cmd_check(&file),
        Command::Run { file, db } => cmd_run(&file, &db),
        Command::Ingest {
            file,
            target,
            data,
            db,
        } => cmd_ingest(&file, &target, &data, &db),
        Command::Lsp => cmd_lsp(),
    }
}

fn cmd_lsp() -> ExitCode {
    match mensura_lsp::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: language server: {e}");
            ExitCode::FAILURE
        }
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

fn cmd_check(path: &Path) -> ExitCode {
    let Some(src) = read_source(path) else {
        return ExitCode::FAILURE;
    };
    match frontend(path, &src) {
        Ok(program) => {
            // Registries are `Schema`s too (ADR 0033), so they are counted
            // under their own noun rather than reported as stores.
            let registries = program
                .schemas
                .iter()
                .filter(|s| s.kind == StoreKind::Registry)
                .count();
            let stores = program.schemas.len() - registries;
            let views = program.views.len();

            // A bare `0 stores` still reads as the useful "nothing here",
            // but suppress it once another count carries that news.
            let mut parts = Vec::new();
            if stores > 0 || (registries == 0 && views == 0) {
                parts.push(format!(
                    "{stores} {}",
                    if stores == 1 { "store" } else { "stores" }
                ));
            }
            if registries > 0 {
                parts.push(format!(
                    "{registries} {}",
                    if registries == 1 {
                        "registry"
                    } else {
                        "registries"
                    }
                ));
            }
            if views > 0 {
                parts.push(format!(
                    "{views} {}",
                    if views == 1 { "view" } else { "views" }
                ));
            }
            println!("ok: {}", parts.join(", "));
            ExitCode::SUCCESS
        }
        Err(()) => ExitCode::FAILURE,
    }
}

fn cmd_run(path: &Path, db_path: &Path) -> ExitCode {
    let Some(src) = read_source(path) else {
        return ExitCode::FAILURE;
    };
    let Ok(program) = frontend(path, &src) else {
        return ExitCode::FAILURE;
    };

    let Some(mut backend) = open_db(db_path) else {
        return ExitCode::FAILURE;
    };
    if db_path.as_os_str() == ":memory:" {
        eprintln!("note: using an in-memory database; pass --db <path> to persist");
    }
    for schema in &program.schemas {
        let noun = schema.kind.keyword();
        match backend.ensure_store(schema) {
            Ok(EnsureOutcome::Created) => {
                println!(
                    "created {noun} {} ({} columns)",
                    schema.store,
                    schema.columns.len()
                );
            }
            Ok(EnsureOutcome::AlreadyExists) => {
                println!("{noun} {} already exists", schema.store);
            }
            Err(e) => {
                eprintln!("error: {noun} {}: {e}", schema.store);
                return ExitCode::FAILURE;
            }
        }
    }
    match materialize_views(&mut backend, &program) {
        Ok(views) => {
            for (name, rows) in views {
                let noun = if rows == 1 { "row" } else { "rows" };
                println!("materialized view {name} ({rows} {noun})");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Decode a batch of records and append it to a store or registry
/// (`docs/toolkit/05-ingestion.md`, ADR 0034).
fn cmd_ingest(path: &Path, target: &str, data: &Path, db_path: &Path) -> ExitCode {
    let Some(src) = read_source(path) else {
        return ExitCode::FAILURE;
    };
    // Typecheck first, so ingestion never writes against an unresolved
    // schema.
    let Ok(program) = frontend(path, &src) else {
        return ExitCode::FAILURE;
    };

    let Some(schema) = program.schemas.iter().find(|s| s.store == target) else {
        if program.views.iter().any(|v| v.name == target) {
            eprintln!(
                "error: `{target}` is a view; a view is computed from its \
                 sources, not written to"
            );
        } else {
            let mut known: Vec<&str> = program.schemas.iter().map(|s| s.store.as_str()).collect();
            known.sort_unstable();
            eprintln!(
                "error: no store or registry named `{target}` in {}{}",
                path.display(),
                if known.is_empty() {
                    String::new()
                } else {
                    format!("; it declares {}", known.join(", "))
                }
            );
        }
        return ExitCode::FAILURE;
    };

    let payload = if data.as_os_str() == "-" {
        match std::io::read_to_string(std::io::stdin()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: cannot read standard input: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        match std::fs::read_to_string(data) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: cannot read {}: {e}", data.display());
                return ExitCode::FAILURE;
            }
        }
    };

    let rows = match decode_jsonl(schema, &payload) {
        Ok(rows) => rows,
        Err(e) => {
            let where_ = if data.as_os_str() == "-" {
                "<stdin>".to_string()
            } else {
                data.display().to_string()
            };
            eprintln!("error: {where_}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let Some(mut backend) = open_db(db_path) else {
        return ExitCode::FAILURE;
    };
    if db_path.as_os_str() == ":memory:" {
        eprintln!("note: using an in-memory database; pass --db <path> to persist");
    }
    // Every `domain` target must exist before its referent, and foreign keys
    // are enforced (ADR 0034 decision 5), so ensure the whole program's
    // tables rather than just this one.
    for s in &program.schemas {
        if let Err(e) = backend.ensure_store(s) {
            eprintln!("error: {} {}: {e}", s.kind.keyword(), s.store);
            return ExitCode::FAILURE;
        }
    }

    // One transaction: every record lands or none does.
    match backend.apply(&schema.shape(), &Delta::appending(rows)) {
        Ok(applied) => {
            let noun = if applied.inserted == 1 { "row" } else { "rows" };
            println!(
                "appended {} {noun} to {} {target}",
                applied.inserted,
                schema.kind.keyword()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Open the backing database, reporting a failure to stderr.
fn open_db(db_path: &Path) -> Option<SqliteBackend> {
    let opened = if db_path.as_os_str() == ":memory:" {
        SqliteBackend::open_in_memory()
    } else {
        SqliteBackend::open(db_path)
    };
    match opened {
        Ok(backend) => Some(backend),
        Err(e) => {
            eprintln!("error: cannot open database {}: {e}", db_path.display());
            None
        }
    }
}

/// The shared compiler frontend: lex, parse, and resolve `src`, reporting every
/// diagnostic to stderr.  Returns the resolved program on success, or `Err(())`
/// once diagnostics have been printed.  `check`, `run`, and `ingest` all build
/// on it.
fn frontend(path: &Path, src: &str) -> Result<mensura_types::ResolvedProgram, ()> {
    let tokens = match tokenize(src) {
        Ok(tokens) => tokens,
        Err(err) => {
            report(path, src, &err.message, err.span);
            return Err(());
        }
    };
    let program = match parse(&tokens) {
        Ok(program) => program,
        Err(err) => {
            report(path, src, &err.message, err.span);
            return Err(());
        }
    };
    mensura_types::resolve(&program).map_err(|errors| {
        for err in &errors {
            report(path, src, &err.message, err.span);
        }
    })
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
