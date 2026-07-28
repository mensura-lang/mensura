//! Pipeline (table-expression) type checking: the Tier A operations over
//! `TableType` (`docs/language/09-typing-reference.md` sections 6 and 10).
//!
//! This layer sits above [`crate::expr_check`]: it types a table-valued
//! expression against a set of named [`Sources`], dispatching each `|>` stage to
//! an operation handler that transforms the input [`TableType`]. Each operation's
//! key-first lambda body (`|k|` / `|k, r|` / `|k, b|`, ADR 0015) is typed by
//! `expr_check` against a key/row/group context derived from the input table.
//! Like `resolve` and `expr_check`, it collects all diagnostics rather than
//! failing fast.

use std::collections::BTreeMap;

use mensura_syntax::{BinOp, Block, Expr, ExprKind, Span, Stmt};

use crate::expr_check::{Context, Optionality, Ty, TypeError, type_expr};
use crate::model::ColumnType;
use crate::suggest::suffix;
use crate::table::{
    Cardinality, Column, Completeness, Content, Exhaustive, Functional, Lineage, Qualifiers,
    SplitId, TableType, Totality,
};

/// The type of a table-valued (pipeline) expression.
#[derive(Clone, Debug, PartialEq)]
pub enum PipeTy {
    Table(TableType),
    /// A pair of tables: produced by `split`, consumed by `union` (section 6.5).
    Pair(TableType, TableType),
}

/// The named source tables in scope (a store presented to a pipeline,
/// `10-views.md`, "Sources resolve by name"), together with the ambient
/// value environment every expression site sees (the intrinsic base units,
/// top-level consts, and imported modules; `12-modules-and-imports.md`).
#[derive(Clone, Debug, Default)]
pub struct Sources {
    bound: BTreeMap<String, PipeTy>,
    ambient: crate::expr_check::Ambient,
}

impl Sources {
    pub fn new() -> Self {
        Sources::default()
    }

    /// Add a store source, presented as a single table.
    pub fn with(mut self, name: &str, table: TableType) -> Self {
        self.bound.insert(name.to_string(), PipeTy::Table(table));
        self
    }

    /// Set the ambient value environment expression sites resolve against.
    pub fn with_ambient(mut self, ambient: crate::expr_check::Ambient) -> Self {
        self.ambient = ambient;
        self
    }

    /// Bind a name to any pipeline value (used by a view body's `let`, which may
    /// hold a `split` pair as well as a table).
    fn bind(&mut self, name: &str, pipe: PipeTy) {
        self.bound.insert(name.to_string(), pipe);
    }

    fn get(&self, name: &str) -> Option<&PipeTy> {
        self.bound.get(name)
    }
}

fn te(message: impl Into<String>, span: Span) -> TypeError {
    TypeError {
        message: message.into(),
        span,
    }
}

fn error(message: impl Into<String>, span: Span) -> Vec<TypeError> {
    vec![te(message, span)]
}

/// Type a pipeline expression, collecting all diagnostics.
pub fn type_pipeline(sources: &Sources, expr: &Expr) -> Result<PipeTy, Vec<TypeError>> {
    match &expr.kind {
        ExprKind::Name(name) => match sources.get(name) {
            Some(pipe) => Ok(pipe.clone()),
            None if sources.ambient.contains_key(name) => Err(error(
                format!("`{name}` is a constant, not a table"),
                expr.span,
            )),
            None => {
                let hint = suffix(name, sources.bound.keys().cloned().collect::<Vec<_>>());
                Err(error(format!("unknown source `{name}`{hint}"), expr.span))
            }
        },
        ExprKind::Tuple(items) if items.len() == 2 => {
            let a =
                type_pipeline(sources, &items[0]).and_then(|p| expect_table(p, items[0].span))?;
            let b =
                type_pipeline(sources, &items[1]).and_then(|p| expect_table(p, items[1].span))?;
            Ok(PipeTy::Pair(a, b))
        }
        ExprKind::Binary(BinOp::Pipe, lhs, rhs) => {
            let input = type_pipeline(sources, lhs)?;
            apply_application(sources, rhs, Some(input))
        }
        ExprKind::App(..) => apply_application(sources, expr, None),
        _ => Err(error("not a pipeline expression", expr.span)),
    }
}

/// Reset each output table's gradings to match its own scalar cardinality
/// (ADR 0024): the conservative rule for every operation without a
/// mechanized transport witness; see [`dispatch_op`].
fn sync_functional(output: PipeTy) -> PipeTy {
    match output {
        PipeTy::Table(mut t) => {
            t.sync_functional();
            PipeTy::Table(t)
        }
        PipeTy::Pair(mut a, mut b) => {
            a.sync_functional();
            b.sync_functional();
            PipeTy::Pair(a, b)
        }
    }
}

fn expect_table(pipe: PipeTy, span: Span) -> Result<TableType, Vec<TypeError>> {
    match pipe {
        PipeTy::Table(table) => Ok(table),
        PipeTy::Pair(..) => Err(error("expected a single table, found a pair", span)),
    }
}

/// Apply a pipeline operation to its input, whether the input arrived from the
/// left of a `|>` (`piped` is `Some`) or as the operation's trailing argument in
/// a bare application `op args data` (`piped` is `None`).  Both spellings
/// converge here, so a stage is checked identically either way (ADR 0018,
/// `docs/toolkit/01-application-checking.md`).
fn apply_application(
    sources: &Sources,
    op_expr: &Expr,
    piped: Option<PipeTy>,
) -> Result<PipeTy, Vec<TypeError>> {
    let (head, mut args) = flatten_app(op_expr);
    let ExprKind::Name(op) = &head.kind else {
        return Err(error("expected a pipeline operation", op_expr.span));
    };
    let input = match piped {
        Some(input) => input,
        None => {
            // Bare application: the piped input is the trailing argument.
            // Peeling it is sound only for a saturated stage; an unsaturated
            // form is partial application (ADR 0018, open question 2), not yet
            // supported.
            let Some(last) = args.pop() else {
                return Err(error("a pipeline operation needs an input", head.span));
            };
            type_pipeline(sources, last)?
        }
    };
    dispatch_op(sources, op, &args, input, head.span)
}

/// Dispatch a resolved pipeline operation to its handler.  Shared by the pipe
/// and bare-application forms via [`apply_application`].
fn dispatch_op(
    sources: &Sources,
    op: &str,
    args: &[&Expr],
    input: PipeTy,
    span: Span,
) -> Result<PipeTy, Vec<TypeError>> {
    let result = match op {
        "promote" => op_promote(input, args, span),
        "demote" => op_demote(input, args, span),
        "flat_map" => op_flat_map(sources, input, args, span),
        "map_bags" => op_map_bags(sources, input, args, span),
        "split" => op_split(sources, input, args, span),
        "union" => op_union(input, args, span),
        "lookup" => op_join(sources, input, args, span, JoinKind::Left),
        "lookup_total" => op_join(sources, input, args, span, JoinKind::Inner),
        "unpivot" => op_unpivot(input, args, span),
        "pivot" => op_pivot(input, args, span),
        "assume" => op_assume(input, args, span),
        "completeness_check" => op_completeness_check(sources, input, args, span),
        other => {
            // TODO(ADR-0025): `map` is a deliberately vacant name. Give it a
            // pointed diagnostic ("no `map` in Mensura: `flat_map` receives a
            // row, `map_bags` receives the bag") instead of the generic
            // edit-distance suggestion below.
            const OPS: [&str; 12] = [
                "promote",
                "demote",
                "flat_map",
                "map_bags",
                "split",
                "union",
                "lookup",
                "lookup_total",
                "unpivot",
                "pivot",
                "assume",
                "completeness_check",
            ];
            let hint = suffix(other, OPS.iter().map(|s| s.to_string()));
            Err(error(
                format!("unsupported operation `{other}`{hint}"),
                span,
            ))
        }
    }?;
    // Gradings are transformed only where a witness backs the transform
    // (ADR 0024): the key moves derive cardinality from them, and the
    // content-identity stages carry them.  Every other operation resets
    // them to match its own output cardinality, the conservative rule
    // until the per-op transport table is mechanized.
    Ok(match op {
        "promote" | "demote" | "assume" | "completeness_check" => result,
        _ => sync_functional(result),
    })
}

#[derive(Clone, Copy)]
enum JoinKind {
    Left,
    Inner,
}

/// `lookup` / `lookup_total right (|k, r| key)` (section 6.4, Tier A): join a fixed
/// right table by a key over the left row. Adds the right table's non-key
/// columns; `lookup` makes them optional, `lookup_total` keeps their totality.
/// The right table is a store (`Singletons`, functional), so cardinality is
/// preserved; completeness on the left and lineage are preserved. `lookup`
/// never drops a row, so it carries `exhaustive` (`lookup_exhaustive`);
/// `lookup_total` can drop unmatched rows out of a fiber, destroying it
/// (ADR 0020 section 2).
fn op_join(
    sources: &Sources,
    input: PipeTy,
    args: &[&Expr],
    span: Span,
    kind: JoinKind,
) -> Result<PipeTy, Vec<TypeError>> {
    let left = expect_table(input, span)?;
    let [right_arg, key_arg] = args else {
        return Err(error("a join expects a right table and a key lambda", span));
    };
    let ExprKind::Name(right_name) = &right_arg.kind else {
        return Err(error(
            "a join's right side must be a source name",
            right_arg.span,
        ));
    };
    let right = match sources.get(right_name) {
        Some(PipeTy::Table(t)) => t,
        Some(PipeTy::Pair(..)) => {
            return Err(error(
                format!("`{right_name}` is a pair of tables, not a single join target"),
                right_arg.span,
            ));
        }
        None => {
            return Err(error(
                format!("unknown source `{right_name}`"),
                right_arg.span,
            ));
        }
    };
    let [right_key] = right.content.key.as_slice() else {
        return Err(error(
            "a join's right table must have a single key column",
            right_arg.span,
        ));
    };

    let (params, body) = lambda_params(&[key_arg], "join", 2, key_arg.span)?;
    let ctx = Context::row(&sources.ambient, params[0], params[1], &left);
    let key_ty = type_expr(&ctx, body)?;
    match key_ty.known_value_domain() {
        Some(domain) if *domain == right_key.domain => {}
        Some(_) => {
            return Err(error(
                format!("join key does not match `{right_name}`'s key domain"),
                body.span,
            ));
        }
        None => return Err(error("a join key must be a single known value", body.span)),
    }

    let mut columns = left.content.columns.clone();
    let mut totality = left.qualifiers.totality.clone();
    let mut errs = Vec::new();
    for rc in &right.content.columns {
        let clash = columns.iter().any(|c| c.name == rc.name)
            || left.content.key.iter().any(|c| c.name == rc.name);
        if clash {
            errs.push(te(
                format!("join would duplicate column `{}`", rc.name),
                right_arg.span,
            ));
            continue;
        }
        columns.push(rc.clone());
        let optional = match kind {
            JoinKind::Left => true,
            JoinKind::Inner => right.qualifiers.totality.is_optional(&rc.name),
        };
        if optional {
            totality.mark_optional(rc.name.clone());
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    Ok(PipeTy::Table(TableType {
        content: Content {
            key: left.content.key,
            columns,
        },
        qualifiers: Qualifiers {
            cardinality: left.qualifiers.cardinality,
            totality,
            completeness: left.qualifiers.completeness,
            exhaustive: match kind {
                JoinKind::Left => left.qualifiers.exhaustive,
                JoinKind::Inner => Exhaustive::new(),
            },
            functional: Functional::new(),
            lineage: left.qualifiers.lineage,
        },
    }))
}

/// `flat_map |k, r| collection` (section 6.1, Tier A, ADR 0015): the formal row
/// multiset. The body yields a collection of value rows (`( )` empty drops, a
/// bare row keeps, `(a, b)` expands, an `if` filters or branches). The non-key
/// columns are the collection's row schema; the key is preserved; cardinality
/// is the maximum collection size (`<= 1` keeps the input bound, `>= 2` is a
/// `Bag`). A body that can drop (some branch yields fewer rows than it was
/// given) may leave holes in a fiber, so it forfeits `exhaustive`; a provably
/// non-dropping body carries it (`map_exhaustive`, ADR 0020 section 2).
fn op_flat_map(
    sources: &Sources,
    input: PipeTy,
    args: &[&Expr],
    span: Span,
) -> Result<PipeTy, Vec<TypeError>> {
    let table = expect_table(input, span)?;
    let (params, body) = lambda_params(args, "flat_map", 2, span)?;
    let ctx = Context::row(&sources.ambient, params[0], params[1], &table);
    let (schema, sizes) = row_collection(&ctx, body, &table)?;
    let Some(schema) = schema else {
        return Err(error(
            "a `flat_map` body that always drops the row cannot infer the output \
             columns; keep at least one row in some branch",
            body.span,
        ));
    };
    let (columns, totality) = schema_to_content(schema);
    let cardinality = if sizes.max <= 1 {
        table.qualifiers.cardinality
    } else {
        Cardinality::Bag
    };
    let exhaustive = if sizes.min >= 1 {
        table.qualifiers.exhaustive
    } else {
        Exhaustive::new()
    };
    Ok(PipeTy::Table(TableType {
        content: Content {
            key: table.content.key,
            columns,
        },
        qualifiers: Qualifiers {
            cardinality,
            totality,
            completeness: table.qualifiers.completeness,
            exhaustive,
            functional: Functional::new(),
            lineage: table.qualifiers.lineage,
        },
    }))
}

/// The collection-size bounds of a `flat_map` body: how few and how many output
/// rows one input row can yield.
#[derive(Clone, Copy)]
struct SizeBounds {
    min: usize,
    max: usize,
}

/// One column of a `flat_map` output row, before it becomes table content.
struct RowColumn {
    name: String,
    domain: ColumnType,
    opt: Optionality,
}

/// The shared schema of the value rows a `flat_map` body yields.
type RowSchema = Vec<RowColumn>;

/// Type a `flat_map` body as a collection of value rows (ADR 0015), returning the
/// shared row schema (or `None` if the body always drops) and the collection's
/// size bounds. `( )` is empty (0), `(a, b, ...)` expands (n), an `if`
/// branches (the bounds join), and any other body is a single row (1).
fn row_collection(
    ctx: &Context,
    body: &Expr,
    input: &TableType,
) -> Result<(Option<RowSchema>, SizeBounds), Vec<TypeError>> {
    match &body.kind {
        ExprKind::Tuple(items) => {
            if items.is_empty() {
                return Ok((None, SizeBounds { min: 0, max: 0 }));
            }
            let mut errs = Vec::new();
            let mut schema: Option<RowSchema> = None;
            for item in items {
                match single_row_schema(ctx, item, input) {
                    Ok(s) => {
                        schema = match schema {
                            None => Some(s),
                            Some(prev) => match unify_row_schema(&prev, &s, item.span) {
                                Ok(merged) => Some(merged),
                                Err(e) => {
                                    errs.extend(e);
                                    Some(prev)
                                }
                            },
                        };
                    }
                    Err(e) => errs.extend(e),
                }
            }
            if !errs.is_empty() {
                return Err(errs);
            }
            Ok((
                schema,
                SizeBounds {
                    min: items.len(),
                    max: items.len(),
                },
            ))
        }
        ExprKind::If { cond, then, els } => {
            let mut errs = require_known_bool(ctx, cond);
            let then_rc = row_collection(ctx, then, input);
            let els_rc = row_collection(ctx, els, input);
            let then_rc = match then_rc {
                Ok(rc) => Some(rc),
                Err(e) => {
                    errs.extend(e);
                    None
                }
            };
            let els_rc = match els_rc {
                Ok(rc) => Some(rc),
                Err(e) => {
                    errs.extend(e);
                    None
                }
            };
            let (Some((then_schema, then_size)), Some((els_schema, els_size))) = (then_rc, els_rc)
            else {
                return Err(errs);
            };
            if !errs.is_empty() {
                return Err(errs);
            }
            // A `( )` branch adopts the other branch's schema.
            let schema = match (then_schema, els_schema) {
                (Some(a), Some(b)) => Some(unify_row_schema(&a, &b, body.span)?),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            Ok((
                schema,
                SizeBounds {
                    min: then_size.min.min(els_size.min),
                    max: then_size.max.max(els_size.max),
                },
            ))
        }
        _ => Ok((
            Some(single_row_schema(ctx, body, input)?),
            SizeBounds { min: 1, max: 1 },
        )),
    }
}

/// Type one value row of a `flat_map` body: a record literal `(.a = ...)` or a value
/// row (e.g. the parameter `r`). Anything else is not a row.
fn single_row_schema(
    ctx: &Context,
    expr: &Expr,
    input: &TableType,
) -> Result<RowSchema, Vec<TypeError>> {
    match &expr.kind {
        ExprKind::Record(fields) => {
            let mut schema = Vec::new();
            let mut errs = Vec::new();
            for field in fields {
                let name = &field.name.name;
                if input.content.key.iter().any(|c| &c.name == name) {
                    errs.push(te(
                        format!("a `flat_map` row may not set the key column `{name}`"),
                        field.name.span,
                    ));
                    continue;
                }
                match type_expr(ctx, &field.value) {
                    Err(e) => errs.extend(e),
                    Ok(ty) => match column_of(&ty) {
                        Some((domain, opt)) => schema.push(RowColumn {
                            name: name.clone(),
                            domain,
                            opt,
                        }),
                        None => errs.push(te(
                            format!("field `{name}` is not a single value"),
                            field.value.span,
                        )),
                    },
                }
            }
            if errs.is_empty() {
                Ok(schema)
            } else {
                Err(errs)
            }
        }
        _ => match type_expr(ctx, expr)? {
            Ty::Record(fields) => Ok(fields
                .into_iter()
                .filter_map(|(name, ty)| {
                    column_of(&ty).map(|(domain, opt)| RowColumn { name, domain, opt })
                })
                .collect()),
            _ => Err(error(
                "a `flat_map` body must yield rows: a record `(.a = ...)`, the value \
                 row `r`, `( )` to drop, or `(a, b)` to expand",
                expr.span,
            )),
        },
    }
}

/// Unify two row schemas (the branches of an `if` or the items of an expanding
/// collection): same columns in the same order and domains; a column optional in
/// either side is optional in the result. A mismatch is a located error.
fn unify_row_schema(a: &RowSchema, b: &RowSchema, span: Span) -> Result<RowSchema, Vec<TypeError>> {
    if a.len() != b.len() {
        return Err(error(
            "a `flat_map` collection's rows must share one schema (column count differs)",
            span,
        ));
    }
    let mut merged = Vec::with_capacity(a.len());
    for (ca, cb) in a.iter().zip(b) {
        if ca.name != cb.name || ca.domain != cb.domain {
            return Err(error(
                "a `flat_map` collection's rows must share one schema (a column differs)",
                span,
            ));
        }
        let opt = if ca.opt == Optionality::Optional || cb.opt == Optionality::Optional {
            Optionality::Optional
        } else {
            Optionality::Total
        };
        merged.push(RowColumn {
            name: ca.name.clone(),
            domain: ca.domain.clone(),
            opt,
        });
    }
    Ok(merged)
}

/// Lower a row schema into table content columns and their totality.
fn schema_to_content(schema: RowSchema) -> (Vec<Column>, Totality) {
    let mut columns = Vec::with_capacity(schema.len());
    let mut totality = Totality::all_total();
    for c in schema {
        if c.opt == Optionality::Optional {
            totality.mark_optional(c.name.clone());
        }
        columns.push(Column {
            name: c.name,
            domain: c.domain,
        });
    }
    (columns, totality)
}

/// Require an expression to be a known boolean (an `if` condition), returning any
/// diagnostics.
fn require_known_bool(ctx: &Context, cond: &Expr) -> Vec<TypeError> {
    match type_expr(ctx, cond) {
        Err(e) => e,
        Ok(Ty::Bool) => Vec::new(),
        Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt: Optionality::Total,
        }) => Vec::new(),
        Ok(_) => error("an `if` condition must be a known boolean", cond.span),
    }
}

/// `map_bags |k, b| record` (section 6.2, Tier A): transform each group. The
/// result cardinality is **inferred from the return**: all single-valued fields
/// are the aggregate shape (one row per key, `Singletons`); bag-valued fields are
/// the window shape (one output row per input row, `Bag`). A **reducing** body
/// (the aggregate shape) folds each key's bag, which is silently wrong on a
/// partial bag, so it demands completeness over the current key (ADR 0023);
/// at a `Singletons` input the obligation discharges trivially, since a
/// present key's single row is the identity's whole fiber
/// (`fiberCompleteWrt_of_functional`). The key, completeness, and lineage
/// are preserved; `exhaustive` is carried (one output row per present key in
/// the aggregate shape, one per input row in the window shape, so no fiber
/// loses a row; `fiberMap_exhaustive`, with `aggregate_exhaustive` the
/// aggregate-shape special case).
fn op_map_bags(
    sources: &Sources,
    input: PipeTy,
    args: &[&Expr],
    span: Span,
) -> Result<PipeTy, Vec<TypeError>> {
    let table = expect_table(input, span)?;
    let (params, body) = lambda_params(args, "map_bags", 2, span)?;
    let ctx = Context::bag(&sources.ambient, params[0], params[1], &table);
    let (columns, totality, cardinality) = bag_record_content(&ctx, body)?;
    if cardinality == Cardinality::Singletons
        && table.qualifiers.cardinality == Cardinality::Bag
        && table.qualifiers.completeness != Completeness::Complete
    {
        return Err(error(
            "a reducing `map_bags` needs completeness over the current key (a \
             fold over a partial bag is silently wrong); establish it with \
             `completeness_check { ... }` or `assume { complete }` first",
            span,
        ));
    }
    Ok(PipeTy::Table(TableType {
        content: Content {
            key: table.content.key,
            columns,
        },
        qualifiers: Qualifiers {
            cardinality,
            totality,
            completeness: table.qualifiers.completeness,
            exhaustive: table.qualifiers.exhaustive,
            functional: Functional::new(),
            lineage: table.qualifiers.lineage,
        },
    }))
}

/// Type a `map_bags` record body into columns, totality, and the result
/// cardinality (section 6.2). Single-valued fields are aggregates (`Singletons`);
/// bag-valued fields are window values (`Bag`); a mix of the two is rejected.
fn bag_record_content(
    ctx: &Context,
    body: &Expr,
) -> Result<(Vec<Column>, Totality, Cardinality), Vec<TypeError>> {
    let ExprKind::Record(fields) = &body.kind else {
        return Err(error("`map_bags`'s lambda must return a record", body.span));
    };
    if fields.is_empty() {
        return Err(error(
            "`map_bags`'s record needs at least one field",
            body.span,
        ));
    }
    let mut columns = Vec::new();
    let mut totality = Totality::all_total();
    let mut errs = Vec::new();
    let mut saw_aggregate = false;
    let mut saw_window = false;
    for field in fields {
        match type_expr(ctx, &field.value) {
            Err(e) => errs.extend(e),
            Ok(Ty::Bag { domain, opt }) => {
                saw_window = true;
                columns.push(Column {
                    name: field.name.name.clone(),
                    domain,
                });
                if opt == Optionality::Optional {
                    totality.mark_optional(field.name.name.clone());
                }
            }
            Ok(ty) => match column_of(&ty) {
                Some((domain, opt)) => {
                    saw_aggregate = true;
                    columns.push(Column {
                        name: field.name.name.clone(),
                        domain,
                    });
                    if opt == Optionality::Optional {
                        totality.mark_optional(field.name.name.clone());
                    }
                }
                None => errs.push(te(
                    format!("field `{}` is not a value or a bag", field.name.name),
                    field.value.span,
                )),
            },
        }
    }
    if saw_aggregate && saw_window {
        errs.push(te(
            "a `map_bags` record must be all aggregates (one row per key) or all \
             window values (a bag), not a mix",
            body.span,
        ));
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    let cardinality = if saw_window {
        Cardinality::Bag
    } else {
        Cardinality::Singletons
    };
    Ok((columns, totality, cardinality))
}

/// `split |k| pred` (section 6.5, Tier A): route each key to one side of a pair
/// by a predicate over the key. Adds sibling lineage tags; content, cardinality,
/// and completeness are unchanged on both sides. `exhaustive` is destroyed on
/// both sides: the predicate reads the full key, so it can send two variants of
/// the same residual fiber to different sides (`split_not_exhaustive`,
/// ADR 0020 section 2).
fn op_split(
    sources: &Sources,
    input: PipeTy,
    args: &[&Expr],
    span: Span,
) -> Result<PipeTy, Vec<TypeError>> {
    let table = expect_table(input, span)?;
    let (params, body) = lambda_params(args, "split", 1, span)?;
    let ctx = Context::key(&sources.ambient, params[0], &table);
    if type_expr(&ctx, body)? != Ty::Bool {
        return Err(error("`split`'s predicate must be a boolean", body.span));
    }
    let id = SplitId(span.start as u32);
    let (left, right) = table.qualifiers.lineage.split(id);
    Ok(PipeTy::Pair(
        split_side(&table, left),
        split_side(&table, right),
    ))
}

/// `union` (section 6.5, Tier A): the union of a pair of tables of the same
/// schema. Cardinality is `Singletons` iff both inputs are and their lineages
/// are disjoint, else `Bag`; completeness holds iff both inputs are complete;
/// `exhaustive` holds where both inputs carry it (a union of full fibers is
/// full, `bind_exhaustive`); the lineage tag-sets union.
fn op_union(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    if !args.is_empty() {
        return Err(error("`union` takes no arguments", span));
    }
    let (a, b) = match input {
        PipeTy::Pair(a, b) => (a, b),
        PipeTy::Table(_) => return Err(error("`union` expects a pair of tables", span)),
    };
    if a.content != b.content {
        return Err(error(
            "`union` requires both tables to have the same schema",
            span,
        ));
    }
    let disjoint = a.qualifiers.lineage.disjoint(&b.qualifiers.lineage);
    let cardinality = if a.qualifiers.cardinality == Cardinality::Singletons
        && b.qualifiers.cardinality == Cardinality::Singletons
        && disjoint
    {
        Cardinality::Singletons
    } else {
        Cardinality::Bag
    };
    let completeness = if a.qualifiers.completeness == Completeness::Complete
        && b.qualifiers.completeness == Completeness::Complete
    {
        Completeness::Complete
    } else {
        Completeness::Incomplete
    };
    let exhaustive: Exhaustive = a
        .qualifiers
        .exhaustive
        .intersection(&b.qualifiers.exhaustive)
        .cloned()
        .collect();
    Ok(PipeTy::Table(TableType {
        content: a.content,
        qualifiers: Qualifiers {
            cardinality,
            totality: a.qualifiers.totality,
            completeness,
            exhaustive,
            functional: Functional::new(),
            lineage: a.qualifiers.lineage.union(&b.qualifiers.lineage),
        },
    }))
}

/// One side of a `split`: the same table under a branch lineage, with the
/// row-presence fact forfeited (`split_not_exhaustive`).
fn split_side(table: &TableType, lineage: Lineage) -> TableType {
    TableType {
        content: table.content.clone(),
        qualifiers: Qualifiers {
            cardinality: table.qualifiers.cardinality,
            totality: table.qualifiers.totality.clone(),
            completeness: table.qualifiers.completeness,
            exhaustive: Exhaustive::new(),
            functional: Functional::new(),
            lineage,
        },
    }
}

/// Extract a key-first lambda's parameter names and body, requiring exactly
/// `arity` parameters (ADR 0015): 1 for `split` (`|k|`), 2 for `flat_map` /
/// `map_bags` / the join key (`|k, r|`). A `_` parameter is kept verbatim and
/// binds nothing in the context (the ignored key).
fn lambda_params<'a>(
    args: &[&'a Expr],
    op: &str,
    arity: usize,
    span: Span,
) -> Result<(Vec<&'a str>, &'a Expr), Vec<TypeError>> {
    let [arg] = args else {
        return Err(error(format!("`{op}` expects one lambda argument"), span));
    };
    let ExprKind::Lambda { params, body, .. } = &arg.kind else {
        return Err(error(format!("`{op}` expects a lambda"), arg.span));
    };
    if params.len() != arity {
        let shape = if arity == 1 { "`|k|`" } else { "`|k, r|`" };
        return Err(error(
            format!("`{op}`'s lambda takes {arity} parameter(s), {shape}"),
            arg.span,
        ));
    }
    Ok((
        params.iter().map(|p| p.name.as_str()).collect(),
        body.as_ref(),
    ))
}

/// The column domain and totality a value type contributes, or `None` for a
/// bag, a nested record (window/nested returns are deferred), or a function
/// (which never enters a column, ADR 0030).
fn column_of(ty: &Ty) -> Option<(ColumnType, Optionality)> {
    match ty {
        Ty::Value { domain, opt } => Some((domain.clone(), *opt)),
        Ty::Bool => Some((ColumnType::Bool, Optionality::Total)),
        Ty::Bag { .. } | Ty::Record(_) | Ty::Fn(_) => None,
    }
}

/// `promote cols` (section 6.3, Tier A): promote each named column into the
/// key. A column must be total to enter the key (ADR 0013, and the
/// `demote_promote` inverse-domain side condition, ADR 0024); completeness
/// and lineage are preserved. `exhaustive` is **forfeited**,
/// against ADR 0020 section 2's "preserved" sketch: the promoted column
/// refines the residual key, which can cut a fiber (rows
/// `(s, math, score=5)` and `(s, port, score=7)` are exhaustive in the
/// subject axis at residual key `s`, but not at `(s, 5)` after
/// `promote score`), so the carry is unsound as stated.
///
/// Cardinality is **derived from the gradings** (ADR 0024): the gradings are
/// facts about the flat table, so the move leaves them untouched and re-runs
/// the subset check against the grown key. A `singletons` input stays
/// `singletons` (`promote_functional`), and a `bag` whose grading fits the
/// new key becomes `singletons`, the consumption being definitional (the
/// grading *is* `Functional (promote T)` for the promoted columns).
fn op_promote(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    if args.is_empty() {
        return Err(error("`promote` needs at least one column", span));
    }
    let mut errs = Vec::new();
    let mut cols: Vec<(&str, Span)> = Vec::new();
    for arg in args {
        let ExprKind::Name(col) = &arg.kind else {
            errs.push(te("`promote` expects column names", arg.span));
            continue;
        };
        cols.push((col, arg.span));
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    for (col, span) in &cols {
        if let Err(e) = promote_to_key(&mut table, col, *span) {
            errs.push(e);
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    table.qualifiers.exhaustive.clear();
    table.derive_cardinality();
    Ok(PipeTy::Table(table))
}

fn promote_to_key(table: &mut TableType, col: &str, span: Span) -> Result<(), TypeError> {
    if table.content.key.iter().any(|c| c.name == col) {
        return Err(te(format!("`{col}` is already in the key"), span));
    }
    let Some(pos) = table.content.columns.iter().position(|c| c.name == col) else {
        return Err(te(format!("unknown column `{col}`"), span));
    };
    if !table.content.columns[pos].domain.is_key_eligible() {
        return Err(te(
            format!(
                "`promote` cannot promote `{col}`: its type is not key-eligible \
                 (a continuous `real` measurement is not an identity)"
            ),
            span,
        ));
    }
    if table.qualifiers.totality.is_optional(col) {
        return Err(te(
            format!("`promote` requires `{col}` to be total; narrow it first"),
            span,
        ));
    }
    let column = table.content.columns.remove(pos);
    table.content.key.push(column);
    Ok(())
}

/// `demote cols` (section 6.3, Tier B, ADR 0017 as amended by ADR 0023):
/// drop key components into the non-key part. Content: the named key
/// columns become ordinary columns. Cardinality: **derived from the
/// gradings** (ADR 0024): the move leaves the gradings untouched and re-runs
/// the subset check against the shrunken key, so a genuine coarsening
/// rises to `bag` (no grading fits the retained key) while the round trip
/// `promote c |> demote c` re-derives `singletons` from the source
/// grading (`demote_promote`). Completeness:
/// **propagated**, not demanded: a table complete against a reference at the
/// fine key stays complete against the coarsened reference at the retained key
/// (`demote_completeWrt`); the consumer is the reducing `map_bags`
/// downstream. Lineage: **dropped** (`demote_not_preservesDisjoint`), the
/// lineage break that keeps `demote` Tier B on its own.
fn op_demote(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    if args.is_empty() {
        return Err(error("`demote` needs at least one column", span));
    }
    let mut errs = Vec::new();
    let mut to_drop: Vec<String> = Vec::new();
    for arg in args {
        let ExprKind::Name(col) = &arg.kind else {
            errs.push(te("`demote` expects column names", arg.span));
            continue;
        };
        if !table.content.key.iter().any(|c| &c.name == col) {
            errs.push(te(format!("not an key column `{col}`"), arg.span));
            continue;
        }
        if to_drop.contains(col) {
            errs.push(te(
                format!("`demote` names `{col}` more than once"),
                arg.span,
            ));
            continue;
        }
        to_drop.push(col.clone());
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    // `to_drop` is now distinct, so this counts distinct dropped key columns.
    if to_drop.len() == table.content.key.len() {
        return Err(error("`demote` must leave at least one key column", span));
    }
    // Move each dropped key column into the non-key part.
    let (dropped, kept): (Vec<Column>, Vec<Column>) = table
        .content
        .key
        .drain(..)
        .partition(|c| to_drop.contains(&c.name));
    table.content.key = kept;
    table.content.columns.extend(dropped);
    table.derive_cardinality();
    table.qualifiers.lineage = Lineage::dropped();
    // Completeness carries over unchanged: whatever fact held at the fine key
    // holds against the coarsened reference at the retained key
    // (`demote_completeWrt`, ADR 0023).
    // `exhaustive` is forfeited: ADR 0020 section 2 sketches the retained-axis
    // carry (a union of full fibers is full), but the key-changing propagation
    // rows are the ADR's open formal work item, so the checker stays
    // conservative until they are mechanized.
    table.qualifiers.exhaustive.clear();
    Ok(PipeTy::Table(table))
}

/// `unpivot name value` (section 6.6, Tier A, ADR 0020): fold **all**
/// attribute columns, which must share one domain, into one `value` column,
/// spreading their *names* into a new `enum` key column `name`. A missing
/// cell yields no row (drop semantics), so `value` is total by construction
/// (`unpivotDrop_minimal`). Cardinality and lineage are preserved
/// (`unpivotDrop_splitSafe`, `unpivotDrop_preservesDisjoint`). Establishes
/// `exhaustive(name)` exactly when every folded column is total
/// (`unpivotDrop_exhaustive`); an optional folded column drops cells, which
/// leaves holes and forfeits the row-presence facts.
fn op_unpivot(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    let [name_arg, value_arg] = args else {
        return Err(error(
            "`unpivot` takes a name column and a value column; it folds all \
             attribute columns (project first to exclude one)",
            span,
        ));
    };
    let ExprKind::Name(name_col) = &name_arg.kind else {
        return Err(error(
            "`unpivot`'s name column must be an identifier",
            name_arg.span,
        ));
    };
    let ExprKind::Name(value_col) = &value_arg.kind else {
        return Err(error(
            "`unpivot`'s value column must be an identifier",
            value_arg.span,
        ));
    };
    if name_col == value_col {
        return Err(error(
            "`unpivot`'s name and value columns must differ",
            name_arg.span,
        ));
    }

    // The fold is total over the attributes (exclusion is upstream
    // projection), and they must share one domain.
    let Some(first) = table.content.columns.first() else {
        return Err(error(
            "`unpivot` needs at least one attribute column to fold",
            span,
        ));
    };
    let domain = first.domain.clone();
    let mut errs = Vec::new();
    for c in &table.content.columns[1..] {
        if c.domain != domain {
            errs.push(te(
                format!(
                    "`unpivot` folds all attribute columns, which must share one \
                     domain; `{}` differs (project it away first)",
                    c.name
                ),
                span,
            ));
        }
    }

    // The new name and value columns must not collide with an key column
    // (every attribute is folded, so only the key survives the fold).
    for (arg, new_col) in [(name_arg, name_col), (value_arg, value_col)] {
        if table.content.key.iter().any(|c| &c.name == new_col) {
            errs.push(te(
                format!("`unpivot` would duplicate column `{new_col}`"),
                arg.span,
            ));
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    // Fold: the attribute names become the `name` enum key, their cells the
    // `value` column. A known cell keeps its row, a missing cell yields none,
    // so `value` is total regardless of the folded columns' totality.
    let folded: Vec<String> = table
        .content
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let all_total = folded.iter().all(|c| table.qualifiers.totality.is_total(c));
    table.content.columns.clear();
    for col in &folded {
        table.qualifiers.totality.narrow(col);
    }
    table.content.columns.push(Column {
        name: value_col.clone(),
        domain,
    });
    table.content.key.push(Column {
        name: name_col.clone(),
        domain: ColumnType::Enum {
            name: name_col.clone(),
            variants: folded,
        },
    });
    // A pre-existing row-presence fact would survive a total fold (ADR 0020
    // section 2), but that carry crosses a key change, which is the ADR's
    // open formal work item; the checker keeps only the fact it can establish
    // by mechanism.
    table.qualifiers.exhaustive.clear();
    if all_total {
        table.qualifiers.exhaustive.insert(name_col.clone());
    } else {
        // Dropped cells leave holes in the fibers: nothing is established,
        // and the input's completeness fact is forfeited with the rows.
        table.qualifiers.completeness = Completeness::Incomplete;
    }
    Ok(PipeTy::Table(table))
}

/// `pivot name value` (section 6.6, Tier B, ADR 0020): the inverse of
/// `unpivot`, in one form. `name` must be an enum-domained **key** column
/// (`promote` promotes an attribute first); the input must be
/// `singletons` with `value` as its only attribute (the key discipline is
/// what makes each spread cell well-defined). It consumes **no completeness
/// fact**: an absent (key, variant) row becomes a missing cell, and the
/// spread columns are total iff `exhaustive(name)` holds and `value` is
/// total (`pivot_total_of_exhaustive`; the total `value` column is the
/// `Minimal` hypothesis). Not split-invariant, so lineage is dropped
/// (`pivot_not_splitInvariant`).
fn op_pivot(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    let [name_arg, value_arg] = args else {
        return Err(error(
            "`pivot` takes a name column and a value column",
            span,
        ));
    };
    let ExprKind::Name(name_col) = &name_arg.kind else {
        return Err(error(
            "`pivot`'s name column must be an identifier",
            name_arg.span,
        ));
    };
    let ExprKind::Name(value_col) = &value_arg.kind else {
        return Err(error(
            "`pivot`'s value column must be an identifier",
            value_arg.span,
        ));
    };
    if name_col == value_col {
        return Err(error(
            "`pivot`'s name and value columns must differ",
            value_arg.span,
        ));
    }

    // `name` spreads an key column; an attribute is rejected with the
    // composition hint (ADR 0020: the bag long form is `promote` away).
    let Some(idx_pos) = table.content.key.iter().position(|c| &c.name == name_col) else {
        if table.content.columns.iter().any(|c| &c.name == name_col) {
            return Err(error(
                format!(
                    "`pivot` spreads an key column; promote `{name_col}` with \
                     `promote {name_col}` first"
                ),
                name_arg.span,
            ));
        }
        return Err(error(format!("unknown column `{name_col}`"), name_arg.span));
    };
    let ColumnType::Enum { variants, .. } = table.content.key[idx_pos].domain.clone() else {
        return Err(error(
            format!("`pivot` requires `{name_col}` to be a finite-enumerable enum"),
            name_arg.span,
        ));
    };
    if table.content.key.len() == 1 {
        return Err(error(
            "`pivot` must leave at least one key column",
            name_arg.span,
        ));
    }
    if table.qualifiers.cardinality != Cardinality::Singletons {
        return Err(error(
            "`pivot` requires a singletons input: each (key, name) cell must hold \
             at most one value; aggregate upstream first",
            span,
        ));
    }
    let Some(value_pos) = table
        .content
        .columns
        .iter()
        .position(|c| &c.name == value_col)
    else {
        return Err(error(
            format!("unknown column `{value_col}`"),
            value_arg.span,
        ));
    };
    if table.content.columns.len() != 1 {
        return Err(error(
            format!(
                "`pivot` requires `{value_col}` to be the only attribute column; \
                 drop or aggregate the others first"
            ),
            span,
        ));
    }
    let value_domain = table.content.columns[value_pos].domain.clone();
    // A spread variant must not collide with a retained key column (the
    // name key and value column are about to be removed).
    let mut errs = Vec::new();
    for variant in &variants {
        if table
            .content
            .key
            .iter()
            .any(|c| &c.name == variant && &c.name != name_col)
        {
            errs.push(te(
                format!("`pivot` would duplicate column `{variant}`"),
                name_arg.span,
            ));
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    table.content.key.remove(idx_pos);
    table.content.columns.clear();
    // The totality upgrade (ADR 0020): the rectangle supplies the row at
    // every (key, variant) and the total value column supplies the value in
    // it (`pivot_total_of_exhaustive`). A single-variant enum is exhaustive
    // trivially (`exhaustive_of_subsingleton`). Otherwise a spread cell may
    // be absent-row-turned-missing, so the columns are optional.
    let value_total = table.qualifiers.totality.is_total(value_col);
    let rectangle = table.qualifiers.exhaustive.contains(name_col.as_str()) || variants.len() == 1;
    let spread_total = rectangle && value_total;
    table.qualifiers.totality.narrow(value_col);
    for variant in variants {
        if !spread_total {
            table.qualifiers.totality.mark_optional(variant.clone());
        }
        table.content.columns.push(Column {
            name: variant,
            domain: value_domain.clone(),
        });
    }
    // The name axis leaves the key, consuming its fact. A fact over another
    // axis of the residual key would survive (ADR 0020 section 2), but that
    // carry crosses a key change, the ADR's open formal work item, so the
    // checker stays conservative.
    table.qualifiers.exhaustive.clear();
    table.qualifiers.lineage = Lineage::dropped();
    Ok(PipeTy::Table(table))
}

/// `assume { complete }` (section 8, ADR 0017): admit a completeness obligation
/// by fiat, locally and visibly. The block holds the single recognized claim
/// `complete`. In M1 the only consumable obligation is completeness, so that is
/// the only claim accepted.
fn op_assume(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    let [block_arg] = args else {
        return Err(error("`assume` takes a `{ ... }` block of claims", span));
    };
    let ExprKind::Block(block) = &block_arg.kind else {
        return Err(error("`assume` expects a `{ ... }` block", block_arg.span));
    };
    let mut errs = Vec::new();
    for stmt in &block.stmts {
        match stmt {
            Stmt::Expr(e) => match &e.kind {
                ExprKind::Name(claim) if claim == "complete" => {
                    table.qualifiers.completeness = Completeness::Complete;
                }
                _ => errs.push(te("`assume` accepts only the claim `complete`", e.span)),
            },
            Stmt::Let { value, .. } => {
                errs.push(te(
                    "`assume` blocks hold claims, not `let` bindings",
                    value.span,
                ));
            }
            Stmt::Assert(e) => {
                errs.push(te(
                    "use `completeness_check` for `assert`, not `assume`",
                    e.span,
                ));
            }
        }
    }
    if errs.is_empty() {
        Ok(PipeTy::Table(table))
    } else {
        Err(errs)
    }
}

/// `completeness_check { assert <bool>; ... }` (section 8, ADR 0017): a pipe
/// stage whose boolean asserts witness that the partition is complete over the
/// current key. Each assert is a boolean over the key context (`k`). Establishes
/// `Complete`; all other qualifiers preserved.
fn op_completeness_check(
    sources: &Sources,
    input: PipeTy,
    args: &[&Expr],
    span: Span,
) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    let [block_arg] = args else {
        return Err(error(
            "`completeness_check` takes a `{ assert ... }` block",
            span,
        ));
    };
    let ExprKind::Block(block) = &block_arg.kind else {
        return Err(error(
            "`completeness_check` expects a `{ ... }` block",
            block_arg.span,
        ));
    };
    let mut errs = Vec::new();
    let mut assert_count = 0;
    let ctx = Context::key(&sources.ambient, "k", &table);
    for stmt in &block.stmts {
        match stmt {
            Stmt::Assert(e) => {
                assert_count += 1;
                errs.extend(require_known_bool(&ctx, e));
            }
            Stmt::Let { value, .. } => {
                errs.push(te(
                    "a `completeness_check` block holds only `assert`s",
                    value.span,
                ));
            }
            Stmt::Expr(e) => {
                errs.push(te(
                    "a `completeness_check` block holds only `assert`s",
                    e.span,
                ));
            }
        }
    }
    if assert_count == 0 {
        errs.push(te(
            "`completeness_check` needs at least one `assert` to witness completeness",
            block.span,
        ));
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    table.qualifiers.completeness = Completeness::Complete;
    Ok(PipeTy::Table(table))
}

/// Decompose a curried application `f a b c` into the head `f` and the argument
/// list `[a, b, c]`. A non-application returns `(expr, [])`.
fn flatten_app(expr: &Expr) -> (&Expr, Vec<&Expr>) {
    let mut args = Vec::new();
    let mut cur = expr;
    while let ExprKind::App(func, arg) = &cur.kind {
        args.push(arg.as_ref());
        cur = func;
    }
    args.reverse();
    (cur, args)
}

/// Type a view body (a block hosting a pipeline, `docs/language/10-views.md`):
/// each `let` binding extends the source environment, and the final statement
/// is a trailing expression whose value is the materialized result. `assert`
/// (Tier B / completeness) is deferred.
pub fn type_view_body(sources: &Sources, block: &Block) -> Result<PipeTy, Vec<TypeError>> {
    let mut env = sources.clone();
    let mut errs = Vec::new();
    let mut result: Option<PipeTy> = None;
    let last = block.stmts.len().saturating_sub(1);
    for (i, stmt) in block.stmts.iter().enumerate() {
        match stmt {
            Stmt::Let { name, value, .. } => match type_pipeline(&env, value) {
                Ok(pipe) => env.bind(&name.name, pipe),
                Err(e) => errs.extend(e),
            },
            Stmt::Assert(e) => {
                errs.push(te("`assert` in a view body is not yet supported", e.span));
            }
            Stmt::Expr(e) if i == last => match type_pipeline(&env, e) {
                Ok(pipe) => result = Some(pipe),
                Err(er) => errs.extend(er),
            },
            Stmt::Expr(e) => {
                errs.push(te(
                    "a view body allows only `let` bindings before its final result expression",
                    e.span,
                ));
            }
        }
    }
    match result {
        Some(pipe) if errs.is_empty() => Ok(pipe),
        Some(_) => Err(errs),
        None => {
            errs.push(te("a view body must end in a table expression", block.span));
            Err(errs)
        }
    }
}

/// Type a view body and require it to materialize a single table (a view is not
/// a bare pair, `10-views.md`). Returns the output table type.
pub fn type_view(sources: &Sources, body: &Block) -> Result<TableType, Vec<TypeError>> {
    match type_view_body(sources, body)? {
        PipeTy::Table(table) => Ok(table),
        PipeTy::Pair(..) => Err(error(
            "a view must materialize a single table, not a pair",
            body.span,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Column as StorageColumn, ColumnRole, ColumnType, Schema};
    use crate::table::Cardinality;

    fn scol(name: &str, ty: ColumnType, role: ColumnRole, optional: bool) -> StorageColumn {
        StorageColumn {
            name: name.to_string(),
            ty,
            role,
            optional,
            span: Span::new(0, 0),
        }
    }

    fn from_cols(store: &str, unit: &str, columns: Vec<StorageColumn>) -> TableType {
        TableType::from_store(&Schema {
            store: store.to_string(),
            unit: unit.to_string(),
            columns,
            cardinality: Cardinality::Singletons,
            span: Span::new(0, 0),
        })
    }

    fn sample_sources() -> Sources {
        let readings = from_cols(
            "readings",
            "Reading",
            vec![
                scol("ts", ColumnType::Int, ColumnRole::Key, false),
                scol("machine", ColumnType::String, ColumnRole::Attr, false),
                scol("temperature", ColumnType::Real, ColumnRole::Attr, false),
                scol("peak", ColumnType::Real, ColumnRole::Attr, true),
                scol("flag", ColumnType::Bool, ColumnRole::Attr, false),
                scol("note", ColumnType::String, ColumnRole::Attr, true),
            ],
        );
        let machines = from_cols(
            "machines",
            "Machine",
            vec![
                scol("machine", ColumnType::String, ColumnRole::Key, false),
                scol("vendor", ColumnType::String, ColumnRole::Attr, false),
            ],
        );
        Sources::new()
            .with("readings", readings)
            .with("machines", machines)
    }

    fn pipe_ty(sources: &Sources, src: &str) -> Result<PipeTy, Vec<TypeError>> {
        let toks = mensura_syntax::tokenize(src).expect("lex");
        let expr = mensura_syntax::parse_expr(&toks).expect("parse");
        type_pipeline(sources, &expr)
    }

    fn table_of(pipe: PipeTy) -> TableType {
        match pipe {
            PipeTy::Table(t) => t,
            PipeTy::Pair(..) => panic!("expected a single table, found a pair"),
        }
    }

    #[test]
    fn source_name_is_its_table() {
        let s = sample_sources();
        let PipeTy::Table(t) = pipe_ty(&s, "readings").expect("a table") else {
            panic!("readings should be a table");
        };
        assert_eq!(t.content.key[0].name, "ts");
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn unknown_source_errors() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "ghost").expect_err("unknown source");
        assert!(errs[0].message.contains("unknown source `ghost`"));
    }

    #[test]
    fn tuple_of_two_is_a_pair() {
        let s = sample_sources();
        assert!(matches!(
            pipe_ty(&s, "(readings, readings)"),
            Ok(PipeTy::Pair(..))
        ));
    }

    #[test]
    fn unknown_operation_errors() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> nope").expect_err("unknown op");
        assert!(errs[0].message.contains("unsupported operation `nope`"));
    }

    #[test]
    fn unknown_source_suggests_a_close_name() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readngs").expect_err("typo");
        assert!(errs[0].message.contains("did you mean `readings`?"));
    }

    #[test]
    fn unknown_operation_suggests_a_close_name() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> demot machine").expect_err("typo");
        assert!(errs[0].message.contains("did you mean `demote`?"));
    }

    #[test]
    fn promote_promotes_a_total_column() {
        let s = sample_sources();
        let t = table_of(pipe_ty(&s, "readings |> promote machine").expect("ok"));
        assert!(t.content.key.iter().any(|c| c.name == "machine"));
        assert!(!t.content.columns.iter().any(|c| c.name == "machine"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn promote_rejects_optional_column() {
        let s = sample_sources();
        // `note` is key-eligible (string) but optional.
        let errs = pipe_ty(&s, "readings |> promote note").expect_err("optional");
        assert!(errs[0].message.contains("to be total"));
    }

    #[test]
    fn promote_rejects_real_column() {
        let s = sample_sources();
        // `temperature` is a real measurement: not key-eligible (ADR 0014).
        let errs = pipe_ty(&s, "readings |> promote temperature").expect_err("real");
        assert!(errs[0].message.contains("key-eligible"));
    }

    #[test]
    fn promote_unknown_column_errors() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> promote bogus").expect_err("unknown column");
        assert!(errs[0].message.contains("unknown column `bogus`"));
    }

    #[test]
    fn map_derives_columns_preserving_cardinality() {
        let s = sample_sources();
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> flat_map |k, r| (.hot = r.temperature > 30.0)",
            )
            .expect("ok"),
        );
        assert!(t.content.key.iter().any(|c| c.name == "ts"));
        assert_eq!(t.content.columns.len(), 1);
        assert_eq!(t.content.columns[0].name, "hot");
        assert_eq!(t.content.columns[0].domain, ColumnType::Bool);
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn map_propagates_field_errors() {
        let s = sample_sources();
        // `peak` is optional, so a scalar on it is rejected by expr_check.
        let errs =
            pipe_ty(&s, "readings |> flat_map |k, r| (.x = r.peak + 1)").expect_err("optional");
        assert!(errs[0].message.contains("known value"));
    }

    #[test]
    fn map_keeps_the_value_row() {
        let s = sample_sources();
        // `|k, r| r` is the identity: the value columns are preserved, the key
        // and cardinality unchanged (ADR 0015).
        let t = table_of(pipe_ty(&s, "readings |> flat_map |k, r| r").expect("ok"));
        assert!(t.content.key.iter().any(|c| c.name == "ts"));
        for name in ["machine", "temperature", "peak", "flag", "note"] {
            assert!(t.content.columns.iter().any(|c| c.name == name), "{name}");
        }
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn map_filters_by_dropping_a_branch() {
        let s = sample_sources();
        // `if r.flag then r else ()` keeps or drops a row: a filter, still
        // `Singletons` (max collection size is 1).
        let t = table_of(
            pipe_ty(&s, "readings |> flat_map |_, r| if r.flag then r else ()").expect("ok"),
        );
        assert!(t.content.columns.iter().any(|c| c.name == "temperature"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn map_expands_to_a_bag() {
        let s = sample_sources();
        // A two-row collection expands each input row: cardinality becomes `Bag`.
        let t = table_of(pipe_ty(&s, "readings |> flat_map |k, r| (r, r)").expect("ok"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
    }

    #[test]
    fn map_that_always_drops_cannot_infer_schema() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> flat_map |k, r| ()").expect_err("no schema");
        assert!(errs[0].message.contains("infer the output"));
    }

    #[test]
    fn map_row_may_not_set_the_index() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> flat_map |k, r| (.ts = 1)").expect_err("key column");
        assert!(errs[0].message.contains("key column `ts`"));
    }

    #[test]
    fn map_bags_summarizes_to_singletons() {
        let s = sample_sources();
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine \
                 |> map_bags |k, b| (.temp_mean = sum b.temperature / to_real (count b.temperature), .temp_max = max b.temperature)",
            )
            .expect("ok"),
        );
        assert!(t.content.key.iter().any(|c| c.name == "machine"));
        assert!(t.content.columns.iter().any(|c| c.name == "temp_mean"));
        assert!(t.content.columns.iter().any(|c| c.name == "temp_max"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn map_bags_rejects_non_numeric_aggregate() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> map_bags |k, b| (.m = sum b.machine)")
            .expect_err("non-numeric");
        assert!(errs[0].message.contains("numeric bag"));
    }

    #[test]
    fn map_bags_with_a_bag_field_stays_a_bag() {
        let s = sample_sources();
        // A bag-valued field is the window shape: one output row per input row.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> map_bags |k, b| (.temps = b.temperature)",
            )
            .expect("ok"),
        );
        assert!(t.content.columns.iter().any(|c| c.name == "temps"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
    }

    #[test]
    fn map_bags_rejects_mixed_aggregate_and_window() {
        let s = sample_sources();
        let errs = pipe_ty(
            &s,
            "readings |> map_bags |k, b| (.m = sum b.temperature, .t = b.temperature)",
        )
        .expect_err("mixed");
        assert!(errs.iter().any(|e| e.message.contains("not a mix")));
    }

    #[test]
    fn split_yields_disjoint_halves() {
        let s = sample_sources();
        let PipeTy::Pair(a, b) = pipe_ty(&s, "readings |> split |k| k.ts > 100").expect("ok")
        else {
            panic!("split yields a pair");
        };
        assert!(a.qualifiers.lineage.disjoint(&b.qualifiers.lineage));
        assert_eq!(a.content, b.content);
        assert_eq!(a.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn split_rejects_non_bool_predicate() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> split |k| k.ts").expect_err("non-bool");
        assert!(errs[0].message.contains("must be a boolean"));
    }

    #[test]
    fn split_predicate_sees_only_index() {
        let s = sample_sources();
        // `machine` is a column, not in the key, so it is unknown in the key.
        let errs = pipe_ty(&s, "readings |> split |k| k.machine == \"m1\"").expect_err("unknown");
        assert!(errs[0].message.contains("unknown column `machine`"));
    }

    #[test]
    fn bind_reconstructs_disjoint_split() {
        let s = sample_sources();
        let t = table_of(pipe_ty(&s, "readings |> split |k| k.ts > 100 |> union").expect("ok"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn bind_of_overlapping_inputs_is_a_bag() {
        let s = sample_sources();
        let t = table_of(pipe_ty(&s, "(readings, readings) |> union").expect("ok"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
    }

    #[test]
    fn bind_requires_a_pair() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> union").expect_err("not a pair");
        assert!(errs[0].message.contains("expects a pair"));
    }

    #[test]
    fn lookup_adds_optional_columns() {
        let s = sample_sources();
        let t =
            table_of(pipe_ty(&s, "readings |> lookup machines (|_, r| r.machine)").expect("ok"));
        assert!(t.content.columns.iter().any(|c| c.name == "vendor"));
        assert!(t.qualifiers.totality.is_optional("vendor"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn lookup_total_keeps_totality() {
        let s = sample_sources();
        let t = table_of(
            pipe_ty(&s, "readings |> lookup_total machines (|_, r| r.machine)").expect("ok"),
        );
        assert!(t.qualifiers.totality.is_total("vendor"));
    }

    #[test]
    fn join_key_domain_must_match() {
        let s = sample_sources();
        // `ts` is a number, but `machines` is keyed by a string.
        let errs = pipe_ty(&s, "readings |> lookup machines (|k, r| k.ts)").expect_err("domain");
        assert!(errs[0].message.contains("key domain"));
    }

    /// A homogeneous wide source, the shape `unpivot` folds: all attributes
    /// share one domain. `sparse` marks `hi` optional.
    fn wide_source(sparse: bool) -> Sources {
        let wide = from_cols(
            "wide",
            "Slot",
            vec![
                scol("ts", ColumnType::Int, ColumnRole::Key, false),
                scol("lo", ColumnType::Real, ColumnRole::Attr, false),
                scol("hi", ColumnType::Real, ColumnRole::Attr, sparse),
            ],
        );
        Sources::new().with("wide", wide)
    }

    #[test]
    fn unpivot_folds_all_attributes_and_establishes_exhaustive() {
        let s = wide_source(false);
        let t = table_of(pipe_ty(&s, "wide |> unpivot metric reading").expect("ok"));
        let metric = t
            .content
            .key
            .iter()
            .find(|c| c.name == "metric")
            .expect("metric in the key");
        assert!(matches!(metric.domain, ColumnType::Enum { .. }));
        assert_eq!(t.content.columns.len(), 1);
        assert_eq!(t.content.columns[0].name, "reading");
        // Drop semantics: the long value column is total by construction.
        assert!(t.qualifiers.totality.is_total("reading"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        // Every folded column is total: the rectangle holds by mechanism
        // (`unpivotDrop_exhaustive`).
        assert!(t.qualifiers.exhaustive.contains("metric"));
    }

    #[test]
    fn unpivot_of_an_optional_column_establishes_nothing() {
        let s = wide_source(true);
        let t = table_of(pipe_ty(&s, "wide |> unpivot metric reading").expect("ok"));
        // The dropped cells leave holes: no rectangle fact; the value column
        // is still total (a missing cell yields no row).
        assert!(t.qualifiers.exhaustive.is_empty());
        assert!(t.qualifiers.totality.is_total("reading"));
    }

    #[test]
    fn unpivot_rejects_mismatched_domains() {
        let s = sample_sources();
        // `readings` mixes real, bool, and string attributes: no shared domain.
        let errs = pipe_ty(&s, "readings |> unpivot metric reading").expect_err("domain mismatch");
        assert!(errs[0].message.contains("share one domain"));
    }

    #[test]
    fn unpivot_needs_an_attribute_to_fold() {
        let bare = from_cols(
            "bare",
            "Slot",
            vec![scol("ts", ColumnType::Int, ColumnRole::Key, false)],
        );
        let s = Sources::new().with("bare", bare);
        let errs = pipe_ty(&s, "bare |> unpivot metric reading").expect_err("no attributes");
        assert!(errs[0].message.contains("at least one attribute"));
    }

    #[test]
    fn unpivot_rejects_the_retired_column_list() {
        let s = wide_source(false);
        // ADR 0016's list form is gone: the fold is total over the attributes.
        let errs = pipe_ty(&s, "wide |> unpivot metric reading (lo, hi)").expect_err("list form");
        assert!(errs[0].message.contains("folds all attribute columns"));
    }

    /// A source with a non-key `enum` column: `pivot` rejects it with the
    /// `promote` hint (the bag long form is composition, ADR 0020).
    fn enum_source() -> Sources {
        let obs = from_cols(
            "obs",
            "Obs",
            vec![
                scol("ts", ColumnType::Int, ColumnRole::Key, false),
                scol(
                    "metric",
                    ColumnType::Enum {
                        name: "Metric".to_string(),
                        variants: vec!["lo".to_string(), "hi".to_string()],
                    },
                    ColumnRole::Attr,
                    false,
                ),
                scol("reading", ColumnType::Real, ColumnRole::Attr, false),
                scol("tag", ColumnType::String, ColumnRole::Attr, false),
            ],
        );
        Sources::new().with("obs", obs)
    }

    #[test]
    fn pivot_inverts_unpivot_with_no_discharge() {
        let s = wide_source(false);
        let whole = table_of(pipe_ty(&s, "wide").expect("source"));
        let t = table_of(
            pipe_ty(&s, "wide |> unpivot metric reading |> pivot metric reading").expect("ok"),
        );
        // The round trip restores the wide content with no `assume` and no
        // completeness discharge (`pivot_unpivotDrop`); the rectangle
        // established by `unpivot` upgrades the spread columns to total.
        assert_eq!(t.content, whole.content);
        assert!(t.qualifiers.totality.is_total("lo"));
        assert!(t.qualifiers.totality.is_total("hi"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        // `pivot` drops lineage (`pivot_not_splitInvariant`) and consumes the
        // rectangle.
        assert_eq!(t.qualifiers.lineage, Lineage::root());
        assert!(t.qualifiers.exhaustive.is_empty());
    }

    #[test]
    fn pivot_of_a_sparse_fold_spreads_optional_columns() {
        let s = wide_source(true);
        // `hi` is optional: `unpivot` establishes nothing, and `pivot`
        // honestly yields optional spread columns (no obligation, ADR 0020).
        let t = table_of(
            pipe_ty(&s, "wide |> unpivot metric reading |> pivot metric reading").expect("ok"),
        );
        assert!(t.qualifiers.totality.is_optional("lo"));
        assert!(t.qualifiers.totality.is_optional("hi"));
    }

    #[test]
    fn pivot_rejects_an_attribute_name_column() {
        let s = enum_source();
        // `metric` sits in attribute position: the bag long form is reachable
        // by composition, so the rejection points at `promote`.
        let errs = pipe_ty(&s, "obs |> pivot metric reading").expect_err("attribute position");
        assert!(errs[0].message.contains("promote"));
    }

    #[test]
    fn pivot_after_promote_spreads_a_promoted_enum() {
        let s = enum_source();
        let t = table_of(
            pipe_ty(
                &s,
                "obs |> flat_map |_, r| (.metric = r.metric, .reading = r.reading) \
                 |> promote metric |> pivot metric reading",
            )
            .expect("ok"),
        );
        assert!(t.content.columns.iter().any(|c| c.name == "lo"));
        assert!(t.content.columns.iter().any(|c| c.name == "hi"));
        // Stored sparse data pivots directly: no rectangle fact, so the
        // spread columns are honestly optional (ADR 0020).
        assert!(t.qualifiers.totality.is_optional("lo"));
        assert!(t.qualifiers.totality.is_optional("hi"));
    }

    #[test]
    fn pivot_demands_singletons() {
        let s = wide_source(false);
        // A bag input (expanded by map) cannot pivot: cells are not card <= 1.
        let errs = pipe_ty(
            &s,
            "wide |> unpivot metric reading |> flat_map |k, r| (r, r) |> pivot metric reading",
        )
        .expect_err("bag");
        assert!(errs[0].message.contains("singletons"));
    }

    #[test]
    fn pivot_name_column_must_be_enum() {
        let s = sample_sources();
        // `ts` is an key column, but `int` is not finite-enumerable.
        let errs = pipe_ty(&s, "readings |> promote machine |> pivot ts machine")
            .expect_err("not enumerable");
        assert!(errs[0].message.contains("finite-enumerable"));
    }

    #[test]
    fn pivot_requires_the_value_to_be_the_only_attribute() {
        let s = wide_source(false);
        // A surviving second attribute must be dropped or aggregated first.
        let errs = pipe_ty(
            &s,
            "wide |> unpivot metric reading \
             |> flat_map |_, r| (.reading = r.reading, .extra = 1) \
             |> pivot metric reading",
        )
        .expect_err("extra attribute");
        assert!(errs[0].message.contains("only attribute"));
    }

    #[test]
    fn pivot_must_leave_an_index_column() {
        // A long table keyed by the name axis alone cannot pivot.
        let long = from_cols(
            "long",
            "Slot",
            vec![
                scol(
                    "metric",
                    ColumnType::Enum {
                        name: "Metric".to_string(),
                        variants: vec!["lo".to_string(), "hi".to_string()],
                    },
                    ColumnRole::Key,
                    false,
                ),
                scol("reading", ColumnType::Real, ColumnRole::Attr, false),
            ],
        );
        let s = Sources::new().with("long", long);
        let errs = pipe_ty(&s, "long |> pivot metric reading").expect_err("empty key");
        assert!(errs[0].message.contains("at least one key"));
    }

    #[test]
    fn demote_propagates_completeness_to_the_coarser_key() {
        let s = sample_sources();
        // Promote `machine`, establish completeness, then genuinely coarsen
        // by dropping `ts`: result is a bag (no grading fits the retained
        // key), still complete (against the coarsened reference,
        // `demote_completeWrt`), with lineage dropped.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> assume { complete } \
                 |> demote ts",
            )
            .expect("ok"),
        );
        assert!(!t.content.key.iter().any(|c| c.name == "ts"));
        assert!(t.content.columns.iter().any(|c| c.name == "ts"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
        assert_eq!(t.qualifiers.completeness, Completeness::Complete);
        assert_eq!(t.qualifiers.lineage, Lineage::root());
    }

    #[test]
    fn demote_alone_needs_no_completeness() {
        let s = sample_sources();
        // ADR 0023: a rekey with no downstream reducer is admitted on its
        // own; a possibly partial bag is an honest representation of the rows
        // present, and the obligation belongs to the reducer.  Shrinking a
        // column other than the promoted one keeps this a genuine rekey
        // rather than an ADR 0024 round trip.
        let t = table_of(pipe_ty(&s, "readings |> promote machine |> demote ts").expect("ok"));
        assert!(t.content.key.iter().any(|c| c.name == "machine"));
        assert!(t.content.columns.iter().any(|c| c.name == "ts"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
        assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
        assert_eq!(t.qualifiers.lineage, Lineage::root());
    }

    #[test]
    fn reducing_map_bags_over_a_bag_demands_completeness() {
        let s = sample_sources();
        // The fold lands where the unsoundness is (ADR 0023): a reducing
        // `map_bags` over a possibly partial bag is rejected without an
        // establish step.  The rekey must not be an exact ADR 0024 round
        // trip, or the singletons input would discharge trivially.
        let errs = pipe_ty(
            &s,
            "readings |> promote machine |> demote ts \
             |> map_bags |k, b| (.n = count b.temperature)",
        )
        .expect_err("incomplete bag");
        assert!(errs[0].message.contains("reducing `map_bags`"), "{errs:?}");
    }

    /// Equality up to attribute order (ADR 0024): a round trip restores the
    /// key (order included), the attribute *set*, and every qualifier;
    /// demoted columns re-enter at the end of the attribute list, so the
    /// attribute order itself is not part of the restoration.
    fn assert_restores(t: &TableType, base: &TableType) {
        assert_eq!(t.content.key, base.content.key);
        let names = |cols: &[Column]| -> BTreeMap<String, ColumnType> {
            cols.iter()
                .map(|c| (c.name.clone(), c.domain.clone()))
                .collect()
        };
        assert_eq!(names(&t.content.columns), names(&base.content.columns));
        assert_eq!(t.qualifiers, base.qualifiers);
    }

    #[test]
    fn round_trip_restores_the_source_type() {
        let s = sample_sources();
        // ADR 0024: the promote/demote round trip re-derives the source's
        // type from the surviving grading (`demote_promote` at the type
        // level): `singletons` again, every qualifier restored.
        let base = table_of(pipe_ty(&s, "readings").expect("ok"));
        let t = table_of(pipe_ty(&s, "readings |> promote machine |> demote machine").expect("ok"));
        assert_restores(&t, &base);
    }

    #[test]
    fn round_trip_restores_in_the_demote_first_order() {
        // The opposite composition (`promote_demote`, ADR 0024) restores
        // unconditionally: the demoted column is total by construction, and
        // the source grading fits the re-grown key.
        let events = from_cols(
            "events",
            "Event",
            vec![
                scol("ts", ColumnType::Int, ColumnRole::Key, false),
                scol("machine", ColumnType::String, ColumnRole::Key, false),
                scol("temperature", ColumnType::Real, ColumnRole::Attr, false),
            ],
        );
        let s = Sources::new().with("events", events);
        let base = table_of(pipe_ty(&s, "events").expect("ok"));
        let t = table_of(pipe_ty(&s, "events |> demote machine |> promote machine").expect("ok"));
        assert_restores(&t, &base);
    }

    #[test]
    fn chained_moves_round_trip_through_one_grading() {
        let s = sample_sources();
        // Chained promotions need no stack (ADR 0024): the single source
        // grading survives every move untouched, and each demotion re-runs
        // the subset check against the shrunken key.
        let base = table_of(pipe_ty(&s, "readings").expect("ok"));
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> promote flag \
                 |> demote flag |> demote machine",
            )
            .expect("ok"),
        );
        assert_restores(&t, &base);
    }

    #[test]
    fn round_trip_survives_a_content_identity_stage() {
        let s = sample_sources();
        // What a snapshot mechanism could not do (ADR 0024): the gradings
        // ride through `assume`/`completeness_check`, so the round trip
        // still restores `singletons` with an establish step between the
        // moves.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> assume { complete } \
                 |> demote machine",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        assert_eq!(t.qualifiers.completeness, Completeness::Complete);
    }

    #[test]
    fn promotion_consumes_a_grading_from_a_bag() {
        let s = sample_sources();
        // The cross-view shape in one pipeline (ADR 0024): the bag over
        // `machine` still carries the `{ts}` grading, so promoting `ts`
        // back is `singletons` again; consumption is definitional (the
        // grading *is* `Functional (promote T)`).
        let bag = table_of(pipe_ty(&s, "readings |> promote machine |> demote ts").expect("ok"));
        assert_eq!(bag.qualifiers.cardinality, Cardinality::Bag);
        let t = table_of(
            pipe_ty(&s, "readings |> promote machine |> demote ts |> promote ts").expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn an_intervening_map_resets_the_gradings() {
        let s = sample_sources();
        // The conservative rule (ADR 0024): `flat_map` has no transport witness,
        // so it resets the gradings to its own output cardinality and the
        // later demotion is a genuine coarsening, not a round trip.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine \
                 |> flat_map |k, r| (.temperature = r.temperature) \
                 |> demote machine",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
    }

    #[test]
    fn reducing_map_bags_after_an_exact_round_trip_is_admitted() {
        let s = sample_sources();
        // ADR 0024 composed with ADR 0023: the round trip restores the
        // `singletons` cardinality, so the reducer's obligation discharges
        // trivially (`fiberCompleteWrt_of_functional`) with no establish step.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> demote machine \
                 |> map_bags |k, b| (.n = count b.temperature)",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn completeness_propagates_through_demote_to_the_reducer() {
        let s = sample_sources();
        // The establish step may sit before or after `demote`; either way
        // the reducer's demand is met (ADR 0023).
        for src in [
            "readings |> promote machine |> assume { complete } |> demote machine \
             |> map_bags |k, b| (.n = count b.temperature)",
            "readings |> promote machine |> demote machine |> assume { complete } \
             |> map_bags |k, b| (.n = count b.temperature)",
        ] {
            let t = table_of(pipe_ty(&s, src).expect("ok"));
            assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        }
    }

    #[test]
    fn window_map_bags_over_a_bag_needs_no_completeness() {
        let s = sample_sources();
        // Only the reducing shape consumes the fact; a window body (a bag
        // return) is one output row per input row, faithful on a partial bag.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> demote machine \
                 |> map_bags |k, b| (.temps = b.temperature)",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
    }

    #[test]
    fn reducing_map_bags_over_singletons_discharges_trivially() {
        let s = sample_sources();
        // At `card <= 1` a present key's single row is its whole fiber
        // (`fiberCompleteWrt_of_functional`), so the ordinary aggregation over
        // a plain store is ceremony-free (ADR 0023).
        let t = table_of(
            pipe_ty(&s, "readings |> map_bags |k, b| (.m = max b.temperature)").expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn demote_cannot_empty_the_index() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> demote ts").expect_err("empty key");
        assert!(errs[0].message.contains("at least one key"));
    }

    #[test]
    fn demote_unknown_column_errors() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> demote bogus").expect_err("unknown");
        assert!(errs[0].message.contains("not an key column `bogus`"));
    }

    #[test]
    fn assume_establishes_completeness() {
        let s = sample_sources();
        let t = table_of(pipe_ty(&s, "readings |> assume { complete }").expect("ok"));
        assert_eq!(t.qualifiers.completeness, Completeness::Complete);
    }

    #[test]
    fn assume_rejects_unknown_claim() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> assume { whatever }").expect_err("unknown claim");
        assert!(errs[0].message.contains("assume"));
    }

    #[test]
    fn completeness_check_establishes_completeness() {
        let s = sample_sources();
        let t = table_of(
            pipe_ty(&s, "readings |> completeness_check { assert k.ts > 0 }").expect("ok"),
        );
        assert_eq!(t.qualifiers.completeness, Completeness::Complete);
    }

    #[test]
    fn completeness_check_needs_a_boolean_assert() {
        let s = sample_sources();
        let errs =
            pipe_ty(&s, "readings |> completeness_check { assert k.ts }").expect_err("non-bool");
        assert!(errs[0].message.contains("boolean"));
    }

    #[test]
    fn completeness_check_needs_at_least_one_assert() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> completeness_check { }").expect_err("empty");
        assert!(errs[0].message.contains("at least one"));
    }

    #[test]
    fn unpivot_rejects_duplicate_output_column() {
        let s = wide_source(false);
        // The new name column collides with the `ts` key column.
        let errs = pipe_ty(&s, "wide |> unpivot ts reading").expect_err("duplicate");
        assert!(errs[0].message.contains("would duplicate column `ts`"));
    }

    #[test]
    fn unpivot_rejects_name_equal_value() {
        let s = wide_source(false);
        let errs = pipe_ty(&s, "wide |> unpivot x x").expect_err("name == value");
        assert!(errs[0].message.contains("must differ"));
    }

    #[test]
    fn pivot_rejects_name_equal_value() {
        let s = enum_source();
        let errs = pipe_ty(&s, "obs |> pivot metric metric").expect_err("name == value");
        assert!(errs[0].message.contains("must differ"));
    }

    #[test]
    fn pivot_rejects_variant_colliding_with_a_column() {
        // A retained key column `lo` collides with the spread variant `lo`.
        let long = from_cols(
            "long",
            "Slot",
            vec![
                scol("ts", ColumnType::Int, ColumnRole::Key, false),
                scol("lo", ColumnType::Int, ColumnRole::Key, false),
                scol(
                    "metric",
                    ColumnType::Enum {
                        name: "Metric".to_string(),
                        variants: vec!["lo".to_string(), "hi".to_string()],
                    },
                    ColumnRole::Key,
                    false,
                ),
                scol("reading", ColumnType::Real, ColumnRole::Attr, false),
            ],
        );
        let s = Sources::new().with("long", long);
        let errs = pipe_ty(&s, "long |> pivot metric reading").expect_err("collision");
        assert!(errs[0].message.contains("would duplicate column `lo`"));
    }

    // Propagation of the rectangle fact (ADR 0020 section 2): each rule below
    // is backed by a theorem in `formal/Mensura/` (section 11 of the typing
    // reference); the key-changing carries stay conservative until their
    // formal work item lands.

    #[test]
    fn exhaustive_survives_a_non_dropping_map() {
        let s = wide_source(false);
        // The body has no `( )` branch: `map_exhaustive`.
        let t = table_of(
            pipe_ty(
                &s,
                "wide |> unpivot metric reading |> flat_map |_, r| (.r2 = r.reading)",
            )
            .expect("ok"),
        );
        assert!(t.qualifiers.exhaustive.contains("metric"));
    }

    #[test]
    fn a_dropping_map_forfeits_exhaustive() {
        let s = wide_source(false);
        // A filter can empty one variant of a fiber (`map_exhaustive`'s
        // non-dropping hypothesis is necessary).
        let t = table_of(
            pipe_ty(
                &s,
                "wide |> unpivot metric reading \
                 |> flat_map |_, r| if r.reading > 0.0 then r else ()",
            )
            .expect("ok"),
        );
        assert!(t.qualifiers.exhaustive.is_empty());
    }

    #[test]
    fn exhaustive_survives_map_bags_and_bind_of_carriers() {
        let s = wide_source(false);
        // `fiberMap_exhaustive` (both `map_bags` shapes) and
        // `bind_exhaustive` (a union of full fibers is full).
        let t = table_of(
            pipe_ty(
                &s,
                "wide |> unpivot metric reading \
                 |> map_bags |_, b| (.reading = max b.reading)",
            )
            .expect("ok"),
        );
        assert!(t.qualifiers.exhaustive.contains("metric"));
        let bound = table_of(
            pipe_ty(
                &s,
                "(wide |> unpivot metric reading, wide |> unpivot metric reading) |> union",
            )
            .expect("ok"),
        );
        assert!(bound.qualifiers.exhaustive.contains("metric"));
    }

    #[test]
    fn split_forfeits_exhaustive() {
        let s = wide_source(false);
        // A key predicate can cut a fiber across sides
        // (`split_not_exhaustive`), so both branches lose the fact and the
        // rebound table carries none.
        let t = table_of(
            pipe_ty(
                &s,
                "wide |> unpivot metric reading |> split |k| k.ts > 100 |> union",
            )
            .expect("ok"),
        );
        assert!(t.qualifiers.exhaustive.is_empty());
    }

    #[test]
    fn promote_forfeits_exhaustive() {
        // The promoted column refines the residual key, which can cut a
        // fiber: the ADR 0020 section 2 sketch does not hold for this row,
        // so the checker forfeits the fact (see `op_promote`).
        let wide = from_cols(
            "wide",
            "Slot",
            vec![
                scol("ts", ColumnType::Int, ColumnRole::Key, false),
                scol("lo", ColumnType::String, ColumnRole::Attr, false),
                scol("hi", ColumnType::String, ColumnRole::Attr, false),
            ],
        );
        let s = Sources::new().with("wide", wide);
        let t =
            table_of(pipe_ty(&s, "wide |> unpivot metric reading |> promote reading").expect("ok"));
        assert!(t.qualifiers.exhaustive.is_empty());
    }

    #[test]
    fn lookup_total_forfeits_exhaustive() {
        let wide = from_cols(
            "wide",
            "Slot",
            vec![
                scol("ts", ColumnType::Int, ColumnRole::Key, false),
                scol("lo", ColumnType::String, ColumnRole::Attr, false),
                scol("hi", ColumnType::String, ColumnRole::Attr, false),
            ],
        );
        let machines = from_cols(
            "machines",
            "Machine",
            vec![
                scol("machine", ColumnType::String, ColumnRole::Key, false),
                scol("vendor", ColumnType::String, ColumnRole::Attr, false),
            ],
        );
        let s = Sources::new().with("wide", wide).with("machines", machines);
        // An unmatched row drops out of its fiber; `lookup` keeps it
        // (`lookup_exhaustive`), `lookup_total` does not.
        let kept = table_of(
            pipe_ty(
                &s,
                "wide |> unpivot metric reading |> lookup machines (|_, r| r.reading)",
            )
            .expect("ok"),
        );
        assert!(kept.qualifiers.exhaustive.contains("metric"));
        let dropped = table_of(
            pipe_ty(
                &s,
                "wide |> unpivot metric reading |> lookup_total machines (|_, r| r.reading)",
            )
            .expect("ok"),
        );
        assert!(dropped.qualifiers.exhaustive.is_empty());
    }

    #[test]
    fn demote_rejects_repeated_column() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> promote machine |> demote machine machine")
            .expect_err("repeated");
        assert!(errs[0].message.contains("more than once"));
    }

    // The two worked examples from docs/language/10-views.md, end to end.

    #[test]
    fn worked_example_machine_temperature() {
        let s = sample_sources();
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine \
                 |> map_bags |k, b| (.temp_mean = sum b.temperature / to_real (count b.temperature), .temp_max = max b.temperature)",
            )
            .expect("machine_temperature types"),
        );
        assert!(t.content.key.iter().any(|c| c.name == "ts"));
        assert!(t.content.key.iter().any(|c| c.name == "machine"));
        assert_eq!(t.content.columns.len(), 2);
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn worked_example_full_dataset_reconstructs() {
        let s = sample_sources();
        let whole = table_of(pipe_ty(&s, "readings").expect("source"));
        let rebound = table_of(
            pipe_ty(&s, "readings |> split |k| k.ts > 100 |> union").expect("full_dataset"),
        );
        // Binding the disjoint split halves reconstructs the schema and keeps
        // `singletons` (union_split, 09 §11).
        assert_eq!(rebound.content, whole.content);
        assert_eq!(rebound.qualifiers.cardinality, Cardinality::Singletons);
    }

    fn view_body(src: &str) -> Block {
        let toks = mensura_syntax::tokenize(src).expect("lex");
        let program = mensura_syntax::parse(&toks).expect("parse");
        match program.items.into_iter().next().expect("an item") {
            mensura_syntax::Item::View(v) => v.body,
            _ => panic!("expected a view"),
        }
    }

    #[test]
    fn view_body_typechecks_machine_temperature() {
        let s = sample_sources();
        let body = view_body(
            "view machine_temperature { readings |> promote machine \
             |> map_bags |k, b| (.temp_max = max b.temperature) }",
        );
        let t = type_view(&s, &body).expect("ok");
        assert!(t.content.key.iter().any(|c| c.name == "machine"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn view_body_accepts_bare_application() {
        // The exact path the `bare_application` corpus case takes (resolve ->
        // type_view -> type_pipeline): a view body written `op args data`, equal
        // to its `data |> op args` pipe mirror (ADR 0018).
        let s = sample_sources();
        let bare = view_body("view v { promote machine readings }");
        let piped = view_body("view v { readings |> promote machine }");
        assert_eq!(type_view(&s, &bare), type_view(&s, &piped));
    }

    #[test]
    fn view_body_threads_let_bindings() {
        let s = sample_sources();
        let body = view_body(
            "view full_dataset { let parts = readings |> split |k| k.ts > 100; parts |> union }",
        );
        let t = type_view(&s, &body).expect("ok");
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn view_must_materialize_a_single_table() {
        let s = sample_sources();
        let body = view_body("view bad { readings |> split |k| k.ts > 100 }");
        let errs = type_view(&s, &body).expect_err("pair");
        assert!(errs[0].message.contains("single table"));
    }

    #[test]
    fn view_assert_is_deferred() {
        let s = sample_sources();
        let body = view_body("view bad { assert true; readings }");
        let errs = type_view(&s, &body).expect_err("assert");
        assert!(errs[0].message.contains("assert"));
    }

    // ADR 0018: `data |> op args` and `op args data` are one application, checked
    // identically. The result type carries no source spans, so the two spellings
    // produce an equal `PipeTy` (`docs/toolkit/01-application-checking.md`).

    #[test]
    fn application_equals_pipe_for_promote() {
        let s = sample_sources();
        assert_eq!(
            pipe_ty(&s, "promote machine readings"),
            pipe_ty(&s, "readings |> promote machine"),
        );
    }

    #[test]
    fn application_equals_pipe_for_map() {
        let s = sample_sources();
        // The lambda is parenthesized in the bare form so the trailing `readings`
        // is the operation's input, not part of the lambda body.
        assert_eq!(
            pipe_ty(&s, "flat_map (|k, r| r) readings"),
            pipe_ty(&s, "readings |> flat_map |k, r| r"),
        );
    }

    #[test]
    fn application_equals_pipe_for_join() {
        let s = sample_sources();
        assert_eq!(
            pipe_ty(&s, "lookup machines (|_, r| r.machine) readings"),
            pipe_ty(&s, "readings |> lookup machines (|_, r| r.machine)"),
        );
    }

    #[test]
    fn application_equals_pipe_for_bind() {
        let s = sample_sources();
        // The pair is the input in both spellings (a tuple is one argument).
        assert_eq!(
            pipe_ty(&s, "union (readings, readings)"),
            pipe_ty(&s, "(readings, readings) |> union"),
        );
    }

    #[test]
    fn application_and_pipe_share_the_unknown_op_diagnostic() {
        let s = sample_sources();
        let bare = pipe_ty(&s, "nope readings").expect_err("unknown op");
        let piped = pipe_ty(&s, "readings |> nope").expect_err("unknown op");
        // The span differs (the op sits at a different offset in each spelling),
        // but the resolution and message are identical.
        assert_eq!(bare[0].message, piped[0].message);
        assert!(bare[0].message.contains("unsupported operation `nope`"));
    }

    #[test]
    fn application_and_pipe_share_the_arity_diagnostic() {
        let s = sample_sources();
        let bare = pipe_ty(&s, "flat_map readings").expect_err("missing lambda");
        let piped = pipe_ty(&s, "readings |> flat_map").expect_err("missing lambda");
        assert_eq!(bare[0].message, piped[0].message);
        assert!(bare[0].message.contains("lambda"));
    }

    #[test]
    fn bare_partial_application_is_not_supported() {
        let s = sample_sources();
        // There is no partial application (ADR 0018 open question 2): the trailing
        // argument is always the input, so `promote machine` reads `machine` as
        // the input table (an unknown source) rather than a partially applied
        // stage. It is rejected, not invented.
        let errs = pipe_ty(&s, "promote machine").expect_err("no input");
        assert!(errs[0].message.contains("unknown source `machine`"));
    }
}
