//! Name resolution: AST [`Program`] -> resolved [`Schema`]s.
//!
//! Resolution collects *all* diagnostics rather than failing on the first,
//! since stores and units are largely independent.  It enforces the current
//! "basic only" scope by rejecting compound units, compound fields, and
//! `domain` blocks with clear "not yet supported" errors.
//!
//! Shapes (`docs/language/03-shapes.md`) are validated here too: each store
//! that claims conformance with a `:` clause is checked against the shape's
//! structure.  Shapes carry no storage, so they produce no [`Schema`]; only
//! stores do.

use std::collections::{HashMap, HashSet};

use mensura_syntax::{Item, Program, ShapeDecl, ShapeRef, Span, StoreDecl, TypeExpr, UnitDecl};

use crate::model::{Column, ColumnRole, ColumnType, Schema};

/// A resolution failure, located by a source span.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolveError {
    pub message: String,
    pub span: Span,
}

impl ResolveError {
    fn new(message: impl Into<String>, span: Span) -> Self {
        ResolveError {
            message: message.into(),
            span,
        }
    }
}

/// Resolve a parsed program into one [`Schema`] per store, or every error
/// found along the way.
pub fn resolve(program: &Program) -> Result<Vec<Schema>, Vec<ResolveError>> {
    let mut errors = Vec::new();

    // Pass 1: collect unit, store, and shape names (separate namespaces).
    let mut units: HashMap<&str, &UnitDecl> = HashMap::new();
    let mut store_names: HashSet<&str> = HashSet::new();
    let mut stores: Vec<&StoreDecl> = Vec::new();
    let mut shapes: HashMap<&str, &ShapeDecl> = HashMap::new();

    for item in &program.items {
        match item {
            Item::Unit(u) => {
                if units.insert(&u.name.name, u).is_some() {
                    errors.push(ResolveError::new(
                        format!("duplicate unit `{}`", u.name.name),
                        u.name.span,
                    ));
                }
            }
            Item::Store(s) => {
                if !store_names.insert(&s.name.name) {
                    errors.push(ResolveError::new(
                        format!("duplicate store `{}`", s.name.name),
                        s.name.span,
                    ));
                }
                stores.push(s);
            }
            Item::Shape(sh) => {
                if shapes.insert(&sh.name.name, sh).is_some() {
                    errors.push(ResolveError::new(
                        format!("duplicate shape `{}`", sh.name.name),
                        sh.name.span,
                    ));
                }
            }
        }
    }

    // Pass 2: resolve each shape's structure, for conformance checks below.
    let mut resolved_shapes: HashMap<&str, ResolvedShape> = HashMap::new();
    for (name, sh) in &shapes {
        match resolve_shape(sh, &units) {
            Ok(rs) => {
                resolved_shapes.insert(name, rs);
            }
            Err(mut errs) => errors.append(&mut errs),
        }
    }

    // Pass 3: resolve each store, then check the shapes it claims.
    let mut schemas = Vec::new();
    for s in &stores {
        match resolve_store(s, &units) {
            Ok(schema) => {
                check_conformance(s, &schema, &shapes, &resolved_shapes, &units, &mut errors);
                schemas.push(schema);
            }
            Err(mut errs) => errors.append(&mut errs),
        }
    }

    if errors.is_empty() {
        Ok(schemas)
    } else {
        Err(errors)
    }
}

fn resolve_store(
    s: &StoreDecl,
    units: &HashMap<&str, &UnitDecl>,
) -> Result<Schema, Vec<ResolveError>> {
    let Some(unit) = units.get(s.unit.name.as_str()) else {
        return Err(vec![ResolveError::new(
            format!("unknown unit `{}`", s.unit.name),
            s.unit.span,
        )]);
    };

    let mut errors = Vec::new();

    if let Some(first) = s.domain.first() {
        errors.push(ResolveError::new(
            "compound stores are not yet supported (`domain` block)",
            first.span,
        ));
    }

    // Columns in storage order: index fields, then const, then var.  Compound
    // units surface here: an index field whose type references another unit is
    // rejected by `resolve_type`.
    let mut columns = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for f in &unit.fields {
        add_column(
            &mut columns,
            &mut seen,
            &mut errors,
            &f.name.name,
            f.name.span,
            &f.ty,
            ColumnRole::Index,
            units,
        );
    }
    for f in &s.consts {
        add_column(
            &mut columns,
            &mut seen,
            &mut errors,
            &f.name.name,
            f.name.span,
            &f.ty,
            ColumnRole::Const,
            units,
        );
    }
    for f in &s.vars {
        add_column(
            &mut columns,
            &mut seen,
            &mut errors,
            &f.name.name,
            f.name.span,
            &f.ty,
            ColumnRole::Var,
            units,
        );
    }

    if errors.is_empty() {
        Ok(Schema {
            store: s.name.name.clone(),
            unit: s.unit.name.clone(),
            columns,
            span: s.span,
        })
    } else {
        Err(errors)
    }
}

/// How a shape constrains the unit of a conforming store.
enum ShapeUnit {
    /// No `unit { ... }` clause: any unit conforms.
    Agnostic,
    /// `unit { Person }`: the store must tabulate this concrete unit.
    Concrete(String),
    /// `unit { U }` where `U` is a `Unit` parameter: the store must tabulate
    /// the unit supplied for that parameter at the use site.
    Param(String),
}

/// A shape resolved for conformance: its unit parameters in order, how it
/// constrains the unit, and its `const`/`var` attributes as columns.  Unlike
/// a [`Schema`] it carries no index columns (those come from the unit) and no
/// storage; it exists only to check conformance.
struct ResolvedShape {
    unit_params: Vec<String>,
    unit: ShapeUnit,
    columns: Vec<Column>,
}

fn resolve_shape(
    sh: &ShapeDecl,
    units: &HashMap<&str, &UnitDecl>,
) -> Result<ResolvedShape, Vec<ResolveError>> {
    let mut errors = Vec::new();

    // Parameters.  This slice supports `Unit` parameters only; value
    // parameters (`string`, ...) are parsed but deferred.
    let mut unit_params: Vec<String> = Vec::new();
    let mut seen_params: HashSet<&str> = HashSet::new();
    for p in &sh.params {
        if !seen_params.insert(p.name.name.as_str()) {
            errors.push(ResolveError::new(
                format!("duplicate parameter `{}`", p.name.name),
                p.name.span,
            ));
        }
        match p.kind.name.as_str() {
            "Unit" => unit_params.push(p.name.name.clone()),
            "string" | "number" | "bool" | "date" => errors.push(ResolveError::new(
                format!(
                    "value parameters are not yet supported (parameter `{}: {}`)",
                    p.name.name, p.kind.name
                ),
                p.kind.span,
            )),
            other => errors.push(ResolveError::new(
                format!("unknown parameter kind `{other}`"),
                p.kind.span,
            )),
        }
    }

    // Unit clause: optional, and may name a `Unit` parameter or a real unit.
    let unit = match &sh.unit {
        None => ShapeUnit::Agnostic,
        Some(u) if unit_params.iter().any(|p| p == &u.name) => ShapeUnit::Param(u.name.clone()),
        Some(u) if units.contains_key(u.name.as_str()) => ShapeUnit::Concrete(u.name.clone()),
        Some(u) => {
            errors.push(ResolveError::new(
                format!("unknown unit `{}`", u.name),
                u.span,
            ));
            ShapeUnit::Agnostic
        }
    };

    let mut columns = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for f in &sh.consts {
        add_column(
            &mut columns,
            &mut seen,
            &mut errors,
            &f.name.name,
            f.name.span,
            &f.ty,
            ColumnRole::Const,
            units,
        );
    }
    for f in &sh.vars {
        add_column(
            &mut columns,
            &mut seen,
            &mut errors,
            &f.name.name,
            f.name.span,
            &f.ty,
            ColumnRole::Var,
            units,
        );
    }

    if errors.is_empty() {
        Ok(ResolvedShape {
            unit_params,
            unit,
            columns,
        })
    } else {
        Err(errors)
    }
}

/// Check every shape a store claims with its `:` clause.  After binding the
/// shape's `Unit` parameters to the arguments supplied, a store conforms when
/// it tabulates the required unit (if any) and carries every shape attribute
/// with the same name, role (`const`/`var`), and type; extra store attributes
/// are fine.  Each failure is a separate diagnostic pointing at the claim.
fn check_conformance(
    s: &StoreDecl,
    schema: &Schema,
    shapes: &HashMap<&str, &ShapeDecl>,
    resolved: &HashMap<&str, ResolvedShape>,
    units: &HashMap<&str, &UnitDecl>,
    errors: &mut Vec<ResolveError>,
) {
    for claim in &s.conforms {
        let name = claim.name.name.as_str();
        let Some(shape) = resolved.get(name) else {
            // A claimed shape that exists but failed to resolve already
            // reported its own errors; only an entirely unknown name is new.
            if !shapes.contains_key(name) {
                errors.push(ResolveError::new(
                    format!("unknown shape `{name}`"),
                    claim.span,
                ));
            }
            continue;
        };

        if claim.args.len() != shape.unit_params.len() {
            errors.push(ResolveError::new(
                format!(
                    "store `{}` claims `{}` with {} argument(s), but the shape declares {}",
                    s.name.name,
                    shape_ref_label(claim),
                    claim.args.len(),
                    shape.unit_params.len()
                ),
                claim.span,
            ));
            continue;
        }

        // Bind each `Unit` parameter to its argument; arguments must be units.
        let mut bindings: HashMap<&str, &str> = HashMap::new();
        let mut args_ok = true;
        for (param, arg) in shape.unit_params.iter().zip(&claim.args) {
            if !units.contains_key(arg.name.as_str()) {
                errors.push(ResolveError::new(
                    format!("unknown unit `{}`", arg.name),
                    arg.span,
                ));
                args_ok = false;
            }
            bindings.insert(param.as_str(), arg.name.as_str());
        }
        if !args_ok {
            continue;
        }

        // Unit check, unless the shape is unit-agnostic.  `required` is set
        // only when the shape pins a unit and the store disagrees.
        let required = match &shape.unit {
            ShapeUnit::Agnostic => None,
            ShapeUnit::Concrete(u) => Some(u.as_str()),
            ShapeUnit::Param(p) => Some(bindings[p.as_str()]),
        }
        .filter(|req| schema.unit != *req);
        if let Some(req) = required {
            errors.push(ResolveError::new(
                format!(
                    "store `{}` claims `{}`, which tabulates `{}`, but the store tabulates `{}`",
                    s.name.name,
                    shape_ref_label(claim),
                    req,
                    schema.unit
                ),
                claim.span,
            ));
            continue;
        }

        for want in &shape.columns {
            match schema.columns.iter().find(|c| c.name == want.name) {
                None => errors.push(ResolveError::new(
                    format!(
                        "store `{}` claims `{}` but is missing attribute `{}`",
                        s.name.name,
                        shape_ref_label(claim),
                        want.name
                    ),
                    claim.span,
                )),
                Some(have) if have.role != want.role => errors.push(ResolveError::new(
                    format!(
                        "store `{}` claims `{}`: attribute `{}` is `{}` in the shape but `{}` in the store",
                        s.name.name,
                        shape_ref_label(claim),
                        want.name,
                        role_word(want.role),
                        role_word(have.role)
                    ),
                    claim.span,
                )),
                Some(have) if have.ty != want.ty => errors.push(ResolveError::new(
                    format!(
                        "store `{}` claims `{}`: attribute `{}` has type `{}` in the shape but `{}` in the store",
                        s.name.name,
                        shape_ref_label(claim),
                        want.name,
                        type_name(&want.ty),
                        type_name(&have.ty)
                    ),
                    claim.span,
                )),
                Some(_) => {}
            }
        }
    }
}

/// Render a conformance claim for diagnostics: `Tabular(Person)` or, with no
/// arguments, just `PersonRecord`.
fn shape_ref_label(r: &ShapeRef) -> String {
    if r.args.is_empty() {
        r.name.name.clone()
    } else {
        let args = r
            .args
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}({})", r.name.name, args)
    }
}

fn role_word(role: ColumnRole) -> &'static str {
    match role {
        ColumnRole::Index => "index",
        ColumnRole::Const => "const",
        ColumnRole::Var => "var",
    }
}

fn type_name(ty: &ColumnType) -> String {
    match ty {
        ColumnType::String => "string".into(),
        ColumnType::Number => "number".into(),
        ColumnType::Bool => "bool".into(),
        ColumnType::Date => "date".into(),
        ColumnType::Enum(variants) => {
            let inner = variants
                .iter()
                .map(|v| format!("\"{v}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!("enum({inner})")
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_column(
    columns: &mut Vec<Column>,
    seen: &mut HashSet<String>,
    errors: &mut Vec<ResolveError>,
    name: &str,
    name_span: Span,
    ty: &TypeExpr,
    role: ColumnRole,
    units: &HashMap<&str, &UnitDecl>,
) {
    if !seen.insert(name.to_string()) {
        errors.push(ResolveError::new(
            format!("duplicate column `{name}`"),
            name_span,
        ));
        return;
    }
    match resolve_type(ty, units) {
        Ok(ct) => columns.push(Column {
            name: name.to_string(),
            ty: ct,
            role,
            span: name_span,
        }),
        Err(e) => errors.push(e),
    }
}

fn resolve_type(
    ty: &TypeExpr,
    units: &HashMap<&str, &UnitDecl>,
) -> Result<ColumnType, ResolveError> {
    match ty {
        TypeExpr::Named(id) => match id.name.as_str() {
            "string" => Ok(ColumnType::String),
            "number" => Ok(ColumnType::Number),
            "bool" => Ok(ColumnType::Bool),
            "date" => Ok(ColumnType::Date),
            other if units.contains_key(other) => Err(ResolveError::new(
                format!("compound fields are not yet supported (references unit `{other}`)"),
                id.span,
            )),
            other => Err(ResolveError::new(
                format!("unknown type `{other}`"),
                id.span,
            )),
        },
        TypeExpr::Enum { variants, .. } => {
            let mut seen = HashSet::new();
            for v in variants {
                if !seen.insert(v.value.as_str()) {
                    return Err(ResolveError::new(
                        format!("duplicate enum variant `{}`", v.value),
                        v.span,
                    ));
                }
            }
            Ok(ColumnType::Enum(
                variants.iter().map(|v| v.value.clone()).collect(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_str(src: &str) -> Result<Vec<Schema>, Vec<ResolveError>> {
        let tokens = mensura_syntax::tokenize(src).expect("should lex");
        let program = mensura_syntax::parse(&tokens).expect("should parse");
        resolve(&program)
    }

    fn errors(src: &str) -> Vec<ResolveError> {
        resolve_str(src).expect_err("should fail to resolve")
    }

    #[test]
    fn resolves_basic_example() {
        let src = r#"
            unit Person { id: string }
            unit Department { code: string }

            store Departments {
              unit { Department }
              const { name: string }
            }
            store Persons {
              unit { Person }
              const { birthdate: date }
              var   { last_name: string }
            }
            store Students {
              unit { Person }
              const { admission: date }
              var   { status: enum("active", "inactive") }
            }
        "#;
        let schemas = resolve_str(src).expect("should resolve");
        assert_eq!(schemas.len(), 3);

        let students = schemas.iter().find(|s| s.store == "Students").unwrap();
        assert_eq!(students.unit, "Person");
        let cols: Vec<(&str, ColumnRole, &ColumnType)> = students
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.role, &c.ty))
            .collect();
        assert_eq!(
            cols,
            vec![
                ("id", ColumnRole::Index, &ColumnType::String),
                ("admission", ColumnRole::Const, &ColumnType::Date),
                (
                    "status",
                    ColumnRole::Var,
                    &ColumnType::Enum(vec!["active".into(), "inactive".into()]),
                ),
            ]
        );
    }

    #[test]
    fn unknown_unit_is_rejected() {
        let errs = errors("store S { unit { Ghost } const { a: string } }");
        assert!(errs[0].message.contains("unknown unit `Ghost`"));
    }

    #[test]
    fn compound_unit_field_is_rejected() {
        let src = r#"
            unit Department { code: string }
            unit Course { department: Department }
            store Courses { unit { Course } }
        "#;
        let errs = errors(src);
        assert!(
            errs[0]
                .message
                .contains("compound fields are not yet supported")
        );
    }

    #[test]
    fn domain_block_is_rejected() {
        let src = r#"
            unit Person { id: string }
            store S {
              unit { Person }
              domain { x: Other }
            }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| e.message.contains("`domain` block")));
    }

    #[test]
    fn duplicate_column_is_rejected() {
        // `id` is both the index field and a const attribute.
        let src = r#"
            unit Person { id: string }
            store S { unit { Person } const { id: string } }
        "#;
        let errs = errors(src);
        assert!(errs[0].message.contains("duplicate column `id`"));
    }

    #[test]
    fn unknown_type_is_rejected() {
        let errs = errors("unit U { x: widget } store S { unit { U } }");
        assert!(errs[0].message.contains("unknown type `widget`"));
    }

    #[test]
    fn duplicate_enum_variant_is_rejected() {
        let src = r#"unit U { id: string } store S { unit { U } var { c: enum("a", "a") } }"#;
        let errs = errors(src);
        assert!(errs[0].message.contains("duplicate enum variant `a`"));
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let errs = errors("unit U { a: string } unit U { b: string }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("duplicate unit `U`"))
        );
    }

    #[test]
    fn independent_errors_are_all_reported() {
        // An unknown unit in one store and an unknown type in another: two
        // distinct diagnostics, not just the first.
        let src = r#"
            unit U { id: string }
            store A { unit { Ghost } }
            store B { unit { U } const { x: widget } }
        "#;
        let errs = errors(src);
        assert_eq!(errs.len(), 2);
    }

    #[test]
    fn conforming_store_resolves() {
        let src = r#"
            unit Person { id: string }
            shape PersonRecord {
              unit { Person }
              const { admission: date }
            }
            store Students : PersonRecord {
              unit { Person }
              const { admission: date }
              var   { status: enum("active", "inactive") }
            }
        "#;
        // The store carries an extra attribute (`status`); conformance only
        // requires the shape's attributes to be present.
        let schemas = resolve_str(src).expect("should resolve");
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].store, "Students");
    }

    #[test]
    fn marker_shape_conforms_on_unit_alone() {
        let src = r#"
            unit Person { id: string }
            shape Anything { unit { Person } }
            store Persons : Anything { unit { Person } const { birthdate: date } }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 1);
    }

    #[test]
    fn unknown_shape_is_rejected() {
        let src = r#"
            unit Person { id: string }
            store Students : Ghost { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("unknown shape `Ghost`"))
        );
    }

    #[test]
    fn missing_shape_attribute_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape PersonRecord { unit { Person } const { admission: date } }
            store Students : PersonRecord { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("missing attribute `admission`"))
        );
    }

    #[test]
    fn wrong_unit_is_rejected() {
        let src = r#"
            unit Person { id: string }
            unit Course { code: string }
            shape PersonRecord { unit { Person } const { admission: date } }
            store Courses : PersonRecord { unit { Course } const { admission: date } }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| e.message.contains("tabulates `Person`")
            && e.message.contains("tabulates `Course`")));
    }

    #[test]
    fn wrong_attribute_type_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape PersonRecord { unit { Person } const { admission: date } }
            store Students : PersonRecord { unit { Person } const { admission: string } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("type `date` in the shape but `string`"))
        );
    }

    #[test]
    fn wrong_attribute_role_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape PersonRecord { unit { Person } const { admission: date } }
            store Students : PersonRecord { unit { Person } var { admission: date } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("`const` in the shape but `var`"))
        );
    }

    #[test]
    fn duplicate_shape_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape S { unit { Person } }
            shape S { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("duplicate shape `S`"))
        );
    }

    #[test]
    fn unit_parameter_shape_conforms() {
        let src = r#"
            unit Person { id: string }
            shape Tabular(U: Unit) { unit { U } }
            store Persons : Tabular(Person) { unit { Person } const { birthdate: date } }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 1);
    }

    #[test]
    fn unit_agnostic_shape_conforms_to_any_unit() {
        // `Named` has no unit clause, so a `Department` store conforms purely
        // by carrying the required `name` attribute.
        let src = r#"
            unit Department { code: string }
            shape Named { const { name: string } }
            store Departments : Named { unit { Department } const { name: string } }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 1);
    }

    #[test]
    fn wrong_unit_argument_is_rejected() {
        let src = r#"
            unit Person { id: string }
            unit Course { code: string }
            shape Tabular(U: Unit) { unit { U } }
            store S : Tabular(Course) { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| e.message.contains("tabulates `Course`")
            && e.message.contains("tabulates `Person`")));
    }

    #[test]
    fn arity_mismatch_is_rejected() {
        // `Tabular` declares one parameter; claiming it with none is an error.
        let src = r#"
            unit Person { id: string }
            shape Tabular(U: Unit) { unit { U } }
            store S : Tabular { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("with 0 argument(s)")
                    && e.message.contains("declares 1"))
        );
    }

    #[test]
    fn extra_argument_on_plain_shape_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape PersonRecord { unit { Person } }
            store S : PersonRecord(Person) { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("with 1 argument(s)")
                    && e.message.contains("declares 0"))
        );
    }

    #[test]
    fn unknown_unit_argument_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape Tabular(U: Unit) { unit { U } }
            store S : Tabular(Ghost) { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("unknown unit `Ghost`"))
        );
    }

    #[test]
    fn duplicate_parameter_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape D(U: Unit, U: Unit) { unit { U } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("duplicate parameter `U`"))
        );
    }

    #[test]
    fn value_parameter_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape NumericCol(col: string) { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("value parameters are not yet supported"))
        );
    }

    #[test]
    fn committed_example_resolves() {
        // The worked example under docs/examples must keep parsing and
        // resolving so it cannot rot.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/examples/college-stores.mensura");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let schemas = resolve_str(&src).expect("example should resolve");
        assert_eq!(schemas.len(), 3);
    }
}
