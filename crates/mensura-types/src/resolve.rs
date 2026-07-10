//! Name resolution: AST [`Program`] -> a resolved [`ResolvedProgram`].
//!
//! Resolution collects *all* diagnostics rather than failing on the first,
//! since stores and units are largely independent.  It enforces the current
//! "basic only" scope by rejecting compound units, compound fields, and
//! `domain` blocks with clear "not yet supported" errors.
//!
//! Shapes (`docs/language/03-shapes.md`) are validated here too: each store or
//! view that claims conformance with a `:` clause is checked against the shape's
//! structure.  A store is checked against its declared columns; a view against
//! its computed output content, ignoring cardinality
//! (`docs/language/10-views.md`).  Shapes carry no storage, so they produce no
//! [`Schema`]; a store yields a [`Schema`] and a view a [`ViewPlan`]
//! (`docs/toolkit/04-processing-layer.md`).

use std::collections::{BTreeSet, HashMap, HashSet};

use mensura_syntax::{
    Block, EnumDecl, Expr, ExprKind, Field, Item, NameSeg, NameTemplate, Program, ShapeArg,
    ShapeDecl, ShapeRef, Span, Stmt, StoreDecl, TypeExpr, UnitDecl, ViewDecl, is_identifier,
};

use crate::model::{Column, ColumnRole, ColumnType, ResolvedProgram, Schema, ViewPlan};
use crate::pipe_check::{Sources, type_view};
use crate::table::TableType;

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

/// The casing convention a declared name must follow
/// (`docs/language/05-naming-and-casing.md`).
#[derive(Clone, Copy)]
enum Case {
    /// Types: `unit`, `shape`, and `Unit`-kind shape parameters.
    Pascal,
    /// Terms: `store` names, attributes, and `string`-kind parameters.
    Snake,
}

/// True if `name` has at least one character with a case distinction.  A
/// caseless identifier (for example a CJK name such as `温度`) is exempt from
/// the convention, so the check returns early for it.
fn has_cased(name: &str) -> bool {
    name.chars().any(|c| c.is_uppercase() || c.is_lowercase())
}

/// snake_case: no uppercase character (every character is lowercase or
/// caseless), with `_` allowed as a separator.
fn is_snake_case(name: &str) -> bool {
    !name.chars().any(char::is_uppercase)
}

/// PascalCase: the first cased character is uppercase, and there is no `_`.
fn is_pascal_case(name: &str) -> bool {
    if name.contains('_') {
        return false;
    }
    match name.chars().find(|c| c.is_uppercase() || c.is_lowercase()) {
        Some(c) => c.is_uppercase(),
        None => false,
    }
}

/// Check a declared name against the casing convention, recording a
/// diagnostic when it is violated.  Caseless names are exempt.  `what` names
/// the construct for the message (e.g. "store", "unit", "attribute").
fn check_case(name: &str, span: Span, case: Case, what: &str, errors: &mut Vec<ResolveError>) {
    if !has_cased(name) {
        return;
    }
    let ok = match case {
        Case::Pascal => is_pascal_case(name),
        Case::Snake => is_snake_case(name),
    };
    if ok {
        return;
    }
    let (style, hint) = match case {
        Case::Pascal => (
            "PascalCase",
            "start with an uppercase letter and use no underscores",
        ),
        Case::Snake => ("snake_case", "use lowercase words separated by `_`"),
    };
    errors.push(ResolveError::new(
        format!("{what} `{name}` must be {style}: {hint}"),
        span,
    ));
}

/// Resolve a parsed program into one [`Schema`] per store and one
/// [`ViewPlan`] per view, or every error found along the way.
pub fn resolve(program: &Program) -> Result<ResolvedProgram, Vec<ResolveError>> {
    let mut errors = Vec::new();

    // Pass 1: collect unit, store, shape, and enum names (separate namespaces).
    let mut units: HashMap<&str, &UnitDecl> = HashMap::new();
    let mut store_names: HashSet<&str> = HashSet::new();
    let mut stores: Vec<&StoreDecl> = Vec::new();
    let mut shapes: HashMap<&str, &ShapeDecl> = HashMap::new();
    let mut enums: HashMap<&str, &EnumDecl> = HashMap::new();
    let mut views: Vec<&ViewDecl> = Vec::new();

    for item in &program.items {
        match item {
            Item::Unit(u) => {
                check_case(&u.name.name, u.name.span, Case::Pascal, "unit", &mut errors);
                // Index field names are checked here, once per unit, rather
                // than in `add_column` (which runs once per store that uses
                // the unit) to avoid duplicate diagnostics.
                for f in &u.fields {
                    if let Some(lit) = f.name.as_literal() {
                        check_case(lit, f.name.span, Case::Snake, "attribute", &mut errors);
                    }
                }
                if units.insert(&u.name.name, u).is_some() {
                    errors.push(ResolveError::new(
                        format!("duplicate unit `{}`", u.name.name),
                        u.name.span,
                    ));
                }
            }
            Item::Store(s) => {
                check_case(&s.name.name, s.name.span, Case::Snake, "store", &mut errors);
                if !store_names.insert(&s.name.name) {
                    errors.push(ResolveError::new(
                        format!("duplicate store `{}`", s.name.name),
                        s.name.span,
                    ));
                }
                stores.push(s);
            }
            Item::Shape(sh) => {
                check_case(
                    &sh.name.name,
                    sh.name.span,
                    Case::Pascal,
                    "shape",
                    &mut errors,
                );
                if shapes.insert(&sh.name.name, sh).is_some() {
                    errors.push(ResolveError::new(
                        format!("duplicate shape `{}`", sh.name.name),
                        sh.name.span,
                    ));
                }
            }
            Item::Enum(e) => {
                check_case(&e.name.name, e.name.span, Case::Pascal, "enum", &mut errors);
                let mut seen = HashSet::new();
                for v in &e.variants {
                    if !seen.insert(v.value.as_str()) {
                        errors.push(ResolveError::new(
                            format!("duplicate enum variant `{}`", v.value),
                            v.span,
                        ));
                    }
                }
                if enums.insert(&e.name.name, e).is_some() {
                    errors.push(ResolveError::new(
                        format!("duplicate enum `{}`", e.name.name),
                        e.name.span,
                    ));
                }
            }
            Item::View(v) => {
                check_case(&v.name.name, v.name.span, Case::Snake, "view", &mut errors);
                views.push(v);
            }
        }
    }

    // Stores and views share one table namespace at the storage level
    // (`docs/toolkit/04-processing-layer.md`): both materialize as a table
    // named after the declaration, so a collision is an error here, not a
    // surprise at runtime.
    let mut view_names: HashSet<&str> = HashSet::new();
    for v in &views {
        if !view_names.insert(&v.name.name) {
            errors.push(ResolveError::new(
                format!("duplicate view `{}`", v.name.name),
                v.name.span,
            ));
        }
        if store_names.contains(v.name.name.as_str()) {
            errors.push(ResolveError::new(
                format!(
                    "view `{}` collides with a store of the same name: stores and \
                     views share one table namespace",
                    v.name.name
                ),
                v.name.span,
            ));
        }
    }

    // Pass 2: resolve each shape's structure, for conformance checks below.
    let mut resolved_shapes: HashMap<&str, ResolvedShape> = HashMap::new();
    for (name, sh) in &shapes {
        match resolve_shape(sh, &units, &enums) {
            Ok(rs) => {
                resolved_shapes.insert(name, rs);
            }
            Err(mut errs) => errors.append(&mut errs),
        }
    }

    // Pass 3: resolve each store, then check the shapes it claims.
    let mut schemas = Vec::new();
    for s in &stores {
        match resolve_store(s, &units, &enums) {
            Ok(schema) => {
                check_conformance(s, &schema, &shapes, &resolved_shapes, &units, &mut errors);
                schemas.push(schema);
            }
            Err(mut errs) => errors.append(&mut errs),
        }
    }

    // Pass 4: type-check each view's body against the store schemas presented
    // as table sources (`docs/language/10-views.md`), and lower each checked
    // view into a `ViewPlan` for the processing layer
    // (`docs/toolkit/04-processing-layer.md`).
    let mut view_plans = Vec::new();
    if !views.is_empty() {
        let mut sources = Sources::new();
        for schema in &schemas {
            sources = sources.with(&schema.store, TableType::from_store(schema));
        }
        for v in &views {
            match type_view(&sources, &v.body) {
                Ok(output) => {
                    // Check the optional `: Shape` conformance clause against
                    // the view's computed output content (index + named
                    // columns by type/totality), ignoring cardinality
                    // (10-views.md, ADR 0013).
                    check_view_conformance(
                        v,
                        &output,
                        &shapes,
                        &resolved_shapes,
                        &units,
                        &enums,
                        &mut errors,
                    );
                    view_plans.push(view_plan(v, &output, &store_names));
                }
                Err(errs) => {
                    for e in errs {
                        errors.push(ResolveError::new(e.message, e.span));
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(ResolvedProgram {
            schemas,
            views: view_plans,
        })
    } else {
        Err(errors)
    }
}

/// Lower a checked view into its [`ViewPlan`]: the output columns in storage
/// order (read off the checked table type), the computed cardinality, the
/// body, and the stores it reads.
fn view_plan(v: &ViewDecl, output: &TableType, store_names: &HashSet<&str>) -> ViewPlan {
    let mut columns = Vec::new();
    for c in &output.content.index {
        columns.push(Column {
            name: c.name.clone(),
            ty: c.domain.clone(),
            role: ColumnRole::Index,
            optional: false,
            span: v.name.span,
        });
    }
    for c in &output.content.columns {
        columns.push(Column {
            name: c.name.clone(),
            ty: c.domain.clone(),
            role: ColumnRole::Attr,
            optional: output.qualifiers.totality.is_optional(&c.name),
            span: v.name.span,
        });
    }
    ViewPlan {
        name: v.name.name.clone(),
        columns,
        cardinality: output.qualifiers.cardinality,
        body: v.body.clone(),
        sources: collect_sources(&v.body, store_names),
        span: v.span,
    }
}

/// The store names a view body mentions, sorted.  A bare name is a source
/// only in pipeline position, but matching every name against the store set
/// over-approximates harmlessly (the runtime would scan a store it does not
/// read); shadowing store names with lambda parameters is not a concern the
/// slice needs to resolve.
fn collect_sources(body: &Block, stores: &HashSet<&str>) -> Vec<String> {
    let mut found = BTreeSet::new();
    collect_block(body, stores, &mut found);
    found.into_iter().collect()
}

fn collect_block(block: &Block, stores: &HashSet<&str>, found: &mut BTreeSet<String>) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { value, .. } => collect_expr(value, stores, found),
            Stmt::Assert(e) | Stmt::Expr(e) => collect_expr(e, stores, found),
        }
    }
}

fn collect_expr(expr: &Expr, stores: &HashSet<&str>, found: &mut BTreeSet<String>) {
    match &expr.kind {
        ExprKind::Name(name) => {
            if stores.contains(name.as_str()) {
                found.insert(name.clone());
            }
        }
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) => {}
        ExprKind::Member(base, _) => collect_expr(base, stores, found),
        ExprKind::App(f, a) => {
            collect_expr(f, stores, found);
            collect_expr(a, stores, found);
        }
        ExprKind::Unary(_, e) => collect_expr(e, stores, found),
        ExprKind::Binary(_, l, r) => {
            collect_expr(l, stores, found);
            collect_expr(r, stores, found);
        }
        ExprKind::Presence(base, _) => collect_expr(base, stores, found),
        ExprKind::Lambda { body, .. } => collect_expr(body, stores, found),
        ExprKind::Tuple(items) => {
            for item in items {
                collect_expr(item, stores, found);
            }
        }
        ExprKind::Record(fields) => {
            for f in fields {
                collect_expr(&f.value, stores, found);
            }
        }
        ExprKind::Block(b) => collect_block(b, stores, found),
        ExprKind::If { cond, then, els } => {
            collect_expr(cond, stores, found);
            collect_expr(then, stores, found);
            collect_expr(els, stores, found);
        }
    }
}

fn resolve_store(
    s: &StoreDecl,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
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

    // Columns in storage order: index fields, then attributes in declaration
    // order.  Compound units surface here: an index field whose type
    // references another unit is rejected by `resolve_type`.
    let mut columns = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for f in &unit.fields {
        add_column(
            &mut columns,
            &mut seen,
            &mut errors,
            f,
            ColumnRole::Index,
            units,
            enums,
        );
    }
    for f in &s.attrs {
        add_column(
            &mut columns,
            &mut seen,
            &mut errors,
            f,
            ColumnRole::Attr,
            units,
            enums,
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

/// The kind of a shape parameter.  Only these two are supported; numeric and
/// predicate parameters are deferred.
enum ParamKind {
    Unit,
    Str,
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

/// One resolved shape attribute: its (possibly interpolated) name template
/// and resolved type.
struct ResolvedAttr {
    name: NameTemplate,
    ty: ColumnType,
    /// Value totality demanded of a conforming store's column (ADR 0010).
    optional: bool,
}

/// A shape resolved for conformance: its parameters in order (with kinds),
/// how it constrains the unit, and its attributes.  Attribute names are kept
/// as templates because they are not concrete until a claim binds the shape's
/// `string` parameters.
struct ResolvedShape {
    params: Vec<(String, ParamKind)>,
    unit: ShapeUnit,
    attrs: Vec<ResolvedAttr>,
}

fn resolve_shape(
    sh: &ShapeDecl,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
) -> Result<ResolvedShape, Vec<ResolveError>> {
    let mut errors = Vec::new();

    // Parameters.  `Unit` and `string` are supported; numeric/predicate
    // parameter types are deferred.
    let mut params: Vec<(String, ParamKind)> = Vec::new();
    let mut seen_params: HashSet<&str> = HashSet::new();
    for p in &sh.params {
        if !seen_params.insert(p.name.name.as_str()) {
            errors.push(ResolveError::new(
                format!("duplicate parameter `{}`", p.name.name),
                p.name.span,
            ));
        }
        let kind = match p.kind.name.as_str() {
            "Unit" => Some(ParamKind::Unit),
            "string" => Some(ParamKind::Str),
            "int" | "real" | "bool" | "date" => {
                errors.push(ResolveError::new(
                    format!(
                        "`{}` parameters are not yet supported; use `Unit` or `string`",
                        p.kind.name
                    ),
                    p.kind.span,
                ));
                None
            }
            other => {
                errors.push(ResolveError::new(
                    format!("unknown parameter kind `{other}`"),
                    p.kind.span,
                ));
                None
            }
        };
        if let Some(k) = kind {
            // A `Unit` parameter is a type parameter (PascalCase); a `string`
            // parameter is a value parameter (snake_case).
            let case = match k {
                ParamKind::Unit => Case::Pascal,
                ParamKind::Str => Case::Snake,
            };
            check_case(&p.name.name, p.name.span, case, "parameter", &mut errors);
            params.push((p.name.name.clone(), k));
        }
    }

    // Unit clause: optional; if it names a parameter, that must be a `Unit`.
    let unit = match &sh.unit {
        None => ShapeUnit::Agnostic,
        Some(u) => match params.iter().find(|(n, _)| n == &u.name) {
            Some((_, ParamKind::Unit)) => ShapeUnit::Param(u.name.clone()),
            Some((_, ParamKind::Str)) => {
                errors.push(ResolveError::new(
                    format!("`{}` is a `string` parameter, not a unit", u.name),
                    u.span,
                ));
                ShapeUnit::Agnostic
            }
            None if units.contains_key(u.name.as_str()) => ShapeUnit::Concrete(u.name.clone()),
            None => {
                errors.push(ResolveError::new(
                    format!("unknown unit `{}`", u.name),
                    u.span,
                ));
                ShapeUnit::Agnostic
            }
        },
    };

    // The `string` parameters, for validating template interpolation.
    let str_params: HashSet<&str> = params
        .iter()
        .filter(|(_, k)| matches!(k, ParamKind::Str))
        .map(|(n, _)| n.as_str())
        .collect();

    // Attributes.  A name may interpolate `string` parameters; its type must
    // be a primitive or enum (compound types stay deferred via `resolve_type`).
    let mut attrs = Vec::new();
    let mut seen_literals: HashSet<&str> = HashSet::new();
    for a in &sh.attrs {
        for seg in &a.name.segments {
            let NameSeg::Param(p) = seg else { continue };
            if !str_params.contains(p.name.as_str()) {
                errors.push(ResolveError::new(
                    format!("`{}` is not a `string` parameter of this shape", p.name),
                    p.span,
                ));
            }
        }
        // A literal attribute name is checked here; an interpolated one is
        // checked on the conforming store's resolved column instead.
        if let Some(lit) = a.name.as_literal() {
            check_case(lit, a.name.span, Case::Snake, "attribute", &mut errors);
            if !seen_literals.insert(lit) {
                errors.push(ResolveError::new(
                    format!("duplicate attribute `{lit}`"),
                    a.name.span,
                ));
            }
        }
        match resolve_type(&a.ty, units, enums) {
            Ok(ty) => attrs.push(ResolvedAttr {
                name: a.name.clone(),
                ty,
                optional: a.ty.is_optional(),
            }),
            Err(e) => errors.push(e),
        }
    }

    if errors.is_empty() {
        Ok(ResolvedShape {
            params,
            unit,
            attrs,
        })
    } else {
        Err(errors)
    }
}

/// Check every shape a store claims with its `:` clause.  Arguments are bound
/// to parameters by position (a unit name to a `Unit` parameter, a string to
/// a `string` parameter), `string` bindings are interpolated into attribute
/// names, and the store must tabulate the required unit (if any) and carry
/// every attribute with the same name and type.  Each failure is a
/// separate diagnostic pointing at the claim.
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

        let subject = format!("store `{}`", s.name.name);
        let Some((unit_bind, str_bind)) = bind_shape_args(&subject, shape, claim, units, errors)
        else {
            continue;
        };

        // Unit check, unless the shape is unit-agnostic.  `required` is set
        // only when the shape pins a unit and the store disagrees.
        let required = match &shape.unit {
            ShapeUnit::Agnostic => None,
            ShapeUnit::Concrete(u) => Some(u.as_str()),
            ShapeUnit::Param(p) => Some(unit_bind[p.as_str()]),
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

        for attr in &shape.attrs {
            let want = render_template(&attr.name, &str_bind);
            if !is_identifier(&want) {
                errors.push(ResolveError::new(
                    format!(
                        "store `{}` claims `{}`: interpolated attribute name `{}` is not a valid identifier",
                        s.name.name,
                        shape_ref_label(claim),
                        want
                    ),
                    claim.span,
                ));
                continue;
            }
            match schema.columns.iter().find(|c| c.name == want) {
                None => errors.push(ResolveError::new(
                    format!(
                        "store `{}` claims `{}` but is missing attribute `{}`",
                        s.name.name,
                        shape_ref_label(claim),
                        want
                    ),
                    claim.span,
                )),
                Some(have) if have.ty != attr.ty => errors.push(ResolveError::new(
                    format!(
                        "store `{}` claims `{}`: attribute `{}` has type `{}` in the shape but `{}` in the store",
                        s.name.name,
                        shape_ref_label(claim),
                        want,
                        type_name(&attr.ty),
                        type_name(&have.ty)
                    ),
                    claim.span,
                )),
                Some(have) if have.optional != attr.optional => errors.push(ResolveError::new(
                    format!(
                        "store `{}` claims `{}`: attribute `{}` is `{}` in the shape but `{}` in the store",
                        s.name.name,
                        shape_ref_label(claim),
                        want,
                        totality_word(attr.optional),
                        totality_word(have.optional)
                    ),
                    claim.span,
                )),
                Some(_) => {}
            }
        }
    }
}

/// Positional argument bindings for a shape claim: the `Unit`-parameter map and
/// the `string`-parameter map, each from parameter name to the bound value.
type ShapeBindings<'a> = (HashMap<&'a str, &'a str>, HashMap<&'a str, &'a str>);

/// Bind a conformance claim's arguments to a shape's parameters by position,
/// checking arity and each argument's kind.  Shared by store and view
/// conformance.  `subject` names the claiming construct for the arity
/// diagnostic (for example ``store `persons` `` or ``view `machine_temp` ``).
/// Returns the unit and `string` bindings, or `None` if any check failed (each
/// failure is recorded on `errors`).
fn bind_shape_args<'a>(
    subject: &str,
    shape: &'a ResolvedShape,
    claim: &'a ShapeRef,
    units: &HashMap<&str, &UnitDecl>,
    errors: &mut Vec<ResolveError>,
) -> Option<ShapeBindings<'a>> {
    if claim.args.len() != shape.params.len() {
        errors.push(ResolveError::new(
            format!(
                "{subject} claims `{}` with {} argument(s), but the shape declares {}",
                shape_ref_label(claim),
                claim.args.len(),
                shape.params.len()
            ),
            claim.span,
        ));
        return None;
    }

    // Bind arguments to parameters by position, checking each kind.
    let mut unit_bind: HashMap<&str, &str> = HashMap::new();
    let mut str_bind: HashMap<&str, &str> = HashMap::new();
    let mut args_ok = true;
    for ((pname, pkind), arg) in shape.params.iter().zip(&claim.args) {
        match (pkind, arg) {
            (ParamKind::Unit, ShapeArg::Unit(id)) => {
                if !units.contains_key(id.name.as_str()) {
                    errors.push(ResolveError::new(
                        format!("unknown unit `{}`", id.name),
                        id.span,
                    ));
                    args_ok = false;
                }
                unit_bind.insert(pname.as_str(), id.name.as_str());
            }
            (ParamKind::Str, ShapeArg::Str(lit)) => {
                str_bind.insert(pname.as_str(), lit.value.as_str());
            }
            (ParamKind::Unit, ShapeArg::Str(_)) => {
                errors.push(ResolveError::new(
                    format!("parameter `{pname}` expects a unit name, but a string was given"),
                    arg.span(),
                ));
                args_ok = false;
            }
            (ParamKind::Str, ShapeArg::Unit(id)) => {
                errors.push(ResolveError::new(
                    format!(
                        "parameter `{pname}` expects a string, but `{}` was given",
                        id.name
                    ),
                    arg.span(),
                ));
                args_ok = false;
            }
        }
    }
    if args_ok {
        Some((unit_bind, str_bind))
    } else {
        None
    }
}

/// Check every shape a view claims with its `:` clause against the view's
/// computed output (`docs/language/10-views.md`, "Constraining a view with a
/// shape").  Unlike a store, a view has no declared unit, so the check is
/// content-only:
///
/// - a unit-fixing shape requires the output's index to be exactly the unit's
///   index fields, by name and type (cardinality is left free: a view may be
///   `bag`);
/// - each shape attribute must appear among the output columns, index or
///   non-index, with a matching type and totality.
fn check_view_conformance(
    view: &ViewDecl,
    output: &TableType,
    shapes: &HashMap<&str, &ShapeDecl>,
    resolved: &HashMap<&str, ResolvedShape>,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
    errors: &mut Vec<ResolveError>,
) {
    for claim in &view.conforms {
        let name = claim.name.name.as_str();
        let Some(shape) = resolved.get(name) else {
            // As for stores: a shape that resolved with errors already reported
            // them; only an entirely unknown name is new here.
            if !shapes.contains_key(name) {
                errors.push(ResolveError::new(
                    format!("unknown shape `{name}`"),
                    claim.span,
                ));
            }
            continue;
        };

        let subject = format!("view `{}`", view.name.name);
        let Some((unit_bind, str_bind)) = bind_shape_args(&subject, shape, claim, units, errors)
        else {
            continue;
        };

        // A unit-fixing shape constrains the output's index structurally: a
        // view carries no declared unit, so conformance is that the output's
        // index columns are exactly the unit's index fields (10-views.md, "The
        // unit clause").
        let required_unit = match &shape.unit {
            ShapeUnit::Agnostic => None,
            ShapeUnit::Concrete(u) => Some(u.as_str()),
            ShapeUnit::Param(p) => Some(unit_bind[p.as_str()]),
        };
        if let Some(uname) = required_unit
            && let Some(unit) = units.get(uname).copied()
        {
            let expected = unit_index_columns(unit, units, enums);
            let actual: Vec<(String, ColumnType)> = output
                .content
                .index
                .iter()
                .map(|c| (c.name.clone(), c.domain.clone()))
                .collect();
            if !same_columns(&expected, &actual) {
                errors.push(ResolveError::new(
                    format!(
                        "view `{}` claims `{}`, which tabulates `{}`, but its index is {} rather than {}",
                        view.name.name,
                        shape_ref_label(claim),
                        uname,
                        render_columns(&actual),
                        render_columns(&expected)
                    ),
                    claim.span,
                ));
                continue;
            }
        }

        // Content attributes: each must appear somewhere in the output (index
        // or non-index), with a matching type and totality.
        for attr in &shape.attrs {
            let want = render_template(&attr.name, &str_bind);
            if !is_identifier(&want) {
                errors.push(ResolveError::new(
                    format!(
                        "view `{}` claims `{}`: interpolated attribute name `{}` is not a valid identifier",
                        view.name.name,
                        shape_ref_label(claim),
                        want
                    ),
                    claim.span,
                ));
                continue;
            }
            let found = output
                .content
                .index
                .iter()
                .chain(output.content.columns.iter())
                .find(|c| c.name == want);
            match found {
                None => errors.push(ResolveError::new(
                    format!(
                        "view `{}` claims `{}` but is missing attribute `{}`",
                        view.name.name,
                        shape_ref_label(claim),
                        want
                    ),
                    claim.span,
                )),
                Some(have) if have.domain != attr.ty => errors.push(ResolveError::new(
                    format!(
                        "view `{}` claims `{}`: attribute `{}` has type `{}` in the shape but `{}` in the view",
                        view.name.name,
                        shape_ref_label(claim),
                        want,
                        type_name(&attr.ty),
                        type_name(&have.domain)
                    ),
                    claim.span,
                )),
                Some(_) if output.qualifiers.totality.is_optional(&want) != attr.optional => errors
                    .push(ResolveError::new(
                        format!(
                            "view `{}` claims `{}`: attribute `{}` is `{}` in the shape but `{}` in the view",
                            view.name.name,
                            shape_ref_label(claim),
                            want,
                            totality_word(attr.optional),
                            totality_word(output.qualifiers.totality.is_optional(&want))
                        ),
                        claim.span,
                    )),
                Some(_) => {}
            }
        }
    }
}

/// Resolve a unit's index fields to their `(name, type)` pairs, for structural
/// conformance of a view claiming a unit-fixing shape.  Fields that fail to
/// resolve (for example a still-unsupported compound field) are skipped: such
/// a unit cannot back a store either, and its error surfaces where it is used.
fn unit_index_columns(
    unit: &UnitDecl,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
) -> Vec<(String, ColumnType)> {
    let mut out = Vec::new();
    for f in &unit.fields {
        if let (Ok(name), Ok(ty)) = (
            literal_field_name(&f.name),
            resolve_type(&f.ty, units, enums),
        ) {
            out.push((name, ty));
        }
    }
    out
}

/// Two column lists match iff they carry the same names, each with the same
/// type.  Order is not significant: a view's index is a set of key columns, and
/// pipeline stages may present them in any order.
fn same_columns(a: &[(String, ColumnType)], b: &[(String, ColumnType)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(name, ty)| b.iter().any(|(bn, bt)| bn == name && bt == ty))
}

/// Render a column list as ``a: int, b: string`` for diagnostics.
fn render_columns(cols: &[(String, ColumnType)]) -> String {
    if cols.is_empty() {
        return "empty".into();
    }
    cols.iter()
        .map(|(name, ty)| format!("`{name}: {}`", type_name(ty)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Concatenate a name template, substituting each `string` parameter with its
/// bound argument.  The bindings are complete by the time this runs (arity and
/// declaration validation guarantee every parameter is a bound `string`).
fn render_template(name: &NameTemplate, str_bind: &HashMap<&str, &str>) -> String {
    let mut out = String::new();
    for seg in &name.segments {
        match seg {
            NameSeg::Lit(s) => out.push_str(s),
            NameSeg::Param(p) => out.push_str(str_bind.get(p.name.as_str()).copied().unwrap_or("")),
        }
    }
    out
}

/// Render a conformance claim for diagnostics: `Tabular[Person]`,
/// `Ageable["birthdate"]`, or, with no arguments, just `PersonRecord`.
fn shape_ref_label(r: &ShapeRef) -> String {
    if r.args.is_empty() {
        r.name.name.clone()
    } else {
        let args = r
            .args
            .iter()
            .map(|a| match a {
                ShapeArg::Unit(id) => id.name.clone(),
                ShapeArg::Str(s) => format!("\"{}\"", s.value),
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}[{}]", r.name.name, args)
    }
}

/// The total/optional axis as a word for diagnostics (ADR 0010).
fn totality_word(optional: bool) -> &'static str {
    if optional { "optional" } else { "total" }
}

fn type_name(ty: &ColumnType) -> String {
    match ty {
        ColumnType::String => "string".into(),
        ColumnType::Int => "int".into(),
        ColumnType::Real => "real".into(),
        ColumnType::Bool => "bool".into(),
        ColumnType::Date => "date".into(),
        ColumnType::Enum { name, .. } => name.clone(),
    }
}

fn add_column(
    columns: &mut Vec<Column>,
    seen: &mut HashSet<String>,
    errors: &mut Vec<ResolveError>,
    field: &Field,
    role: ColumnRole,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
) {
    // Units and stores carry no parameters, so a field name must render to a
    // plain identifier with no interpolation.
    let name = match literal_field_name(&field.name) {
        Ok(name) => name,
        Err(e) => {
            errors.push(e);
            return;
        }
    };
    if !seen.insert(name.clone()) {
        errors.push(ResolveError::new(
            format!("duplicate column `{name}`"),
            field.name.span,
        ));
        return;
    }
    // Index fields come from the unit and are checked at its declaration;
    // here only the store's own attributes are checked.
    if role != ColumnRole::Index {
        check_case(&name, field.name.span, Case::Snake, "attribute", errors);
    }
    // An index field is always known: whether the row exists at all is
    // cardinality, a separate axis from value missingness (ADR 0010).  `?` on
    // an index field is rejected; an attribute may be optional.
    let is_index = role == ColumnRole::Index;
    if is_index && let Some(span) = field.ty.optional {
        errors.push(ResolveError::new(
            format!("an index field cannot be optional: drop the `?` on `{name}`"),
            span,
        ));
    }
    let ct = match resolve_type(&field.ty, units, enums) {
        Ok(ct) => ct,
        Err(e) => {
            errors.push(e);
            return;
        }
    };
    // A key is identified by equality, so an index field must be key-eligible
    // (ADR 0014); `real` is a continuous measurement, not an identity.
    if is_index && !ct.is_key_eligible() {
        errors.push(ResolveError::new(
            format!(
                "an index field must be a key-eligible type: `{}` cannot be a key",
                type_name(&ct)
            ),
            field.ty.name.span,
        ));
    }
    columns.push(Column {
        name,
        ty: ct,
        role,
        optional: field.ty.is_optional() && !is_index,
        span: field.name.span,
    });
}

/// Render a name template that may not interpolate: units and stores have no
/// parameters in scope.  Errors if the name has a `{param}` hole or does not
/// render to a valid identifier.
fn literal_field_name(name: &NameTemplate) -> Result<String, ResolveError> {
    let mut rendered = String::new();
    for seg in &name.segments {
        match seg {
            NameSeg::Lit(s) => rendered.push_str(s),
            NameSeg::Param(p) => {
                return Err(ResolveError::new(
                    format!(
                        "`{}` is a shape parameter, but units and stores have none to interpolate",
                        p.name
                    ),
                    p.span,
                ));
            }
        }
    }
    if !is_identifier(&rendered) {
        return Err(ResolveError::new(
            format!("`{rendered}` is not a valid attribute name"),
            name.span,
        ));
    }
    Ok(rendered)
}

fn resolve_type(
    ty: &TypeExpr,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
) -> Result<ColumnType, ResolveError> {
    // Resolve only the base type here; optionality (`?`) is read from the
    // `TypeExpr` by the caller, which knows the column's role (an index field
    // may not be optional; ADR 0010).
    let id = &ty.name;
    match id.name.as_str() {
        "string" => Ok(ColumnType::String),
        "int" => Ok(ColumnType::Int),
        "real" => Ok(ColumnType::Real),
        "bool" => Ok(ColumnType::Bool),
        "date" => Ok(ColumnType::Date),
        other if enums.contains_key(other) => {
            let e = enums[other];
            Ok(ColumnType::Enum {
                name: e.name.name.clone(),
                variants: e.variants.iter().map(|v| v.value.clone()).collect(),
            })
        }
        other if units.contains_key(other) => Err(ResolveError::new(
            format!("compound fields are not yet supported (references unit `{other}`)"),
            id.span,
        )),
        other => Err(ResolveError::new(
            format!("unknown type `{other}`"),
            id.span,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_program(src: &str) -> Result<ResolvedProgram, Vec<ResolveError>> {
        let tokens = mensura_syntax::tokenize(src).expect("should lex");
        let program = mensura_syntax::parse(&tokens).expect("should parse");
        resolve(&program)
    }

    fn resolve_str(src: &str) -> Result<Vec<Schema>, Vec<ResolveError>> {
        resolve_program(src).map(|p| p.schemas)
    }

    fn errors(src: &str) -> Vec<ResolveError> {
        resolve_str(src).expect_err("should fail to resolve")
    }

    const ATTENTION: &str = r#"
        unit Machine { id: string }
        enum MachineStatus { "operational" "degraded" "failure" }
        store machines {
          unit { Machine }
          attr {
            status: MachineStatus
            last_service: date?
          }
        }
        view attention_needed {
          machines |> map |_, r| if r.status == "degraded" then r else ()
        }
    "#;

    #[test]
    fn view_lowers_to_a_plan() {
        let program = resolve_program(ATTENTION).expect("should resolve");
        assert_eq!(program.views.len(), 1);
        let plan = &program.views[0];
        assert_eq!(plan.name, "attention_needed");
        assert_eq!(plan.sources, vec!["machines".to_string()]);
        assert_eq!(plan.cardinality, crate::table::Cardinality::Singletons);

        // Output columns in storage order: the index, then the computed
        // attributes in the order the checker produced them (a whole-row
        // `map` body yields them alphabetically).
        let cols: Vec<(&str, ColumnRole, bool)> = plan
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.role, c.optional))
            .collect();
        assert_eq!(
            cols,
            vec![
                ("id", ColumnRole::Index, false),
                ("last_service", ColumnRole::Attr, true),
                ("status", ColumnRole::Attr, false),
            ]
        );

        // A `singletons` view is keyed at the storage level.
        assert!(plan.shape().keyed);
    }

    #[test]
    fn view_colliding_with_a_store_is_an_error() {
        let src = r#"
            unit Machine { id: string }
            store machines {
              unit { Machine }
              attr { commissioned: date }
            }
            view machines {
              machines |> map |_, r| r
            }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("collides with a store")),
            "expected a table-namespace collision error, got: {errs:?}"
        );
    }

    #[test]
    fn duplicate_view_is_an_error() {
        let src = r#"
            unit Machine { id: string }
            store machines {
              unit { Machine }
              attr { commissioned: date }
            }
            view v {
              machines |> map |_, r| r
            }
            view v {
              machines |> map |_, r| r
            }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter().any(|e| e.message.contains("duplicate view")),
            "expected a duplicate-view error, got: {errs:?}"
        );
    }

    #[test]
    fn resolves_basic_example() {
        let src = r#"
            unit Person { id: string }
            unit Department { code: string }
            enum Status { "active" "inactive" }

            store departments {
              unit { Department }
              attr { name: string }
            }
            store persons {
              unit { Person }
              attr { birthdate: date }
              attr { last_name: string }
            }
            store students {
              unit { Person }
              attr { admission: date }
              attr { status: Status }
            }
        "#;
        let schemas = resolve_str(src).expect("should resolve");
        assert_eq!(schemas.len(), 3);

        let students = schemas.iter().find(|s| s.store == "students").unwrap();
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
                ("admission", ColumnRole::Attr, &ColumnType::Date),
                (
                    "status",
                    ColumnRole::Attr,
                    &ColumnType::Enum {
                        name: "Status".into(),
                        variants: vec!["active".into(), "inactive".into()],
                    },
                ),
            ]
        );
    }

    #[test]
    fn unknown_unit_is_rejected() {
        let errs = errors("store s { unit { Ghost } attr { a: string } }");
        assert!(errs[0].message.contains("unknown unit `Ghost`"));
    }

    #[test]
    fn compound_unit_field_is_rejected() {
        let src = r#"
            unit Department { code: string }
            unit Course { department: Department }
            store courses { unit { Course } }
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
            store s {
              unit { Person }
              domain { x: Other }
            }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| e.message.contains("`domain` block")));
    }

    #[test]
    fn duplicate_column_is_rejected() {
        // `id` is both the index field and a declared attribute.
        let src = r#"
            unit Person { id: string }
            store s { unit { Person } attr { id: string } }
        "#;
        let errs = errors(src);
        assert!(errs[0].message.contains("duplicate column `id`"));
    }

    #[test]
    fn unknown_type_is_rejected() {
        let errs = errors("unit U { x: widget } store s { unit { U } }");
        assert!(errs[0].message.contains("unknown type `widget`"));
    }

    #[test]
    fn backtick_literal_name_in_store_resolves() {
        // A backtick-quoted literal name is the same as the bare identifier.
        let src = r#"
            unit Person { id: string }
            store s { unit { Person } attr { `extra`: string } }
        "#;
        let schema = &resolve_str(src).expect("should resolve")[0];
        assert!(schema.columns.iter().any(|c| c.name == "extra"));
    }

    #[test]
    fn interpolation_in_store_is_rejected() {
        // A store has no parameters, so a `{param}` name cannot resolve.
        let src = r#"
            unit Person { id: string }
            store s { unit { Person } attr { `{x}`: string } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("none to interpolate"))
        );
    }

    #[test]
    fn duplicate_enum_variant_is_rejected() {
        let src = r#"enum Bad { "a" "a" }"#;
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
            store a { unit { Ghost } }
            store b { unit { U } attr { x: widget } }
        "#;
        let errs = errors(src);
        assert_eq!(errs.len(), 2);
    }

    #[test]
    fn conforming_store_resolves() {
        let src = r#"
            unit Person { id: string }
            enum Status { "active" "inactive" }
            shape PersonRecord {
              unit { Person }
              attr { admission: date }
            }
            store students : PersonRecord {
              unit { Person }
              attr { admission: date }
              attr { status: Status }
            }
        "#;
        // The store carries an extra attribute (`status`); conformance only
        // requires the shape's attributes to be present.
        let schemas = resolve_str(src).expect("should resolve");
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].store, "students");
    }

    #[test]
    fn marker_shape_conforms_on_unit_alone() {
        let src = r#"
            unit Person { id: string }
            shape Anything { unit { Person } }
            store persons : Anything { unit { Person } attr { birthdate: date } }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 1);
    }

    #[test]
    fn unknown_shape_is_rejected() {
        let src = r#"
            unit Person { id: string }
            store students : Ghost { unit { Person } }
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
            shape PersonRecord { unit { Person } attr { admission: date } }
            store students : PersonRecord { unit { Person } }
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
            shape PersonRecord { unit { Person } attr { admission: date } }
            store courses : PersonRecord { unit { Course } attr { admission: date } }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| e.message.contains("tabulates `Person`")
            && e.message.contains("tabulates `Course`")));
    }

    #[test]
    fn wrong_attribute_type_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape PersonRecord { unit { Person } attr { admission: date } }
            store students : PersonRecord { unit { Person } attr { admission: string } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("type `date` in the shape but `string`"))
        );
    }

    #[test]
    fn repeated_attr_blocks_merge() {
        // Several `attr` blocks are equivalent to one listing all attributes
        // (ADR 0019); a duplicate across blocks is the within-block error.
        let ok = r#"
            unit Person { id: string }
            store students { unit { Person } attr { a: date } attr { b: string } }
        "#;
        let schemas = resolve_str(ok).expect("should resolve");
        let names: Vec<&str> = schemas[0].columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "a", "b"]);

        let dup = r#"
            unit Person { id: string }
            store students { unit { Person } attr { a: date } attr { a: date } }
        "#;
        let errs = errors(dup);
        assert!(errs[0].message.contains("duplicate column `a`"));
    }

    #[test]
    fn optional_attribute_resolves() {
        // A `?` makes the column optional; a bare type stays total (ADR 0010).
        let src = r#"
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr { last_service: date? }
              attr { vibration: real }
            }
        "#;
        let schemas = resolve_str(src).expect("should resolve");
        let readings = schemas.iter().find(|s| s.store == "readings").unwrap();
        let by = |n: &str| readings.columns.iter().find(|c| c.name == n).unwrap();
        assert!(by("last_service").optional);
        assert!(!by("vibration").optional);
        // The index is total even though `?` was not (and may not be) written.
        assert!(!by("id").optional);
    }

    #[test]
    fn optional_index_field_is_rejected() {
        // Whether a row exists is cardinality, not value missingness; `?` on an
        // index field is a hard error (ADR 0010).
        let src = r#"
            unit Machine { id: string? }
            store readings { unit { Machine } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("index field cannot be optional"))
        );
    }

    #[test]
    fn totality_mismatch_in_conformance_is_rejected() {
        // A shape demanding a total attribute is not satisfied by an optional
        // store column, and vice versa.
        let src = r#"
            unit Person { id: string }
            shape PersonRecord { unit { Person } attr { admission: date } }
            store students : PersonRecord { unit { Person } attr { admission: date? } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("`total` in the shape but `optional`"))
        );
    }

    #[test]
    fn optional_attribute_conforms_when_shape_agrees() {
        let src = r#"
            unit Person { id: string }
            shape PersonRecord { unit { Person } attr { nickname: string? } }
            store students : PersonRecord { unit { Person } attr { nickname: string? } }
        "#;
        resolve_str(src).expect("matching totality should conform");
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
            shape Tabular[U: Unit] { unit { U } }
            store persons : Tabular[Person] { unit { Person } attr { birthdate: date } }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 1);
    }

    #[test]
    fn unit_agnostic_shape_conforms_to_any_unit() {
        // `Named` has no unit clause, so a `Department` store conforms purely
        // by carrying the required `name` attribute.
        let src = r#"
            unit Department { code: string }
            shape Named { attr { name: string } }
            store departments : Named { unit { Department } attr { name: string } }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 1);
    }

    #[test]
    fn wrong_unit_argument_is_rejected() {
        let src = r#"
            unit Person { id: string }
            unit Course { code: string }
            shape Tabular[U: Unit] { unit { U } }
            store s : Tabular[Course] { unit { Person } }
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
            shape Tabular[U: Unit] { unit { U } }
            store s : Tabular { unit { Person } }
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
            store s : PersonRecord[Person] { unit { Person } }
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
            shape Tabular[U: Unit] { unit { U } }
            store s : Tabular[Ghost] { unit { Person } }
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
            shape D[U: Unit, U: Unit] { unit { U } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("duplicate parameter `U`"))
        );
    }

    #[test]
    fn numeric_parameter_is_rejected() {
        // `Unit` and `string` are supported; other parameter types are not.
        let src = r#"
            unit Person { id: string }
            shape Weighted[n: real] { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("`real` parameters are not yet supported")
        }));
    }

    #[test]
    fn string_parameter_shape_conforms_with_interpolation() {
        // `Ageable` is unit-agnostic and names its date field via a `string`
        // parameter, so `Person` and `Department` conform with different names.
        let src = r#"
            unit Person { id: string }
            unit Department { code: string }
            shape Ageable[date_field: string] {
              attr { `{date_field}`: date }
            }
            store persons : Ageable["birthdate"] {
              unit { Person }
              attr { birthdate: date }
            }
            store departments : Ageable["foundation_day"] {
              unit { Department }
              attr { foundation_day: date }
            }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 2);
    }

    #[test]
    fn interpolated_template_conforms() {
        let src = r#"
            unit Person { id: string }
            shape NormalizedCol[col: string] {
              attr {
                `{col}`:   real
                `{col}_z`: real
              }
            }
            store students : NormalizedCol["height"] {
              unit { Person }
              attr {
                height:   real
                height_z: real
              }
            }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 1);
    }

    #[test]
    fn missing_interpolated_attribute_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape Ageable[date_field: string] { attr { `{date_field}`: date } }
            store persons : Ageable["birthdate"] { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("missing attribute `birthdate`"))
        );
    }

    #[test]
    fn string_argument_for_unit_parameter_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape Tabular[U: Unit] { unit { U } }
            store s : Tabular["Person"] { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("expects a unit name, but a string was given")
        }));
    }

    #[test]
    fn unit_argument_for_string_parameter_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape Ageable[date_field: string] { attr { `{date_field}`: date } }
            store persons : Ageable[birthdate] { unit { Person } attr { birthdate: date } }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("expects a string, but `birthdate` was given")
        }));
    }

    #[test]
    fn template_referencing_unknown_parameter_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape Bad[col: string] { attr { `{other}`: real } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("`other` is not a `string` parameter"))
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

    #[test]
    fn fleet_monitoring_example_resolves() {
        // The fleet-monitoring example grows milestone by milestone; its
        // compilable subset must keep resolving.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/examples/fleet-monitoring.mensura");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let schemas = resolve_str(&src).expect("example should resolve");
        assert_eq!(schemas.len(), 1);
    }

    // --- Casing convention (docs/language/05-naming-and-casing.md) ---

    #[test]
    fn conforming_casing_resolves() {
        // PascalCase types, snake_case store and attributes: no casing errors.
        let src = r#"
            unit Machine { id: string }
            store temperature_readings {
              unit { Machine }
              attr { temp_mean: real }
            }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 1);
    }

    #[test]
    fn non_snake_store_is_rejected() {
        let errs = errors("unit U { id: string } store TempReadings { unit { U } }");
        assert!(errs.iter().any(|e| {
            e.message
                .contains("store `TempReadings` must be snake_case")
        }));
    }

    #[test]
    fn non_pascal_unit_is_rejected() {
        let errs = errors("unit machine { id: string } store s { unit { machine } }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("unit `machine` must be PascalCase"))
        );
    }

    #[test]
    fn non_pascal_shape_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape person_record { unit { Person } }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("shape `person_record` must be PascalCase")
        }));
    }

    #[test]
    fn non_snake_attribute_is_rejected() {
        let src = r#"
            unit Person { id: string }
            store s { unit { Person } attr { birthDate: date } }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("attribute `birthDate` must be snake_case")
        }));
    }

    #[test]
    fn non_snake_index_attribute_is_rejected() {
        // An index field is checked once, at the unit, and only once even when
        // several stores tabulate that unit.
        let src = r#"
            unit Person { personId: string }
            store a { unit { Person } }
            store b { unit { Person } }
        "#;
        let errs = errors(src);
        let casing: Vec<_> = errs
            .iter()
            .filter(|e| {
                e.message
                    .contains("attribute `personId` must be snake_case")
            })
            .collect();
        assert_eq!(casing.len(), 1, "index field casing reported exactly once");
    }

    #[test]
    fn non_snake_string_parameter_is_rejected() {
        let src = r#"
            unit Person { id: string }
            shape Ageable[dateField: string] { attr { `{dateField}`: date } }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("parameter `dateField` must be snake_case")
        }));
    }

    #[test]
    fn unit_parameter_keeps_pascal_case() {
        // A `Unit` parameter is a type parameter, so PascalCase is correct and
        // must not be flagged.
        let src = r#"
            unit Person { id: string }
            shape Tabular[U: Unit] { unit { U } }
            store persons : Tabular[Person] { unit { Person } attr { birthdate: date } }
        "#;
        assert!(resolve_str(src).is_ok());
    }

    #[test]
    fn caseless_names_are_exempt() {
        // Identifiers with no cased characters (CJK) carry no case distinction,
        // so the convention does not constrain them in any position.
        let src = r#"
            unit 温度 { 标识: string }
            store 温度表 { unit { 温度 } attr { 测量: string } }
        "#;
        assert!(resolve_str(src).is_ok());
    }

    #[test]
    fn view_body_is_type_checked() {
        let ok = r#"
            unit Machine { id: string }
            store readings { unit { Machine } attr { temperature: real } }
            view machine_summary { readings |> group_map |k, g| (.temp_max = max g.temperature) }
        "#;
        resolve_str(ok).expect("a valid view resolves");

        let bad = r#"
            unit Machine { id: string }
            store readings { unit { Machine } attr { temperature: real } }
            view bad { readings |> group_map |k, g| (.x = g.temperature + 1.0) }
        "#;
        let errs = errors(bad);
        assert!(errs.iter().any(|e| e.message.contains("bag")));
    }

    // A summarizing view whose output index is `Machine`'s key and whose one
    // non-index column is `temp_max: real` (`docs/language/10-views.md`).
    const SUMMARY_VIEW: &str = r#"
        unit Machine { id: string }
        store readings { unit { Machine } attr { temperature: real } }
    "#;

    #[test]
    fn view_conforms_to_unit_fixing_shape() {
        // A unit-fixing shape checks the output's index structurally: the view
        // ends in `group_map`, so its index is `Machine`'s `id`.
        let src = format!(
            "{SUMMARY_VIEW}
            shape Tabular[U: Unit] {{ unit {{ U }} }}
            view machine_summary : Tabular[Machine] {{
              readings |> group_map |k, g| (.temp_max = max g.temperature)
            }}"
        );
        resolve_str(&src).expect("view carrying Machine's index conforms");
    }

    #[test]
    fn view_with_wrong_unit_index_is_rejected() {
        // The view's index is `id`, but `Site`'s index is `code`, so a claim of
        // `Tabular[Site]` fails the structural index check.
        let src = format!(
            "{SUMMARY_VIEW}
            unit Site {{ code: string }}
            shape Tabular[U: Unit] {{ unit {{ U }} }}
            view v : Tabular[Site] {{
              readings |> group_map |k, g| (.temp_max = max g.temperature)
            }}"
        );
        let errs = errors(&src);
        assert!(
            errs.iter().any(
                |e| e.message.contains("tabulates `Site`") && e.message.contains("rather than")
            )
        );
    }

    #[test]
    fn view_conforms_to_content_shape() {
        // A content shape checks named columns by name, type, and totality;
        // the same `attr` contract spans stores and views (ADR 0019).
        let src = format!(
            "{SUMMARY_VIEW}
            shape HasMax {{ attr {{ temp_max: real }} }}
            view machine_summary : HasMax {{
              readings |> group_map |k, g| (.temp_max = max g.temperature)
            }}"
        );
        resolve_str(&src).expect("view carrying temp_max conforms");
    }

    #[test]
    fn view_missing_content_attribute_is_rejected() {
        let src = format!(
            "{SUMMARY_VIEW}
            shape HasMin {{ attr {{ temp_min: real }} }}
            view v : HasMin {{
              readings |> group_map |k, g| (.temp_max = max g.temperature)
            }}"
        );
        let errs = errors(&src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("missing attribute `temp_min`"))
        );
    }

    #[test]
    fn view_content_attribute_wrong_type_is_rejected() {
        let src = format!(
            "{SUMMARY_VIEW}
            shape HasMax {{ attr {{ temp_max: string }} }}
            view v : HasMax {{
              readings |> group_map |k, g| (.temp_max = max g.temperature)
            }}"
        );
        let errs = errors(&src);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("type `string` in the shape but `real` in the view")
        }));
    }
}
