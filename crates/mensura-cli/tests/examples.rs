//! Worked examples gate: every `.mensura` file under `docs/examples/` must lex,
//! parse, and resolve cleanly (the same frontend `mensura check` runs).  These
//! are the documentation's living examples, so a language change that breaks one
//! fails CI here rather than silently rotting the docs.

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

/// Collect the `.mensura` examples under `docs/examples/` (repo root, two levels
/// up from this crate), sorted for a stable order.
fn examples() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("a readable dir entry").path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "mensura"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no examples in {}", dir.display());
    files
}

#[test]
fn all_examples_type_check() {
    for path in examples() {
        let src = std::fs::read_to_string(&path).expect("readable example");
        assert!(
            accepts(&src),
            "docs/examples/{} no longer type-checks",
            path.file_name().unwrap().to_string_lossy()
        );
    }
}
