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

use crate::expr_check::{
    Context, Optionality, Ty, TypeError, combiner_has_identity, field_reduction, type_expr,
};
use crate::model::ColumnType;
use crate::suggest::suffix;
use crate::table::{
    Arranged, Cardinality, Column, Completeness, Content, Exhaustive, Functional, Lineage,
    Qualifiers, Rectangles, Reductions, SplitId, TableType, Totality, WindowFact, Windows,
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
    /// The evaluated top-level consts and the imported module
    /// environments, for the one kind of operation argument that is a
    /// compile-time *value* rather than a selector: `window`'s extents
    /// (ADR 0037 decision 3).
    ///
    /// The ambient above carries only *types*, so `si.minute` reaches an
    /// expression site as `time[real]` with its magnitude erased.  An
    /// extent has to be the magnitude, so the values travel separately.
    consts: BTreeMap<String, crate::consts::ConstValue>,
    modules: BTreeMap<String, &'static crate::modules::ModuleEnv>,
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

    /// Set the compile-time values an extent argument evaluates against
    /// (ADR 0037 decision 3, ADR 0030's const machinery).
    pub fn with_consts(
        mut self,
        consts: BTreeMap<String, crate::consts::ConstValue>,
        modules: BTreeMap<String, &'static crate::modules::ModuleEnv>,
    ) -> Self {
        self.consts = consts;
        self.modules = modules;
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
        "window" => op_window(sources, input, args, span),
        "closed" => op_closed(input, args, span),
        "latest" => op_latest(input, args, span),
        "dense" => op_dense(sources, input, args, span),
        other => {
            // TODO(ADR-0025): `map` is a deliberately vacant name. Give it a
            // pointed diagnostic ("no `map` in Mensura: `flat_map` receives a
            // row, `map_bags` receives the bag") instead of the generic
            // edit-distance suggestion below.
            const OPS: [&str; 16] = [
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
                "window",
                "closed",
                "latest",
                "dense",
            ];
            let hint = suffix(other, OPS.iter().map(|s| s.to_string()));
            Err(error(
                format!("unsupported operation `{other}`{hint}"),
                span,
            ))
        }
    }?;
    // Gradings are transformed only where a witness backs the transform
    // (ADR 0024): the key moves derive cardinality from them, the
    // content-identity stages carry them, and `window` extends each of
    // them by its fresh key column, witnessed by `Mensura.window_functional`
    // (ADR 0037 decision 2).  Every other operation resets them to match
    // its own output cardinality, the conservative rule until the per-op
    // transport table is mechanized.
    let result = match op {
        "promote" | "demote" | "assume" | "completeness_check" | "window" | "closed" => result,
        // `dense` adds rows at fresh keys of the key it already has, so
        // every grading it was handed still holds (ADR 0038 decision 4's
        // fact is about the window column, and the gradings are what let
        // the subsequent `demote` re-derive its cardinality).
        "dense" => result,
        _ => sync_functional(result),
    };
    // The two grid facts of ADR 0038 each have one producer and one
    // consumer, one stage apart, so they are cleared centrally rather than
    // at each site: `map_bags` records which columns a single combiner
    // produced, `dense` reads them and records the completed grid, and
    // `demote` consumes that grid and clears it itself.  The
    // content-identity stages carry both, having changed nothing either
    // fact speaks about.
    Ok(match op {
        "map_bags" | "assume" | "completeness_check" | "dense" | "demote" => result,
        _ => clear_grid_facts(result),
    })
}

/// Drop the completed-grid and single-fold facts (ADR 0038), the
/// conservative rule for every operation between their producer and their
/// consumer; see [`dispatch_op`].
fn clear_grid_facts(output: PipeTy) -> PipeTy {
    fn clear(table: &mut TableType) {
        table.qualifiers.rectangles.clear();
        table.qualifiers.reductions.clear();
    }
    match output {
        PipeTy::Table(mut t) => {
            clear(&mut t);
            PipeTy::Table(t)
        }
        PipeTy::Pair(mut a, mut b) => {
            clear(&mut a);
            clear(&mut b);
            PipeTy::Pair(a, b)
        }
    }
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
            arranged: Arranged::Unclaimed,
            // A join is not content-identity in ADR 0024's sense (it
            // widens the row and, for the inner form, can drop one), so
            // the window facts and the source's intake contracts are reset
            // conservatively (ADR 0037 decision 2).
            windows: Windows::none(),
            rectangles: Rectangles::new(),
            reductions: Reductions::new(),
            contracts: Vec::new(),
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
            arranged: Arranged::Unclaimed,
            // The body computes a fresh row, so neither a window grid nor
            // an intake contract survives (ADR 0037 decision 2).
            windows: Windows::none(),
            rectangles: Rectangles::new(),
            reductions: Reductions::new(),
            contracts: Vec::new(),
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
            Ty::Record(fields) => {
                let mut schema = Vec::new();
                flatten_row_fields(String::new(), fields, &mut schema);
                Ok(schema)
            }
            _ => Err(error(
                "a `flat_map` body must yield rows: a record `(.a = ...)`, the value \
                 row `r`, `( )` to drop, or `(a, b)` to expand",
                expr.span,
            )),
        },
    }
}

/// Flatten a (possibly nested) row record back into dotted row columns
/// (ADR 0032): a unit-reference group forwarded whole becomes its flattened
/// columns again.  Recursing in map order yields the flat names already
/// sorted, because `.` orders below every identifier character; this is the
/// order the evaluator's whole-row rule uses.  Fields that are not single
/// values (and not groups) are dropped, as the flat arm always did.
fn flatten_row_fields(prefix: String, fields: BTreeMap<String, Ty>, out: &mut Vec<RowColumn>) {
    for (name, ty) in fields {
        let full = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}.{name}")
        };
        match ty {
            Ty::Record(sub) => flatten_row_fields(full, sub, out),
            other => {
                if let Some((domain, opt)) = column_of(&other) {
                    out.push(RowColumn {
                        name: full,
                        domain,
                        opt,
                    });
                }
            }
        }
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

/// Require an expression to be a *total* boolean (an `if` condition): the
/// branching boundary of ADR 0039 decision 3.
fn require_known_bool(ctx: &Context, cond: &Expr) -> Vec<TypeError> {
    match type_expr(ctx, cond) {
        Err(e) => e,
        Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt: Optionality::Total,
        }) => Vec::new(),
        Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt: Optionality::Optional,
        }) => error(
            "an `if` condition may be missing; state the absent-row policy \
             with `?? true` or `?? false` (ADR 0039)",
            cond.span,
        ),
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
/// (`fiberCompleteWrt_of_functional`). The key and lineage are preserved.
/// Completeness is carried through: sound for today's bodies because both
/// shapes emit at least one output row per present fiber, but a property of
/// the body language, not of `map_bags` (no named preservation lemma yet;
/// open question in `docs/language/07-pipelines.md`). `exhaustive` is
/// carried on the same non-emptying grounds (one output row per present key
/// in the aggregate shape, one per input row in the window shape, so no
/// fiber loses a row; `fiberMap_exhaustive`, with `aggregate_exhaustive`
/// the aggregate-shape special case).
fn op_map_bags(
    sources: &Sources,
    input: PipeTy,
    args: &[&Expr],
    span: Span,
) -> Result<PipeTy, Vec<TypeError>> {
    let table = expect_table(input, span)?;
    let (params, body) = lambda_params(args, "map_bags", 2, span)?;
    let ctx = Context::bag(&sources.ambient, params[0], params[1], &table);
    let (columns, totality, cardinality, reductions) = bag_record_content(&ctx, body)?;
    if cardinality == Cardinality::Singletons
        && table.qualifiers.cardinality == Cardinality::Bag
        && table.qualifiers.completeness != Completeness::Complete
    {
        return Err(error(
            "a reducing `map_bags` needs completeness over the current key (a \
             fold over a partial bag is silently wrong); establish it with \
             `completeness_check { ... }` or `assume { complete }` first, \
             after any `demote` (the fact does not survive a key coarsening)",
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
            arranged: Arranged::Unclaimed,
            // The grid facts cross this stage, because `dense` runs after
            // the reduction by construction (ADR 0038 decision 1) and needs
            // the stride and the bound.  What does *not* survive is the
            // point column the fact names: the attributes are computed from
            // the fiber, so `closed`, whose whole test is over the point,
            // demands it downstream and finds it gone.
            windows: table.qualifiers.windows.clone(),
            rectangles: Rectangles::new(),
            // Which columns a single combiner produced, for `dense` to
            // read an identity from (ADR 0038 decision 2).
            reductions,
            contracts: Vec::new(),
            lineage: table.qualifiers.lineage,
        },
    }))
}

/// Type a `map_bags` record body into columns, totality, the result
/// cardinality, and the single-fold columns (section 6.2). Single-valued
/// fields are aggregates (`Singletons`); bag-valued fields are window values
/// (`Bag`); a mix of the two is rejected.
///
/// The fourth output is ADR 0038 decision 2's recognition, done here because
/// here is where the field's defining expression is in hand: a stage
/// downstream sees columns and cannot ask what produced one.
fn bag_record_content(
    ctx: &Context,
    body: &Expr,
) -> Result<(Vec<Column>, Totality, Cardinality, Reductions), Vec<TypeError>> {
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
    let mut reductions = Reductions::new();
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
                    if let Some(op) = field_reduction(ctx, &field.value) {
                        reductions.insert(field.name.name.clone(), op);
                    }
                }
                // The fiber gets its own wording: a bare `b` is the single
                // most likely way to land here, and the fix (project or
                // count) is worth naming (ADR 0031, Decision 1).
                None if matches!(ty, Ty::Rows(_)) => errs.push(te(
                    format!(
                        "field `{}` is a bag of rows; project a column \
                         (`b.name`) or count the group (`#b`)",
                        field.name.name
                    ),
                    field.value.span,
                )),
                // A descending marker is an order annotation, not a value, so
                // it names its own home rather than reporting a bare "not a
                // value" (ADR 0031, Decision 7).
                None if matches!(ty, Ty::Desc(_)) => errs.push(te(
                    format!(
                        "field `{}` is a descending marker, which orders a \
                         scan's key and is never stored; drop the `desc`, or \
                         move it into the order key",
                        field.name.name
                    ),
                    field.value.span,
                )),
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
    Ok((columns, totality, cardinality, reductions))
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
    if !matches!(
        type_expr(&ctx, body)?,
        Ty::Value {
            domain: ColumnType::Bool,
            opt: Optionality::Total,
        }
    ) {
        return Err(error(
            "`split`'s predicate must be a known boolean",
            body.span,
        ));
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
            arranged: Arranged::Unclaimed,
            // Only the facts both branches carry: a merge must not invent a
            // grid one side does not have (the rule `exhaustive` follows
            // just above).  The intake contract does not survive at all,
            // since a union of two tables is not one registry's intake.
            windows: a.qualifiers.windows.intersect(&b.qualifiers.windows),
            rectangles: Rectangles::new(),
            reductions: Reductions::new(),
            contracts: Vec::new(),
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
            arranged: Arranged::Unclaimed,
            // A split cuts rows out of the table, so a grid that was
            // complete over it no longer is, and the surviving half is not
            // the registry's intake either.
            windows: Windows::none(),
            rectangles: Rectangles::new(),
            reductions: Reductions::new(),
            contracts: Vec::new(),
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
/// bag, a bag of rows (the fiber is a type-level notion, ADR 0031 Decision
/// 10), a nested record (window/nested returns are deferred), a descending
/// marker (an order annotation, not a value, ADR 0031 Decision 7), or a
/// function (which never enters a column, ADR 0030).
///
/// This is the storage boundary: returning `None` for the fiber is what keeps
/// nested collections out of a column, so the rows type cannot smuggle one in,
/// and returning `None` for `Ty::Desc` is what makes "never storable" hold by
/// construction rather than by a rule someone must remember to write.
fn column_of(ty: &Ty) -> Option<(ColumnType, Optionality)> {
    match ty {
        Ty::Value { domain, opt } => Some((domain.clone(), *opt)),
        Ty::Bag { .. } | Ty::Rows(_) | Ty::Record(_) | Ty::Desc(_) | Ty::Fn(_) | Ty::Builtin(_) => {
            None
        }
    }
}

/// `promote cols` (section 6.3, Tier A): promote each named column into the
/// key. A column must be total to enter the key (ADR 0013, and the
/// `demote_promote` inverse-domain side condition, ADR 0024); lineage is
/// preserved. Completeness is re-derived from the graded cardinality
/// (ADR 0035): a result graded `singletons` is `Complete` (a present
/// singleton fiber is its whole fiber, `fiberCompleteWrt_of_functional`,
/// and this is what restores the fact on a `demote c |> promote c` round
/// trip, keeping the pair truly inverse), and a `bag` result preserves the
/// incoming fact, since refining the key partitions each fiber by row
/// content and a whole fiber partitions into whole sub-fibers.
/// `exhaustive` is **forfeited**,
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
    // A graded `singletons` result re-derives `Complete` (ADR 0035): a
    // present singleton fiber is whole (`fiberCompleteWrt_of_functional`).
    // A `bag` result keeps the incoming fact (fiber splitting preserves
    // whole fibers); no clearing arm, unlike `demote`, because refining a
    // key never merges fibers.
    if table.qualifiers.cardinality == Cardinality::Singletons {
        table.qualifiers.completeness = Completeness::Complete;
    }
    Ok(PipeTy::Table(table))
}

/// `window w p size stride` (section 6.7, Tier A, ADR 0037 decisions 1, 2,
/// and 3): replicate each row into every window that contains its point `p`,
/// adding the window's start as a fresh key column `w` with `p`'s domain.
///
/// Specified as a derived form (a replicating `flat_map` then `promote w`),
/// which is what makes it Tier A by construction: split-safety and
/// disjointness come from the composition, mechanized as
/// `Mensura.window_splitSafe`.  It is a builtin rather than a library
/// binding twice over: the replication arity is data-dependent, and `w` and
/// `p` are column names, which are not values.
///
/// Content: `w` joins the key with `p`'s domain; `p` itself is untouched and
/// stays wherever it was.  Cardinality and gradings: **extended, not reset**.
/// The replication is injective on (input identity, `w`), so each grading `G`
/// becomes `G + {w}` (`Mensura.window_functional`).  That is the whole reason
/// the fact is tracked: it keeps a downstream scan's tie-freedom derivable
/// inside a window fiber, so `window` then `demote p` needs no ceremony.
/// Rewriting rather than adding is the point, since after replication the
/// table is no longer functional over `G` alone.
fn op_window(
    sources: &Sources,
    input: PipeTy,
    args: &[&Expr],
    span: Span,
) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    let [w_arg, p_arg, size_arg, stride_arg] = args else {
        return Err(error(
            "`window` takes a window column, a point column, a size, and a \
             stride, as in `window w taken_at (15.0 * si.minute) \
             (5.0 * si.minute)`",
            span,
        ));
    };
    let ExprKind::Name(w) = &w_arg.kind else {
        return Err(error(
            "`window`'s window column must be an identifier",
            w_arg.span,
        ));
    };
    let ExprKind::Name(p) = &p_arg.kind else {
        return Err(error(
            "`window`'s point column must be an identifier",
            p_arg.span,
        ));
    };

    // `w` is fresh, unlike every other column argument in the algebra: the
    // operation creates it.  The collision check is `unpivot`'s.
    if table
        .content
        .key
        .iter()
        .chain(&table.content.columns)
        .any(|c| &c.name == w)
    {
        return Err(error(
            format!("`window` would duplicate column `{w}`; name the window column something new"),
            w_arg.span,
        ));
    }

    // `p` must exist, in the key or among the attributes: a window is over a
    // point, and ADR 0037 decision 2 leaves that point where it is.
    let Some(point) = table
        .content
        .key
        .iter()
        .chain(&table.content.columns)
        .find(|c| &c.name == p)
        .cloned()
    else {
        if is_group_prefix(&table.content.key, p) || is_group_prefix(&table.content.columns, p) {
            return Err(error(
                format!(
                    "`{p}` is a unit-reference group; windows over its \
                     components are not yet supported (ADR 0032)"
                ),
                p_arg.span,
            ));
        }
        return Err(error(format!("unknown column `{p}`"), p_arg.span));
    };
    if table.qualifiers.totality.is_optional(p) {
        return Err(error(
            format!(
                "`window` needs `{p}` total: a missing point lands in no \
                 window, so there is nothing to replicate it into"
            ),
            p_arg.span,
        ));
    }

    // The extents are const expressions of type `diff(domain(p))` (ADR 0037
    // decision 3, ADR 0036 decision 4), held here in the point's storage
    // grain so `closed` can later compare `w + size + lateness` in one unit.
    let size = const_extent(sources, size_arg, &point, "size")?;
    let stride = const_extent(sources, stride_arg, &point, "stride")?;

    table.content.key.push(Column {
        name: w.clone(),
        domain: point.domain.clone(),
    });
    table.qualifiers.functional = table
        .qualifiers
        .functional
        .iter()
        .map(|grading| {
            let mut extended = grading.clone();
            extended.insert(w.clone());
            extended
        })
        .collect();
    table.derive_cardinality();
    // The windowing fact, the sibling of `unpivot`'s `exhaustive(axis)`:
    // established by construction here and consumed by `closed`.  It
    // inherits the source's intake contract on `p` when there is one; that
    // is what makes `closed`'s establishment mechanism-grade rather than a
    // claim (ADR 0037 decision 4).
    let contract = table
        .qualifiers
        .contracts
        .iter()
        .find(|c| &c.column == p)
        .cloned();
    table.qualifiers.windows.record(
        w.clone(),
        WindowFact {
            point: p.clone(),
            size,
            stride,
            contract,
            closed: false,
        },
    );
    // The key changed, so the rectangle fact goes the way it does under the
    // other key moves.
    table.qualifiers.exhaustive.clear();
    Ok(PipeTy::Table(table))
}

/// `latest p` (section 6.9, ADR 0037 decision 7): keep, per fiber, the row
/// with the maximal point `p`.
///
/// A **reduction**, not a window: fiber-to-row, so the result is
/// `singletons` at the current key with `p` an ordinary total attribute.
/// Formally the kept row is `getLast (arrange p fiber)`, deterministic by
/// `Mensura.IsArrangement.unique` given tie-freedom.
///
/// It demands both facts the ordered reductions demand, with no special
/// case: **tie-freedom** of `p` (a grading, or `assume { arranged }`,
/// exactly as a scan), because the argmax of a tied key is not determined;
/// and **completeness** at the current key (ADR 0023), because a partial
/// bag's "latest" is silently wrong.
///
/// **`p` must already be an attribute.**  ADR 0037 decision 7 also
/// specifies a fused form over a key column (`demote p`, then the argmax),
/// and that form is not shipped: its completeness demand would be
/// undischargeable, since the coarsening happens inside the operation, so a
/// claim before it sits at the fine key (which ADR 0035 says nothing
/// survives) and a claim after it comes too late.  The ADR's own example
/// puts the claim in that useless position.  Rejecting the form and naming
/// the explicit spelling keeps every accepted use dischargeable.
fn op_latest(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    let [p_arg] = args else {
        return Err(error(
            "`latest` takes one point column, as in `latest taken_at`",
            span,
        ));
    };
    let ExprKind::Name(p) = &p_arg.kind else {
        return Err(error(
            "`latest`'s point column must be an identifier",
            p_arg.span,
        ));
    };

    if table.content.key.iter().any(|c| &c.name == p) {
        return Err(error(
            format!(
                "`latest` needs `{p}` in the fiber, not in the key: coarsening \
                 inside the operation would leave its completeness demand with \
                 nowhere to stand, so write the coarsening out (`demote {p}`, \
                 then the claim the fold rests on, then `latest {p}`)"
            ),
            p_arg.span,
        ));
    }
    let Some(point) = table.content.columns.iter().find(|c| &c.name == p).cloned() else {
        return Err(error(format!("unknown column `{p}`"), p_arg.span));
    };
    if !point.domain.is_orderable() {
        return Err(error(
            format!(
                "`latest` orders by `{p}`, which must land in an orderable \
                 domain (`int`, `real`, a dimensioned real, `date`, or \
                 `instant`), found `{}`",
                crate::resolve::type_name(&point.domain)
            ),
            p_arg.span,
        ));
    }
    if table.qualifiers.totality.is_optional(p) {
        return Err(error(
            format!(
                "`latest` needs `{p}` total: a missing point has no position in \
                 the order, so there is no latest row to keep"
            ),
            p_arg.span,
        ));
    }

    // Tie-freedom, the same tier 1 discharge a scan gets: a projection key
    // is injective on the fiber when `key + {p}` contains a grading
    // (`Mensura.keyInjOn_demote_tag`), and `assume { arranged }` is the
    // tier 3 hatch for everything else.
    if table.qualifiers.arranged != Arranged::Assumed {
        let mut with = table.key_names();
        with.insert(p.clone());
        if !table
            .qualifiers
            .functional
            .iter()
            .any(|grading| grading.is_subset(&with))
        {
            return Err(error(
                format!(
                    "`latest`'s point `{p}` may have ties, so the latest row is \
                     not determined: nothing says at most one row per key \
                     shares it.  Order by a column projected out of the key \
                     (`demote` carries that fact), or claim it with \
                     `assume {{ arranged }}`"
                ),
                p_arg.span,
            ));
        }
    }

    // Completeness, the demand every reducer makes (ADR 0023): a partial
    // bag's latest row is silently wrong.  A `singletons` input discharges
    // it trivially, a present key's single row being its whole fiber.
    if table.qualifiers.cardinality == Cardinality::Bag
        && table.qualifiers.completeness != Completeness::Complete
    {
        return Err(error(
            "`latest` reduces each fiber to one row, which is silently wrong \
             on a partial bag, so it needs completeness over the current key; \
             establish it with `completeness_check { ... }` or \
             `assume { complete }` first, after any `demote` (the fact does \
             not survive a key coarsening)",
            span,
        ));
    }

    // Fiber-to-row: one row per present key, so the result is `singletons`
    // and the gradings follow from the key (the conservative `sync_functional`
    // in `dispatch_op` does that, since `latest` is not in its carve-out).
    table.qualifiers.cardinality = Cardinality::Singletons;
    table.qualifiers.completeness = Completeness::Complete;
    table.qualifiers.windows.clear();
    table.qualifiers.contracts.clear();
    Ok(PipeTy::Table(table))
}

/// `closed` (section 6.7, ADR 0037 decision 4): drop every window that is
/// still open and establish `Complete` at the current key on the survivors.
///
/// Not a new qualifier and not a new algebra primitive: a **new
/// establishment mechanism** for the existing completeness fact, joining
/// `completeness_check`/`assume` (ADR 0017), the registry rule (ADR 0033),
/// and the exhaustive-axis rule (ADR 0035).  Like `completeness_check` it is
/// a checked stage; unlike it, it *drops* rows rather than asserting over
/// them, because an open window is not an error, it is a window whose answer
/// does not exist yet, and absence is the honest representation.
///
/// The establishment is mechanism-grade: the windowing fact supplies `size`
/// and the source contract supplies `lateness`, both enforced, so "no row of
/// this window can still arrive" is a theorem about the intake
/// (`Mensura.closedWindow_stable`) rather than a claim.  Without a
/// `lateness` declaration there is no mechanism and the stage is rejected,
/// leaving `assume { complete }` as the visible fallback.
fn op_closed(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    if !args.is_empty() {
        return Err(error(
            "`closed` takes no arguments: the extent comes from the `window` \
             stage and the lateness bound from the source's declaration",
            args[0].span,
        ));
    }

    // A live window column in the current key, whose point has been demoted
    // into the fiber.  That is the shape the stage is specified over, and it
    // is what makes the window a reduction target.
    let key = table.key_names();
    let live: Vec<&String> = table
        .qualifiers
        .windows
        .columns()
        .filter(|w| key.contains(*w))
        .collect();
    let [window] = live.as_slice() else {
        if live.len() > 1 {
            return Err(error(
                "`closed` needs exactly one window column in the key, and this \
                 table has several; close one grid at a time",
                span,
            ));
        }
        return Err(error(
            "`closed` needs a window column in the key, established by a \
             `window` stage upstream (ADR 0037): there is none here, so there \
             is no grid whose windows could be open or closed",
            span,
        ));
    };
    let window = (*window).clone();
    let fact = table
        .qualifiers
        .windows
        .get(&window)
        .expect("the column came from the fact map")
        .clone();

    if fact.closed {
        return Err(error(
            format!(
                "`{window}`'s grid is already closed, and closing it twice says \
                 nothing new: drop this stage"
            ),
            span,
        ));
    }

    if table.content.key.iter().any(|c| c.name == fact.point) {
        return Err(error(
            format!(
                "`closed` needs `{}` in the fiber, not in the key: the window \
                 is only whole once its points are grouped into it, so \
                 `demote {}` first",
                fact.point, fact.point
            ),
            span,
        ));
    }
    if !table.content.columns.iter().any(|c| c.name == fact.point) {
        return Err(error(
            format!(
                "`closed` tests each window against its own points, and `{}` is \
                 no longer a column here: a reduction consumed it, so close the \
                 grid before reducing it",
                fact.point
            ),
            span,
        ));
    }

    let Some(contract) = &fact.contract else {
        return Err(error(
            format!(
                "`closed` needs a `lateness` contract on `{}` to know when a \
                 window can no longer receive a row; the source declares \
                 none, so there is no mechanism here and the claim must be \
                 made visibly with `assume {{ complete }}` instead (ADR 0037)",
                fact.point
            ),
            span,
        ));
    };

    // A watermark serves one grain (ADR 0041 decision 2), so a row can only
    // be tested against its own grain's.  The grain columns therefore have
    // to be identifiable per row, which at this point means in the key.
    let missing: Vec<&String> = contract
        .grain
        .iter()
        .filter(|g| !key.contains(*g))
        .collect();
    if !missing.is_empty() {
        let names: Vec<String> = missing.iter().map(|g| format!("`{g}`")).collect();
        return Err(error(
            format!(
                "`closed` needs the contract's watermark grain in the key, and \
                 {} {} no longer there: a watermark is per grain (ADR 0041), so \
                 without it a row cannot be measured against the producer it \
                 came from",
                names.join(", "),
                if missing.len() == 1 { "is" } else { "are" }
            ),
            span,
        ));
    }

    // The establishment.  It is the *arrival*-completeness the registry
    // mechanism gives at its own key, transported to the window key: every
    // row the intake will ever accept for this window is present.  Whether a
    // device's silence was a genuinely absent reading or one lost before the
    // intake is outside the type system, the same boundary ADR 0033 draws.
    table.qualifiers.completeness = Completeness::Complete;
    // Record that this grid now has an upper bound, which is what `dense`
    // reads when it completes the grid (ADR 0038 decision 3).
    table.qualifiers.windows.mark_closed(&window);
    Ok(PipeTy::Table(table))
}

/// `dense w population bound` (section 6.10, ADR 0038): complete the window
/// grid after the reduction, one row per entity per closed slot.
///
/// A window holding no row is absence (ADR 0037 decision 1), which is right
/// for the operation and leaves "how many intervals did this machine report
/// nothing" unanswerable, since the intervals in question are not rows.  This
/// stage makes them rows.
///
/// **It runs after the reduction, never before it** (decision 1).  The
/// tempting design materializes the empty fibers and lets the reduction
/// handle them, which would break ADR 0029 decision 4's guarantee that a
/// reducing lambda never sees an empty bag.  Filling reduced rows keeps that
/// guarantee and computes the same table
/// (`Mensura.dense_fiberMap_foldFiber`), and it keeps the conclusion that
/// matters: a filled row is a *reduced* row, so a count of zero says zero
/// rows were reduced rather than that a placeholder was invented and counted.
///
/// **Two of the four grid inputs are given, and must be** (decision 3).
/// Stride and origin come from the `window` declaration and the upper bound
/// from `closed`; the population (which entities should have windows) and
/// each entity's lower bound are policy that no data determines.  Inferring
/// them is the ADR 0034 repair pattern: taking the earliest observed window
/// as the bound would make a sensor offline on day one look like a sensor
/// not yet installed, and inferring the population from the windowed bag
/// would silently omit the entity that never reported, which is the case the
/// stage exists to expose.
///
/// **What it establishes.**  `Complete` at its own key, mechanism-grade
/// (the mechanism is the grid enumeration), plus the rectangularity fact a
/// subsequent `demote w` consumes to re-derive completeness at the coarsened
/// key rather than clear it (decision 4,
/// `Mensura.demote_fiberCompleteWrt_dense`).  A column produced by a single
/// fold at an identity-carrying combiner stays total and fills with that
/// identity; every other column, compound expressions included, goes
/// optional, because there is no maximum of nothing and a sentinel would lie
/// (decision 2).
fn op_dense(
    sources: &Sources,
    input: PipeTy,
    args: &[&Expr],
    span: Span,
) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    let [w_arg, pop_arg, bound_arg] = args else {
        return Err(error(
            "`dense` takes a window column, the store whose rows are the \
             population, and that store's lower-bound column, as in `dense w \
             machines activated`; the last two are policy and are never \
             inferred (ADR 0038 decision 3)",
            span,
        ));
    };

    let ExprKind::Name(window) = &w_arg.kind else {
        return Err(error(
            "`dense`'s first argument names the window column to complete",
            w_arg.span,
        ));
    };
    let key = table.key_names();
    if !key.contains(window) {
        return Err(error(
            format!(
                "`dense` needs `{window}` in the key: the grid it completes is \
                 the window column's, and a column outside the key indexes \
                 nothing"
            ),
            w_arg.span,
        ));
    }
    let Some(fact) = table.qualifiers.windows.get(window).cloned() else {
        return Err(error(
            format!(
                "`{window}` carries no window fact, so its values are not known \
                 to lie on a grid: `dense` completes what a `window` stage \
                 built (ADR 0038 decision 5), and over a raw order key there \
                 is no step to complete against"
            ),
            w_arg.span,
        ));
    };
    if !fact.closed {
        return Err(error(
            "`dense` bounds the grid above by closedness, so `closed` must run \
             upstream: without it the stage would fill windows past the \
             watermark, declaring a future that has not happened confirmed \
             empty (ADR 0038 decision 3)",
            span,
        ));
    }
    if table.qualifiers.cardinality != Cardinality::Singletons {
        return Err(error(
            "`dense` completes reduced rows, one per (entity, window), and this \
             table is still a bag: reduce it with `map_bags` first (ADR 0038 \
             decision 1 fills after the reduction, so no lambda ever faces an \
             empty bag)",
            span,
        ));
    }

    // The population.  Resolved by name like a join's right side, and
    // `singletons`, since one row per entity is what makes the grid a
    // rectangle rather than a product.
    let ExprKind::Name(pop_name) = &pop_arg.kind else {
        return Err(error(
            "`dense`'s population must be a source name (the store whose rows \
             say which entities should have windows)",
            pop_arg.span,
        ));
    };
    let population = match sources.get(pop_name) {
        Some(PipeTy::Table(t)) => t,
        Some(PipeTy::Pair(..)) => {
            return Err(error(
                format!("`{pop_name}` is a pair of tables, not a population"),
                pop_arg.span,
            ));
        }
        None => {
            let hint = suffix(pop_name, sources.bound.keys().cloned().collect::<Vec<_>>());
            return Err(error(
                format!("unknown source `{pop_name}`{hint}"),
                pop_arg.span,
            ));
        }
    };
    if population.qualifiers.cardinality != Cardinality::Singletons {
        return Err(error(
            format!(
                "`dense` needs a `singletons` population, and `{pop_name}` is a \
                 bag: one row per entity is what makes the completed grid a \
                 rectangle"
            ),
            pop_arg.span,
        ));
    }

    // The population is keyed like the windowed rows minus the window
    // column, which is the only way a population row can say which grid it
    // bounds.  Compared by name and domain, as a join compares its key.
    let residual: Vec<&Column> = table
        .content
        .key
        .iter()
        .filter(|c| &c.name != window)
        .collect();
    let matches = population.content.key.len() == residual.len()
        && population
            .content
            .key
            .iter()
            .zip(&residual)
            .all(|(p, r)| p.name == r.name && p.domain == r.domain);
    if !matches {
        let residual_names: Vec<String> =
            residual.iter().map(|c| format!("`{}`", c.name)).collect();
        let pop_names: Vec<String> = population
            .content
            .key
            .iter()
            .map(|c| format!("`{}`", c.name))
            .collect();
        return Err(error(
            format!(
                "`dense` needs `{pop_name}` keyed like the windowed rows without \
                 the window column, and it is not: the rows are keyed by {} \
                 beside `{window}`, while `{pop_name}` is keyed by {}",
                residual_names.join(", "),
                pop_names.join(", ")
            ),
            pop_arg.span,
        ));
    }

    // The per-entity lower bound: a column on the population, total, and in
    // the window column's own domain, since the grid aligns the bound to a
    // slot.
    let ExprKind::Name(bound) = &bound_arg.kind else {
        return Err(error(
            "`dense`'s last argument names the population's lower-bound column \
             (where each entity's history starts)",
            bound_arg.span,
        ));
    };
    let Some(bound_col) = population
        .content
        .columns
        .iter()
        .chain(population.content.key.iter())
        .find(|c| &c.name == bound)
    else {
        let hint = suffix(
            bound,
            population
                .content
                .columns
                .iter()
                .chain(population.content.key.iter())
                .map(|c| c.name.clone()),
        );
        return Err(error(
            format!("`{pop_name}` has no column `{bound}`{hint}"),
            bound_arg.span,
        ));
    };
    if population.qualifiers.totality.is_optional(bound) {
        return Err(error(
            format!(
                "`dense` needs `{bound}` total: an entity whose lower bound is \
                 missing has no grid to complete, and skipping it silently is \
                 the absence this stage exists to remove"
            ),
            bound_arg.span,
        ));
    }
    let window_col = table
        .content
        .key
        .iter()
        .find(|c| &c.name == window)
        .expect("the window column is in the key");
    if bound_col.domain != window_col.domain {
        let extra = if bound_col.domain == ColumnType::Date {
            ": a calendar bound needs the zone-dependent `date` to `instant` \
             conversion ADR 0036 defers, so record the bound as the absolute \
             event it is"
        } else {
            ""
        };
        return Err(error(
            format!(
                "`dense` needs `{bound}` in `{window}`'s domain ({}), and it is \
                 {}{extra}",
                crate::resolve::type_name(&window_col.domain),
                crate::resolve::type_name(&bound_col.domain)
            ),
            bound_arg.span,
        ));
    }

    // Decision 2's typing consequence, and the point of the stage: a column
    // whose combiner has an identity fills with it, and every other column
    // is honestly absent on a filled row.
    let widen: Vec<String> = table
        .content
        .columns
        .iter()
        .filter(|c| {
            !table
                .qualifiers
                .reductions
                .get(&c.name)
                .copied()
                .is_some_and(combiner_has_identity)
        })
        .map(|c| c.name.clone())
        .collect();
    for name in widen {
        table.qualifiers.totality.mark_optional(name);
    }
    table.qualifiers.completeness = Completeness::Complete;
    table.qualifiers.rectangles.insert(window.clone());
    Ok(PipeTy::Table(table))
}

/// One `window` extent: a const expression of the point's difference type
/// (ADR 0036 decision 4), positive, and exact in the point's storage grain.
///
/// The same shape as a `lateness` bound (`resolve.rs::resolve_lateness`), and
/// deliberately the same diagnostics: both are compile-time durations against
/// a point column, and an author who has met one should recognize the other.
/// Unlike a bound, an extent must be strictly positive: a zero-width window
/// contains nothing and a zero stride is not a grid.
fn const_extent(
    sources: &Sources,
    arg: &Expr,
    point: &Column,
    what: &str,
) -> Result<i64, Vec<TypeError>> {
    use crate::consts::ConstValue;
    let value =
        crate::consts::eval_const_expr(arg, &sources.consts, &sources.modules).map_err(|errs| {
            errs.into_iter()
                .map(|e| te(e.message, e.span))
                .collect::<Vec<_>>()
        })?;
    match (&point.domain, &value) {
        (ColumnType::Instant, ConstValue::Real { magnitude, dim })
            if *dim == crate::units::Dimension::base("time").expect("time is a base axis") =>
        {
            match crate::temporal::whole_milliseconds(*magnitude) {
                Ok(ms) if ms > 0 => Ok(ms),
                Ok(_) => Err(error(
                    format!("`window`'s {what} must be positive"),
                    arg.span,
                )),
                Err(message) => Err(error(message, arg.span)),
            }
        }
        (ColumnType::Instant, other) => Err(error(
            format!(
                "`window`'s {what} must be a duration (`time[real]`, the \
                 difference type of `instant`), found `{}`",
                other.describe()
            ),
            arg.span,
        )),
        (ColumnType::Int, ConstValue::Int(n)) if *n > 0 => Ok(*n),
        (ColumnType::Int, ConstValue::Int(_)) => Err(error(
            format!("`window`'s {what} must be positive"),
            arg.span,
        )),
        (ColumnType::Int, other) => Err(error(
            format!(
                "`window`'s {what} must be an `int` (the difference type of \
                 `int`), found `{}`",
                other.describe()
            ),
            arg.span,
        )),
        (ColumnType::Date, _) => Err(error(
            format!(
                "a window over `{}` is not yet supported: `diff(date)` is \
                 deferred (ADR 0036 decision 4)",
                point.name
            ),
            arg.span,
        )),
        (other, _) => Err(error(
            format!(
                "`window` needs an orderable point column whose domain has a \
                 difference type (`instant` or `int`), and `{}` is `{}`",
                point.name,
                crate::resolve::type_name(other)
            ),
            arg.span,
        )),
    }
}

/// Whether `name` is a unit-reference group prefix among `cols` (ADR 0032):
/// some flattened column continues it past a `.`.
fn is_group_prefix(cols: &[Column], name: &str) -> bool {
    cols.iter().any(|c| {
        c.name.len() > name.len()
            && c.name.starts_with(name)
            && c.name.as_bytes()[name.len()] == b'.'
    })
}

fn promote_to_key(table: &mut TableType, col: &str, span: Span) -> Result<(), TypeError> {
    if table.content.key.iter().any(|c| c.name == col) {
        return Err(te(format!("`{col}` is already in the key"), span));
    }
    let Some(pos) = table.content.columns.iter().position(|c| c.name == col) else {
        if is_group_prefix(&table.content.columns, col) {
            return Err(te(
                format!(
                    "`{col}` is a unit-reference group; key moves on its \
                     components are not yet supported (ADR 0032)"
                ),
                span,
            ));
        }
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

/// `demote cols` (section 6.3, Tier B, ADR 0017 as amended by ADR 0023 and
/// ADR 0035): drop key components into the non-key part. Content: the named
/// key columns become ordinary columns. Cardinality: **derived from the
/// gradings** (ADR 0024): the move leaves the gradings untouched and re-runs
/// the subset check against the shrunken key, so a genuine coarsening
/// rises to `bag` (no grading fits the retained key) while the round trip
/// `promote c |> demote c` re-derives `singletons` from the source
/// grading (`demote_promote`). Completeness: **re-derived from the graded
/// cardinality** (ADR 0035): a `singletons` result is `Complete` (a present
/// singleton fiber is its whole fiber, `fiberCompleteWrt_of_functional`),
/// while a genuine coarsening **clears** the fact, since merging fibers
/// turns an absent fine key into a gap inside a coarse fiber
/// (ADR 0035's recorded fiber-gap counterexample); a reducer over the
/// coarsened bag
/// establishes the fact after the `demote`, never before. Lineage:
/// **dropped** (`demote_not_preservesDisjoint`), the lineage break that
/// keeps `demote` Tier B on its own.
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
            if is_group_prefix(&table.content.key, col) {
                errs.push(te(
                    format!(
                        "`{col}` is a unit-reference group; key moves on its \
                         components are not yet supported (ADR 0032)"
                    ),
                    arg.span,
                ));
            } else {
                errs.push(te(format!("not an key column `{col}`"), arg.span));
            }
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
    let was_complete = table.qualifiers.completeness == Completeness::Complete;
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
    // Was the coarsening along axes already known rectangular?  `exhaustive`
    // (ADR 0020) says every residual key present carries its row for *every*
    // variant of the axis, which is exactly the absence the clearing rule
    // below guards against, so the two facts compose (ADR 0035, open
    // questions): the coarse bag at a present key is the union of the fibers
    // over the axis, exhaustiveness makes every one of them present, and
    // fiber-completeness makes each whole, so the union is whole.  Multi-
    // column demotes chain, hence `all`.
    //
    // A completed window grid (ADR 0038 decision 4) is the second fact with
    // this shape, and the same argument covers it: `dense` materialized every
    // slot between the entity's declared lower bound and the closed upper
    // bound, so the coarse fiber is the whole rectangle rather than a sample
    // of it (`Mensura.demote_fiberCompleteWrt_dense`).  This is the one place
    // a genuinely coarsening `demote` re-establishes completeness from a
    // checked fact, and it is available only over a grid, where the step is a
    // compile-time constant and closedness bounds the run above.
    let rectangular = was_complete
        && !to_drop.is_empty()
        && to_drop.iter().all(|c| {
            table.qualifiers.exhaustive.contains(c.as_str())
                || table.qualifiers.rectangles.contains(c.as_str())
        });
    // Completeness is re-derived from the graded cardinality (ADR 0035): the
    // fact is about the current key against a fixed intended population, and
    // a genuine coarsening forfeits it, because a whole key absent at the
    // fine key becomes a gap inside a coarse fiber
    // (ADR 0035's recorded fiber-gap counterexample; the reference-relative
    // `demote_completeWrt` remains true but co-coarsens the reference, which
    // is not what the reducing `map_bags` consumes). At a graded
    // `singletons` result (an exact ADR 0024 round trip) the fact is
    // re-derived instead: a present singleton fiber is its whole fiber
    // (`fiberCompleteWrt_of_functional`), which keeps the key moves a true
    // inverse pair on the whole qualifier vector.
    table.qualifiers.completeness = match table.qualifiers.cardinality {
        Cardinality::Singletons => Completeness::Complete,
        // A coarsening along exhaustive axes only is not a genuine loss:
        // see `rectangular` above.
        Cardinality::Bag if rectangular => Completeness::Complete,
        Cardinality::Bag => Completeness::Incomplete,
    };
    table.qualifiers.lineage = Lineage::dropped();
    // `exhaustive` is forfeited: ADR 0020 section 2 sketches the retained-axis
    // carry (a union of full fibers is full), but the key-changing propagation
    // rows are the ADR's open formal work item, so the checker stays
    // conservative until they are mechanized.
    table.qualifiers.exhaustive.clear();
    // The grid fact is consumed here and spent: it says the window column's
    // values are the whole grid *per residual key*, which is a statement
    // about the key this move just changed.
    table.qualifiers.rectangles.clear();
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
                // The second claim ADR 0017 anticipated ("the block form
                // generalizes later without a surface change"): tie-freedom of a
                // scan's order key, ADR 0029 Decision 11's tier 3.  Needed
                // because the ordered primitives demand what no grading covers
                // for a computed or ungraded key.
                ExprKind::Name(claim) if claim == "arranged" => {
                    table.qualifiers.arranged = Arranged::Assumed;
                }
                _ => errs.push(te(
                    "`assume` accepts the claims `complete` and `arranged`",
                    e.span,
                )),
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
    use mensura_syntax::StoreKind;

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
            kind: StoreKind::Store,
            unit: unit.to_string(),
            columns,
            cardinality: Cardinality::Singletons,
            foreign_keys: Vec::new(),
            lateness: Vec::new(),
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
                scol("taken_at", ColumnType::Date, ColumnRole::Attr, false),
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
            .with_ambient(ambient_with_modules(&["bag", "series"]))
    }

    /// The ambient a program gets from `import bag` (ADR 0031, Decision 8).
    /// These tests type pipeline *expressions* rather than whole programs, so
    /// there is no `import` item to resolve; injecting the module's env here
    /// is the equivalent, and keeps the fixtures reading as a user would
    /// write them.
    fn ambient_with_bag() -> crate::expr_check::Ambient {
        ambient_with_modules(&["bag"])
    }

    /// The ambient a program gets from importing each named bundled module.
    /// These tests type bare pipelines, so there is no `import` item to
    /// resolve; injecting the modules' envs is the equivalent.
    fn ambient_with_modules(names: &[&str]) -> crate::expr_check::Ambient {
        let mut ambient = crate::expr_check::intrinsics();
        for name in names {
            let env = crate::modules::bundled(name)
                .unwrap_or_else(|| panic!("`{name}` is bundled"))
                .as_ref()
                .unwrap_or_else(|e| panic!("`{name}` resolves cleanly: {e:?}"));
            let members = env
                .values
                .iter()
                .filter_map(|(n, v)| Some((n.clone(), v.ty()?)))
                .collect();
            ambient.insert((*name).to_string(), crate::expr_check::Ty::Record(members));
        }
        ambient
    }

    /// The bundled module environments as `Sources::with_consts` wants
    /// them: `window`'s extents are compile-time *values*, so the ambient's
    /// types are not enough (ADR 0037 decision 3).
    fn modules_map(names: &[&str]) -> BTreeMap<String, &'static crate::modules::ModuleEnv> {
        names
            .iter()
            .map(|name| {
                let env = crate::modules::bundled(name)
                    .unwrap_or_else(|| panic!("`{name}` is bundled"))
                    .as_ref()
                    .unwrap_or_else(|e| panic!("`{name}` resolves cleanly: {e:?}"));
                ((*name).to_string(), env)
            })
            .collect()
    }

    /// The fleet's shape in miniature: a registry keyed by
    /// `(machine_id, taken_at)` with an `instant` point and a declared
    /// intake contract, which is what `window` needs and `sample_sources`
    /// (whose `taken_at` is a `date` attribute) cannot provide.
    fn windowed_sources(with_contract: bool) -> Sources {
        let schema = Schema {
            store: "readings".to_string(),
            kind: StoreKind::Registry,
            unit: "Reading".to_string(),
            columns: vec![
                scol("machine_id", ColumnType::String, ColumnRole::Key, false),
                scol("taken_at", ColumnType::Instant, ColumnRole::Key, false),
                scol("temperature", ColumnType::Real, ColumnRole::Attr, false),
                scol("seq", ColumnType::Int, ColumnRole::Attr, false),
                scol("day", ColumnType::Date, ColumnRole::Attr, false),
                scol("peak", ColumnType::Real, ColumnRole::Attr, true),
                scol("note", ColumnType::String, ColumnRole::Attr, true),
            ],
            cardinality: Cardinality::Singletons,
            foreign_keys: Vec::new(),
            lateness: if with_contract {
                vec![crate::model::Lateness {
                    column: "taken_at".to_string(),
                    bound: 600_000,
                    grain: vec!["machine_id".to_string()],
                    span: Span::new(0, 0),
                }]
            } else {
                Vec::new()
            },
            span: Span::new(0, 0),
        };
        // The population `dense` completes a grid over (ADR 0038 decision 3):
        // keyed like the windowed rows without the window column, with an
        // absolute lower bound and one calendar column to reject against.
        let machines = Schema {
            store: "machines".to_string(),
            kind: StoreKind::Store,
            unit: "Machine".to_string(),
            columns: vec![
                scol("machine_id", ColumnType::String, ColumnRole::Key, false),
                scol("activated", ColumnType::Instant, ColumnRole::Attr, false),
                scol("commissioned", ColumnType::Date, ColumnRole::Attr, false),
                scol("retired", ColumnType::Instant, ColumnRole::Attr, true),
            ],
            cardinality: Cardinality::Singletons,
            foreign_keys: Vec::new(),
            lateness: Vec::new(),
            span: Span::new(0, 0),
        };
        Sources::new()
            .with("readings", TableType::from_store(&schema))
            .with("machines", TableType::from_store(&machines))
            .with_ambient(ambient_with_modules(&["bag", "series", "si"]))
            .with_consts(BTreeMap::new(), modules_map(&["si"]))
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
        // `peak` is a `real?`, so `+ 1` is an int/real mismatch: optionality
        // lifts (ADR 0039), the domain rule still rejects.
        let errs =
            pipe_ty(&s, "readings |> flat_map |k, r| (.x = r.peak + 1)").expect_err("mismatch");
        assert!(errs[0].message.contains("same type"));
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
                 |> map_bags |k, b| (.temp_mean = bag.sum b.temperature / to_real (#b.temperature), .temp_max = bag.max b.temperature)",
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
        let errs = pipe_ty(&s, "readings |> map_bags |k, b| (.m = bag.sum b.machine)")
            .expect_err("non-numeric");
        // The demand now comes from the combiner table rather than from a
        // per-aggregate signature: `bag.sum` is `fold `+``, and the `+` row
        // folds a numeric domain (ADR 0031, Decision 6).
        assert!(
            errs[0].message.contains("folds a numeric domain"),
            "got: {}",
            errs[0].message
        );
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
            "readings |> map_bags |k, b| (.m = bag.sum b.temperature, .t = b.temperature)",
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
        assert!(errs[0].message.contains("must be a known boolean"));
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
        Sources::new()
            .with("wide", wide)
            .with_ambient(ambient_with_bag())
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
        let s = Sources::new()
            .with("bare", bare)
            .with_ambient(ambient_with_bag());
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
        Sources::new()
            .with("obs", obs)
            .with_ambient(ambient_with_bag())
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
        let s = Sources::new()
            .with("long", long)
            .with_ambient(ambient_with_bag());
        let errs = pipe_ty(&s, "long |> pivot metric reading").expect_err("empty key");
        assert!(errs[0].message.contains("at least one key"));
    }

    #[test]
    fn demote_clears_completeness_on_a_genuine_coarsening() {
        let s = sample_sources();
        // Promote `machine`, establish completeness, then genuinely coarsen
        // by dropping `ts`: result is a bag (no grading fits the retained
        // key) and the fact is **forfeited** (ADR 0035), establish step
        // notwithstanding: an absent fine key becomes a gap inside a coarse
        // fiber, so the claim made at the fine key says nothing about the
        // bags the coarse key folds.  Lineage dropped as before.
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
        assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
        assert_eq!(t.qualifiers.lineage, Lineage::root());
    }

    // --- demote along an exhaustive axis (ADR 0035, adopted) -------------

    /// A wide source whose folded columns are both total, so `unpivot`
    /// establishes `exhaustive`.  `optional` makes one column `?`, which
    /// defeats it (a dropped cell leaves the variant absent).
    fn paired_source(optional: bool) -> Sources {
        Sources::new().with(
            "paired",
            TableType::from_store(&Schema {
                store: "paired".to_string(),
                kind: StoreKind::Registry,
                unit: "Slot".to_string(),
                columns: vec![
                    scol("slot", ColumnType::String, ColumnRole::Key, false),
                    scol("internal", ColumnType::Real, ColumnRole::Attr, false),
                    scol("external", ColumnType::Real, ColumnRole::Attr, optional),
                ],
                cardinality: Cardinality::Singletons,
                foreign_keys: Vec::new(),
                lateness: Vec::new(),
                span: Span::new(0, 0),
            }),
        )
    }

    #[test]
    fn demote_along_an_exhaustive_axis_keeps_completeness() {
        // The composition ADR 0035 records: `exhaustive(sensor)` rules out
        // exactly the absences the clearing rule guards against, so the
        // coarse bag is the whole variant set and the fold is faithful.
        let s = paired_source(false);
        let t =
            table_of(pipe_ty(&s, "paired |> unpivot sensor reading |> demote sensor").expect("ok"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
        assert_eq!(t.qualifiers.completeness, Completeness::Complete);
    }

    #[test]
    fn a_reducer_over_an_exhaustive_demote_needs_no_assume() {
        let s = paired_source(false);
        pipe_ty(
            &s,
            "paired |> unpivot sensor reading |> demote sensor \
             |> map_bags |k, b| (.n = #b)",
        )
        .expect("the rectangle discharges the reducer");
    }

    #[test]
    fn demote_without_the_exhaustive_fact_still_clears() {
        // One folded column optional: `unpivot` establishes nothing, so the
        // coarsening is a genuine one and ADR 0035's rule applies.
        let s = paired_source(true);
        let t =
            table_of(pipe_ty(&s, "paired |> unpivot sensor reading |> demote sensor").expect("ok"));
        assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
    }

    #[test]
    fn demoting_a_non_exhaustive_axis_alongside_clears() {
        // `sensor` is exhaustive; `taken_at` is an ordinary key column and
        // is not.  The rule needs *every* demoted column to be rectangular,
        // so naming both clears the fact.
        let s = Sources::new().with(
            "paired",
            TableType::from_store(&Schema {
                store: "paired".to_string(),
                kind: StoreKind::Registry,
                unit: "Slot".to_string(),
                columns: vec![
                    scol("slot", ColumnType::String, ColumnRole::Key, false),
                    scol("taken_at", ColumnType::Date, ColumnRole::Key, false),
                    scol("internal", ColumnType::Real, ColumnRole::Attr, false),
                    scol("external", ColumnType::Real, ColumnRole::Attr, false),
                ],
                cardinality: Cardinality::Singletons,
                foreign_keys: Vec::new(),
                lateness: Vec::new(),
                span: Span::new(0, 0),
            }),
        );
        let t = table_of(
            pipe_ty(
                &s,
                "paired |> unpivot sensor reading |> demote sensor taken_at",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
    }

    #[test]
    fn a_filter_defeats_the_exhaustive_demote() {
        // `flat_map` can drop rows, so it forfeits `exhaustive`; the
        // coarsening is then genuine again.
        let s = paired_source(false);
        let t = table_of(
            pipe_ty(
                &s,
                "paired |> unpivot sensor reading \
                 |> flat_map |k, r| if k.sensor == \"internal\" then r else () \
                 |> demote sensor",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.completeness, Completeness::Incomplete);
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
             |> map_bags |k, b| (.n = #b.temperature)",
        )
        .expect_err("incomplete bag");
        assert!(errs[0].message.contains("reducing `map_bags`"), "{errs:?}");
    }

    fn registry_readings(cardinality: Cardinality) -> TableType {
        let (machine_role, ts) = match cardinality {
            // A bag registry is keyed by the entity; time is an observation.
            Cardinality::Bag => (ColumnRole::Key, ColumnRole::Attr),
            // A singletons registry keys the reading itself.
            Cardinality::Singletons => (ColumnRole::Attr, ColumnRole::Key),
        };
        TableType::from_store(&Schema {
            store: "readings".to_string(),
            kind: StoreKind::Registry,
            unit: "Reading".to_string(),
            columns: vec![
                scol("ts", ColumnType::Int, ts, false),
                scol("machine", ColumnType::String, machine_role, false),
                scol("temperature", ColumnType::Real, ColumnRole::Attr, false),
            ],
            cardinality,
            foreign_keys: Vec::new(),
            lateness: Vec::new(),
            span: Span::new(0, 0),
        })
    }

    #[test]
    fn a_bag_registry_reduces_at_its_own_key_with_no_establish_step() {
        // The surviving contentful case (ADR 0033 as amended by ADR 0035):
        // an `attr*` registry pins the reference population per entity, so a
        // reducer at the registry's own key needs no ceremony.  The same
        // pipeline over a bag *store* is rejected
        // (`reducing_map_bags_over_a_bag_demands_completeness` shows the
        // shape).
        let s = sample_sources().with("registry_readings", registry_readings(Cardinality::Bag));
        let t = table_of(
            pipe_ty(
                &s,
                "registry_readings |> map_bags |k, b| (.n = #b.temperature)",
            )
            .expect("a bag registry discharges the reducer at its own key"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn a_demoted_registry_no_longer_discharges_the_reducer() {
        // ADR 0035: the by-mechanism fact holds at the registry's own key
        // and does not survive the coarsening; recording every reading
        // received is not receiving every reading that happened.  The
        // reducer over the demoted bag demands its own establishment step,
        // placed after the `demote`.
        let s = sample_sources().with(
            "registry_readings",
            registry_readings(Cardinality::Singletons),
        );
        let errs = pipe_ty(
            &s,
            "registry_readings |> promote machine |> demote ts \
             |> map_bags |k, b| (.n = #b.temperature)",
        )
        .expect_err("the fact was forfeited at the coarsening");
        assert!(errs[0].message.contains("reducing `map_bags`"), "{errs:?}");
        pipe_ty(
            &s,
            "registry_readings |> promote machine |> demote ts \
             |> assume { complete } |> map_bags |k, b| (.n = #b.temperature)",
        )
        .expect("establishing after the demote discharges the reducer");
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
        let s = Sources::new()
            .with("events", events)
            .with_ambient(ambient_with_bag());
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
        // moves, and with it `Complete`: the exact round trip is graded
        // `singletons`, so the key move re-derives the fact rather than
        // clearing it (ADR 0035), keeping the pair a true inverse on the
        // whole qualifier vector.
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
    fn a_promote_that_rederives_singletons_rederives_completeness() {
        let s = sample_sources();
        // The demote-first order (ADR 0024's `promote_demote`) restores the
        // fact too (ADR 0035): the coarsening clears it with the grading
        // surviving, and promoting the column back re-derives `singletons`
        // and with it the trivial fiber fact
        // (`fiberCompleteWrt_of_functional`).  The re-derivation fires on
        // the graded cardinality, not only on exact round trips.
        let bag = table_of(pipe_ty(&s, "readings |> promote machine |> demote ts").expect("ok"));
        assert_eq!(bag.qualifiers.completeness, Completeness::Incomplete);
        let t = table_of(
            pipe_ty(&s, "readings |> promote machine |> demote ts |> promote ts").expect("ok"),
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
                 |> map_bags |k, b| (.n = #b.temperature)",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn the_establish_step_sits_after_the_demote() {
        let s = sample_sources();
        // ADR 0035: the fact is about the current key, so the discharge for
        // a reducer over a coarsened bag is placed after the `demote`.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> demote ts |> assume { complete } \
                 |> map_bags |k, b| (.n = #b.temperature)",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn an_establish_step_before_the_demote_is_forfeited() {
        let s = sample_sources();
        // The same pipeline with the claim made at the fine key: the
        // coarsening forfeits it (ADR 0035), so the reducer still rejects.
        let errs = pipe_ty(
            &s,
            "readings |> promote machine |> assume { complete } |> demote ts \
             |> map_bags |k, b| (.n = #b.temperature)",
        )
        .expect_err("the fine-key claim does not survive the coarsening");
        assert!(errs[0].message.contains("reducing `map_bags`"), "{errs:?}");
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

    /// **The completeness demand is per combiner, not per output shape**
    /// (ADR 0037 decision 5, settling ADR 0029's flag; this test's ancestor
    /// was named for the shape rule the decision retired).  A fold-admitting
    /// scan contains its reduction (`Mensura.scanl_getLast_eq_foldBag`), so
    /// over a genuinely coarsened bag it is rejected exactly as the reducing
    /// shape is, and `assume { complete }` is the visible discharge; the
    /// output is still the window shape (a bag) once the fact holds.
    #[test]
    fn a_fold_admitting_scan_demands_completeness_like_a_reducer() {
        let s = sample_sources();
        let errs = pipe_ty(
            &s,
            "readings |> promote machine |> demote ts \
             |> map_bags |k, b| (.run = series.running_max (|r| r.temperature) (|r| r.ts) b)",
        )
        .expect_err("running_max contains bag.max");
        assert!(
            errs[0].message.contains("contains its reduction"),
            "unexpected: {}",
            errs[0].message
        );
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> demote ts |> assume { complete } \
                 |> map_bags |k, b| (.run = series.running_max (|r| r.temperature) (|r| r.ts) b)",
            )
            .expect("discharged"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
    }

    /// The keep combiners stay ceremony-free at the very same fiber: their
    /// outputs are claims about adjacency among *present* rows, which a
    /// partial bag represents honestly (ADR 0037 decision 5).
    #[test]
    fn a_keep_combiner_scan_demands_no_completeness() {
        let s = sample_sources();
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> demote ts \
                 |> map_bags |k, b| (.prev = series.lag (|r| r.temperature) (|r| r.ts) b)",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
        assert!(t.qualifiers.totality.is_optional("prev"));
    }

    /// **`series.lag` yields an optional column**, with no pipe-layer rule to
    /// make it so: `lag` is a `prescan` at keep-right, keep-right has no
    /// identity, and an exclusive scan's first position folds the empty prefix.
    /// The optionality is decided in `type_scan` and flows through the record's
    /// existing `mark_optional` path, which is the evidence that the primitive
    /// decomposition carries its own consequences.
    #[test]
    fn lag_yields_an_optional_column() {
        let s = sample_sources();
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> demote machine \
                 |> map_bags |k, b| (.prev = series.lag (|r| r.temperature) (|r| r.taken_at) b)",
            )
            .expect("ok"),
        );
        assert!(
            t.qualifiers.totality.is_optional("prev"),
            "the earliest row in a group has no predecessor, so `lag` is optional"
        );
        // Its inclusive sibling has no such hole: every prefix `1..i` is
        // non-empty, so `running_max` stays total at the same combiner class.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> promote machine |> demote machine \
                 |> map_bags |k, b| (.run = series.running_max (|r| r.temperature) (|r| r.taken_at) b)",
            )
            .expect("ok"),
        );
        assert!(!t.qualifiers.totality.is_optional("run"));
    }

    /// A descending marker orders a key and is not a value, so it cannot be a
    /// column.  `column_of` returning `None` for it is what makes that hold by
    /// construction (ADR 0031, Decision 7).
    #[test]
    fn a_descending_marker_cannot_be_a_column() {
        let s = sample_sources();
        let errs =
            pipe_ty(&s, "readings |> map_bags |k, b| (.d = desc 1)").expect_err("not storable");
        assert!(
            errs.iter()
                .any(|e| e.message.contains("desc") || e.message.contains("descending")),
            "unexpected: {:?}",
            errs.iter().map(|e| &e.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn reducing_map_bags_over_singletons_discharges_trivially() {
        let s = sample_sources();
        // At `card <= 1` a present key's single row is its whole fiber
        // (`fiberCompleteWrt_of_functional`), so the ordinary aggregation over
        // a plain store is ceremony-free (ADR 0023).
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> map_bags |k, b| (.m = bag.max b.temperature)",
            )
            .expect("ok"),
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
        let s = Sources::new()
            .with("long", long)
            .with_ambient(ambient_with_bag());
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
                 |> map_bags |_, b| (.reading = bag.max b.reading)",
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
        let s = Sources::new()
            .with("wide", wide)
            .with_ambient(ambient_with_bag());
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
        let s = Sources::new()
            .with("wide", wide)
            .with("machines", machines)
            .with_ambient(ambient_with_bag());
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
                 |> map_bags |k, b| (.temp_mean = bag.sum b.temperature / to_real (#b.temperature), .temp_max = bag.max b.temperature)",
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
             |> map_bags |k, b| (.temp_max = bag.max b.temperature) }",
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

    /// `window w p size stride` adds `w` to the key with `p`'s domain and
    /// leaves `p` where it was (ADR 0037 decisions 1 and 2).
    #[test]
    fn window_extends_the_key_with_the_point_domain() {
        let s = windowed_sources(true);
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> window w taken_at (15.0 * si.minute) (5.0 * si.minute)",
            )
            .expect("ok"),
        );
        assert_eq!(
            t.content
                .key
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["machine_id", "taken_at", "w"]
        );
        assert_eq!(t.content.key[2].domain, ColumnType::Instant);
        // The replication is injective on (input identity, `w`), so a
        // `singletons` source stays `singletons` at the extended key.
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    /// **The reason the fact is tracked** (ADR 0037 decision 2): each
    /// grading `G` becomes `G + {w}`, so after the window the times are
    /// still unique inside one `(machine, window)` bag and a scan's
    /// tie-freedom discharges with no ceremony, exactly as it does without
    /// the window.
    #[test]
    fn window_extends_the_gradings_so_a_scan_stays_ceremony_free() {
        let s = windowed_sources(true);
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute)",
            )
            .expect("ok"),
        );
        let extended: std::collections::BTreeSet<String> = ["machine_id", "taken_at", "w"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(
            t.qualifiers.functional.contains(&extended),
            "expected the grading to extend with `w`, found {:?}",
            t.qualifiers.functional
        );
        // The payoff, end to end: demote the point and scan inside the
        // window fiber with no `assume { arranged }`.  The scan at `>>` also
        // demands completeness (ADR 0037 decision 5), which `assume` states
        // here because `closed` has not shipped yet.
        assert!(
            pipe_ty(
                &s,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
                 |> demote taken_at \
                 |> assume { complete } \
                 |> map_bags |k, b| (.run = series.running_max (|r| r.temperature) (|r| r.taken_at) b)",
            )
            .is_ok(),
            "the extended grading should discharge the scan's order key"
        );
    }

    /// A window over an `int` point needs no unit machinery: `diff(int)` is
    /// `int` (ADR 0036 decision 4), which is what count-based windows ride
    /// on (ADR 0037 decision 3).
    #[test]
    fn window_accepts_an_int_point_with_int_extents() {
        let s = windowed_sources(true);
        let t = table_of(pipe_ty(&s, "readings |> window w seq 100 10").expect("ok"));
        assert_eq!(t.content.key[2].domain, ColumnType::Int);
    }

    /// The canonical M5 program (ADR 0037's worked example) carries **no
    /// `assume` at all**: every fact is established by a mechanism and
    /// consumed by a demand, which is the language's whole pitch.
    #[test]
    fn closed_establishes_completeness_for_the_reducer() {
        let s = windowed_sources(true);
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
                 |> demote taken_at \
                 |> closed \
                 |> map_bags |k, b| (.peak = bag.max b.temperature)",
            )
            .expect("no assume anywhere"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        assert_eq!(
            t.content
                .key
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["machine_id", "w"]
        );
        // Without `closed` the same pipeline is the ADR 0023 rejection: the
        // coarsening cleared the registry's fine-key fact.
        let errs = pipe_ty(
            &s,
            "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
             |> demote taken_at \
             |> map_bags |k, b| (.peak = bag.max b.temperature)",
        )
        .expect_err("no establishment");
        assert!(errs[0].message.contains("needs completeness"));
    }

    /// `latest p` reduces each fiber to its maximal-point row, demanding
    /// tie-freedom and completeness exactly as the other ordered
    /// reductions do (ADR 0037 decision 7).
    #[test]
    fn latest_reduces_to_the_maximal_point_row() {
        let s = windowed_sources(true);
        // The explicit spelling: coarsen, claim what the fold rests on,
        // then reduce.  Tie-freedom comes from the surviving grading, so
        // there is no `assume { arranged }`.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> demote taken_at |> assume { complete } |> latest taken_at",
            )
            .expect("ok"),
        );
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        assert_eq!(
            t.content
                .key
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["machine_id"]
        );
        // `p` survives as an ordinary total attribute.
        assert!(t.content.columns.iter().any(|c| c.name == "taken_at"));
        assert!(t.qualifiers.totality.is_total("taken_at"));
        // After `closed` both demands discharge by mechanism, with no
        // claim anywhere in the pipeline.
        assert!(
            pipe_ty(
                &s,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
                 |> demote taken_at |> closed |> latest taken_at",
            )
            .is_ok()
        );
    }

    #[test]
    fn latest_rejections_name_their_rule() {
        let s = windowed_sources(true);
        for (src, needle) in [
            // The fused key-column form is not shipped: its completeness
            // demand would have nowhere to stand (ADR 0037 decision 7).
            ("readings |> latest taken_at", "not in the key"),
            // The reducer's two facts, each demanded on its own terms.
            (
                "readings |> demote taken_at |> latest taken_at",
                "needs completeness",
            ),
            (
                "readings |> demote taken_at |> assume { complete } |> latest temperature",
                "may have ties",
            ),
            // A point needs an order, and it needs to be there at all.
            (
                "readings |> demote taken_at |> assume { complete } |> latest note",
                "orderable domain",
            ),
            (
                "readings |> demote taken_at |> assume { complete } |> latest peak",
                "needs `peak` total",
            ),
            (
                "readings |> demote taken_at |> latest nope",
                "unknown column",
            ),
            (
                "readings |> demote taken_at |> latest",
                "takes one point column",
            ),
        ] {
            let errs = pipe_ty(&s, src).expect_err(src);
            assert!(
                errs.iter().any(|e| e.message.contains(needle)),
                "`{needle}` not found for `{src}`: {:?}",
                errs.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn dense_completes_the_grid() {
        let s = windowed_sources(true);
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
                 |> demote taken_at \
                 |> closed \
                 |> map_bags |k, b| (.n = #b.temperature, .peak = bag.max b.temperature) \
                 |> dense w machines activated",
            )
            .expect("the ADR 0038 worked example"),
        );
        // Decision 2: `#b` is a fold at `+`, which has an identity, so `n`
        // stays total and fills with zero; `bag.max` has none, so `peak` is
        // honestly absent on a filled row.
        assert!(t.qualifiers.totality.is_total("n"));
        assert!(t.qualifiers.totality.is_optional("peak"));
        // Decision 4: the fill establishes completeness, and the fact
        // survives the one key move the silence query needs.
        assert_eq!(t.qualifiers.completeness, Completeness::Complete);
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
        let silence = table_of(
            pipe_ty(
                &s,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
                 |> demote taken_at \
                 |> closed \
                 |> map_bags |k, b| (.n = #b.temperature) \
                 |> dense w machines activated \
                 |> demote w \
                 |> map_bags |k, b| (.silent = bag.sum b.n)",
            )
            .expect("no assume anywhere"),
        );
        assert_eq!(
            silence
                .content
                .key
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["machine_id"]
        );
        // Without the fill, the same coarsening clears the fact and the
        // reducer demands it back (ADR 0035, ADR 0023).
        let errs = pipe_ty(
            &s,
            "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
             |> demote taken_at \
             |> closed \
             |> map_bags |k, b| (.n = #b.temperature) \
             |> demote w \
             |> map_bags |k, b| (.silent = bag.sum b.n)",
        )
        .expect_err("the coarsening cleared the fact");
        assert!(errs[0].message.contains("needs completeness"));
    }

    #[test]
    fn dense_rejections_name_their_rule() {
        let s = windowed_sources(true);
        let windowed = "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
                        |> demote taken_at";
        let reduced = format!("{windowed} |> closed |> map_bags |k, b| (.n = #b.temperature)");
        for (src, needle) in [
            // Arity: both policy arguments are required (decision 3).
            (
                format!("{reduced} |> dense w"),
                "takes a window column".to_string(),
            ),
            // No grid: `dense` is the window-grid stage, not `resample`
            // (decision 5).
            (
                format!("{reduced} |> dense machine_id machines activated"),
                "carries no window fact".to_string(),
            ),
            // A column outside the key indexes nothing.
            (
                format!("{reduced} |> dense n machines activated"),
                "needs `n` in the key".to_string(),
            ),
            // No upper bound without `closed` (decision 3).
            // A grid nobody closed has no upper bound, even where the
            // reducer's own demand was met by a visible claim instead.
            (
                format!(
                    "{windowed} |> assume {{ complete }} \
                         |> map_bags |k, b| (.n = #b.temperature) \
                         |> dense w machines activated"
                ),
                "bounds the grid above by closedness".to_string(),
            ),
            // Before the reduction there is nothing to fill (decision 1).
            (
                format!("{windowed} |> closed |> dense w machines activated"),
                "completes reduced rows".to_string(),
            ),
            // The population must be keyed like the windowed rows.
            (
                format!("{reduced} |> dense w readings activated"),
                "keyed like the windowed rows".to_string(),
            ),
            (
                format!("{reduced} |> dense w fleet activated"),
                "unknown source `fleet`".to_string(),
            ),
            // The bound is a column of the population, total, and in the
            // window column's domain.
            (
                format!("{reduced} |> dense w machines started"),
                "no column `started`".to_string(),
            ),
            (
                format!("{reduced} |> dense w machines retired"),
                "needs `retired` total".to_string(),
            ),
            (
                format!("{reduced} |> dense w machines commissioned"),
                "ADR 0036 defers".to_string(),
            ),
        ] {
            let errs = pipe_ty(&s, &src).expect_err(&src);
            assert!(
                errs.iter().any(|e| e.message.contains(&needle)),
                "`{needle}` not found for `{src}`: {:?}",
                errs.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn closed_rejections_name_their_rule() {
        let contracted = windowed_sources(true);
        let bare = windowed_sources(false);
        for (s, src, needle) in [
            // No grid at all.
            (
                &contracted,
                "readings |> closed",
                "needs a window column in the key",
            ),
            // The point is still in the key, so the window is not yet whole.
            (
                &contracted,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) |> closed",
                "in the fiber, not in the key",
            ),
            // No contract: no mechanism, so the claim must be visible.
            (
                &bare,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
                 |> demote taken_at |> closed",
                "assume { complete }",
            ),
            // The watermark grain must survive to the point of the filter
            // (ADR 0041 decision 2).
            (
                &contracted,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
                 |> demote taken_at machine_id |> closed",
                "watermark grain",
            ),
            (
                &contracted,
                "readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute) \
                 |> demote taken_at |> closed w",
                "takes no arguments",
            ),
        ] {
            let errs = pipe_ty(s, src).expect_err(src);
            assert!(
                errs.iter().any(|e| e.message.contains(needle)),
                "`{needle}` not found for `{src}`: {:?}",
                errs.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn window_rejections_name_their_rule() {
        let s = windowed_sources(true);
        for (src, needle) in [
            // `w` is fresh, unlike every other column argument.
            (
                "readings |> window temperature taken_at (15.0 * si.minute) (5.0 * si.minute)",
                "would duplicate column",
            ),
            (
                "readings |> window w nope (15.0 * si.minute) (5.0 * si.minute)",
                "unknown column",
            ),
            // The point needs a difference type (ADR 0036 decision 4).
            (
                "readings |> window w day (15.0 * si.minute) (5.0 * si.minute)",
                "deferred",
            ),
            (
                "readings |> window w temperature (15.0 * si.minute) (5.0 * si.minute)",
                "difference type",
            ),
            (
                "readings |> window w note (15.0 * si.minute) (5.0 * si.minute)",
                "total",
            ),
            // The extents are const, of the point's difference type,
            // positive, and exact on the millisecond grid.
            (
                "readings |> window w taken_at 15 (5.0 * si.minute)",
                "must be a duration",
            ),
            (
                "readings |> window w taken_at (0.0 * si.minute) (5.0 * si.minute)",
                "must be positive",
            ),
            (
                "readings |> window w taken_at (15.0 * si.minute) (-5.0 * si.minute)",
                "must be positive",
            ),
            (
                "readings |> window w taken_at (0.0001 * second) (5.0 * si.minute)",
                "not rounded",
            ),
            ("readings |> window w taken_at", "takes a window column"),
        ] {
            let errs = pipe_ty(&s, src).expect_err(src);
            assert!(
                errs.iter().any(|e| e.message.contains(needle)),
                "`{needle}` not found for `{src}`: {:?}",
                errs.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
        }
    }
}
