//! Loading the program at startup: lex, parse, and resolve, refusing to
//! start on any diagnostic exactly as `mensura check` does (ADR 0045
//! decision 1).

use std::path::Path;

use mensura_syntax::{Span, parse, tokenize};
use mensura_types::ResolvedProgram;

/// Resolve the program at `path`, or return every diagnostic in the
/// `error: ...` / `--> file:line:col` form the CLI prints.
pub fn load_program(path: &Path) -> Result<ResolvedProgram, Vec<String>> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| vec![format!("error: cannot read {}: {e}", path.display())])?;
    resolve_source(path, &src)
}

/// Resolve program text, attributing diagnostics to `path`.
pub fn resolve_source(path: &Path, src: &str) -> Result<ResolvedProgram, Vec<String>> {
    let report = |message: &str, span: Span| {
        let (line, col) = line_col(src, span.start);
        format!("error: {message}\n  --> {}:{line}:{col}", path.display())
    };
    let tokens = tokenize(src).map_err(|e| vec![report(&e.message, e.span)])?;
    let program = parse(&tokens).map_err(|e| vec![report(&e.message, e.span)])?;
    mensura_types::resolve(&program)
        .map_err(|errors| errors.iter().map(|e| report(&e.message, e.span)).collect())
}

/// A byte offset as a 1-based (line, column) pair, the column counted in
/// Unicode scalar values.
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
