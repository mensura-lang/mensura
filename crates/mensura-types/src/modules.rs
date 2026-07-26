//! Bundled module resolution (`docs/language/12-modules-and-imports.md`,
//! ADR 0027).
//!
//! A bare `import name` resolves against the modules that ship with the
//! toolchain, and only those (ADR 0027, Decision 6): no manifest, no
//! filesystem, no network.  A module is ordinary Mensura source embedded
//! at compile time, restricted to `let` items; it resolves to a
//! [`ModuleEnv`] of evaluated const bindings, memoized for the process
//! (the environment depends only on the intrinsics, never on the
//! importer).
//!
//! Spans carry no file identity yet, so a diagnostic from inside a bundled
//! module is reported as a message prefixed with the module name; the
//! importer attaches it to the `import` item's span.  Bundled modules are
//! compiled in this repository's CI, so such an error is effectively
//! internal.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use mensura_syntax::{Item, LetKind, parse, tokenize};

use crate::consts::{ConstDecl, ConstValue, eval_const_bindings};
use crate::model::ColumnType;
use crate::resolve::ResolveError;

/// A resolved module: its exported const bindings.  (Type-level exports,
/// i.e. dimension aliases, wait for module-qualified names in type
/// position; see `12-modules-and-imports.md`, "Deferred".)
#[derive(Debug)]
pub struct ModuleEnv {
    pub values: BTreeMap<String, ConstValue>,
}

/// The `si` standard library (ADR 0028), embedded at compile time.  The
/// oracle test in `stdlib_si` pins each binding's resolved dimension and
/// magnitude.
const SI_SOURCE: &str = include_str!("../stdlib/si.mensura");

/// Resolve a bundled module by name, memoized.  `None` means no bundled
/// module has that name (the importer's "unknown module" case); `Err` is
/// the module's own diagnostics, already prefixed with its name.
pub(crate) fn bundled(name: &str) -> Option<&'static Result<ModuleEnv, Vec<String>>> {
    static SI: OnceLock<Result<ModuleEnv, Vec<String>>> = OnceLock::new();
    match name {
        "si" => Some(SI.get_or_init(|| load("si", SI_SOURCE))),
        _ => None,
    }
}

/// Compile one bundled module's source into its environment.
fn load(name: &str, src: &str) -> Result<ModuleEnv, Vec<String>> {
    let prefixed = |msg: &str| format!("in module `{name}`: {msg}");
    let tokens = tokenize(src).map_err(|e| vec![prefixed(&e.message)])?;
    let program = parse(&tokens).map_err(|e| vec![prefixed(&e.message)])?;

    let mut decls = Vec::new();
    let mut errors = Vec::new();
    for item in &program.items {
        match item {
            Item::Let(l) => match &l.kind {
                LetKind::Value { ty, value } => decls.push(ConstDecl {
                    name: &l.name,
                    ty: ty.as_ref(),
                    value,
                }),
                LetKind::DimAlias { .. } => errors.push(prefixed(
                    "module type-level exports (dimension aliases) are not yet supported",
                )),
            },
            Item::Import(_) => errors.push(prefixed(
                "imports inside a bundled module are not yet supported",
            )),
            _ => errors.push(prefixed(
                "a module exports only `let` bindings; it cannot declare stores, views, \
                 units, shapes, or enums",
            )),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // No units, enums, or aliases exist inside a module, so an ascription
    // resolves against the built-in vocabulary only.
    let no_names = |ty: &mensura_syntax::TypeExpr| -> Result<ColumnType, ResolveError> {
        crate::resolve::resolve_type_builtin(ty)
    };
    let modules = BTreeMap::new();
    let (values, errs) = eval_const_bindings(&decls, &modules, &no_names);
    if errs.is_empty() {
        Ok(ModuleEnv { values })
    } else {
        Err(errs.iter().map(|e| prefixed(&e.message)).collect())
    }
}
