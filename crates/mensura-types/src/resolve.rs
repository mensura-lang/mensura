//! Name resolution: AST [`Program`] -> a resolved [`ResolvedProgram`].
//!
//! Resolution collects *all* diagnostics rather than failing on the first,
//! since stores and units are largely independent.  It enforces the current
//! "basic only" scope by rejecting compound units, compound fields, and
//! `domain` blocks with clear "not yet supported" errors.
//!
//! Shapes (`docs/language/03-shapes.md`) are validated here too: each store or
//! view that claims conformance with a `:` clause is checked against the shape's
//! structure and cardinality.  A store is checked against its declared columns;
//! a view against its computed output content and cardinality (ADR 0022 amends
//! ADR 0012: the shape's `attr` / `attr*` blocks now constrain cardinality,
//! `docs/language/10-views.md`).  Shapes carry no storage, so they produce no
//! [`Schema`]; a store yields a [`Schema`] and a view a [`ViewPlan`]
//! (`docs/toolkit/04-processing-layer.md`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use mensura_syntax::{
    Attr, Block, EnumDecl, Expr, ExprKind, Field, Ident, Item, LetKind, NameSeg, NameTemplate,
    Program, ShapeArg, ShapeDecl, ShapeRef, Span, Stmt, StoreDecl, TypeExpr, TypeKind, UnitDecl,
    ViewDecl, is_identifier,
};

use crate::consts::{ConstDecl, eval_const_bindings};
use crate::model::{Column, ColumnRole, ColumnType, ResolvedProgram, Schema, ViewPlan};
use crate::modules::ModuleEnv;
use crate::pipe_check::{Sources, type_view};
use crate::table::{Cardinality, TableType};
use crate::units::Dimension;

/// A resolution failure, located by a source span.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolveError {
    pub message: String,
    pub span: Span,
}

impl ResolveError {
    pub(crate) fn new(message: impl Into<String>, span: Span) -> Self {
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
    let mut dim_aliases: DimAliases = HashMap::new();
    let mut imports: Vec<&mensura_syntax::ImportDecl> = Vec::new();
    let mut const_decls: Vec<ConstDecl> = Vec::new();

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
            Item::Import(i) => {
                check_case(
                    &i.name.name,
                    i.name.span,
                    Case::Snake,
                    "import",
                    &mut errors,
                );
                imports.push(i);
            }
            Item::Let(l) => match &l.kind {
                LetKind::DimAlias { params, body } => {
                    collect_dim_alias(&l.name, params, body, &mut dim_aliases, &mut errors);
                }
                LetKind::Value { ty, value } => {
                    check_case(&l.name.name, l.name.span, Case::Snake, "let", &mut errors);
                    const_decls.push(ConstDecl {
                        name: &l.name,
                        ty: ty.as_ref(),
                        value,
                    });
                }
            },
        }
    }

    // Validate each alias body in an environment where its parameter stands
    // for itself; this also catches cross-alias reference cycles.
    for (name, def) in &dim_aliases {
        let env = TlEnv {
            units: &units,
            enums: &enums,
            aliases: &dim_aliases,
            param: Some((def.param, TlBacking::Param(def.param.to_string()))),
        };
        let mut stack = vec![name.to_string()];
        match eval_tl(def.body, &env, &mut stack) {
            Ok(TlValue::Dim(_) | TlValue::Applied { .. }) => {}
            Ok(TlValue::Plain(ct)) => errors.push(ResolveError::new(
                format!(
                    "the body of dimension alias `{name}` must be a dimension, found `{}`",
                    type_name(&ct)
                ),
                def.body.span(),
            )),
            Err(e) => errors.push(e),
        }
    }

    // The value namespace (`12-modules-and-imports.md`): imports, top-level
    // `let`s (both kinds), and the intrinsic base units form one namespace,
    // and a collision is an error, not a shadow (ADR 0027, Decision 3).  A
    // value name may not reuse a store or view name either: pipeline
    // positions resolve table names and expression positions resolve value
    // names, and one name in both would read ambiguously (and make constant
    // folding unsound).
    let mut table_names: HashSet<&str> = store_names.clone();
    for v in &views {
        table_names.insert(&v.name.name);
    }
    let mut value_names: HashSet<&str> = HashSet::new();
    for item in &program.items {
        let (name, what) = match item {
            Item::Import(i) => (&i.name, "import"),
            Item::Let(l) => (&l.name, "`let` binding"),
            _ => continue,
        };
        if crate::units::BASE_UNITS.contains(&name.name.as_str()) {
            errors.push(ResolveError::new(
                format!(
                    "`{}` is an intrinsic base unit and cannot be redeclared",
                    name.name
                ),
                name.span,
            ));
            continue;
        }
        // The expression-level builtins are ambient intrinsics (ADR 0027,
        // Decision 4) and join the collision rule: now that a binding can
        // be a function (ADR 0030), `let sum { |x| x }` would otherwise
        // silently shadow the aggregate in application head position.
        // The six aggregates have left (ADR 0031, Decision 8): they are const
        // bindings in `bag` now, so `sum`, `min`, `max`, `any`, `all`, and
        // `count` are ordinary names a user may declare.  The protection
        // shrinks with the intrinsics, and covers only what is still ambient.
        // ADR 0031 Decision 7 adds the ordered primitives and the `desc` order
        // marker; the derived window vocabulary (`cumsum`, `lag`, ...) stays out,
        // since it lives in the bundled `series` module.
        const EXPR_BUILTINS: [&str; 6] = ["to_real", "fold", "map", "scan", "prescan", "desc"];
        if EXPR_BUILTINS.contains(&name.name.as_str()) {
            errors.push(ResolveError::new(
                format!(
                    "`{}` is an ambient builtin and cannot be redeclared",
                    name.name
                ),
                name.span,
            ));
            continue;
        }
        if !value_names.insert(&name.name) {
            errors.push(ResolveError::new(
                format!("duplicate {what} `{}`", name.name),
                name.span,
            ));
        }
        if table_names.contains(name.name.as_str()) {
            errors.push(ResolveError::new(
                format!(
                    "{what} `{}` collides with a store or view of the same name",
                    name.name
                ),
                name.span,
            ));
        }
    }

    // Resolve imports: a bare import is bundled-only (ADR 0027, Decision 6).
    // A bundled module's own diagnostics carry no usable span here (spans
    // have no file identity yet), so they report at the import site.
    let mut modules: BTreeMap<String, &'static ModuleEnv> = BTreeMap::new();
    for i in &imports {
        match crate::modules::bundled(&i.name.name) {
            None => errors.push(ResolveError::new(
                format!(
                    "unknown module `{}`: a bare import resolves against the \
                     bundled modules only ({})",
                    i.name.name,
                    crate::modules::BUNDLED_MODULES
                        .iter()
                        .map(|m| format!("`{m}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                i.name.span,
            )),
            Some(Err(messages)) => {
                for message in messages {
                    errors.push(ResolveError::new(message.clone(), i.span));
                }
            }
            Some(Ok(env)) => {
                modules.insert(i.name.name.clone(), env);
            }
        }
    }

    // Evaluate the const bindings: order-independent and non-recursive, so
    // evaluation is demand-driven with cycle detection (`crate::consts`).
    let ascription = |ty: &TypeExpr| -> Result<ColumnType, ResolveError> {
        resolve_type(ty, &units, &enums, &dim_aliases)
    };
    let (const_values, const_errors) = eval_const_bindings(&const_decls, &modules, &ascription);
    errors.extend(const_errors);

    // The ambient value environment every expression site sees: the
    // intrinsics, the consts, and each module as a record of its members
    // (`si.km` then types through ordinary member access).
    let mut ambient = crate::expr_check::intrinsics();
    for (name, value) in &const_values {
        // A function binding has no expression type yet (ADR 0030): it is
        // skipped, so a view referencing it reports an unknown name until
        // the checker's function type lands.
        if let Some(ty) = value.ty() {
            ambient.insert(name.clone(), ty);
        }
    }
    for (name, env) in &modules {
        // `from_module` marks a member function's body as belonging to another
        // file, so a diagnostic from inside it reports at the call site rather
        // than at an offset into the module's source (spans have no file
        // identity yet, as the import diagnostics above also work around).
        let members = env
            .values
            .iter()
            .filter_map(|(n, v)| Some((n.clone(), v.ty()?.from_module())))
            .collect();
        ambient.insert(name.clone(), crate::expr_check::Ty::Record(members));
    }
    let subst = crate::lower::Subst::new(&const_values, &modules);

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
        match resolve_shape(sh, &units, &enums, &dim_aliases) {
            Ok(rs) => {
                resolved_shapes.insert(name, rs);
            }
            Err(mut errs) => errors.append(&mut errs),
        }
    }

    // Pass 3: resolve each store, then check the shapes it claims.
    let mut schemas = Vec::new();
    for s in &stores {
        match resolve_store(s, &units, &enums, &dim_aliases) {
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
        // Expression sites inside view bodies see the ambient environment:
        // the intrinsics, the top-level consts, and the imported modules.
        let mut sources = Sources::new().with_ambient(ambient.clone());
        for schema in &schemas {
            sources = sources.with(&schema.store, TableType::from_store(schema));
        }
        for v in &views {
            match type_view(&sources, &v.body) {
                Ok(output) => {
                    // Check the optional `: Shape` conformance clause against
                    // the view's computed output content (key + named
                    // columns by type/totality) and cardinality (10-views.md,
                    // ADR 0013, ADR 0022).
                    check_view_conformance(
                        v,
                        &output,
                        &shapes,
                        &resolved_shapes,
                        &units,
                        &enums,
                        &dim_aliases,
                        &mut errors,
                    );
                    // Constant-fold the body before it reaches the
                    // runtime (`crate::lower`).
                    let mut plan = view_plan(v, &output, &store_names);
                    crate::lower::lower_view_body(&mut plan.body, &subst);
                    view_plans.push(plan);
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
    for c in &output.content.key {
        columns.push(Column {
            name: c.name.clone(),
            ty: c.domain.clone(),
            role: ColumnRole::Key,
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
        // A combiner names an operator from the closed table, never a store.
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Str(_)
        | ExprKind::Bool(_)
        | ExprKind::Combiner(_) => {}
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
    aliases: &DimAliases,
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

    let cardinality = declared_cardinality(&s.attrs, "store", &mut errors);

    // Columns in storage order: key fields, then attributes in declaration
    // order.  Compound units surface here: an key field whose type
    // references another unit is rejected by `resolve_type`.
    let mut columns = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for f in &unit.fields {
        add_column(
            &mut columns,
            &mut seen,
            &mut errors,
            f,
            ColumnRole::Key,
            units,
            enums,
            aliases,
        );
    }
    for a in &s.attrs {
        add_column(
            &mut columns,
            &mut seen,
            &mut errors,
            &a.field,
            ColumnRole::Attr,
            units,
            enums,
            aliases,
        );
    }

    if errors.is_empty() {
        Ok(Schema {
            store: s.name.name.clone(),
            unit: s.unit.name.clone(),
            columns,
            cardinality,
            span: s.span,
        })
    } else {
        Err(errors)
    }
}

/// The cardinality a store or shape declares through its attribute blocks
/// (ADR 0022): `Singletons` when every attribute is `attr`, `Bag` when every
/// attribute is `attr*`.  Mixing the two is the ADR's deferred refinement (a
/// singleton column inside a `bag` store needs bag-construction syntax that
/// does not exist yet), so it is rejected here; per-entity constants belong
/// in a companion `singletons` store joined via `domain`.  `what` names the
/// construct for the diagnostic ("store" or "shape").
fn declared_cardinality(attrs: &[Attr], what: &str, errors: &mut Vec<ResolveError>) -> Cardinality {
    let many = attrs.iter().filter(|a| a.many.is_some()).count();
    if many == 0 {
        Cardinality::Singletons
    } else if many == attrs.len() {
        Cardinality::Bag
    } else {
        let star = attrs
            .iter()
            .find_map(|a| a.many)
            .expect("a mixed attribute list has an `attr*` block");
        errors.push(ResolveError::new(
            format!(
                "a {what} cannot mix `attr` and `attr*` blocks: an `attr` column \
                 inside a `bag` {what} is not yet supported; keep per-entity \
                 constants in a companion `singletons` store (ADR 0022)"
            ),
            star,
        ));
        Cardinality::Bag
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
/// how it constrains the unit, its attributes, and the cardinality it
/// demands of a conforming table (ADR 0022, amending ADR 0012).  Attribute
/// names are kept as templates because they are not concrete until a claim
/// binds the shape's `string` parameters.
struct ResolvedShape {
    params: Vec<(String, ParamKind)>,
    unit: ShapeUnit,
    attrs: Vec<ResolvedAttr>,
    /// `Singletons` when the shape has no `attr*` block (the strict reading:
    /// an all-`attr` shape requires a `singletons` target), `Bag` when its
    /// attributes are `attr*`.
    cardinality: Cardinality,
}

fn resolve_shape(
    sh: &ShapeDecl,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
    aliases: &DimAliases,
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

    // The cardinality the shape demands of a conforming table (ADR 0022):
    // computed from its `attr` / `attr*` blocks exactly as for a store.
    let cardinality = declared_cardinality(&sh.attrs, "shape", &mut errors);

    // Attributes.  A name may interpolate `string` parameters; its type must
    // be a primitive or enum (compound types stay deferred via `resolve_type`).
    let mut attrs = Vec::new();
    let mut seen_literals: HashSet<&str> = HashSet::new();
    for a in &sh.attrs {
        let f = &a.field;
        for seg in &f.name.segments {
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
        if let Some(lit) = f.name.as_literal() {
            check_case(lit, f.name.span, Case::Snake, "attribute", &mut errors);
            if !seen_literals.insert(lit) {
                errors.push(ResolveError::new(
                    format!("duplicate attribute `{lit}`"),
                    f.name.span,
                ));
            }
        }
        match resolve_type(&f.ty, units, enums, aliases) {
            Ok(ty) => attrs.push(ResolvedAttr {
                name: f.name.clone(),
                ty,
                optional: f.ty.is_optional(),
            }),
            Err(e) => errors.push(e),
        }
    }

    if errors.is_empty() {
        Ok(ResolvedShape {
            params,
            unit,
            attrs,
            cardinality,
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

        // Cardinality (ADR 0022, amending ADR 0012): a shape with no `attr*`
        // requires a `singletons` target; `attr*` attributes require the
        // columns be bag-valued, which in the uniform-store scope means a
        // `bag` store.
        if schema.cardinality != shape.cardinality {
            errors.push(ResolveError::new(
                format!(
                    "store `{}` claims `{}`, which requires a `{}` tabulation \
                     ({}), but the store is `{}`",
                    s.name.name,
                    shape_ref_label(claim),
                    cardinality_word(shape.cardinality),
                    attr_block_word(shape.cardinality),
                    cardinality_word(schema.cardinality)
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
/// shape").  Unlike a store, a view has no declared unit, so the structural
/// check is content-based:
///
/// - a unit-fixing shape requires the output's key to be exactly the unit's
///   key fields, by name and type;
/// - each shape attribute must appear among the output columns, key or
///   non-key, with a matching type and totality;
/// - the shape's cardinality (its `attr` / `attr*` blocks, ADR 0022 amending
///   ADR 0012) must match the output's computed cardinality: an all-`attr`
///   shape requires `singletons`, an `attr*` shape requires `bag`.
#[allow(clippy::too_many_arguments)]
fn check_view_conformance(
    view: &ViewDecl,
    output: &TableType,
    shapes: &HashMap<&str, &ShapeDecl>,
    resolved: &HashMap<&str, ResolvedShape>,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
    aliases: &DimAliases,
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

        // A unit-fixing shape constrains the output's key structurally: a
        // view carries no declared unit, so conformance is that the output's
        // key columns are exactly the unit's key fields (10-views.md, "The
        // unit clause").
        let required_unit = match &shape.unit {
            ShapeUnit::Agnostic => None,
            ShapeUnit::Concrete(u) => Some(u.as_str()),
            ShapeUnit::Param(p) => Some(unit_bind[p.as_str()]),
        };
        if let Some(uname) = required_unit
            && let Some(unit) = units.get(uname).copied()
        {
            let expected = unit_key_columns(unit, units, enums, aliases);
            let actual: Vec<(String, ColumnType)> = output
                .content
                .key
                .iter()
                .map(|c| (c.name.clone(), c.domain.clone()))
                .collect();
            if !same_columns(&expected, &actual) {
                errors.push(ResolveError::new(
                    format!(
                        "view `{}` claims `{}`, which tabulates `{}`, but its key is {} rather than {}",
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

        // Cardinality (ADR 0022, amending ADR 0012): the shape's `attr` /
        // `attr*` blocks constrain the output's computed cardinality.
        if output.qualifiers.cardinality != shape.cardinality {
            errors.push(ResolveError::new(
                format!(
                    "view `{}` claims `{}`, which requires a `{}` table ({}), \
                     but the view's output is `{}`",
                    view.name.name,
                    shape_ref_label(claim),
                    cardinality_word(shape.cardinality),
                    attr_block_word(shape.cardinality),
                    cardinality_word(output.qualifiers.cardinality)
                ),
                claim.span,
            ));
            continue;
        }

        // Content attributes: each must appear somewhere in the output (key
        // or non-key), with a matching type and totality.
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
                .key
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

/// Resolve a unit's key fields to their `(name, type)` pairs, for structural
/// conformance of a view claiming a unit-fixing shape.  Fields that fail to
/// resolve (for example a still-unsupported compound field) are skipped: such
/// a unit cannot back a store either, and its error surfaces where it is used.
fn unit_key_columns(
    unit: &UnitDecl,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
    aliases: &DimAliases,
) -> Vec<(String, ColumnType)> {
    let mut out = Vec::new();
    for f in &unit.fields {
        if let (Ok(name), Ok(ty)) = (
            literal_field_name(&f.name),
            resolve_type(&f.ty, units, enums, aliases),
        ) {
            out.push((name, ty));
        }
    }
    out
}

/// Two column lists match iff they carry the same names, each with the same
/// type.  Order is not significant: a view's key is a set of key columns, and
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

/// The cardinality axis as a word for diagnostics (ADR 0022).
fn cardinality_word(c: Cardinality) -> &'static str {
    match c {
        Cardinality::Singletons => "singletons",
        Cardinality::Bag => "bag",
    }
}

/// The attribute-block spelling that declares a cardinality, for diagnostics
/// (ADR 0022).
fn attr_block_word(c: Cardinality) -> &'static str {
    match c {
        Cardinality::Singletons => "its attributes are `attr`",
        Cardinality::Bag => "its attributes are `attr*`",
    }
}

pub(crate) fn type_name(ty: &ColumnType) -> String {
    match ty {
        ColumnType::String => "string".into(),
        ColumnType::Int => "int".into(),
        ColumnType::Real => "real".into(),
        ColumnType::Quantity(dim) => dim.type_name(),
        ColumnType::Bool => "bool".into(),
        ColumnType::Date => "date".into(),
        ColumnType::Enum { name, .. } => name.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn add_column(
    columns: &mut Vec<Column>,
    seen: &mut HashSet<String>,
    errors: &mut Vec<ResolveError>,
    field: &Field,
    role: ColumnRole,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
    aliases: &DimAliases,
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
    if role != ColumnRole::Key {
        check_case(&name, field.name.span, Case::Snake, "attribute", errors);
    }
    // An key field is always known: whether the row exists at all is
    // cardinality, a separate axis from value missingness (ADR 0010).  `?` on
    // an key field is rejected; an attribute may be optional.
    let is_key = role == ColumnRole::Key;
    if is_key && let Some(span) = field.ty.optional {
        errors.push(ResolveError::new(
            format!("an key field cannot be optional: drop the `?` on `{name}`"),
            span,
        ));
    }
    let ct = match resolve_type(&field.ty, units, enums, aliases) {
        Ok(ct) => ct,
        Err(e) => {
            errors.push(e);
            return;
        }
    };
    // A key is identified by equality, so an key field must be key-eligible
    // (ADR 0014); `real` is a continuous measurement, not an identity.
    if is_key && !ct.is_key_eligible() {
        errors.push(ResolveError::new(
            format!(
                "an key field must be a key-eligible type: `{}` cannot be a key",
                type_name(&ct)
            ),
            field.ty.span(),
        ));
    }
    columns.push(Column {
        name,
        ty: ct,
        role,
        optional: field.ty.is_optional() && !is_key,
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

/// Collect one dimension alias declaration into the alias namespace
/// (ADR 0026, Decision 8): a lowercase name that does not shadow the
/// built-in type vocabulary, exactly one PascalCase backing parameter, and
/// no duplicates.  The body is validated after pass 1, once every alias is
/// known.
fn collect_dim_alias<'a>(
    name: &'a Ident,
    params: &'a [Ident],
    body: &'a TypeExpr,
    aliases: &mut DimAliases<'a>,
    errors: &mut Vec<ResolveError>,
) {
    check_case(
        &name.name,
        name.span,
        Case::Snake,
        "dimension alias",
        errors,
    );
    let builtin = Dimension::base(&name.name).is_some()
        || matches!(
            name.name.as_str(),
            "string" | "int" | "real" | "bool" | "date"
        );
    if builtin {
        errors.push(ResolveError::new(
            format!(
                "`{}` is a built-in type name and cannot be redeclared",
                name.name
            ),
            name.span,
        ));
        return;
    }
    let [param] = params else {
        errors.push(ResolveError::new(
            "a dimension alias takes exactly one backing parameter",
            name.span,
        ));
        return;
    };
    check_case(
        &param.name,
        param.span,
        Case::Pascal,
        "alias parameter",
        errors,
    );
    // A duplicate alias name is reported by the value-namespace pass (all
    // `let`s and imports share it), so the insert may overwrite silently.
    aliases.insert(
        &name.name,
        DimAliasDef {
            param: &param.name,
            body,
        },
    );
}

/// A collected dimension alias (`let name[T] = <type-level expr>`, ADR 0026
/// Decision 8): its single backing parameter and its unexpanded body.
/// Aliases are transparent, fully applied, and non-recursive; expansion
/// happens in `eval_tl` and a cycle is a diagnostic.
pub(crate) struct DimAliasDef<'a> {
    pub param: &'a str,
    pub body: &'a TypeExpr,
}

pub(crate) type DimAliases<'a> = HashMap<&'a str, DimAliasDef<'a>>;

/// What a type-level expression evaluates to
/// (`docs/language/11-physical-units.md`).
enum TlValue {
    /// A plain scalar domain (`string`, `int`, an enum, ...).
    Plain(ColumnType),
    /// A bare dimension: not a column type until applied to a backing.
    Dim(Dimension),
    /// A dimension applied to a backing.
    Applied { dim: Dimension, backing: TlBacking },
}

/// The backing slot of an applied dimension: `real` today, or an alias's
/// backing parameter inside its body.
#[derive(Clone, PartialEq)]
enum TlBacking {
    Real,
    Param(String),
}

/// The environment a type-level expression evaluates in.
struct TlEnv<'a> {
    units: &'a HashMap<&'a str, &'a UnitDecl>,
    enums: &'a HashMap<&'a str, &'a EnumDecl>,
    aliases: &'a DimAliases<'a>,
    /// Inside an alias body: the backing parameter's name and what it
    /// stands for at this expansion (the actual backing when the alias is
    /// applied, or itself when the declaration is being validated).
    param: Option<(&'a str, TlBacking)>,
}

/// Resolve a type expression against the built-in vocabulary only (no
/// units, enums, or aliases in scope): the environment a bundled module's
/// ascriptions see (`crate::modules`).
pub(crate) fn resolve_type_builtin(ty: &TypeExpr) -> Result<ColumnType, ResolveError> {
    resolve_type(ty, &HashMap::new(), &HashMap::new(), &HashMap::new())
}

fn resolve_type(
    ty: &TypeExpr,
    units: &HashMap<&str, &UnitDecl>,
    enums: &HashMap<&str, &EnumDecl>,
    aliases: &DimAliases,
) -> Result<ColumnType, ResolveError> {
    // Resolve only the base type here; optionality (`?`) is read from the
    // `TypeExpr` by the caller, which knows the column's role (an key field
    // may not be optional; ADR 0010).
    let env = TlEnv {
        units,
        enums,
        aliases,
        param: None,
    };
    let mut stack = Vec::new();
    match eval_tl(ty, &env, &mut stack)? {
        TlValue::Plain(ct) => Ok(ct),
        TlValue::Applied {
            dim,
            backing: TlBacking::Real,
        } => Ok(dim.applied()),
        // Unreachable from a column position (`param` is `None`), kept as a
        // diagnostic rather than a panic.
        TlValue::Applied { .. } => Err(ResolveError::new(
            "an alias parameter cannot appear outside its alias body",
            ty.span(),
        )),
        TlValue::Dim(d) => Err(ResolveError::new(
            format!(
                "a dimension is not a column type by itself: apply it to a backing, `{}`",
                d.type_name()
            ),
            ty.span(),
        )),
    }
}

/// Evaluate a type-level expression to a [`TlValue`], expanding aliases
/// transparently.  `stack` carries the alias-expansion chain for cycle
/// detection.
fn eval_tl(ty: &TypeExpr, env: &TlEnv, stack: &mut Vec<String>) -> Result<TlValue, ResolveError> {
    match &ty.kind {
        TypeKind::Named(id) => eval_tl_name(id, env),
        TypeKind::Apply { base, backing } => {
            let b = eval_backing(backing, env)?;
            match &base.kind {
                TypeKind::Named(id) => apply_named(id, b, env, stack),
                _ => match eval_tl(base, env, stack)? {
                    TlValue::Dim(d) => Ok(TlValue::Applied { dim: d, backing: b }),
                    TlValue::Applied { .. } => Err(ResolveError::new(
                        "this type is already applied to a backing",
                        base.span(),
                    )),
                    TlValue::Plain(ct) => Err(ResolveError::new(
                        format!("`{}` is not a dimension", type_name(&ct)),
                        base.span(),
                    )),
                },
            }
        }
        TypeKind::Mul(a, b) | TypeKind::Div(a, b) => {
            let mul = matches!(ty.kind, TypeKind::Mul(..));
            let lhs = eval_tl(a, env, stack)?;
            let rhs = eval_tl(b, env, stack)?;
            combine_tl(lhs, rhs, mul, a.span(), b.span())
        }
        TypeKind::Pow(base, n) => match eval_tl(base, env, stack)? {
            TlValue::Dim(d) => Ok(TlValue::Dim(d.pow(*n))),
            TlValue::Applied { dim, backing } => Ok(TlValue::Applied {
                dim: dim.pow(*n),
                backing,
            }),
            TlValue::Plain(ct) => Err(ResolveError::new(
                format!("`{}` is not a dimension", type_name(&ct)),
                base.span(),
            )),
        },
    }
}

/// A lone identifier in type position: a base dimension, an alias (which
/// must be applied), a primitive, an enum, or a unit reference.
fn eval_tl_name(id: &Ident, env: &TlEnv) -> Result<TlValue, ResolveError> {
    let name = id.name.as_str();
    if let Some((p, _)) = &env.param
        && *p == name
    {
        return Err(ResolveError::new(
            format!(
                "`{name}` is this alias's backing parameter: it can appear only inside `[...]`"
            ),
            id.span,
        ));
    }
    if let Some(d) = Dimension::base(name) {
        return Ok(TlValue::Dim(d));
    }
    if env.aliases.contains_key(name) {
        return Err(ResolveError::new(
            format!("dimension alias `{name}` must be applied to a backing: write `{name}[real]`"),
            id.span,
        ));
    }
    match name {
        "string" => Ok(TlValue::Plain(ColumnType::String)),
        "int" => Ok(TlValue::Plain(ColumnType::Int)),
        "real" => Ok(TlValue::Plain(ColumnType::Real)),
        "bool" => Ok(TlValue::Plain(ColumnType::Bool)),
        "date" => Ok(TlValue::Plain(ColumnType::Date)),
        other if env.enums.contains_key(other) => {
            let e = env.enums[other];
            Ok(TlValue::Plain(ColumnType::Enum {
                name: e.name.name.clone(),
                variants: e.variants.iter().map(|v| v.value.clone()).collect(),
            }))
        }
        other if env.units.contains_key(other) => Err(ResolveError::new(
            format!("compound fields are not yet supported (references unit `{other}`)"),
            id.span,
        )),
        other => Err(ResolveError::new(
            format!("unknown type `{other}`"),
            id.span,
        )),
    }
}

/// Apply a named base to a backing: `temperature[real]` (a base dimension)
/// or `speed[real]` (an alias, expanded transparently with its parameter
/// bound to the actual backing).
fn apply_named(
    id: &Ident,
    backing: TlBacking,
    env: &TlEnv,
    stack: &mut Vec<String>,
) -> Result<TlValue, ResolveError> {
    let name = id.name.as_str();
    if let Some(d) = Dimension::base(name) {
        return Ok(TlValue::Applied { dim: d, backing });
    }
    if let Some(alias) = env.aliases.get(name) {
        if stack.iter().any(|n| n == name) {
            return Err(ResolveError::new(
                format!("recursive dimension alias `{name}`"),
                id.span,
            ));
        }
        stack.push(name.to_string());
        let inner = TlEnv {
            units: env.units,
            enums: env.enums,
            aliases: env.aliases,
            param: Some((alias.param, backing.clone())),
        };
        let value = eval_tl(alias.body, &inner, stack);
        stack.pop();
        return match value? {
            // A body that never mentions its parameter is a bare dimension;
            // the application supplies the backing.
            TlValue::Dim(d) => Ok(TlValue::Applied { dim: d, backing }),
            applied @ TlValue::Applied { .. } => Ok(applied),
            TlValue::Plain(ct) => Err(ResolveError::new(
                format!(
                    "the body of dimension alias `{name}` is `{}`, not a dimension",
                    type_name(&ct)
                ),
                id.span,
            )),
        };
    }
    Err(ResolveError::new(
        format!("`{name}` is not a dimension, so it cannot take a backing"),
        id.span,
    ))
}

/// The backing slot of a type application: `real`, or the enclosing alias's
/// backing parameter.
fn eval_backing(id: &Ident, env: &TlEnv) -> Result<TlBacking, ResolveError> {
    let name = id.name.as_str();
    if let Some((p, b)) = &env.param
        && *p == name
    {
        return Ok(b.clone());
    }
    match name {
        "real" => Ok(TlBacking::Real),
        "int" => Err(ResolveError::new(
            "`int` is never dimensioned (ADR 0014): the backing of a dimension must be `real`",
            id.span,
        )),
        other => Err(ResolveError::new(
            format!("the backing of a dimension must be `real`, found `{other}`"),
            id.span,
        )),
    }
}

/// Combine two type-level operands under `*` or `/`: dimensions combine
/// with dimensions, applied types with same-backing applied types.
fn combine_tl(
    lhs: TlValue,
    rhs: TlValue,
    mul: bool,
    lspan: Span,
    rspan: Span,
) -> Result<TlValue, ResolveError> {
    let combine = |a: Dimension, b: Dimension| if mul { a * b } else { a / b };
    match (lhs, rhs) {
        (TlValue::Dim(a), TlValue::Dim(b)) => Ok(TlValue::Dim(combine(a, b))),
        (
            TlValue::Applied {
                dim: a,
                backing: ba,
            },
            TlValue::Applied {
                dim: b,
                backing: bb,
            },
        ) => {
            if ba == bb {
                Ok(TlValue::Applied {
                    dim: combine(a, b),
                    backing: ba,
                })
            } else {
                Err(ResolveError::new(
                    "the two sides of a type-level `*`/`/` have different backings",
                    rspan,
                ))
            }
        }
        (TlValue::Plain(ct), _) => Err(ResolveError::new(
            format!("`{}` is not a dimension", type_name(&ct)),
            lspan,
        )),
        (_, TlValue::Plain(ct)) => Err(ResolveError::new(
            format!("`{}` is not a dimension", type_name(&ct)),
            rspan,
        )),
        _ => Err(ResolveError::new(
            "cannot mix an applied type and a bare dimension in one type-level expression",
            rspan,
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
          machines |> flat_map |_, r| if r.status == "degraded" then r else ()
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

        // Output columns in storage order: the key, then the computed
        // attributes in the order the checker produced them (a whole-row
        // `flat_map` body yields them alphabetically).
        let cols: Vec<(&str, ColumnRole, bool)> = plan
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.role, c.optional))
            .collect();
        assert_eq!(
            cols,
            vec![
                ("id", ColumnRole::Key, false),
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
              machines |> flat_map |_, r| r
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
              machines |> flat_map |_, r| r
            }
            view v {
              machines |> flat_map |_, r| r
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
                ("id", ColumnRole::Key, &ColumnType::String),
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
        // `id` is both the key field and a declared attribute.
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
        // The key is total even though `?` was not (and may not be) written.
        assert!(!by("id").optional);
    }

    #[test]
    fn optional_index_field_is_rejected() {
        // Whether a row exists is cardinality, not value missingness; `?` on an
        // key field is a hard error (ADR 0010).
        let src = r#"
            unit Machine { id: string? }
            store readings { unit { Machine } }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("key field cannot be optional"))
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
        // compilable subset must keep resolving.  Today it declares two
        // singletons stores: `machines` and the `readings` history keyed by
        // `(machine_id, taken_at)` that the views `demote` (ADR 0024).
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/examples/fleet-monitoring.mensura");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let schemas = resolve_str(&src).expect("example should resolve");
        assert_eq!(schemas.len(), 2);
        let by = |n: &str| schemas.iter().find(|s| s.store == n).unwrap();
        assert_eq!(
            by("machines").cardinality,
            crate::table::Cardinality::Singletons
        );
        assert_eq!(
            by("readings").cardinality,
            crate::table::Cardinality::Singletons
        );
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
        // An key field is checked once, at the unit, and only once even when
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
        assert_eq!(casing.len(), 1, "key field casing reported exactly once");
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
            import bag
            unit Machine { id: string }
            store readings { unit { Machine } attr { temperature: real } }
            view machine_summary { readings |> map_bags |k, b| (.temp_max = bag.max b.temperature) }
        "#;
        resolve_str(ok).expect("a valid view resolves");

        let bad = r#"
            unit Machine { id: string }
            store readings { unit { Machine } attr { temperature: real } }
            view bad { readings |> map_bags |k, b| (.x = b.temperature + 1.0) }
        "#;
        let errs = errors(bad);
        assert!(errs.iter().any(|e| e.message.contains("bag")));
    }

    // A summarizing view whose output key is `Machine`'s key and whose one
    // non-key column is `temp_max: real` (`docs/language/10-views.md`).
    const SUMMARY_VIEW: &str = r#"
        import bag
        unit Machine { id: string }
        store readings { unit { Machine } attr { temperature: real } }
    "#;

    #[test]
    fn view_conforms_to_unit_fixing_shape() {
        // A unit-fixing shape checks the output's key structurally: the view
        // ends in `map_bags`, so its key is `Machine`'s `id`.
        let src = format!(
            "{SUMMARY_VIEW}
            shape Tabular[U: Unit] {{ unit {{ U }} }}
            view machine_summary : Tabular[Machine] {{
              readings |> map_bags |k, b| (.temp_max = bag.max b.temperature)
            }}"
        );
        resolve_str(&src).expect("view carrying Machine's key conforms");
    }

    #[test]
    fn view_with_wrong_unit_index_is_rejected() {
        // The view's key is `id`, but `Site`'s key is `code`, so a claim of
        // `Tabular[Site]` fails the structural key check.
        let src = format!(
            "{SUMMARY_VIEW}
            unit Site {{ code: string }}
            shape Tabular[U: Unit] {{ unit {{ U }} }}
            view v : Tabular[Site] {{
              readings |> map_bags |k, b| (.temp_max = bag.max b.temperature)
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
              readings |> map_bags |k, b| (.temp_max = bag.max b.temperature)
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
              readings |> map_bags |k, b| (.temp_max = bag.max b.temperature)
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
              readings |> map_bags |k, b| (.temp_max = bag.max b.temperature)
            }}"
        );
        let errs = errors(&src);
        assert!(errs.iter().any(|e| {
            e.message
                .contains("type `string` in the shape but `real` in the view")
        }));
    }

    // --- Store cardinality (ADR 0022) ---

    // A bag store: the machine is the entity, its readings recur.
    const BAG_READINGS: &str = r#"
        import bag
        unit Machine { id: string }
        store readings {
          unit { Machine }
          attr* {
            kelvin: real
            rpm:    int
          }
        }
    "#;

    #[test]
    fn bag_store_resolves_with_bag_cardinality() {
        let schemas = resolve_str(BAG_READINGS).expect("should resolve");
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].cardinality, crate::table::Cardinality::Bag);
        // The storage mapping follows: no primary key for a bag store.
        assert!(!schemas[0].shape().keyed);
    }

    #[test]
    fn plain_store_stays_singletons() {
        // The default is unchanged (ADR 0001): no `attr*` block, `card <= 1`.
        let src = r#"
            unit Machine { id: string }
            store machines { unit { Machine } attr { commissioned: date } }
        "#;
        let schemas = resolve_str(src).expect("should resolve");
        assert_eq!(
            schemas[0].cardinality,
            crate::table::Cardinality::Singletons
        );
        assert!(schemas[0].shape().keyed);
    }

    #[test]
    fn mixed_attr_blocks_are_rejected() {
        // ADR 0022's deferred refinement: an `attr` column inside a `bag`
        // store needs bag-construction syntax that does not exist yet.
        let src = r#"
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr  { location: string }
              attr* { kelvin: real }
            }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("cannot mix `attr` and `attr*`")),
            "expected the mixed-cardinality rejection, got: {errs:?}"
        );
    }

    #[test]
    fn reducing_view_over_a_bag_store_demands_completeness() {
        // A bag store's groups can be partial, so the ADR 0023 reducer
        // obligation bites; `assume { complete }` (or a future source-level
        // fact) discharges it.
        let bad = format!(
            "{BAG_READINGS}
            view stats {{ readings |> map_bags |k, b| (.max_kelvin = bag.max b.kelvin) }}"
        );
        let errs = errors(&bad);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("reducing `map_bags`")),
            "expected the reducer completeness demand, got: {errs:?}"
        );

        let ok = format!(
            "{BAG_READINGS}
            view stats {{
              readings |> assume {{ complete }}
                       |> map_bags |k, b| (.max_kelvin = bag.max b.kelvin)
            }}"
        );
        let program = resolve_program(&ok).expect("assume discharges the reducer");
        assert_eq!(
            program.views[0].cardinality,
            crate::table::Cardinality::Singletons
        );
    }

    #[test]
    fn split_on_a_bag_store_routes_whole_entities() {
        // The point of the entity-keyed bag (ADR 0022): `split` hashes the
        // entity key, so tracked disjointness coincides with the leakage
        // boundary and the split re-binds losslessly.
        let src = format!(
            "{BAG_READINGS}
            view roundtrip {{
              let parts = readings |> split |k| k.id == \"m1\";
              parts |> union
            }}"
        );
        let program = resolve_program(&src).expect("should resolve");
        assert_eq!(program.views[0].cardinality, crate::table::Cardinality::Bag);
    }

    // --- Shapes constrain cardinality (ADR 0022, amending ADR 0012) ---

    #[test]
    fn bag_shape_conforms_to_bag_store() {
        let src = r#"
            unit Machine { id: string }
            shape SensorLog { attr* { kelvin: real } }
            store readings : SensorLog {
              unit { Machine }
              attr* { kelvin: real }
            }
        "#;
        assert_eq!(resolve_str(src).expect("should resolve").len(), 1);
    }

    #[test]
    fn all_attr_shape_rejects_a_bag_store() {
        // The strict reading: a shape with no `attr*` requires `singletons`.
        let src = r#"
            unit Machine { id: string }
            shape Calibrated { attr { kelvin: real } }
            store readings : Calibrated {
              unit { Machine }
              attr* { kelvin: real }
            }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| {
            e.message.contains("requires a `singletons` tabulation")
                && e.message.contains("but the store is `bag`")
        }));
    }

    #[test]
    fn bag_shape_rejects_a_singletons_store() {
        let src = r#"
            unit Machine { id: string }
            shape SensorLog { attr* { kelvin: real } }
            store calibration : SensorLog {
              unit { Machine }
              attr { kelvin: real }
            }
        "#;
        let errs = errors(src);
        assert!(errs.iter().any(|e| {
            e.message.contains("requires a `bag` tabulation")
                && e.message.contains("but the store is `singletons`")
        }));
    }

    #[test]
    fn mixed_shape_blocks_are_rejected() {
        let src = r#"
            shape Confused {
              attr  { a: int }
              attr* { b: int }
            }
        "#;
        let errs = errors(src);
        assert!(
            errs.iter()
                .any(|e| e.message.contains("cannot mix `attr` and `attr*`"))
        );
    }

    #[test]
    fn view_cardinality_is_checked_against_the_claimed_shape() {
        // A bag-shaped view satisfies an `attr*` shape and fails an
        // all-`attr` one; the singletons summary is the mirror image.
        let ok = format!(
            "{BAG_READINGS}
            shape SensorLog {{ attr* {{ kelvin: real }} }}
            view log : SensorLog {{ readings |> flat_map |k, r| r }}"
        );
        resolve_program(&ok).expect("a bag view satisfies an attr* shape");

        let bad = format!(
            "{BAG_READINGS}
            shape Calibrated {{ attr {{ kelvin: real }} }}
            view log : Calibrated {{ readings |> flat_map |k, r| r }}"
        );
        let errs = errors(&bad);
        assert!(errs.iter().any(|e| {
            e.message.contains("requires a `singletons` table")
                && e.message.contains("but the view's output is `bag`")
        }));
    }

    // ADR 0026 (`11-physical-units.md`): dimensioned column types and
    // dimension aliases.

    fn quantity(dim: &str) -> ColumnType {
        ColumnType::Quantity(Dimension::base(dim).unwrap())
    }

    #[test]
    fn dimensioned_columns_resolve() {
        let schemas = resolve_str(
            r#"
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* {
                temperature: temperature[real]
                vibration:   (length / time^2)[real]
              }
            }
        "#,
        )
        .expect("should resolve");
        let cols = &schemas[0].columns;
        assert_eq!(cols[1].ty, quantity("temperature"));
        let accel = Dimension::base("length").unwrap() / Dimension::base("time").unwrap().pow(2);
        assert_eq!(cols[2].ty, ColumnType::Quantity(accel));
    }

    #[test]
    fn dimension_aliases_expand_transparently() {
        let schemas = resolve_str(
            r#"
            let speed[T] { (length / time)[T] }
            let accel[T] { speed[T] / time[T] }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* {
                vibration: accel[real]
                velocity:  speed[real]
              }
            }
        "#,
        )
        .expect("should resolve");
        let cols = &schemas[0].columns;
        let length = Dimension::base("length").unwrap();
        let time = Dimension::base("time").unwrap();
        assert_eq!(cols[1].ty, ColumnType::Quantity(length / time.pow(2)));
        assert_eq!(cols[2].ty, ColumnType::Quantity(length / time));
    }

    #[test]
    fn dimension_type_errors_are_pointed() {
        // A bare dimension is not a column type.
        let errs = errors("unit U { id: string } store s { unit { U } attr { t: temperature } }");
        assert!(errs.iter().any(|e| {
            e.message
                .contains("apply it to a backing, `temperature[real]`")
        }));
        // `int` is never dimensioned.
        let errs =
            errors("unit U { id: string } store s { unit { U } attr { t: temperature[int] } }");
        assert!(errs.iter().any(|e| e.message.contains("never dimensioned")));
        // The backing must be `real`.
        let errs =
            errors("unit U { id: string } store s { unit { U } attr { t: temperature[bogus] } }");
        assert!(errs.iter().any(|e| {
            e.message
                .contains("the backing of a dimension must be `real`")
        }));
        // An alias must be applied.
        let errs = errors(
            "let speed[T] { (length / time)[T] }
             unit U { id: string } store s { unit { U } attr { v: speed } }",
        );
        assert!(errs.iter().any(|e| {
            e.message
                .contains("must be applied to a backing: write `speed[real]`")
        }));
        // Only dimensions combine under the type-level operators.
        let errs = errors(
            "unit U { id: string } store s { unit { U } attr { t: (string / time)[real] } }",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("`string` is not a dimension"))
        );
    }

    #[test]
    fn dimension_alias_declarations_are_validated() {
        // Recursive aliases are cycles.
        let errs = errors(
            "let a[T] { b[T] }
             let b[T] { a[T] }",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("recursive dimension alias"))
        );
        // Exactly one backing parameter.
        let errs = errors("let speed[T, U] { (length / time)[T] }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("exactly one backing parameter"))
        );
        // An alias may not shadow the built-in type vocabulary.
        let errs = errors("let length[T] { (time / time)[T] }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("built-in type name"))
        );
        // Alias names are lowercase; parameters are PascalCase.
        let errs = errors("let Speed[T] { (length / time)[T] }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("must be snake_case"))
        );
    }

    #[test]
    fn dimensioned_key_field_is_rejected() {
        // A dimensioned real is a continuous measurement, not an identity.
        let errs = errors(
            "unit Probe { depth: length[real] }
             store probes { unit { Probe } attr { note: string } }",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("`length[real]` cannot be a key"))
        );
    }

    #[test]
    fn view_preserves_column_dimensions() {
        // The dimension rides the domain through the pipeline checker: a
        // `max` rollup of a dimensioned column stays dimensioned, and unit
        // intrinsics type inside view bodies.
        let program = resolve_program(
            r#"
            import bag
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { temperature: temperature[real] }
            }
            view hottest {
              readings |> assume { complete }
                       |> map_bags |_, b| (.max_temperature = bag.max b.temperature)
            }
        "#,
        )
        .expect("should resolve");
        let plan = &program.views[0];
        let max_t = plan
            .columns
            .iter()
            .find(|c| c.name == "max_temperature")
            .expect("the rollup column");
        assert_eq!(max_t.ty, quantity("temperature"));
    }

    #[test]
    fn view_bodies_see_the_intrinsic_units() {
        // `350.0 * kelvin` types inside a `flat_map` body, and a
        // cross-dimension comparison is a compile error.
        let ok = r#"
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { temperature: temperature[real] }
            }
            view hot {
              readings |> flat_map |_, r| if r.temperature > 350.0 * kelvin then r else ()
            }
        "#;
        resolve_program(ok).expect("should resolve");
        let bad = r#"
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { temperature: temperature[real] }
            }
            view hot {
              readings |> flat_map |_, r| if r.temperature > 350.0 * second then r else ()
            }
        "#;
        let errs = errors(bad);
        assert!(errs.iter().any(|e| e.message.contains("same type")));
    }

    // ADR 0027 (`12-modules-and-imports.md`): imports, top-level consts,
    // and constant lowering.

    #[test]
    fn imports_and_consts_resolve_and_lower() {
        let program = resolve_program(
            r#"
            import si
            let limit { 350.0 * kelvin }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* {
                temperature: temperature[real]
                distance:    length[real]
              }
            }
            view hot {
              readings |> flat_map |_, r|
                if r.temperature > limit and r.distance > 2.0 * si.km then r else ()
            }
        "#,
        )
        .expect("should resolve");
        // The lowered body carries only literals for the consts: `limit`
        // folded to 350.0, `si.km` to 1000.0, and no name references remain.
        let body = format!("{:?}", program.views[0].body);
        assert!(body.contains("Float(350.0)"), "limit folds: {body}");
        assert!(body.contains("Float(1000.0)"), "si.km folds: {body}");
        assert!(
            !body.contains("\"si\""),
            "no module reference remains: {body}"
        );
        assert!(
            !body.contains("\"limit\""),
            "no const reference remains: {body}"
        );
    }

    #[test]
    fn consts_are_order_independent_but_not_recursive() {
        // `km` is declared after its user: order-independence.
        resolve_program(
            "let two_km { 2.0 * km }
             let km { 1000.0 * meter }",
        )
        .expect("order-independent bindings");
        // A cycle is a diagnostic.
        let errs = errors(
            "let a { b + 1.0 }
             let b { a + 1.0 }",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("recursive const binding"))
        );
    }

    #[test]
    fn const_blocks_host_local_lets() {
        // The value body is the ordinary statement block (ADR 0027,
        // Decision 1 as revised): local `let`s scope lexically and the
        // trailing expression is the result.
        let program = resolve_program(
            "let overheat: temperature[real] {
                 let base = 300.0 * kelvin;
                 base + 50.0 * kelvin
             }",
        )
        .expect("a const block with locals resolves");
        assert!(program.schemas.is_empty());
        // An `assert` in a const block is deferred.
        let errs = errors("let x { assert true; 1.0 }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("`assert` in a const binding"))
        );
        // A block without a trailing result has no value.
        let errs = errors("let x { let y = 1.0; }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("must end in its result expression"))
        );
    }

    #[test]
    fn const_ascriptions_are_checked() {
        resolve_program("let limit: temperature[real] { 350.0 * kelvin }")
            .expect("a correct ascription");
        let errs = errors("let limit: temperature[real] { 350.0 }");
        assert!(errs.iter().any(|e| {
            e.message
                .contains("declared `temperature[real]` but its value is `real`")
        }));
    }

    #[test]
    fn const_dimension_mismatch_is_an_error() {
        let errs = errors("let nonsense { meter + second }");
        assert!(errs.iter().any(|e| e.message.contains("dimensions differ")));
    }

    #[test]
    fn const_lambdas_apply_and_curry() {
        // The ADR 0030 motivating program: currying is explicit, partial
        // binding is ordinary application, `three` folds to 3.
        let program = resolve_program(
            r#"
            let add { |a| |b| a + b }
            let add1 { add 1 }
            let three { add1 2 }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { size: int }
            }
            view sized {
              readings |> flat_map |_, r| if r.size > three then r else ()
            }
        "#,
        )
        .expect("const lambdas evaluate");
        let body = format!("{:?}", program.views[0].body);
        assert!(body.contains("Int(3)"), "three folds to 3: {body}");
        // A two-argument spine on a curried function is two ordinary
        // applications, no over-application mechanism.
        resolve_program("let add { |a| |b| a + b }\nlet v { add 1 2 }")
            .expect("a curried spine saturates one step at a time");
    }

    #[test]
    fn tupled_lambdas_take_exactly_their_tuple() {
        // `|a, b|` binds one 2-tuple parameter (ADR 0030, Decision 2).
        resolve_program(
            "let addt { |a, b| a + b }
             let add1t { |a| addt (1, a) }
             let threet { add1t 2 }",
        )
        .expect("a tupled application saturates");
        // Partial binding of a tupled function is an error, with the
        // currying hint.
        let errs = errors("let addt { |a, b| a + b }\nlet bad { addt 1 }");
        assert!(errs.iter().any(|e| {
            e.message.contains("expects a tuple of 2 values")
                && e.message.contains("currying is written")
        }));
        // A tuple of the wrong width names both sides.
        let errs = errors("let addt { |a, b| a + b }\nlet bad { addt (1, 2, 3) }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("expects a tuple of 2 values, found 3"))
        );
    }

    #[test]
    fn applying_a_non_function_is_an_error() {
        let errs = errors("let x { 1 }\nlet y { x 2 }");
        assert!(errs.iter().any(|e| {
            e.message
                .contains("cannot apply a value of type `int`: it is not a function")
        }));
        // Over-applying a saturated tupled call applies its scalar result.
        let errs = errors("let addt { |a, b| a + b }\nlet bad { addt (1, 2) 3 }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("cannot apply a value of type `int`"))
        );
    }

    #[test]
    fn closures_capture_by_value_and_shadow_lexically() {
        // A closure escaping its block keeps the block's locals.
        let program = resolve_program(
            r#"
            let f { let y = 1; |x| x + y }
            let v { f 2 }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { size: int }
            }
            view sized {
              readings |> flat_map |_, r| if r.size > v then r else ()
            }
        "#,
        )
        .expect("capture by value");
        let body = format!("{:?}", program.views[0].body);
        assert!(body.contains("Int(3)"), "v folds to 3: {body}");
        // A parameter shadows a captured local of the same name...
        resolve_program("let f { let x = 10; |x| x }\nlet v { f 2 }")
            .expect("parameter shadows a captured local");
        // ...and a top-level binding; the body still reaches unshadowed
        // top-level names on demand.
        resolve_program(
            "let c { 5 }
             let f { |c| c + 0 }
             let g { |x| x + c }
             let v { f 1 + g 1 }",
        )
        .expect("parameter shadows a top-level binding");
    }

    #[test]
    fn dimensions_flow_through_const_functions() {
        // The ascription checks the *result* of applying the function.
        resolve_program(
            "let vel[T] { (length / time)[T] }
             let per_s { |x| x / second }
             let speed: vel[real] { per_s (100.0 * meter) }",
        )
        .expect("a dimensioned result type-checks its ascription");
        let errs = errors(
            "let per_s { |x| x / second }
             let bad { per_s (100.0 * meter) + 1.0 }",
        );
        assert!(errs.iter().any(|e| e.message.contains("dimensions differ")));
    }

    #[test]
    fn recursive_const_functions_hit_the_depth_limit() {
        // Dynamic recursion escapes the definitional cycle detector (a
        // lambda defers the reference), so the depth guard catches it.
        let errs = errors("let f { |x| f x }\nlet v { f 1 }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("application depth limit"))
        );
    }

    #[test]
    fn const_lambda_shape_errors() {
        // Zero parameters: nothing to apply.
        let errs = errors("let f { || 1 }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("at least one parameter"))
        );
        // Duplicate parameter names.
        let errs = errors("let f { |a, a| a }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("duplicate lambda parameter `a`"))
        );
        // A `: type` ascription on a function binding.
        let errs = errors("let f: real { |x| x }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("cannot carry a `: type` ascription"))
        );
        // Arithmetic on a function names it in the diagnostic.
        let errs = errors("let f { |x| x }\nlet bad { f + 1 }");
        assert!(errs.iter().any(|e| {
            e.message
                .contains("`+` is not defined on `function` and `int`")
        }));
    }

    #[test]
    fn const_functions_type_in_view_bodies() {
        // A view body applies a const function (ADR 0030): the body
        // re-types per call site, so `add 1 2` is an `int` here.
        resolve_program(
            r#"
            let add { |a| |b| a + b }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { size: int }
            }
            view sized {
              readings |> flat_map |_, r| if r.size > add 1 2 then r else ()
            }
        "#,
        )
        .expect("a const function application types in a view body");
        // A bare (unapplied) function in scalar position is an error.
        let errs = errors(
            r#"
            let add { |a| |b| a + b }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { size: int }
            }
            view sized {
              readings |> flat_map |_, r| if r.size > add then r else ()
            }
        "#,
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("found a function (apply it)"))
        );
    }

    #[test]
    fn const_function_bodies_retype_per_call_site() {
        // Exact per-site checking: the same function serves a dimensioned
        // column, and a domain error inside the body reports with the
        // call-site note appended (ADR 0030, Consequences).
        resolve_program(
            r#"
            let warmer { |x| x + 1.0 * kelvin }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { temperature: temperature[real] }
            }
            view hot {
              readings |> flat_map |_, r|
                if warmer r.temperature > 300.0 * kelvin then r else ()
            }
        "#,
        )
        .expect("the body types at the call site's dimension");
        let errs = errors(
            r#"
            let warmer { |x| x + 1.0 * kelvin }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { size: int }
            }
            view hot {
              readings |> flat_map |_, r| if warmer r.size > 2 then r else ()
            }
        "#,
        );
        // The body diagnostic (definition-site span) plus the call note.
        assert!(errs.iter().any(|e| {
            e.message
                .contains("operands of the same type, found int and temperature[real]")
        }));
        assert!(
            errs.iter()
                .any(|e| e.message.contains("while applying `warmer` here"))
        );
    }

    #[test]
    fn const_function_applications_beta_reduce_at_lowering() {
        // ADR 0030, Decision 5: `add1 r.size` reaches the runtime as
        // `1 + r.size`; no function name and no residual lambda beyond the
        // pipeline op's own remain in the lowered body.
        let program = resolve_program(
            r#"
            let add { |a| |b| a + b }
            let add1 { add 1 }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { size: int }
            }
            view sized {
              readings |> flat_map |_, r| if r.size > add1 r.size then r else ()
            }
        "#,
        )
        .expect("should resolve");
        let body = format!("{:?}", program.views[0].body);
        assert!(
            !body.contains("add1"),
            "no function reference remains: {body}"
        );
        assert!(
            body.contains("Int(1)") && body.contains("Add"),
            "the substituted body is inline arithmetic: {body}"
        );
        // A value-level pipe into a const function reduces the same way.
        let program = resolve_program(
            r#"
            let add1 { |b| b + 1 }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { size: int }
            }
            view sized {
              readings |> flat_map |_, r| if (r.size |> add1) > 0 then r else ()
            }
        "#,
        )
        .expect("should resolve");
        let body = format!("{:?}", program.views[0].body);
        assert!(!body.contains("add1"), "the piped function reduces: {body}");
    }

    #[test]
    fn a_qualified_module_function_beta_reduces() {
        // ADR 0031, Decision 8.  `bag.max b.x` must reach the runtime as the
        // `fold` spine the module defines, with no residual qualified name:
        // `si` exports only scalars, so lowering used to assume every module
        // member had a literal, and `bag` is the first to break that.
        let program = resolve_program(
            r#"
            import bag
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr { size: int }
            }
            view summary {
              readings |> assume { complete } |> map_bags |_, b| (.m = bag.max b.size)
            }
        "#,
        )
        .expect("should resolve");
        let body = format!("{:?}", program.views[0].body);
        assert!(
            !body.contains("\"bag\""),
            "the qualified name is reduced away: {body}"
        );
        assert!(
            body.contains("Name(\"fold\")") && body.contains("Combiner(\">>\")"),
            "the module's definition is inlined: {body}"
        );
        // The eta-expansion's fresh parameter is fully applied, so none of its
        // machinery leaks into the lowered body.
        assert!(!body.contains("x__"), "no eta parameter remains: {body}");
    }

    #[test]
    fn beta_reduction_avoids_capturing_the_callers_names() {
        // Substituting `r.size` under the function's own `|r|` binder must
        // not capture the caller's `r`: the binder alpha-renames, and full
        // application then eliminates the renamed parameter entirely.
        let program = resolve_program(
            r#"
            let pick { |x| |r| r + x }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { size: int }
            }
            view sized {
              readings |> flat_map |_, r| if (pick r.size 2) > 0 then r else ()
            }
        "#,
        )
        .expect("should resolve");
        let body = format!("{:?}", program.views[0].body);
        assert!(
            body.contains("Int(2)") && body.contains("size"),
            "reduced to `2 + r.size`-shaped arithmetic: {body}"
        );
        assert!(
            !body.contains("__1"),
            "the renamed binder is fully applied away: {body}"
        );
        assert!(
            !body.contains("pick"),
            "no function reference remains: {body}"
        );
    }

    #[test]
    fn mutually_recursive_functions_hit_the_checker_depth_guard() {
        // Each binding evaluates fine in isolation (a lambda defers the
        // reference), so the recursion only manifests while typing an
        // application in a view body.
        let errs = errors(
            r#"
            let f { |x| g x }
            let g { |x| f x }
            unit Machine { id: string }
            store readings {
              unit { Machine }
              attr* { size: int }
            }
            view sized {
              readings |> flat_map |_, r| if r.size > f 1 then r else ()
            }
        "#,
        );
        assert!(errs.iter().any(|e| e.message.contains("nest too deeply")));
    }

    #[test]
    fn expression_builtins_cannot_be_redeclared() {
        // A binding that would shadow an ambient builtin in head position is
        // an error (ADR 0027, Decision 3).  The protection covers exactly
        // what is still ambient, which after ADR 0031 Decision 8 is the
        // primitives and `to_real`.
        for name in ["to_real", "fold", "map"] {
            let errs = errors(&format!("let {name} {{ |x| x }}"));
            assert!(
                errs.iter().any(|e| e
                    .message
                    .contains(&format!("`{name}` is an ambient builtin"))),
                "`{name}` should still be protected"
            );
        }
        // And the six aggregates are *not* protected any more: they left the
        // initial environment with the `bag` module, so their names returned
        // to users (ADR 0031, Decision 8, and its "Positive" consequence).
        // `docs/language/03-shapes.md`'s `let count = ...` example is legal
        // again because of exactly this.
        for name in ["sum", "min", "max", "count", "any", "all"] {
            assert!(
                resolve_str(&format!("let {name} {{ |x| x }}")).is_ok(),
                "`{name}` should be an ordinary name now"
            );
        }
    }

    #[test]
    fn value_namespace_collisions_are_errors() {
        // An intrinsic cannot be redeclared.
        let errs = errors("let meter { 2.0 }");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("intrinsic base unit"))
        );
        // Imports and lets share one namespace.
        let errs = errors(
            "import si
             let si { 1.0 }",
        );
        assert!(errs.iter().any(|e| e.message.contains("duplicate")));
        // A value name may not reuse a table name.
        let errs = errors(
            "unit Machine { id: string }
             store readings { unit { Machine } attr* { kelvin_r: real } }
             let readings { 1.0 }",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("collides with a store or view"))
        );
    }

    #[test]
    fn unknown_module_and_member_are_pointed() {
        let errs = errors("import geo");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("unknown module `geo`"))
        );
        let errs = errors(
            "import si
             let x { si.bogus }",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("module `si` has no member `bogus`"))
        );
        // A module name is not a value.
        let errs = errors(
            "import si
             let x { si }",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("`si` is a module, not a value"))
        );
    }

    #[test]
    fn a_const_is_not_a_table() {
        let errs = errors(
            "let km { 1000.0 * meter }
             view v { km |> flat_map |_, r| r }",
        );
        assert!(
            errs.iter()
                .any(|e| e.message.contains("`km` is a constant, not a table"))
        );
    }

    #[test]
    fn the_bundled_si_module_resolves() {
        // The embedded stdlib source itself must be clean: importing it
        // surfaces any internal error at the import site.
        resolve_program("import si").expect("si resolves");
    }
}
