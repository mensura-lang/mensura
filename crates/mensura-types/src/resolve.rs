//! Name resolution: AST [`Program`] -> resolved [`Schema`]s.
//!
//! Resolution collects *all* diagnostics rather than failing on the first,
//! since stores and units are largely independent.  It enforces the current
//! "basic only" scope by rejecting compound units, compound fields, and
//! `domain` blocks with clear "not yet supported" errors.

use std::collections::{HashMap, HashSet};

use mensura_syntax::{Item, Program, Span, StoreDecl, TypeExpr, UnitDecl};

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

    // Pass 1: collect unit and store names (separate namespaces).
    let mut units: HashMap<&str, &UnitDecl> = HashMap::new();
    let mut store_names: HashSet<&str> = HashSet::new();
    let mut stores: Vec<&StoreDecl> = Vec::new();

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
        }
    }

    // Pass 2: resolve each store independently.
    let mut schemas = Vec::new();
    for s in &stores {
        match resolve_store(s, &units) {
            Ok(schema) => schemas.push(schema),
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
