//! The must-accept / must-reject corpus (`ROADMAP.md` M0).
//!
//! Every `.mensura` file under `tests/corpus/accept/` must lex, parse, and
//! resolve cleanly; every file under `tests/corpus/reject/` must fail at one of
//! those stages.  This is the same frontend `mensura check` runs, so the corpus
//! is the language's classification gate: it grows feature by feature, and a
//! change that misclassifies any case fails CI.
//!
//! Each case carries a top-of-file comment saying why it is accepted or, for a
//! rejection, which stage rejects it and on what grounds.

use std::path::{Path, PathBuf};

use mensura_syntax::{parse, tokenize};

/// Run the frontend (lex -> parse -> resolve) and report whether it accepts.
fn accepts(src: &str) -> bool {
    let Ok(tokens) = tokenize(src) else {
        return false;
    };
    let Ok(program) = parse(&tokens) else {
        return false;
    };
    mensura_types::resolve(&program).is_ok()
}

/// Collect the `.mensura` cases under `tests/corpus/<kind>`, sorted for a
/// stable order.
fn cases(kind: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus")
        .join(kind);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("a readable dir entry").path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "mensura"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no corpus cases in {}", dir.display());
    files
}

#[test]
fn accept_corpus_all_pass() {
    for path in cases("accept") {
        let src = std::fs::read_to_string(&path).expect("readable case");
        assert!(
            accepts(&src),
            "expected ACCEPT but the frontend rejected {}",
            path.display()
        );
    }
}

#[test]
fn reject_corpus_all_fail() {
    for path in cases("reject") {
        let src = std::fs::read_to_string(&path).expect("readable case");
        assert!(
            !accepts(&src),
            "expected REJECT but the frontend accepted {}",
            path.display()
        );
    }
}
