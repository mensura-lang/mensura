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

#[cfg(test)]
mod tests {
    use super::*;

    /// The `si` oracle (ADR 0028, Decision 2 as revised): each binding's
    /// expected dimension exponent vector and base-unit magnitude, checked
    /// bidirectionally against the resolved module.  This pins the
    /// *checker's output* for the shipped units, which generation could
    /// not; edit this table together with `stdlib/si.mensura`.
    ///
    /// Axis order (`crate::units::BASE_DIMENSIONS`):
    /// time, length, mass, current, temperature, amount, luminosity.
    /// Magnitudes are written as the same f64 expressions the module
    /// evaluates, so equality is exact.
    fn si_oracle() -> Vec<(&'static str, [i32; 7], f64)> {
        const TIME: [i32; 7] = [1, 0, 0, 0, 0, 0, 0];
        const LENGTH: [i32; 7] = [0, 1, 0, 0, 0, 0, 0];
        const MASS: [i32; 7] = [0, 0, 1, 0, 0, 0, 0];
        const FREQ: [i32; 7] = [-1, 0, 0, 0, 0, 0, 0];
        const FORCE: [i32; 7] = [-2, 1, 1, 0, 0, 0, 0];
        const PRESSURE: [i32; 7] = [-2, -1, 1, 0, 0, 0, 0];
        const ENERGY: [i32; 7] = [-2, 2, 1, 0, 0, 0, 0];
        const POWER: [i32; 7] = [-3, 2, 1, 0, 0, 0, 0];
        const CHARGE: [i32; 7] = [1, 0, 0, 1, 0, 0, 0];
        const VOLTAGE: [i32; 7] = [-3, 2, 1, -1, 0, 0, 0];
        let gram = 0.001;
        vec![
            // Base-unit symbols.
            ("s", TIME, 1.0),
            ("m", LENGTH, 1.0),
            ("kg", MASS, 1.0),
            ("mol", [0, 0, 0, 0, 0, 1, 0], 1.0),
            ("cd", [0, 0, 0, 0, 0, 0, 1], 1.0),
            // The gram.
            ("gram", MASS, gram),
            ("g", MASS, gram),
            // Time units.
            ("minute", TIME, 60.0),
            ("hour", TIME, 3600.0),
            ("day", TIME, 86400.0),
            ("h", TIME, 3600.0),
            // Named derived units.
            ("hertz", FREQ, 1.0),
            ("newton", FORCE, 1.0),
            ("pascal", PRESSURE, 1.0),
            ("joule", ENERGY, 1.0),
            ("watt", POWER, 1.0),
            ("coulomb", CHARGE, 1.0),
            ("volt", VOLTAGE, 1.0),
            // Prefixed seconds.
            ("nanosecond", TIME, 0.000000001),
            ("microsecond", TIME, 0.000001),
            ("millisecond", TIME, 0.001),
            ("ns", TIME, 0.000000001),
            ("us", TIME, 0.000001),
            ("ms", TIME, 0.001),
            // Prefixed meters.
            ("nanometer", LENGTH, 0.000000001),
            ("micrometer", LENGTH, 0.000001),
            ("millimeter", LENGTH, 0.001),
            ("centimeter", LENGTH, 0.01),
            ("kilometer", LENGTH, 1000.0),
            ("nm", LENGTH, 0.000000001),
            ("um", LENGTH, 0.000001),
            ("mm", LENGTH, 0.001),
            ("cm", LENGTH, 0.01),
            ("km", LENGTH, 1000.0),
            // Prefixed grams.
            ("nanogram", MASS, 0.000000001 * gram),
            ("microgram", MASS, 0.000001 * gram),
            ("milligram", MASS, 0.001 * gram),
            ("ng", MASS, 0.000000001 * gram),
            ("ug", MASS, 0.000001 * gram),
            ("mg", MASS, 0.001 * gram),
            // Conventional prefixed derived units.
            ("kilopascal", PRESSURE, 1000.0),
            ("megapascal", PRESSURE, 1000000.0),
            ("kilojoule", ENERGY, 1000.0),
            ("kilowatt", POWER, 1000.0),
            ("kilohertz", FREQ, 1000.0),
            ("megahertz", FREQ, 1000000.0),
            ("gigahertz", FREQ, 1000000000.0),
        ]
    }

    #[test]
    fn si_matches_its_oracle() {
        let env = bundled("si")
            .expect("si is bundled")
            .as_ref()
            .expect("si resolves cleanly");
        let oracle = si_oracle();
        // Every oracle row is a binding with the expected dimension and
        // exact base-unit magnitude.
        for (name, exps, magnitude) in &oracle {
            let value = env
                .values
                .get(*name)
                .unwrap_or_else(|| panic!("si has no binding `{name}`: stale oracle row"));
            let ConstValue::Real { magnitude: m, dim } = value else {
                panic!("`{name}` is not a dimensioned real");
            };
            assert_eq!(
                dim.exponents(),
                *exps,
                "`{name}` has dimension `{dim}`, not the oracle's"
            );
            assert_eq!(m, magnitude, "`{name}` has magnitude {m}, not {magnitude}");
        }
        // And every binding has an oracle row (no unlisted binding).
        for name in env.values.keys() {
            assert!(
                oracle.iter().any(|(n, ..)| n == name),
                "binding `{name}` has no oracle row: add it to `si_oracle`"
            );
        }
    }

    #[test]
    fn unknown_bundled_module_is_none() {
        assert!(bundled("geo").is_none());
    }
}
