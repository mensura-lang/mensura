//! Pipeline (table-expression) type checking: the Tier A operations over
//! `TableType` (`docs/language/09-typing-reference.md` sections 6 and 10).
//!
//! This layer sits above [`crate::expr_check`]: it types a table-valued
//! expression against a set of named [`Sources`], dispatching each `|>` stage to
//! an operation handler that transforms the input [`TableType`]. Each operation's
//! lambda body (`|r|` / `|g|` / `|k|`) is typed by `expr_check` against a
//! row/group/key context derived from the input table. Like `resolve` and
//! `expr_check`, it collects all diagnostics rather than failing fast.

use std::collections::BTreeMap;

use mensura_syntax::{BinOp, Block, Expr, ExprKind, Span, Stmt};

use crate::expr_check::{Context, Optionality, Ty, TypeError, type_expr};
use crate::model::ColumnType;
use crate::table::{
    Cardinality, Column, Completeness, Content, Lineage, Qualifiers, SplitId, TableType, Totality,
};

/// The type of a table-valued (pipeline) expression.
#[derive(Clone, Debug, PartialEq)]
pub enum PipeTy {
    Table(TableType),
    /// A pair of tables: produced by `split`, consumed by `bind` (section 6.5).
    Pair(TableType, TableType),
}

/// The named source tables in scope (a store presented to a pipeline,
/// `10-views.md`, "Sources resolve by name").
#[derive(Clone, Debug, Default)]
pub struct Sources {
    bound: BTreeMap<String, PipeTy>,
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
            None => Err(error(format!("unknown source `{name}`"), expr.span)),
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
            apply_op(sources, input, rhs)
        }
        _ => Err(error("not a pipeline expression", expr.span)),
    }
}

fn expect_table(pipe: PipeTy, span: Span) -> Result<TableType, Vec<TypeError>> {
    match pipe {
        PipeTy::Table(table) => Ok(table),
        PipeTy::Pair(..) => Err(error("expected a single table, found a pair", span)),
    }
}

/// Apply a pipeline operation (the right side of a `|>`) to its input table.
fn apply_op(sources: &Sources, input: PipeTy, op_expr: &Expr) -> Result<PipeTy, Vec<TypeError>> {
    let (head, args) = flatten_app(op_expr);
    let ExprKind::Name(op) = &head.kind else {
        return Err(error("expected a pipeline operation", op_expr.span));
    };
    match op.as_str() {
        "extend_key" => op_extend_key(input, &args, head.span),
        "map" => op_map(input, &args, head.span),
        "group_map" => op_group_map(input, &args, head.span),
        "split" => op_split(input, &args, head.span),
        "bind" => op_bind(input, &args, head.span),
        "left_join" => op_join(sources, input, &args, head.span, JoinKind::Left),
        "inner_join" => op_join(sources, input, &args, head.span, JoinKind::Inner),
        _ => Err(error(format!("unsupported operation `{op}`"), head.span)),
    }
}

#[derive(Clone, Copy)]
enum JoinKind {
    Left,
    Inner,
}

/// `left_join` / `inner_join right (|l| key)` (section 6.4, Tier A): join a fixed
/// right table by a key over the left row. Adds the right table's non-index
/// columns; `left_join` makes them optional, `inner_join` keeps their totality.
/// The right table is a store (`Singletons`, functional), so cardinality is
/// preserved; completeness on the left and lineage are preserved.
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
    let [right_key] = right.content.index.as_slice() else {
        return Err(error(
            "a join's right table must have a single index column",
            right_arg.span,
        ));
    };

    let ExprKind::Lambda { params, body, .. } = &key_arg.kind else {
        return Err(error(
            "a join's second argument must be a key lambda",
            key_arg.span,
        ));
    };
    let [param] = params.as_slice() else {
        return Err(error(
            "a join's key lambda takes one parameter",
            key_arg.span,
        ));
    };
    let ctx = Context::row(&param.name, &left);
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
            || left.content.index.iter().any(|c| c.name == rc.name);
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
            index: left.content.index,
            columns,
        },
        qualifiers: Qualifiers {
            cardinality: left.qualifiers.cardinality,
            totality,
            completeness: left.qualifiers.completeness,
            lineage: left.qualifiers.lineage,
        },
    }))
}

/// `map |r| record` (section 6.1, Tier A): a per-row transform. The returned
/// record becomes the non-index columns; the index and cardinality are
/// preserved.
fn op_map(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let table = expect_table(input, span)?;
    let (param, body) = single_lambda(args, "map", span)?;
    let ctx = Context::row(param, &table);
    let (columns, totality) = record_to_content(&ctx, body, "map")?;
    Ok(PipeTy::Table(TableType {
        content: Content {
            index: table.content.index,
            columns,
        },
        qualifiers: Qualifiers {
            cardinality: table.qualifiers.cardinality,
            totality,
            completeness: table.qualifiers.completeness,
            lineage: table.qualifiers.lineage,
        },
    }))
}

/// `group_map |g| record` (section 6.2, Tier A): transform each group. The
/// result cardinality is **inferred from the return**: all single-valued fields
/// are the aggregate shape (one row per key, `Singletons`); bag-valued fields are
/// the window shape (one output row per input row, `Bag`). The index,
/// completeness, and lineage are preserved.
fn op_group_map(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let table = expect_table(input, span)?;
    let (param, body) = single_lambda(args, "group_map", span)?;
    let ctx = Context::group(param, &table);
    let (columns, totality, cardinality) = group_record_content(&ctx, body)?;
    Ok(PipeTy::Table(TableType {
        content: Content {
            index: table.content.index,
            columns,
        },
        qualifiers: Qualifiers {
            cardinality,
            totality,
            completeness: table.qualifiers.completeness,
            lineage: table.qualifiers.lineage,
        },
    }))
}

/// Type a `group_map` record body into columns, totality, and the result
/// cardinality (section 6.2). Single-valued fields are aggregates (`Singletons`);
/// bag-valued fields are window values (`Bag`); a mix of the two is rejected.
fn group_record_content(
    ctx: &Context,
    body: &Expr,
) -> Result<(Vec<Column>, Totality, Cardinality), Vec<TypeError>> {
    let ExprKind::Record(fields) = &body.kind else {
        return Err(error(
            "`group_map`'s lambda must return a record",
            body.span,
        ));
    };
    if fields.is_empty() {
        return Err(error(
            "`group_map`'s record needs at least one field",
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
            "a `group_map` record must be all aggregates (one row per key) or all \
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
/// and completeness are unchanged on both sides.
fn op_split(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let table = expect_table(input, span)?;
    let (param, body) = single_lambda(args, "split", span)?;
    let ctx = Context::key(param, &table);
    if type_expr(&ctx, body)? != Ty::Bool {
        return Err(error("`split`'s predicate must be a boolean", body.span));
    }
    let id = SplitId(span.start as u32);
    let (left, right) = table.qualifiers.lineage.split(id);
    Ok(PipeTy::Pair(
        with_lineage(&table, left),
        with_lineage(&table, right),
    ))
}

/// `bind` (section 6.5, Tier A): the union of a pair of tables of the same
/// schema. Cardinality is `Singletons` iff both inputs are and their lineages
/// are disjoint, else `Bag`; completeness holds iff both inputs are complete;
/// the lineage tag-sets union.
fn op_bind(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    if !args.is_empty() {
        return Err(error("`bind` takes no arguments", span));
    }
    let (a, b) = match input {
        PipeTy::Pair(a, b) => (a, b),
        PipeTy::Table(_) => return Err(error("`bind` expects a pair of tables", span)),
    };
    if a.content != b.content {
        return Err(error(
            "`bind` requires both tables to have the same schema",
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
    Ok(PipeTy::Table(TableType {
        content: a.content,
        qualifiers: Qualifiers {
            cardinality,
            totality: a.qualifiers.totality,
            completeness,
            lineage: a.qualifiers.lineage.union(&b.qualifiers.lineage),
        },
    }))
}

fn with_lineage(table: &TableType, lineage: Lineage) -> TableType {
    TableType {
        content: table.content.clone(),
        qualifiers: Qualifiers {
            cardinality: table.qualifiers.cardinality,
            totality: table.qualifiers.totality.clone(),
            completeness: table.qualifiers.completeness,
            lineage,
        },
    }
}

/// Extract the single one-parameter lambda an operation expects, returning its
/// parameter name and body.
fn single_lambda<'a>(
    args: &[&'a Expr],
    op: &str,
    span: Span,
) -> Result<(&'a str, &'a Expr), Vec<TypeError>> {
    let [arg] = args else {
        return Err(error(format!("`{op}` expects one lambda argument"), span));
    };
    let ExprKind::Lambda { params, body, .. } = &arg.kind else {
        return Err(error(format!("`{op}` expects a lambda"), arg.span));
    };
    let [param] = params.as_slice() else {
        return Err(error(
            format!("`{op}`'s lambda takes one parameter"),
            arg.span,
        ));
    };
    Ok((param.name.as_str(), body.as_ref()))
}

/// Type a record-returning lambda body into output columns and their totality
/// (used by `map` and `group_map`). Each field's value is a value expression
/// typed by `expr_check`.
fn record_to_content(
    ctx: &Context,
    body: &Expr,
    op: &str,
) -> Result<(Vec<Column>, Totality), Vec<TypeError>> {
    let ExprKind::Record(fields) = &body.kind else {
        return Err(error(
            format!("`{op}`'s lambda must return a record"),
            body.span,
        ));
    };
    let mut columns = Vec::new();
    let mut totality = Totality::all_total();
    let mut errs = Vec::new();
    for field in fields {
        match type_expr(ctx, &field.value) {
            Err(e) => errs.extend(e),
            Ok(ty) => match column_of(&ty) {
                Some((domain, opt)) => {
                    columns.push(Column {
                        name: field.name.name.clone(),
                        domain,
                    });
                    if opt == Optionality::Optional {
                        totality.mark_optional(field.name.name.clone());
                    }
                }
                None => errs.push(te(
                    format!("field `{}` is not a single value", field.name.name),
                    field.value.span,
                )),
            },
        }
    }
    if errs.is_empty() {
        Ok((columns, totality))
    } else {
        Err(errs)
    }
}

/// The column domain and totality a value type contributes, or `None` for a bag
/// or nested record (window/nested returns are deferred).
fn column_of(ty: &Ty) -> Option<(ColumnType, Optionality)> {
    match ty {
        Ty::Value { domain, opt } => Some((domain.clone(), *opt)),
        Ty::Bool => Some((ColumnType::Bool, Optionality::Total)),
        Ty::Bag { .. } | Ty::Record(_) => None,
    }
}

/// `extend_key cols` (section 6.3, Tier A): promote each named column into the
/// index. A column must be total to enter the key (ADR 0013); cardinality,
/// completeness, and lineage are preserved.
fn op_extend_key(input: PipeTy, args: &[&Expr], span: Span) -> Result<PipeTy, Vec<TypeError>> {
    let mut table = expect_table(input, span)?;
    if args.is_empty() {
        return Err(error("`extend_key` needs at least one column", span));
    }
    let mut errs = Vec::new();
    for arg in args {
        let ExprKind::Name(col) = &arg.kind else {
            errs.push(te("`extend_key` expects column names", arg.span));
            continue;
        };
        if let Err(e) = promote_to_index(&mut table, col, arg.span) {
            errs.push(e);
        }
    }
    if errs.is_empty() {
        Ok(PipeTy::Table(table))
    } else {
        Err(errs)
    }
}

fn promote_to_index(table: &mut TableType, col: &str, span: Span) -> Result<(), TypeError> {
    if table.content.index.iter().any(|c| c.name == col) {
        return Err(te(format!("`{col}` is already in the index"), span));
    }
    let Some(pos) = table.content.columns.iter().position(|c| c.name == col) else {
        return Err(te(format!("unknown column `{col}`"), span));
    };
    if !table.content.columns[pos].domain.is_key_eligible() {
        return Err(te(
            format!(
                "`extend_key` cannot promote `{col}`: its type is not key-eligible \
                 (a continuous `real` measurement is not an identity)"
            ),
            span,
        ));
    }
    if table.qualifiers.totality.is_optional(col) {
        return Err(te(
            format!("`extend_key` requires `{col}` to be total; narrow it first"),
            span,
        ));
    }
    let column = table.content.columns.remove(pos);
    table.content.index.push(column);
    Ok(())
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
            span: Span::new(0, 0),
        })
    }

    fn sample_sources() -> Sources {
        let readings = from_cols(
            "readings",
            "Reading",
            vec![
                scol("ts", ColumnType::Int, ColumnRole::Index, false),
                scol("machine", ColumnType::String, ColumnRole::Var, false),
                scol("temperature", ColumnType::Real, ColumnRole::Var, false),
                scol("peak", ColumnType::Real, ColumnRole::Var, true),
                scol("flag", ColumnType::Bool, ColumnRole::Var, false),
                scol("note", ColumnType::String, ColumnRole::Var, true),
            ],
        );
        let machines = from_cols(
            "machines",
            "Machine",
            vec![
                scol("machine", ColumnType::String, ColumnRole::Index, false),
                scol("vendor", ColumnType::String, ColumnRole::Var, false),
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
        assert_eq!(t.content.index[0].name, "ts");
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
    fn extend_key_promotes_a_total_column() {
        let s = sample_sources();
        let t = table_of(pipe_ty(&s, "readings |> extend_key machine").expect("ok"));
        assert!(t.content.index.iter().any(|c| c.name == "machine"));
        assert!(!t.content.columns.iter().any(|c| c.name == "machine"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn extend_key_rejects_optional_column() {
        let s = sample_sources();
        // `note` is key-eligible (string) but optional.
        let errs = pipe_ty(&s, "readings |> extend_key note").expect_err("optional");
        assert!(errs[0].message.contains("to be total"));
    }

    #[test]
    fn extend_key_rejects_real_column() {
        let s = sample_sources();
        // `temperature` is a real measurement: not key-eligible (ADR 0014).
        let errs = pipe_ty(&s, "readings |> extend_key temperature").expect_err("real");
        assert!(errs[0].message.contains("key-eligible"));
    }

    #[test]
    fn extend_key_unknown_column_errors() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> extend_key bogus").expect_err("unknown column");
        assert!(errs[0].message.contains("unknown column `bogus`"));
    }

    #[test]
    fn map_derives_columns_preserving_cardinality() {
        let s = sample_sources();
        let t =
            table_of(pipe_ty(&s, "readings |> map |r| (.hot = r.temperature > 30.0)").expect("ok"));
        assert!(t.content.index.iter().any(|c| c.name == "ts"));
        assert_eq!(t.content.columns.len(), 1);
        assert_eq!(t.content.columns[0].name, "hot");
        assert_eq!(t.content.columns[0].domain, ColumnType::Bool);
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn map_propagates_field_errors() {
        let s = sample_sources();
        // `peak` is optional, so a scalar on it is rejected by expr_check.
        let errs = pipe_ty(&s, "readings |> map |r| (.x = r.peak + 1)").expect_err("optional");
        assert!(errs[0].message.contains("known value"));
    }

    #[test]
    fn group_map_summarizes_to_singletons() {
        let s = sample_sources();
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> extend_key machine \
                 |> group_map |g| (.temp_mean = sum g.temperature / to_real (count g.temperature), .temp_max = max g.temperature)",
            )
            .expect("ok"),
        );
        assert!(t.content.index.iter().any(|c| c.name == "machine"));
        assert!(t.content.columns.iter().any(|c| c.name == "temp_mean"));
        assert!(t.content.columns.iter().any(|c| c.name == "temp_max"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn group_map_rejects_non_numeric_aggregate() {
        let s = sample_sources();
        let errs =
            pipe_ty(&s, "readings |> group_map |g| (.m = sum g.machine)").expect_err("non-numeric");
        assert!(errs[0].message.contains("numeric bag"));
    }

    #[test]
    fn group_map_with_a_bag_field_stays_a_bag() {
        let s = sample_sources();
        // A bag-valued field is the window shape: one output row per input row.
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> extend_key machine |> group_map |g| (.temps = g.temperature)",
            )
            .expect("ok"),
        );
        assert!(t.content.columns.iter().any(|c| c.name == "temps"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
    }

    #[test]
    fn group_map_rejects_mixed_aggregate_and_window() {
        let s = sample_sources();
        let errs = pipe_ty(
            &s,
            "readings |> group_map |g| (.m = sum g.temperature, .t = g.temperature)",
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
        // `machine` is a column, not in the index, so it is unknown in the key.
        let errs = pipe_ty(&s, "readings |> split |k| k.machine == \"m1\"").expect_err("unknown");
        assert!(errs[0].message.contains("unknown column `machine`"));
    }

    #[test]
    fn bind_reconstructs_disjoint_split() {
        let s = sample_sources();
        let t = table_of(pipe_ty(&s, "readings |> split |k| k.ts > 100 |> bind").expect("ok"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn bind_of_overlapping_inputs_is_a_bag() {
        let s = sample_sources();
        let t = table_of(pipe_ty(&s, "(readings, readings) |> bind").expect("ok"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Bag);
    }

    #[test]
    fn bind_requires_a_pair() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> bind").expect_err("not a pair");
        assert!(errs[0].message.contains("expects a pair"));
    }

    #[test]
    fn left_join_adds_optional_columns() {
        let s = sample_sources();
        let t =
            table_of(pipe_ty(&s, "readings |> left_join machines (|l| l.machine)").expect("ok"));
        assert!(t.content.columns.iter().any(|c| c.name == "vendor"));
        assert!(t.qualifiers.totality.is_optional("vendor"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn inner_join_keeps_totality() {
        let s = sample_sources();
        let t =
            table_of(pipe_ty(&s, "readings |> inner_join machines (|l| l.machine)").expect("ok"));
        assert!(t.qualifiers.totality.is_total("vendor"));
    }

    #[test]
    fn join_key_domain_must_match() {
        let s = sample_sources();
        // `ts` is a number, but `machines` is keyed by a string.
        let errs = pipe_ty(&s, "readings |> left_join machines (|l| l.ts)").expect_err("domain");
        assert!(errs[0].message.contains("key domain"));
    }

    // The two worked examples from docs/language/10-views.md, end to end.

    #[test]
    fn worked_example_machine_temperature() {
        let s = sample_sources();
        let t = table_of(
            pipe_ty(
                &s,
                "readings |> extend_key machine \
                 |> group_map |g| (.temp_mean = sum g.temperature / to_real (count g.temperature), .temp_max = max g.temperature)",
            )
            .expect("machine_temperature types"),
        );
        assert!(t.content.index.iter().any(|c| c.name == "ts"));
        assert!(t.content.index.iter().any(|c| c.name == "machine"));
        assert_eq!(t.content.columns.len(), 2);
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn worked_example_full_dataset_reconstructs() {
        let s = sample_sources();
        let whole = table_of(pipe_ty(&s, "readings").expect("source"));
        let rebound = table_of(
            pipe_ty(&s, "readings |> split |k| k.ts > 100 |> bind").expect("full_dataset"),
        );
        // Binding the disjoint split halves reconstructs the schema and keeps
        // `singletons` (bind_split, 09 §11).
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
            "view machine_temperature { readings |> extend_key machine \
             |> group_map |g| (.temp_max = max g.temperature) }",
        );
        let t = type_view(&s, &body).expect("ok");
        assert!(t.content.index.iter().any(|c| c.name == "machine"));
        assert_eq!(t.qualifiers.cardinality, Cardinality::Singletons);
    }

    #[test]
    fn view_body_threads_let_bindings() {
        let s = sample_sources();
        let body = view_body(
            "view full_dataset { let parts = readings |> split |k| k.ts > 100; parts |> bind }",
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
}
