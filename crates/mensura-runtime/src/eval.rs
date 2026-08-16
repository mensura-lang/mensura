//! The batch evaluator of the processing layer
//! (`docs/toolkit/04-processing-layer.md`).
//!
//! Evaluates a checked view body ([`mensura_types::ViewPlan`]) over batches
//! of typed rows.  The checker has already established that the body is
//! well-typed, so evaluation cannot fail on shape; every "internal" error
//! here marks a case the frontend is supposed to have ruled out.  All Tier A
//! operations execute (`flat_map`, `map_bags`, `promote`, the joins,
//! `split`/`union`, `unpivot`), plus the Tier B stages `pivot` and `demote`
//! (Tier B only for their lineage effect, ADR 0020/0024; batch evaluation
//! is unaffected), and the establish stages (`assume`,
//! `completeness_check`) are identities: their facts are proven at compile
//! time and trusted at runtime (`ROADMAP.md` M2).

use std::collections::BTreeMap;

use mensura_syntax::{BinOp, Expr, ExprKind, Presence, Stmt, UnOp};
use mensura_types::{ColumnRole, ColumnType, ResolvedProgram, Schema, ViewPlan};

use crate::backend::{StorageBackend, StorageError};
use crate::temporal;
use crate::value::{Row, Value};

/// A processing-layer failure.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalError {
    pub message: String,
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for EvalError {}

fn err<T>(message: impl Into<String>) -> Result<T, EvalError> {
    Err(EvalError {
        message: message.into(),
    })
}

/// An "impossible after type checking" failure, prefixed so a report is
/// recognizably a frontend/runtime disagreement, not a user error.
fn internal<T>(message: impl Into<String>) -> Result<T, EvalError> {
    err(format!("internal: {}", message.into()))
}

/// One column flowing through the evaluator: its name and, when statically
/// known, its domain.  Domains ride along because `pivot` needs the name
/// column's enum variants; a column computed by an expression may lose its
/// domain (`None`), which only matters if a later `pivot` reads it.
#[derive(Clone, Debug, PartialEq)]
struct Col {
    name: String,
    ty: Option<ColumnType>,
}

fn col(name: impl Into<String>, ty: Option<ColumnType>) -> Col {
    Col {
        name: name.into(),
        ty,
    }
}

/// A table value flowing through a pipeline: its key/value column split and
/// its rows (key values first, then attributes, positionally).
#[derive(Clone, Debug, PartialEq)]
pub struct SourceTable {
    key: Vec<Col>,
    attrs: Vec<Col>,
    rows: Vec<Row>,
}

impl SourceTable {
    /// Present a store's scanned rows to the evaluator.  `rows` are in the
    /// schema's column order, as [`StorageBackend::scan`] returns them.
    pub fn from_store(schema: &Schema, rows: Vec<Row>) -> SourceTable {
        let mut key = Vec::new();
        let mut attrs = Vec::new();
        for c in &schema.columns {
            let entry = col(c.name.clone(), Some(c.ty.clone()));
            match c.role {
                ColumnRole::Key => key.push(entry),
                ColumnRole::Attr => attrs.push(entry),
            }
        }
        SourceTable { key, attrs, rows }
    }

    fn key_len(&self) -> usize {
        self.key.len()
    }

    fn attr_position(&self, name: &str) -> Option<usize> {
        self.attrs.iter().position(|c| c.name == name)
    }
}

/// A pipeline value: a single table, or the pair a `split` yields and a
/// `union` consumes.  Mirrors the checker's `PipeTy`.
#[derive(Clone, Debug)]
enum TableVal {
    Table(SourceTable),
    Pair(SourceTable, SourceTable),
}

fn expect_table(v: TableVal) -> Result<SourceTable, EvalError> {
    match v {
        TableVal::Table(t) => Ok(t),
        TableVal::Pair(..) => internal("expected a single table, found a pair"),
    }
}

/// One output row of a `flat_map` body: named values in output-column order.
type NamedRow = Vec<(String, Value)>;

/// A scalar-expression value: a single [`Value`], a group's bag at one key,
/// or a row/key record.
#[derive(Clone, Debug)]
enum RtVal {
    V(Value),
    Bag(Vec<Value>),
    Rec(BTreeMap<String, RtVal>),
}

type Scope = BTreeMap<String, RtVal>;

/// A grouping/join key value.  Index domains are key-eligible (equatable, so
/// never `real`) and never missing, which is what makes this total order
/// sound.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum KeyVal {
    Bool(bool),
    Int(i64),
    Text(String),
}

fn key_of(v: &Value) -> Result<KeyVal, EvalError> {
    match v {
        Value::Int(i) => Ok(KeyVal::Int(*i)),
        Value::Bool(b) => Ok(KeyVal::Bool(*b)),
        Value::String(s) | Value::Date(s) | Value::Instant(s) | Value::Enum(s) => {
            Ok(KeyVal::Text(s.clone()))
        }
        Value::Real(_) => internal("a `real` value as a key"),
        Value::Missing => internal("a missing value as a key"),
    }
}

/// Evaluate a view plan over its sources, returning the materialized rows in
/// the plan's column order (key columns, then attributes).
pub fn eval_view(
    plan: &ViewPlan,
    sources: &BTreeMap<String, SourceTable>,
) -> Result<Vec<Row>, EvalError> {
    let mut env: BTreeMap<String, TableVal> = sources
        .iter()
        .map(|(name, table)| (name.clone(), TableVal::Table(table.clone())))
        .collect();
    let mut result: Option<TableVal> = None;
    let last = plan.body.stmts.len().saturating_sub(1);
    for (i, stmt) in plan.body.stmts.iter().enumerate() {
        match stmt {
            Stmt::Let { name, value, .. } => {
                let table = eval_pipeline(&env, value)?;
                env.insert(name.name.clone(), table);
            }
            Stmt::Expr(e) if i == last => result = Some(eval_pipeline(&env, e)?),
            _ => return internal("view body statement the checker should have rejected"),
        }
    }
    let Some(table) = result else {
        return internal("view body without a trailing table expression");
    };
    align(plan, expect_table(table)?)
}

/// Reorder a table's rows into the plan's output column order.  The checker
/// derived the plan's columns from the same body, so this is normally the
/// identity; a name mismatch is an internal error.
fn align(plan: &ViewPlan, table: SourceTable) -> Result<Vec<Row>, EvalError> {
    if table.rows.is_empty() {
        return Ok(Vec::new());
    }
    let actual: Vec<&String> = table
        .key
        .iter()
        .chain(table.attrs.iter())
        .map(|c| &c.name)
        .collect();
    let expected: Vec<&String> = plan.columns.iter().map(|c| &c.name).collect();
    if actual == expected {
        return Ok(table.rows);
    }
    let mut positions = Vec::with_capacity(expected.len());
    for name in &expected {
        match actual.iter().position(|a| a == name) {
            Some(pos) => positions.push(pos),
            None => {
                return internal(format!("view `{}` computed no column `{name}`", plan.name));
            }
        }
    }
    Ok(table
        .rows
        .into_iter()
        .map(|row| positions.iter().map(|&p| row[p].clone()).collect())
        .collect())
}

/// Evaluate a pipeline expression to a table value.  Mirrors the checker's
/// `type_pipeline`.
fn eval_pipeline(env: &BTreeMap<String, TableVal>, expr: &Expr) -> Result<TableVal, EvalError> {
    match &expr.kind {
        ExprKind::Name(name) => match env.get(name) {
            Some(table) => Ok(table.clone()),
            None => internal(format!("unknown source `{name}`")),
        },
        ExprKind::Tuple(items) if items.len() == 2 => {
            let a = expect_table(eval_pipeline(env, &items[0])?)?;
            let b = expect_table(eval_pipeline(env, &items[1])?)?;
            Ok(TableVal::Pair(a, b))
        }
        ExprKind::Binary(BinOp::Pipe, lhs, rhs) => {
            let input = eval_pipeline(env, lhs)?;
            let (head, args) = flatten_app(rhs);
            apply_op(env, head, &args, input)
        }
        ExprKind::App(..) => {
            let (head, mut args) = flatten_app(expr);
            let Some(last) = args.pop() else {
                return internal("pipeline application without an input");
            };
            let input = eval_pipeline(env, last)?;
            apply_op(env, head, &args, input)
        }
        _ => internal("expression is not a pipeline"),
    }
}

/// Apply a pipeline stage to its input.  The establish stages (`assume`,
/// `completeness_check`) are runtime identities: their facts are proven at
/// compile time and trusted here (`ROADMAP.md` M2).
fn apply_op(
    env: &BTreeMap<String, TableVal>,
    head: &Expr,
    args: &[&Expr],
    input: TableVal,
) -> Result<TableVal, EvalError> {
    let ExprKind::Name(op) = &head.kind else {
        return internal("pipeline stage without an operation name");
    };
    match op.as_str() {
        "flat_map" => Ok(TableVal::Table(eval_flat_map(expect_table(input)?, args)?)),
        "map_bags" => Ok(TableVal::Table(eval_map_bags(expect_table(input)?, args)?)),
        "promote" => Ok(TableVal::Table(eval_promote(expect_table(input)?, args)?)),
        "split" => eval_split(expect_table(input)?, args),
        "union" => Ok(TableVal::Table(eval_bind(input)?)),
        "lookup" => Ok(TableVal::Table(eval_join(
            env,
            expect_table(input)?,
            args,
            JoinKind::Left,
        )?)),
        "lookup_total" => Ok(TableVal::Table(eval_join(
            env,
            expect_table(input)?,
            args,
            JoinKind::Inner,
        )?)),
        "unpivot" => Ok(TableVal::Table(eval_unpivot(expect_table(input)?, args)?)),
        "pivot" => Ok(TableVal::Table(eval_pivot(expect_table(input)?, args)?)),
        "assume" | "completeness_check" => Ok(TableVal::Table(expect_table(input)?)),
        "demote" => Ok(TableVal::Table(eval_demote(expect_table(input)?, args)?)),
        "window" => Ok(TableVal::Table(eval_window(expect_table(input)?, args)?)),
        other => err(format!(
            "`{other}` is not yet executable (docs/toolkit/04-processing-layer.md)"
        )),
    }
}

/// Decompose a curried application `f a b` into `(f, [a, b])`.  Mirrors the
/// checker's copy.
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

/// Extract a key-first lambda's parameter names and body (`arity` 1 or 2).
fn lambda_parts<'a>(
    args: &[&'a Expr],
    arity: usize,
) -> Result<(Vec<&'a str>, &'a Expr), EvalError> {
    let [arg] = args else {
        return internal("operation expects exactly a lambda argument");
    };
    let ExprKind::Lambda { params, body, .. } = &arg.kind else {
        return internal("operation expects a lambda argument");
    };
    if params.len() != arity {
        return internal("lambda with the wrong number of parameters");
    }
    Ok((params.iter().map(|p| p.name.as_str()).collect(), body))
}

/// Bind the key-first parameters over one row, skipping `_` (ADR 0015).
fn row_scope(table: &SourceTable, kname: &str, rname: &str, row: &Row) -> Scope {
    let (key, vals) = row.split_at(table.key_len());
    let mut scope = Scope::new();
    if kname != "_" {
        scope.insert(kname.to_string(), record(&table.key, key));
    }
    if rname != "_" {
        scope.insert(rname.to_string(), record(&table.attrs, vals));
    }
    scope
}

fn record(cols: &[Col], values: &[Value]) -> RtVal {
    let mut fields = BTreeMap::new();
    for (c, v) in cols.iter().zip(values.iter().cloned()) {
        nest_insert(&mut fields, &c.name, RtVal::V(v));
    }
    RtVal::Rec(fields)
}

/// Insert a flat dotted column into a nested record (ADR 0032), mirroring
/// the checker's presentation: `course.department.code` lands as `course`
/// holding `department` holding `code`, so a member chain resolves one step
/// at a time.  The resolver reserves a group's prefix, so a scalar column
/// never collides with a group name.
fn nest_insert(map: &mut BTreeMap<String, RtVal>, name: &str, v: RtVal) {
    match name.split_once('.') {
        None => {
            map.insert(name.to_string(), v);
        }
        Some((head, rest)) => {
            let entry = map
                .entry(head.to_string())
                .or_insert_with(|| RtVal::Rec(BTreeMap::new()));
            let RtVal::Rec(sub) = entry else {
                // Unreachable: the resolver reserves group prefixes.
                return;
            };
            nest_insert(sub, rest, v);
        }
    }
}

/// Flatten a (possibly nested) row record back to dotted named values
/// (ADR 0032).  Recursing in map order yields the flat names already
/// sorted, because `.` orders below every identifier character; this is
/// the order the checker's whole-row rule uses.
fn flatten_rec(
    prefix: &str,
    fields: BTreeMap<String, RtVal>,
    out: &mut Vec<(String, Value)>,
) -> Result<(), EvalError> {
    for (name, v) in fields {
        let full = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}.{name}")
        };
        match v {
            RtVal::V(v) => out.push((full, v)),
            RtVal::Rec(sub) => flatten_rec(&full, sub, out)?,
            RtVal::Bag(_) => return internal("a row field that is not a single value"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// map

/// `flat_map |k, r| collection` (ADR 0015): evaluate the body once per input row;
/// each yielded value row becomes one output row under the same key, and
/// `( )` yields none.
fn eval_flat_map(input: SourceTable, args: &[&Expr]) -> Result<SourceTable, EvalError> {
    let (params, body) = lambda_parts(args, 2)?;
    let Some(schema) = flat_map_row_schema(&input, params[0], params[1], body)? else {
        return internal("a `flat_map` body that always drops (the checker rejects this)");
    };
    let mut rows = Vec::new();
    for row in &input.rows {
        let scope = row_scope(&input, params[0], params[1], row);
        for named in eval_rows(&scope, body)? {
            if named.len() != schema.len()
                || named.iter().zip(&schema).any(|((n, _), c)| *n != c.name)
            {
                return internal("`flat_map` rows with differing schemas");
            }
            let mut out: Row = row[..input.key_len()].to_vec();
            out.extend(named.into_iter().map(|(_, v)| v));
            rows.push(out);
        }
    }
    Ok(SourceTable {
        key: input.key,
        attrs: schema,
        rows,
    })
}

/// The output columns of a `flat_map` body, derived statically so they are known
/// even when the input holds no rows.  Mirrors the checker's
/// `row_collection` on names; domains are carried where a field copies a
/// column, and dropped (`None`) where it computes.
fn flat_map_row_schema(
    input: &SourceTable,
    kname: &str,
    rname: &str,
    body: &Expr,
) -> Result<Option<Vec<Col>>, EvalError> {
    match &body.kind {
        ExprKind::Tuple(items) => {
            let mut schema: Option<Vec<Col>> = None;
            for item in items {
                let s = flat_map_single_row(input, kname, rname, item)?;
                schema = Some(match schema {
                    None => s,
                    Some(prev) => unify_cols(prev, s)?,
                });
            }
            Ok(schema)
        }
        ExprKind::If { then, els, .. } => {
            let a = flat_map_row_schema(input, kname, rname, then)?;
            let b = flat_map_row_schema(input, kname, rname, els)?;
            Ok(match (a, b) {
                (Some(a), Some(b)) => Some(unify_cols(a, b)?),
                (a, b) => a.or(b),
            })
        }
        _ => Ok(Some(flat_map_single_row(input, kname, rname, body)?)),
    }
}

fn flat_map_single_row(
    input: &SourceTable,
    kname: &str,
    rname: &str,
    expr: &Expr,
) -> Result<Vec<Col>, EvalError> {
    match &expr.kind {
        ExprKind::Record(fields) => Ok(fields
            .iter()
            .map(|f| {
                col(
                    f.name.name.clone(),
                    static_domain(input, kname, rname, &f.value),
                )
            })
            .collect()),
        // A whole-row body: the checker reads the row record's fields off a
        // sorted map, so the output columns are alphabetical.
        ExprKind::Name(n) if n == rname => Ok(sorted_cols(&input.attrs)),
        ExprKind::Name(n) if n == kname => Ok(sorted_cols(&input.key)),
        ExprKind::If { then, els, .. } => {
            let a = flat_map_single_row(input, kname, rname, then)?;
            let b = flat_map_single_row(input, kname, rname, els)?;
            unify_cols(a, b)
        }
        _ => internal("a `flat_map` row form the checker should have rejected"),
    }
}

fn sorted_cols(cols: &[Col]) -> Vec<Col> {
    let mut out: Vec<Col> = cols.to_vec();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Unify two column lists: the names must agree (the checker enforced it);
/// a domain survives only where both sides agree on it.
fn unify_cols(a: Vec<Col>, b: Vec<Col>) -> Result<Vec<Col>, EvalError> {
    if a.len() != b.len() || a.iter().zip(&b).any(|(x, y)| x.name != y.name) {
        return internal("`flat_map` rows with differing schemas");
    }
    Ok(a.into_iter()
        .zip(b)
        .map(|(x, y)| {
            let ty = if x.ty == y.ty { x.ty } else { None };
            Col { name: x.name, ty }
        })
        .collect())
}

/// The statically known domain of a scalar expression: a copied column keeps
/// its domain, a literal has its own, and anything computed is `None` (only
/// a later `pivot` would care).
fn static_domain(input: &SourceTable, kname: &str, rname: &str, expr: &Expr) -> Option<ColumnType> {
    match &expr.kind {
        ExprKind::Int(_) => Some(ColumnType::Int),
        ExprKind::Float(_) => Some(ColumnType::Real),
        ExprKind::Str(_) => Some(ColumnType::String),
        ExprKind::Bool(_) => Some(ColumnType::Bool),
        ExprKind::Member(base, field) => match &base.kind {
            ExprKind::Name(n) if n == rname => input
                .attrs
                .iter()
                .find(|c| c.name == field.name)
                .and_then(|c| c.ty.clone()),
            ExprKind::Name(n) if n == kname => input
                .key
                .iter()
                .find(|c| c.name == field.name)
                .and_then(|c| c.ty.clone()),
            _ => None,
        },
        ExprKind::If { then, els, .. } => {
            let a = static_domain(input, kname, rname, then);
            let b = static_domain(input, kname, rname, els);
            if a == b { a } else { None }
        }
        _ => None,
    }
}

/// Evaluate a `flat_map` body as a collection of value rows (ADR 0015): `( )` is
/// empty, `(a, b)` expands, an `if` filters or branches, and any other body
/// is a single row.
fn eval_rows(scope: &Scope, body: &Expr) -> Result<Vec<NamedRow>, EvalError> {
    match &body.kind {
        ExprKind::Tuple(items) => {
            let mut rows = Vec::with_capacity(items.len());
            for item in items {
                rows.push(eval_row(scope, item)?);
            }
            Ok(rows)
        }
        ExprKind::If { cond, then, els } => {
            if eval_bool(scope, cond)? {
                eval_rows(scope, then)
            } else {
                eval_rows(scope, els)
            }
        }
        _ => Ok(vec![eval_row(scope, body)?]),
    }
}

/// Evaluate one value row: a record literal in field order, or a record
/// value (for example the row parameter `r`) in its own field order.
fn eval_row(scope: &Scope, expr: &Expr) -> Result<NamedRow, EvalError> {
    match &expr.kind {
        ExprKind::Record(fields) => {
            let mut row = Vec::with_capacity(fields.len());
            for field in fields {
                let value = eval_value(scope, &field.value)?;
                row.push((field.name.name.clone(), value));
            }
            Ok(row)
        }
        _ => match eval_scalar(scope, expr)? {
            RtVal::Rec(fields) => {
                // A unit-reference group forwarded whole flattens back to
                // its dotted columns (ADR 0032).
                let mut row = Vec::new();
                flatten_rec("", fields, &mut row)?;
                Ok(row)
            }
            _ => internal("a `flat_map` body row is not a record"),
        },
    }
}

// ---------------------------------------------------------------------------
// map_bags

/// `map_bags |k, b| record` (section 6.2): group rows by key and transform
/// each group.  All single-valued fields: one aggregate row per key.  All
/// bag-valued fields: one window row per input row of the group.
fn eval_map_bags(input: SourceTable, args: &[&Expr]) -> Result<SourceTable, EvalError> {
    let (params, body) = lambda_parts(args, 2)?;
    let ExprKind::Record(fields) = &body.kind else {
        return internal("`map_bags` body is not a record");
    };
    let attrs: Vec<Col> = fields
        .iter()
        .map(|f| {
            col(
                f.name.name.clone(),
                bag_static_domain(&input, params[1], &f.value),
            )
        })
        .collect();

    // Group row positions by key, ordered by key value.
    let nkeys = input.key_len();
    let mut groups: BTreeMap<Vec<KeyVal>, Vec<usize>> = BTreeMap::new();
    for (i, row) in input.rows.iter().enumerate() {
        let key: Vec<KeyVal> = row[..nkeys].iter().map(key_of).collect::<Result<_, _>>()?;
        groups.entry(key).or_default().push(i);
    }

    let mut rows = Vec::new();
    for members in groups.values() {
        let key = input.rows[members[0]][..nkeys].to_vec();
        let mut scope = Scope::new();
        if params[0] != "_" {
            scope.insert(params[0].to_string(), record(&input.key, &key));
        }
        if params[1] != "_" {
            // `b` *types* as the fiber, a bag of rows (ADR 0031, Decision 1),
            // but the executor is free to keep the group columnar: the rows
            // type is a type-level notion, and the only thing the surface can
            // do with `b` is project (`b.x`) or count (`#b`), both of which a
            // column-major group answers directly.  Building the bags here
            // materializes exactly those projections, so the two
            // representations are observationally identical.  A row-major
            // representation becomes necessary only when a *row-mapper* fold
            // lands, since its lambda sees one whole row at a time.
            let mut bags: BTreeMap<String, RtVal> = BTreeMap::new();
            for (a, c) in input.attrs.iter().enumerate() {
                let column: Vec<Value> = members
                    .iter()
                    .map(|&i| input.rows[i][nkeys + a].clone())
                    .collect();
                // A unit-reference group nests here too (ADR 0032), so
                // `b.course.name` stays a projection.
                nest_insert(&mut bags, &c.name, RtVal::Bag(column));
            }
            scope.insert(params[1].to_string(), RtVal::Rec(bags));
        }

        let mut aggregates: Vec<Value> = Vec::new();
        let mut windows: Vec<Vec<Value>> = Vec::new();
        for field in fields {
            match eval_scalar(&scope, &field.value)? {
                RtVal::V(v) => aggregates.push(v),
                RtVal::Bag(vs) => windows.push(vs),
                RtVal::Rec(_) => return internal("a `map_bags` field yielded a row"),
            }
        }
        match (aggregates.is_empty(), windows.is_empty()) {
            // All aggregates: one row per key.
            (false, true) => {
                let mut out = key;
                out.extend(aggregates);
                rows.push(out);
            }
            // All windows: one row per group member, zipped element-wise.
            (true, false) => {
                if windows.iter().any(|w| w.len() != members.len()) {
                    return internal("a window bag lost its group's length");
                }
                for i in 0..members.len() {
                    let mut out = key.clone();
                    out.extend(windows.iter().map(|w| w[i].clone()));
                    rows.push(out);
                }
            }
            _ => return internal("a mixed `map_bags` record (the checker rejects this)"),
        }
    }
    Ok(SourceTable {
        key: input.key,
        attrs,
        rows,
    })
}

/// The statically known domain of a `map_bags` field, used to name the output
/// column when no row is available to infer it from.  A window copies its
/// column; `#` is always an `int`; `to_real` is always a `real`.
///
/// The six aggregates used to have entries here.  They are `bag` module
/// bindings now (ADR 0031, Decision 8), so by the time this runs they have
/// already beta-reduced to a `fold` spine, whose domain depends on the
/// mapper and is left to the data.
fn bag_static_domain(input: &SourceTable, bname: &str, expr: &Expr) -> Option<ColumnType> {
    let member_domain = |e: &Expr| match &e.kind {
        ExprKind::Member(base, field) => match &base.kind {
            ExprKind::Name(n) if n == bname => input
                .attrs
                .iter()
                .find(|c| c.name == field.name)
                .and_then(|c| c.ty.clone()),
            _ => None,
        },
        _ => None,
    };
    let applied = |head: &Expr| match &head.kind {
        ExprKind::Name(name) if name == "to_real" => Some(ColumnType::Real),
        _ => None,
    };
    match &expr.kind {
        ExprKind::Member(..) => member_domain(expr),
        // `#` counts, so its column is an `int` whatever it counts.
        ExprKind::Unary(UnOp::Card, _) => Some(ColumnType::Int),
        ExprKind::App(..) => {
            let (head, args) = flatten_app(expr);
            match args[..] {
                [_] => applied(head),
                _ => None,
            }
        }
        ExprKind::Binary(BinOp::Pipe, _, rhs) => applied(rhs),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// promote

/// `promote cols` (section 6.3): promote the named attribute columns into
/// the key, in argument order.
fn eval_promote(mut table: SourceTable, args: &[&Expr]) -> Result<SourceTable, EvalError> {
    for arg in args {
        let ExprKind::Name(name) = &arg.kind else {
            return internal("`promote` expects column names");
        };
        let Some(pos) = table.attr_position(name) else {
            return internal(format!("`promote` on unknown column `{name}`"));
        };
        let nkeys = table.key_len();
        let column = table.attrs.remove(pos);
        table.key.push(column);
        for row in &mut table.rows {
            let v = row.remove(nkeys + pos);
            row.insert(nkeys, v);
        }
    }
    Ok(table)
}

/// `demote cols` (section 6.3): drop the named key columns into the
/// attribute part.  Dropped columns keep their relative key order and
/// re-enter at the end of the attribute list, mirroring the checker's
/// `op_demote` (ADR 0024 section 3).  Its obligations (completeness
/// propagation, the lineage drop that makes it Tier B) are compile-time
/// facts; only the rekeying happens here.
fn eval_demote(mut table: SourceTable, args: &[&Expr]) -> Result<SourceTable, EvalError> {
    let mut names = Vec::new();
    for arg in args {
        let ExprKind::Name(name) = &arg.kind else {
            return internal("`demote` expects column names");
        };
        names.push(name.as_str());
    }
    let dropped: Vec<usize> = table
        .key
        .iter()
        .enumerate()
        .filter(|(_, c)| names.contains(&c.name.as_str()))
        .map(|(i, _)| i)
        .collect();
    if dropped.len() != names.len() {
        return internal("`demote` on a column outside the key");
    }
    // Walk the dropped positions right to left so earlier removals do not
    // shift later ones; each column and value still lands in key order
    // because insertion happens at the front of the already-demoted block.
    for (offset, &pos) in dropped.iter().rev().enumerate() {
        let column = table.key.remove(pos);
        table.attrs.insert(table.attrs.len() - offset, column);
        for row in &mut table.rows {
            let v = row.remove(pos);
            row.insert(row.len() - offset, v);
        }
    }
    Ok(table)
}

// ---------------------------------------------------------------------------
// window
//
// `window w p size stride` (ADR 0037 decision 1): replicate each row into
// every window that contains its point, adding the window's start as a
// fresh key column.  Specified as a replicating `flat_map` followed by
// `promote w`, but evaluated natively, because `eval_flat_map` fixes the
// key and this operation extends it.

/// The window starts containing `point`, in the point's storage grain: the
/// multiples of `stride` in the half-open interval `(point - size, point]`.
///
/// The grid is anchored at the domain's zero (the epoch for an `instant`,
/// ADR 0036 decision 5), so placement is deterministic with no declaration
/// and no data dependence.  Arithmetic is `div_euclid`, not `/`: a
/// pre-epoch point is negative, and truncation toward zero would shift its
/// grid by a whole stride.  This mirrors `Mensura.Units.Instant.windowStarts`
/// in `formal/Mensura/Window/Defs.lean`, whose `mem_windowStarts` proves the
/// interval test `w <= p < w + size`.
fn window_starts(point: i64, size: i64, stride: i64) -> Vec<i64> {
    let last = point.div_euclid(stride);
    let first = (point - size).div_euclid(stride) + 1;
    (first..=last).map(|n| n * stride).collect()
}

fn eval_window(input: SourceTable, args: &[&Expr]) -> Result<SourceTable, EvalError> {
    let [w_arg, p_arg, size_arg, stride_arg] = args else {
        return internal("`window` expects a window column, a point, a size, and a stride");
    };
    let (ExprKind::Name(w), ExprKind::Name(p)) = (&w_arg.kind, &p_arg.kind) else {
        return internal("`window` expects column names");
    };

    // The point may be a key column or an attribute: ADR 0037 decision 2
    // leaves it where it was.
    let nkeys = input.key.len();
    let (point_at, point_ty) = match input.key.iter().position(|c| &c.name == p) {
        Some(i) => (i, input.key[i].ty.clone()),
        None => match input.attrs.iter().position(|c| &c.name == p) {
            Some(i) => (nkeys + i, input.attrs[i].ty.clone()),
            None => return internal("`window` on a column the checker did not find"),
        },
    };

    // The extents are const expressions the checker has already validated;
    // lowering substituted their const names, so they evaluate in an empty
    // scope.  Both sides convert through `whole_milliseconds`, the one
    // exact-or-error predicate (ADR 0036 decision 6).
    let extent = |arg: &Expr, what: &str| -> Result<i64, EvalError> {
        match eval_scalar(&Scope::new(), arg)? {
            RtVal::V(Value::Real(seconds)) => {
                temporal::whole_milliseconds(seconds).map_err(|message| EvalError { message })
            }
            RtVal::V(Value::Int(n)) => Ok(n),
            _ => internal(format!("`window`'s {what} is not a const extent")),
        }
    };
    let size = extent(size_arg, "size")?;
    let stride = extent(stride_arg, "stride")?;
    if stride <= 0 || size <= 0 {
        return internal("`window` extents must be positive");
    }

    // The point in its storage grain, and the inverse for the emitted key.
    let to_grain = |v: &Value| -> Result<i64, EvalError> {
        match v {
            Value::Instant(s) => {
                temporal::instant_to_ms(s).map_err(|message| EvalError { message })
            }
            Value::Int(n) => Ok(*n),
            _ => internal("`window` on a value that is not a point"),
        }
    };
    let from_grain = |g: i64| -> Result<Value, EvalError> {
        match point_ty {
            Some(ColumnType::Instant) => temporal::ms_to_instant(g)
                .map(Value::Instant)
                .map_err(|message| EvalError { message }),
            _ => Ok(Value::Int(g)),
        }
    };

    let mut rows = Vec::new();
    for row in &input.rows {
        let point = to_grain(&row[point_at])?;
        for start in window_starts(point, size, stride) {
            // The key grows by one column, so the window start is spliced
            // in at the end of the key block and the attributes follow.
            let mut out: Row = row[..nkeys].to_vec();
            out.push(from_grain(start)?);
            out.extend_from_slice(&row[nkeys..]);
            rows.push(out);
        }
    }
    let mut key = input.key;
    key.push(col(w.clone(), point_ty));
    Ok(SourceTable {
        key,
        attrs: input.attrs,
        rows,
    })
}

// ---------------------------------------------------------------------------
// split / union

/// `split |k| pred` (section 6.5): route each row by a predicate over its
/// key.  `true` goes left, `false` right; both sides keep the schema.
fn eval_split(input: SourceTable, args: &[&Expr]) -> Result<TableVal, EvalError> {
    let (params, body) = lambda_parts(args, 1)?;
    let nkeys = input.key_len();
    let mut left_rows = Vec::new();
    let mut right_rows = Vec::new();
    for row in &input.rows {
        let mut scope = Scope::new();
        if params[0] != "_" {
            scope.insert(params[0].to_string(), record(&input.key, &row[..nkeys]));
        }
        if eval_bool(&scope, body)? {
            left_rows.push(row.clone());
        } else {
            right_rows.push(row.clone());
        }
    }
    let left = SourceTable {
        key: input.key.clone(),
        attrs: input.attrs.clone(),
        rows: left_rows,
    };
    let right = SourceTable {
        key: input.key,
        attrs: input.attrs,
        rows: right_rows,
    };
    Ok(TableVal::Pair(left, right))
}

/// `union` (section 6.5): the union of a pair of tables of the same schema,
/// left side first.
fn eval_bind(input: TableVal) -> Result<SourceTable, EvalError> {
    let TableVal::Pair(a, b) = input else {
        return internal("`union` expects a pair of tables");
    };
    let key = unify_cols(a.key, b.key)?;
    let attrs = unify_cols(a.attrs, b.attrs)?;
    let mut rows = a.rows;
    rows.extend(b.rows);
    Ok(SourceTable { key, attrs, rows })
}

// ---------------------------------------------------------------------------
// joins

#[derive(Clone, Copy)]
enum JoinKind {
    Left,
    Inner,
}

/// `lookup` / `lookup_total right (|k, r| key)` (section 6.4): join a fixed
/// right table by a key computed over the left row.  The right table is a
/// store (keyed, single key column), so at most one row matches.
fn eval_join(
    env: &BTreeMap<String, TableVal>,
    input: SourceTable,
    args: &[&Expr],
    kind: JoinKind,
) -> Result<SourceTable, EvalError> {
    let [right_arg, key_arg] = args else {
        return internal("a join expects a right table and a key lambda");
    };
    let ExprKind::Name(right_name) = &right_arg.kind else {
        return internal("a join's right side must be a source name");
    };
    let Some(TableVal::Table(right)) = env.get(right_name) else {
        return internal(format!("unknown join target `{right_name}`"));
    };
    if right.key_len() != 1 {
        return internal("a join's right table must have a single key column");
    }
    let mut by_key: BTreeMap<KeyVal, &[Value]> = BTreeMap::new();
    for row in &right.rows {
        if by_key.insert(key_of(&row[0])?, &row[1..]).is_some() {
            return internal(format!("join target `{right_name}` is not keyed"));
        }
    }

    let (params, key_body) = lambda_parts(&[key_arg], 2)?;
    let mut rows = Vec::new();
    for row in &input.rows {
        let scope = row_scope(&input, params[0], params[1], row);
        let key = key_of(&eval_value(&scope, key_body)?)?;
        match (by_key.get(&key), kind) {
            (Some(right_vals), _) => {
                let mut out = row.clone();
                out.extend(right_vals.iter().cloned());
                rows.push(out);
            }
            (None, JoinKind::Left) => {
                let mut out = row.clone();
                out.extend(std::iter::repeat_n(Value::Missing, right.attrs.len()));
                rows.push(out);
            }
            (None, JoinKind::Inner) => {}
        }
    }
    let mut attrs = input.attrs;
    attrs.extend(right.attrs.iter().cloned());
    Ok(SourceTable {
        key: input.key,
        attrs,
        rows,
    })
}

// ---------------------------------------------------------------------------
// unpivot / pivot

/// `unpivot name value` (section 6.6, ADR 0020): fold **all** attribute
/// columns into one `value` column, spreading their names into a new `enum`
/// key column `name`.  A missing cell yields **no row** (drop semantics),
/// so the value column is total by construction (`unpivotDrop` in
/// `formal/Mensura/Table.lean`).
fn eval_unpivot(input: SourceTable, args: &[&Expr]) -> Result<SourceTable, EvalError> {
    let [name_arg, value_arg] = args else {
        return internal("`unpivot` takes a name column and a value column");
    };
    let (ExprKind::Name(name_col), ExprKind::Name(value_col)) = (&name_arg.kind, &value_arg.kind)
    else {
        return internal("`unpivot`'s name and value must be identifiers");
    };
    // The folded columns share one domain (the checker's gate); the value
    // column keeps it where every column still carries it.
    let mut value_ty: Option<ColumnType> = None;
    for (i, c) in input.attrs.iter().enumerate() {
        value_ty = if i == 0 || value_ty == c.ty {
            c.ty.clone()
        } else {
            None
        };
    }

    let variants: Vec<String> = input.attrs.iter().map(|c| c.name.clone()).collect();
    let mut key = input.key.clone();
    key.push(col(
        name_col.clone(),
        Some(ColumnType::Enum {
            name: name_col.clone(),
            variants: variants.clone(),
        }),
    ));
    let attrs = vec![col(value_col.clone(), value_ty)];

    let nkeys = input.key_len();
    let mut rows = Vec::new();
    for row in &input.rows {
        for (fpos, fname) in variants.iter().enumerate() {
            let cell = &row[nkeys + fpos];
            if *cell == Value::Missing {
                continue;
            }
            let mut out: Row = row[..nkeys].to_vec();
            out.push(Value::Enum(fname.clone()));
            out.push(cell.clone());
            rows.push(out);
        }
    }
    Ok(SourceTable { key, attrs, rows })
}

/// `pivot name value` (section 6.6, ADR 0020): the inverse of `unpivot`.
/// `name` is an enum key column and `value` the only attribute (the
/// checker's gates); each residual key's fiber gathers into one wide row,
/// one column per variant, and an absent (key, variant) row becomes a
/// missing cell.  Residual keys come out in key order, like `map_bags`'s
/// groups.
fn eval_pivot(input: SourceTable, args: &[&Expr]) -> Result<SourceTable, EvalError> {
    let [name_arg, value_arg] = args else {
        return internal("`pivot` takes a name column and a value column");
    };
    let (ExprKind::Name(name_col), ExprKind::Name(value_col)) = (&name_arg.kind, &value_arg.kind)
    else {
        return internal("`pivot`'s name and value must be identifiers");
    };
    let Some(name_pos) = input.key.iter().position(|c| &c.name == name_col) else {
        return internal(format!("`pivot` on non-key column `{name_col}`"));
    };
    let Some(ColumnType::Enum { variants, .. }) = input.key[name_pos].ty.clone() else {
        return err(format!(
            "`pivot` cannot recover `{name_col}`'s enum variants: an upstream \
             stage computed the column, losing its declared enum; not yet \
             executable (docs/toolkit/04-processing-layer.md)"
        ));
    };
    if input.attrs.len() != 1 || &input.attrs[0].name != value_col {
        return internal("`pivot`'s value must be the only attribute column");
    }
    let value_ty = input.attrs[0].ty.clone();

    let key: Vec<Col> = input
        .key
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != name_pos)
        .map(|(_, c)| c.clone())
        .collect();
    let attrs: Vec<Col> = variants
        .iter()
        .map(|v| col(v.clone(), value_ty.clone()))
        .collect();

    // Gather each residual key's fiber: one slot per variant, missing where
    // the (key, variant) row is absent.  The `singletons` gate makes each
    // slot single-valued; a second write is a frontend/runtime disagreement.
    let nkeys = input.key_len();
    let mut groups: BTreeMap<Vec<KeyVal>, (Row, Vec<Value>)> = BTreeMap::new();
    for row in &input.rows {
        let residual: Vec<&Value> = row[..nkeys]
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != name_pos)
            .map(|(_, v)| v)
            .collect();
        let key: Vec<KeyVal> = residual
            .iter()
            .map(|v| key_of(v))
            .collect::<Result<_, _>>()?;
        let variant_text = match &row[name_pos] {
            Value::Enum(s) | Value::String(s) => s.clone(),
            _ => return internal("`pivot`'s name cell is not an enum value"),
        };
        let Some(slot) = variants.iter().position(|v| *v == variant_text) else {
            return internal(format!(
                "`pivot` met a value outside `{name_col}`'s declared enum"
            ));
        };
        let entry = groups.entry(key).or_insert_with(|| {
            (
                residual.iter().map(|&v| v.clone()).collect(),
                vec![Value::Missing; variants.len()],
            )
        });
        if entry.1[slot] != Value::Missing {
            return internal("`pivot` on a non-singletons input");
        }
        entry.1[slot] = row[nkeys].clone();
    }

    let rows: Vec<Row> = groups
        .into_values()
        .map(|(mut key, slots)| {
            key.extend(slots);
            key
        })
        .collect();
    Ok(SourceTable { key, attrs, rows })
}

// ---------------------------------------------------------------------------
// scalar expressions

/// Evaluate a scalar expression to a single [`Value`].
fn eval_value(scope: &Scope, expr: &Expr) -> Result<Value, EvalError> {
    match eval_scalar(scope, expr)? {
        RtVal::V(v) => Ok(v),
        RtVal::Bag(_) => internal("expected a single value, found a bag"),
        RtVal::Rec(_) => internal("expected a single value, found a row"),
    }
}

/// Evaluate a boolean expression (an `if` condition, a predicate).
fn eval_bool(scope: &Scope, expr: &Expr) -> Result<bool, EvalError> {
    match eval_value(scope, expr)? {
        Value::Bool(b) => Ok(b),
        _ => internal("condition did not evaluate to a boolean"),
    }
}

/// Evaluate a scalar expression (`docs/language/06-expressions.md`) over the
/// lambda scope.  The checker has enforced the domain rules, so each
/// operator is implemented only on the variants it can meet.
fn eval_scalar(scope: &Scope, expr: &Expr) -> Result<RtVal, EvalError> {
    match &expr.kind {
        ExprKind::Int(i) => Ok(RtVal::V(Value::Int(*i))),
        ExprKind::Float(f) => Ok(RtVal::V(Value::Real(*f))),
        ExprKind::Str(s) => Ok(RtVal::V(Value::String(s.clone()))),
        ExprKind::Bool(b) => Ok(RtVal::V(Value::Bool(*b))),
        ExprKind::Name(name) => match scope.get(name) {
            Some(v) => Ok(v.clone()),
            None => internal(format!("unknown name `{name}`")),
        },
        ExprKind::Member(base, field) => match eval_scalar(scope, base)? {
            RtVal::Rec(fields) => match fields.get(&field.name) {
                Some(v) => Ok(v.clone()),
                None => internal(format!("unknown column `{}`", field.name)),
            },
            _ => internal("member access on a non-record value"),
        },
        ExprKind::Unary(op, operand) => eval_unary(scope, *op, operand),
        ExprKind::Binary(op, lhs, rhs) => eval_binary(scope, *op, lhs, rhs),
        ExprKind::Presence(base, presence) => {
            let missing = matches!(eval_value(scope, base)?, Value::Missing);
            Ok(RtVal::V(Value::Bool(match presence {
                Presence::Known => !missing,
                Presence::Missing => missing,
            })))
        }
        ExprKind::If { cond, then, els } => {
            if eval_bool(scope, cond)? {
                eval_scalar(scope, then)
            } else {
                eval_scalar(scope, els)
            }
        }
        ExprKind::App(..) => {
            let (head, args) = flatten_app(expr);
            apply_value_fn(scope, head, &args)
        }
        _ => internal("expression form the checker should have rejected"),
    }
}

/// The elements a mapper is applied over, one binding per element.
///
/// Over a projected bag the element is a value; over the fiber itself it is a
/// *row* (ADR 0031, Decisions 1 and 4).  The group is stored columnar, so the
/// row case transposes here: this is the one place the two representations
/// actually differ, and it is why `map_bags` records where a row-major
/// executor would matter.
fn mapper_elements(scope: &Scope, bag: &Expr) -> Result<Vec<RtVal>, EvalError> {
    match eval_scalar(scope, bag)? {
        RtVal::Bag(vs) => Ok(vs.into_iter().map(RtVal::V).collect()),
        RtVal::Rec(fields) => {
            // Every column of a group has the same length (they are the same
            // rows), so any one gives the row count.
            let n = group_len(&fields).unwrap_or(0);
            let mut rows = Vec::with_capacity(n);
            for i in 0..n {
                rows.push(group_row(&fields, i)?);
            }
            Ok(rows)
        }
        RtVal::V(_) => internal("a mapper applied to a value the checker should have rejected"),
    }
}

/// The row count of a fiber's columnar record: any column's bag length,
/// searched through nested unit-reference groups (ADR 0032).  `None` for a
/// record with no bag column at any depth.
fn group_len(fields: &BTreeMap<String, RtVal>) -> Option<usize> {
    fields.values().find_map(|v| match v {
        RtVal::Bag(vs) => Some(vs.len()),
        RtVal::Rec(sub) => group_len(sub),
        RtVal::V(_) => None,
    })
}

/// The `i`-th row of a fiber's columnar record, following nested
/// unit-reference groups (ADR 0032): each bag contributes its `i`-th value,
/// each group recurses.
fn group_row(fields: &BTreeMap<String, RtVal>, i: usize) -> Result<RtVal, EvalError> {
    let mut row = BTreeMap::new();
    for (name, col) in fields {
        match col {
            RtVal::Bag(vs) => {
                let Some(v) = vs.get(i) else {
                    return internal("ragged group columns");
                };
                row.insert(name.clone(), RtVal::V(v.clone()));
            }
            RtVal::Rec(sub) => {
                row.insert(name.clone(), group_row(sub, i)?);
            }
            RtVal::V(_) => return internal("a group column that is not a bag"),
        }
    }
    Ok(RtVal::Rec(row))
}

/// Apply a mapper to each element, yielding the mapped values.
fn eval_mapped(scope: &Scope, mapper: &Expr, bag: &Expr) -> Result<Vec<Value>, EvalError> {
    let ExprKind::Lambda { params, body, .. } = &mapper.kind else {
        return internal("a mapper that is not a lambda");
    };
    let [param] = &params[..] else {
        return internal("a mapper that does not take exactly one element");
    };
    let mut out = Vec::new();
    for element in mapper_elements(scope, bag)? {
        let mut inner = scope.clone();
        inner.insert(param.name.clone(), element);
        match eval_scalar(&inner, body)? {
            RtVal::V(v) => out.push(v),
            _ => return internal("a mapper that did not produce a value"),
        }
    }
    Ok(out)
}

/// `fold` (ADR 0031, Decision 4): map each element, then combine with the
/// closed table's operator.  Backed by `formal/Mensura/Fold.lean`: the
/// combiner is associative and commutative, so the bag's arbitrary order does
/// not matter, and the identity lives in the accumulator rather than in a
/// user-supplied seed.
fn eval_fold(
    scope: &Scope,
    combiner: &Expr,
    mapper: &Expr,
    bag: &Expr,
) -> Result<RtVal, EvalError> {
    let ExprKind::Combiner(raw) = &combiner.kind else {
        return internal("a `fold` combiner that is not a backticked operator");
    };
    let mapped = eval_mapped(scope, mapper, bag)?;
    let Some((first, rest)) = mapped.split_first() else {
        // The empty bag.  A group arises from rows and is never empty, so the
        // checker's total result type holds; reaching here means the invariant
        // broke rather than that a combiner identity is missing.
        return internal("`fold` over an empty bag");
    };
    let mut acc = first.clone();
    for v in rest {
        acc = combine(raw, acc, v.clone())?;
    }
    Ok(RtVal::V(acc))
}

/// One step of a fold, dispatched on the combiner's surface spelling.  The
/// table is closed (ADR 0031, Decision 6) and the checker has already vetted
/// both the operator and the domain.
fn combine(op: &str, a: Value, b: Value) -> Result<Value, EvalError> {
    match op {
        "+" => arithmetic(BinOp::Add, a, b),
        "*" => arithmetic(BinOp::Mul, a, b),
        "<<" | ">>" => {
            let ord = compare(&a, &b)?;
            let take_left = if op == "<<" { ord.is_le() } else { ord.is_ge() };
            Ok(if take_left { a } else { b })
        }
        "or" | "and" => match (a, b) {
            (Value::Bool(x), Value::Bool(y)) => {
                Ok(Value::Bool(if op == "or" { x || y } else { x && y }))
            }
            _ => internal("a boolean combiner on non-booleans"),
        },
        // The tacks: keep-left discards each later operand, keep-right each
        // earlier one.  Associative but not commutative, so `fold` refuses
        // them and only a scan (whose key supplies an order) admits them.  The
        // const evaluator's `arith` has the same two arms.
        "<:" => Ok(a),
        ":>" => Ok(b),
        other => internal(format!("`{other}` is not a combiner")),
    }
}

/// `scan` and `prescan` (ADR 0031, Decision 7): arrange the fiber by the key,
/// scan the mapped values along that order, and return one value per input row
/// **in input order**.
///
/// Backed by `formal/Mensura/Arranged.lean`: `IsArrangement` (existence needs
/// no hypothesis, uniqueness is Tier 1), `scanBag`/`prescanBag` as the `tail`
/// and `dropLast` of one `List.scanl`, and `scanFiber_splitSafe`.
///
/// The un-permutation at the end is not incidental.  `eval_map_bags` zips a
/// window field element-wise against the group's members in input order, so a
/// scan that returned its values in *key* order would silently attach each
/// value to the wrong row.  Nothing in the type system catches that, which is
/// why the scatter is written explicitly rather than left to a sort's output.
///
/// Ties resolve to input order, because the sort is stable.  That is
/// deterministic but it is *not* a determinism the type system licenses: Tier 1
/// (`Mensura.IsArrangement.unique`) is what would license it, the checker
/// cannot verify key injectivity on data, and ADR 0031 leaves Tier 3's escape
/// hatch unattached.  So the honest position is a reproducible answer plus this
/// note, rather than an arbitrary order or a rejection.
fn eval_scan(
    scope: &Scope,
    which: &str,
    combiner: &Expr,
    mapper: &Expr,
    key: &Expr,
    bag: &Expr,
) -> Result<RtVal, EvalError> {
    let ExprKind::Combiner(raw) = &combiner.kind else {
        return internal("a scan combiner that is not a backticked operator");
    };
    let exclusive = which == "prescan";
    // Both lambdas read the same rows, so the fiber is transposed once.
    let elements = mapper_elements(scope, bag)?;
    let values = apply_row_fn(scope, mapper, &elements, "a scan mapper")?;
    let keys = apply_row_fn(scope, key, &elements, "a scan order key")?;
    let n = values.len();
    if keys.len() != n {
        return internal("a scan's mapper and key disagree on the row count");
    }
    // Arrange: the permutation that sorts the rows by the key.  Stable, so
    // ties keep input order (see the note above).
    let descending = key_is_descending(key);
    let mut perm: Vec<usize> = (0..n).collect();
    let mut cmp_err = None;
    perm.sort_by(|&i, &j| match compare(&keys[i], &keys[j]) {
        Ok(ord) => {
            if descending {
                ord.reverse()
            } else {
                ord
            }
        }
        Err(e) => {
            cmp_err.get_or_insert(e);
            std::cmp::Ordering::Equal
        }
    });
    if let Some(e) = cmp_err {
        return Err(e);
    }
    // Scan along the arrangement, scattering each result back to its row.
    let mut out = vec![Value::Missing; n];
    let mut acc: Option<Value> = None;
    for (rank, &row) in perm.iter().enumerate() {
        if exclusive {
            // The proper prefix `1..i-1`.  At rank 0 that prefix is empty, so
            // the answer is the combiner's identity, or missing where the
            // domain has none: this is where `lag`'s first row comes from.
            out[row] = match &acc {
                Some(v) => v.clone(),
                None => identity_of(raw, &values[row])?,
            };
            acc = Some(match acc {
                Some(a) => combine(raw, a, values[row].clone())?,
                None => values[row].clone(),
            });
        } else {
            let next = match acc {
                Some(a) => combine(raw, a, values[row].clone())?,
                None => values[row].clone(),
            };
            out[row] = next.clone();
            acc = Some(next);
        }
        let _ = rank;
    }
    Ok(RtVal::Bag(out))
}

/// Apply a one-parameter lambda to each row of a transposed fiber.
fn apply_row_fn(
    scope: &Scope,
    f: &Expr,
    elements: &[RtVal],
    what: &str,
) -> Result<Vec<Value>, EvalError> {
    let ExprKind::Lambda { params, body, .. } = &f.kind else {
        return internal(format!("{what} that is not a lambda"));
    };
    let [param] = &params[..] else {
        return internal(format!("{what} that does not take exactly one row"));
    };
    let mut out = Vec::with_capacity(elements.len());
    for element in elements {
        let mut inner = scope.clone();
        inner.insert(param.name.clone(), element.clone());
        match eval_scalar(&inner, body)? {
            RtVal::V(v) => out.push(v),
            _ => return internal(format!("{what} that did not produce a value")),
        }
    }
    Ok(out)
}

/// Whether a key lambda's body is wrapped in `desc`, i.e. orders the dual.
///
/// The marker is read off the *syntax* rather than carried in a `Value`, for
/// the same reason `Ty::Desc` is not a `ColumnType`: direction is a compile-time
/// annotation on a key, and putting it in the runtime value type would let an
/// order marker reach storage.  Lowering preserves the head, since
/// `spine_head_is_function` beta-reduces a literal lambda applied to a row but
/// leaves an unknown head like `desc` in place with its argument reduced.
fn key_is_descending(key: &Expr) -> bool {
    let ExprKind::Lambda { body, .. } = &key.kind else {
        return false;
    };
    let mut cur = body.as_ref();
    while let ExprKind::App(f, _) = &cur.kind {
        if matches!(&f.kind, ExprKind::Name(n) if n == "desc") {
            return true;
        }
        cur = f;
    }
    false
}

/// The combiner's identity at the sample's domain, for `prescan`'s first
/// position (whose prefix is empty).
///
/// Derived from a sample element rather than from a type, because the runtime
/// does not carry a `ColumnType` here; a dimensioned real is just a real, and
/// its identity is `0.0` at any dimension.  An identity-free combiner returns
/// `Value::Missing`, which the checker has already reflected by typing the
/// column optional (`type_scan`'s matrix), so this is not a silent widening.
fn identity_of(op: &str, sample: &Value) -> Result<Value, EvalError> {
    match op {
        "+" => match sample {
            Value::Int(_) => Ok(Value::Int(0)),
            Value::Real(_) => Ok(Value::Real(0.0)),
            _ => internal("`+` over a non-numeric domain"),
        },
        "*" => match sample {
            Value::Int(_) => Ok(Value::Int(1)),
            Value::Real(_) => Ok(Value::Real(1.0)),
            _ => internal("`*` over a non-numeric domain"),
        },
        "or" => Ok(Value::Bool(false)),
        "and" => Ok(Value::Bool(true)),
        // No smallest element of nothing, and no previous element of the
        // first: the honest answer is absence (ADR 0029 Decision 4).
        "<<" | ">>" | "<:" | ":>" => Ok(Value::Missing),
        other => internal(format!("`{other}` is not a combiner")),
    }
}

/// The value-level builtins: the reduction primitives `fold` and `map`, and
/// `to_real` (`x |> op` and `op x` converge here, ADR 0018).  The derived
/// reductions are gone from here: `bag.sum` and friends beta-reduce to a
/// `fold` spine at lowering, so the runtime only ever sees the primitive
/// (ADR 0031, Decision 8).
fn apply_value_fn(scope: &Scope, head: &Expr, args: &[&Expr]) -> Result<RtVal, EvalError> {
    let ExprKind::Name(name) = &head.kind else {
        return internal("application of a non-name");
    };
    // The reduction primitives take more than one argument (ADR 0031,
    // Decision 11); the checker has already saturated them, so an arity
    // mismatch here is internal.
    match name.as_str() {
        "fold" => {
            let [combiner, mapper, bag] = args else {
                return internal("`fold` expects a combiner, a mapper, and a bag");
            };
            return eval_fold(scope, combiner, mapper, bag);
        }
        "scan" | "prescan" => {
            let [combiner, mapper, key, bag] = args else {
                return internal(format!(
                    "`{name}` expects a combiner, a mapper, a key, and a bag"
                ));
            };
            return eval_scan(scope, name, combiner, mapper, key, bag);
        }
        "map" => {
            let [mapper, bag] = args else {
                return internal("`map` expects a mapper and a bag");
            };
            return Ok(RtVal::Bag(eval_mapped(scope, mapper, bag)?));
        }
        _ => {}
    }
    let [arg] = args else {
        return internal(format!("`{name}` expects one argument"));
    };
    // `desc` is transparent at the value level: the direction it marks is read
    // off the key's syntax by `key_is_descending`, so evaluating it yields the
    // underlying value unchanged.  Keeping the marker out of `Value` is what
    // stops an order annotation from reaching storage.
    if name == "desc" {
        return eval_scalar(scope, arg);
    }
    match (name.as_str(), eval_scalar(scope, arg)?) {
        ("to_real", RtVal::V(Value::Int(i))) => Ok(RtVal::V(Value::Real(i as f64))),
        // `to_real` lifts over the missing axis (ADR 0039): absent in,
        // absent out, at the value and at each bag element.
        ("to_real", RtVal::V(Value::Missing)) => Ok(RtVal::V(Value::Missing)),
        ("to_real", RtVal::Bag(vs)) => Ok(RtVal::Bag(
            vs.into_iter()
                .map(|v| match v {
                    Value::Int(i) => Ok(Value::Real(i as f64)),
                    Value::Missing => Ok(Value::Missing),
                    _ => internal("`to_real` on a non-int bag element"),
                })
                .collect::<Result<_, _>>()?,
        )),
        _ => internal(format!("`{name}` applied to an unsupported value")),
    }
}

fn eval_unary(scope: &Scope, op: UnOp, operand: &Expr) -> Result<RtVal, EvalError> {
    // `#` (ADR 0031, Decision 9) consumes the *bag*, so it is handled before
    // `eval_value` collapses one.  `#e == fold `+` (|_| 1) e`: the mapper
    // discards the element, so a missing value still counts its row, and the
    // empty bag counts zero (the additive identity, not a missing result).
    if op == UnOp::Card {
        return match eval_scalar(scope, operand)? {
            RtVal::Bag(vs) => Ok(RtVal::V(Value::Int(vs.len() as i64))),
            // `#b` counts the fiber's rows.  The group is stored columnar, so
            // any column's length is the row count; they agree because
            // projection preserves cardinality.  An attribute-less group has
            // no column to measure, but also no rows to count that the key
            // does not already determine, so zero is the honest answer.
            RtVal::Rec(fields) => {
                let n = group_len(&fields).unwrap_or(0);
                Ok(RtVal::V(Value::Int(n as i64)))
            }
            RtVal::V(_) => internal("`#` applied to a value the checker should have rejected"),
        };
    }
    let v = eval_value(scope, operand)?;
    let out = match (op, v) {
        // Absence absorbs through the unary operators too (ADR 0039).
        (_, Value::Missing) => Value::Missing,
        (UnOp::Not, Value::Bool(b)) => Value::Bool(!b),
        (UnOp::Neg, Value::Int(i)) => Value::Int(match i.checked_neg() {
            Some(n) => n,
            None => return err("integer overflow in negation"),
        }),
        (UnOp::Neg, Value::Real(r)) => Value::Real(-r),
        _ => return internal("unary operator on an unsupported value"),
    };
    Ok(RtVal::V(out))
}

fn eval_binary(scope: &Scope, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<RtVal, EvalError> {
    // `x |> op` is `op x` (ADR 0018).
    if op == BinOp::Pipe {
        let (head, mut args) = flatten_app(rhs);
        args.push(lhs);
        return apply_value_fn(scope, head, &args);
    }

    let a = eval_value(scope, lhs)?;
    let b = eval_value(scope, rhs)?;
    // Absence absorbs (ADR 0039 decision 1): a missing operand makes every
    // lifted operator's result missing.  This is why `and`/`or` evaluate
    // both sides rather than short-circuiting: `false and missing` is
    // missing, not false (three-valued logic was rejected), and evaluating
    // the discarded side is the tacks' precedent anyway.  `??` is the one
    // exception, being the discharge itself.
    if op != BinOp::Coalesce && (matches!(a, Value::Missing) || matches!(b, Value::Missing)) {
        return Ok(RtVal::V(Value::Missing));
    }
    let out = match op {
        BinOp::And | BinOp::Or => match (&a, &b) {
            (Value::Bool(x), Value::Bool(y)) => Value::Bool(match op {
                BinOp::And => *x && *y,
                _ => *x || *y,
            }),
            _ => return internal("a boolean operator on non-boolean values"),
        },
        BinOp::Eq => Value::Bool(values_equal(&a, &b)?),
        BinOp::Ne => Value::Bool(!values_equal(&a, &b)?),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ord = compare(&a, &b)?;
            Value::Bool(match op {
                BinOp::Lt => ord.is_lt(),
                BinOp::Le => ord.is_le(),
                BinOp::Gt => ord.is_gt(),
                _ => ord.is_ge(),
            })
        }
        // `<<`/`>>` (binary minimum and maximum, ADR 0031 Decision 6): the
        // same total order the comparisons use, returning the operand rather
        // than a boolean.  A tie returns the left operand, which is
        // unobservable since the operands are then equal.
        BinOp::Min | BinOp::Max => {
            let ord = compare(&a, &b)?;
            let take_left = match op {
                BinOp::Min => ord.is_le(),
                _ => ord.is_ge(),
            };
            if take_left { a } else { b }
        }
        // The tacks: `a <: b` is `a`, `a :> b` is `b`.  Both operands are
        // evaluated, so a diagnostic in the discarded one still surfaces.
        BinOp::KeepLeft => a,
        BinOp::KeepRight => b,
        // `??` (ADR 0039 decision 2): the present value, or the default.
        // Both operands are evaluated, like the tacks: the default is
        // ordinarily a literal, and a diagnostic in a computed one should
        // surface whether or not this row needed it.
        BinOp::Coalesce => {
            if matches!(a, Value::Missing) {
                b
            } else {
                a
            }
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => arithmetic(op, a, b)?,
        BinOp::In => {
            return err("`in` is not yet executable (docs/toolkit/04-processing-layer.md)");
        }
        BinOp::Pipe => unreachable!("handled above"),
    };
    Ok(RtVal::V(out))
}

/// Equality on equatable domains (ADR 0014).  An enum value compared with a
/// string literal compares the variant text (section 5.6 of the typing
/// reference).
fn values_equal(a: &Value, b: &Value) -> Result<bool, EvalError> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x == y),
        (Value::Bool(x), Value::Bool(y)) => Ok(x == y),
        (Value::String(x), Value::String(y)) => Ok(x == y),
        (Value::Date(x), Value::Date(y)) => Ok(x == y),
        // Sound because an instant is an exact point on the millisecond
        // grid, in one normalized encoding (ADR 0036).
        (Value::Instant(x), Value::Instant(y)) => Ok(x == y),
        (Value::Enum(x), Value::Enum(y)) => Ok(x == y),
        (Value::Enum(x), Value::String(y)) | (Value::String(x), Value::Enum(y)) => Ok(x == y),
        _ => internal("`==` on values the checker should have rejected"),
    }
}

/// Ordering on orderable domains: `int`, `real`, and the temporal points
/// `date` and `instant` (both normalized to fixed-width text, so
/// lexicographic order is chronological order, ADR 0036 decision 7).
fn compare(a: &Value, b: &Value) -> Result<std::cmp::Ordering, EvalError> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
        (Value::Real(x), Value::Real(y)) => match x.partial_cmp(y) {
            Some(ord) => Ok(ord),
            None => err("comparison with a NaN value"),
        },
        (Value::Date(x), Value::Date(y)) => Ok(x.cmp(y)),
        (Value::Instant(x), Value::Instant(y)) => Ok(x.cmp(y)),
        _ => internal("comparison on values the checker should have rejected"),
    }
}

/// Arithmetic on matching numeric domains (no coercion, ADR 0014); `/` is
/// real-only and `^` on ints requires a non-negative exponent.
fn arithmetic(op: BinOp, a: Value, b: Value) -> Result<Value, EvalError> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => {
            let out = match op {
                BinOp::Add => x.checked_add(y),
                BinOp::Sub => x.checked_sub(y),
                BinOp::Mul => x.checked_mul(y),
                BinOp::Pow => match u32::try_from(y) {
                    Ok(exp) => x.checked_pow(exp),
                    Err(_) => return err("`^` on ints requires a non-negative exponent"),
                },
                BinOp::Div => return internal("`/` on ints (it is real-only)"),
                _ => unreachable!(),
            };
            match out {
                Some(v) => Ok(Value::Int(v)),
                None => err("integer overflow in arithmetic"),
            }
        }
        (Value::Real(x), Value::Real(y)) => Ok(Value::Real(match op {
            BinOp::Add => x + y,
            BinOp::Sub => x - y,
            BinOp::Mul => x * y,
            BinOp::Div => x / y,
            BinOp::Pow => x.powf(y),
            _ => unreachable!(),
        })),
        // A dimensioned base raised to an integer-literal exponent
        // (`r.x ^ 2`, ADR 0026): the only place the checker admits a mixed
        // `real`/`int` pair.
        (Value::Real(x), Value::Int(y)) if op == BinOp::Pow => match i32::try_from(y) {
            Ok(exp) => Ok(Value::Real(x.powi(exp))),
            Err(_) => err("`^` exponent out of range"),
        },
        // The torsor difference (ADR 0036 decision 4): `instant - instant`
        // is a `time[real]`, computed as an exact integer millisecond count
        // and converted *once* to the normalized seconds magnitude
        // (decision 6), never accumulated in floating point.
        (Value::Instant(x), Value::Instant(y)) if op == BinOp::Sub => {
            let xm = temporal::instant_to_ms(&x).map_err(|message| EvalError { message })?;
            let ym = temporal::instant_to_ms(&y).map_err(|message| EvalError { message })?;
            Ok(Value::Real((xm - ym) as f64 / 1000.0))
        }
        // Torsor translation: `instant +/- time[real]`, exact-or-error
        // (ADR 0036 decision 6): the duration must be a whole number of
        // milliseconds, and the result must stay in the representable range.
        (Value::Instant(t), Value::Real(d)) if matches!(op, BinOp::Add | BinOp::Sub) => {
            let ms = temporal::whole_milliseconds(d).map_err(|message| EvalError { message })?;
            let base = temporal::instant_to_ms(&t).map_err(|message| EvalError { message })?;
            let moved = match op {
                BinOp::Add => base.checked_add(ms),
                BinOp::Sub => base.checked_sub(ms),
                _ => unreachable!(),
            };
            let Some(moved) = moved else {
                return err("translation overflows the millisecond grid");
            };
            temporal::ms_to_instant(moved)
                .map(Value::Instant)
                .map_err(|message| EvalError { message })
        }
        _ => internal("arithmetic on values the checker should have rejected"),
    }
}

// ---------------------------------------------------------------------------
// orchestration

/// A `mensura run` failure past the frontend: storage or evaluation.
#[derive(Debug)]
pub enum RunError {
    Storage(StorageError),
    Eval(EvalError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Storage(e) => write!(f, "{e}"),
            RunError::Eval(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<StorageError> for RunError {
    fn from(e: StorageError) -> Self {
        RunError::Storage(e)
    }
}

impl From<EvalError> for RunError {
    fn from(e: EvalError) -> Self {
        RunError::Eval(e)
    }
}

/// Materialize every view of a resolved program over `backend`, in
/// declaration order (`docs/toolkit/04-processing-layer.md`): scan the
/// sources, evaluate the body, replace the view table's contents.  Returns
/// each view's name and row count.
pub fn materialize_views<B: StorageBackend>(
    backend: &mut B,
    program: &ResolvedProgram,
) -> Result<Vec<(String, usize)>, RunError> {
    let mut out = Vec::new();
    for plan in &program.views {
        let mut sources = BTreeMap::new();
        for name in &plan.sources {
            let Some(schema) = program.schemas.iter().find(|s| &s.store == name) else {
                return Err(EvalError {
                    message: format!(
                        "internal: view `{}` reads unknown store `{name}`",
                        plan.name
                    ),
                }
                .into());
            };
            let rows = backend.scan(&schema.shape())?;
            sources.insert(name.clone(), SourceTable::from_store(schema, rows));
        }
        let rows = eval_view(plan, &sources)?;
        backend.materialize_view(&plan.shape(), &rows)?;
        out.push((plan.name.clone(), rows.len()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_arithmetic_follows_the_torsor() {
        // ADR 0036 decision 4 at runtime: difference is a seconds magnitude
        // computed from an exact millisecond count, and translation is
        // exact-or-error (decision 6).
        let t = Value::Instant("2026-08-12T08:00:00.000Z".into());
        let u = Value::Instant("2026-08-12T08:15:00.000Z".into());
        assert_eq!(
            arithmetic(BinOp::Sub, u.clone(), t.clone()),
            Ok(Value::Real(900.0))
        );
        assert_eq!(
            arithmetic(BinOp::Add, t.clone(), Value::Real(900.0)),
            Ok(u.clone())
        );
        assert_eq!(
            arithmetic(BinOp::Sub, u.clone(), Value::Real(900.0)),
            Ok(t.clone())
        );
        // `t + (u - t) == u` across the whole representable range: the
        // round-trip property decision 9 proves on the grid, exercised here
        // through the one seconds<->milliseconds float conversion whose
        // safety decision 6's bound states.
        let first = Value::Instant("0001-01-01T00:00:00.000Z".into());
        let last = Value::Instant("9999-12-31T23:59:59.999Z".into());
        let diff = arithmetic(BinOp::Sub, last.clone(), first.clone()).expect("subtracts");
        assert_eq!(arithmetic(BinOp::Add, first, diff), Ok(last));
    }

    #[test]
    fn translation_is_exact_or_error() {
        let t = Value::Instant("2026-08-12T08:00:00.000Z".into());
        // A tenth of a millisecond is rejected, not rounded.
        let e = arithmetic(BinOp::Add, t.clone(), Value::Real(0.0001)).expect_err("sub-ms");
        assert!(e.message.contains("whole number of milliseconds"), "{e}");
        // A translation past the representable range is an error, not a wrap.
        let last = Value::Instant("9999-12-31T23:59:59.999Z".into());
        let e = arithmetic(BinOp::Add, last, Value::Real(0.001)).expect_err("range");
        assert!(e.message.contains("representable range"), "{e}");
    }

    /// Resolve a source and return its program (the test frontend).
    fn program(src: &str) -> ResolvedProgram {
        let tokens = mensura_syntax::tokenize(src).expect("should lex");
        let parsed = mensura_syntax::parse(&tokens).expect("should parse");
        mensura_types::resolve(&parsed).expect("should resolve")
    }

    const MACHINES: &str = r#"
        import bag
        unit Machine { id: string }
        enum MachineStatus { "operational" "degraded" "failure" }
        store machines {
          unit { Machine }
          attr {
            status: MachineStatus
            hours: int
            last_service: date?
          }
        }
    "#;

    fn machine(id: &str, status: &str, hours: i64, last: Option<&str>) -> Row {
        vec![
            Value::String(id.into()),
            Value::Enum(status.into()),
            Value::Int(hours),
            last.map_or(Value::Missing, |d| Value::Date(d.into())),
        ]
    }

    /// Evaluate the (single) view of `stores + src_view` over the given rows
    /// for the (single) store.
    fn eval_over(stores: &str, src_view: &str, rows_by_store: &[(&str, Vec<Row>)]) -> Vec<Row> {
        try_eval_over(stores, src_view, rows_by_store).expect("should evaluate")
    }

    fn try_eval_over(
        stores: &str,
        src_view: &str,
        rows_by_store: &[(&str, Vec<Row>)],
    ) -> Result<Vec<Row>, EvalError> {
        let src = format!("{stores}\n{src_view}");
        let program = program(&src);
        let plan = &program.views[0];
        let mut sources = BTreeMap::new();
        for (name, rows) in rows_by_store {
            let schema = program
                .schemas
                .iter()
                .find(|s| &s.store == name)
                .expect("store in program");
            sources.insert(
                name.to_string(),
                SourceTable::from_store(schema, rows.clone()),
            );
        }
        eval_view(plan, &sources)
    }

    fn eval(src_view: &str, rows: Vec<Row>) -> Vec<Row> {
        eval_over(MACHINES, src_view, &[("machines", rows)])
    }

    #[test]
    fn map_filters_by_returning_the_empty_collection() {
        let rows = eval(
            r#"view attention_needed {
                 machines |> flat_map |_, r| if r.status == "degraded" then r else ()
               }"#,
            vec![
                machine("m1", "operational", 10, Some("2026-01-01")),
                machine("m2", "degraded", 20, None),
                machine("m3", "failure", 30, None),
            ],
        );
        // The whole-row body yields the attributes in checker (alphabetical)
        // order: hours, last_service, status.
        assert_eq!(
            rows,
            vec![vec![
                Value::String("m2".into()),
                Value::Int(20),
                Value::Missing,
                Value::Enum("degraded".into()),
            ]]
        );
    }

    #[test]
    fn absence_absorbs_through_the_lifted_operators() {
        // ADR 0039 decision 1 at runtime: a missing operand makes every
        // lifted operator's result missing.  The third column pins the
        // absorbing (not Kleene) reading: `false and missing` is missing.
        let rows = eval(
            r#"view lifted {
                 machines |> flat_map |_, r| (
                   .same = r.last_service == r.last_service,
                   .and_true  = (r.hours > 0) and (r.last_service == r.last_service),
                   .and_false = (r.hours < 0) and (r.last_service == r.last_service),
                   .negated   = not (r.last_service == r.last_service)
                 )
               }"#,
            vec![
                machine("m1", "operational", 10, Some("2026-01-01")),
                machine("m2", "degraded", 20, None),
            ],
        );
        assert_eq!(
            rows,
            vec![
                vec![
                    Value::String("m1".into()),
                    Value::Bool(true),
                    Value::Bool(true),
                    Value::Bool(false),
                    Value::Bool(false),
                ],
                vec![
                    Value::String("m2".into()),
                    Value::Missing,
                    Value::Missing,
                    Value::Missing,
                    Value::Missing,
                ],
            ]
        );
    }

    #[test]
    fn to_real_lifts_over_a_missing_value() {
        // Regression: `to_real` was already lifted in the checker, but the
        // evaluator crashed on a missing operand before ADR 0039.
        let rows = eval_over(
            r#"
                unit K { id: string }
                store counts { unit { K } attr { n: int? } }
            "#,
            r#"view reals { counts |> flat_map |_, r| (.x = to_real r.n) }"#,
            &[(
                "counts",
                vec![
                    vec![Value::String("k1".into()), Value::Int(3)],
                    vec![Value::String("k2".into()), Value::Missing],
                ],
            )],
        );
        assert_eq!(
            rows,
            vec![
                vec![Value::String("k1".into()), Value::Real(3.0)],
                vec![Value::String("k2".into()), Value::Missing],
            ]
        );
    }

    #[test]
    fn coalesce_takes_the_present_value_or_the_default() {
        // ADR 0039 decision 2 at runtime: `??` returns the left value when
        // present and the default otherwise; an optional default keeps the
        // chain optional, so a missing left and missing default stay missing.
        let rows = eval(
            r#"view svc {
                 machines |> flat_map |_, r| (
                   .svc  = r.last_service ?? r.last_service,
                   .busy = (r.hours > 15) ?? false
                 )
               }"#,
            vec![
                machine("m1", "operational", 10, Some("2026-01-01")),
                machine("m2", "degraded", 20, None),
            ],
        );
        assert_eq!(
            rows,
            vec![
                vec![
                    Value::String("m1".into()),
                    Value::Date("2026-01-01".into()),
                    Value::Bool(false),
                ],
                vec![
                    Value::String("m2".into()),
                    Value::Missing,
                    Value::Bool(true),
                ],
            ]
        );
    }

    #[test]
    fn map_computes_record_fields_in_field_order() {
        let rows = eval(
            r#"view doubled {
                 machines |> flat_map |_, r| (.twice = r.hours * 2, .flagged = r.status != "operational")
               }"#,
            vec![
                machine("m1", "operational", 10, None),
                machine("m2", "degraded", 20, None),
            ],
        );
        assert_eq!(
            rows,
            vec![
                vec![
                    Value::String("m1".into()),
                    Value::Int(20),
                    Value::Bool(false)
                ],
                vec![
                    Value::String("m2".into()),
                    Value::Int(40),
                    Value::Bool(true)
                ],
            ]
        );
    }

    #[test]
    fn let_bindings_chain_stages() {
        let rows = eval(
            r#"view chained {
                 let flagged = machines |> flat_map |_, r| if r.last_service is missing then r else ();
                 flagged |> flat_map |_, r| (.hours = r.hours)
               }"#,
            vec![
                machine("m1", "operational", 10, Some("2026-01-01")),
                machine("m2", "degraded", 20, None),
            ],
        );
        assert_eq!(rows, vec![vec![Value::String("m2".into()), Value::Int(20)]]);
    }

    #[test]
    fn empty_source_materializes_an_empty_view() {
        let rows = eval(
            r#"view attention_needed {
                 machines |> flat_map |_, r| if r.status == "degraded" then r else ()
               }"#,
            Vec::new(),
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn promote_promotes_a_column_into_the_key() {
        let rows = eval(
            r#"view keyed {
                 machines |> promote hours |> flat_map |k, _| (.h = k.hours)
               }"#,
            vec![
                machine("m1", "operational", 10, None),
                machine("m2", "degraded", 20, None),
            ],
        );
        // The promoted column sits at the end of the key; the flat_map body reads
        // it off `k` to prove it moved.
        assert_eq!(
            rows,
            vec![
                vec![Value::String("m1".into()), Value::Int(10), Value::Int(10)],
                vec![Value::String("m2".into()), Value::Int(20), Value::Int(20)],
            ]
        );
    }

    #[test]
    fn map_bags_aggregates_one_row_per_key() {
        // `flat_map` expands each row to two, then `map_bags` folds each group
        // back to one aggregate row.  The expansion is complete by
        // construction, a fact the checker cannot yet derive, so the reducer's
        // ADR 0023 obligation is discharged with `assume`.
        let rows = eval(
            r#"view stats {
                 let doubled = machines |> flat_map |_, r| (r, r) |> assume { complete };
                 doubled |> map_bags |_, b| (.total = bag.sum b.hours, .n = #b.hours, .worst = bag.max b.hours)
               }"#,
            vec![
                machine("m1", "operational", 10, None),
                machine("m2", "degraded", 20, None),
            ],
        );
        assert_eq!(
            rows,
            vec![
                vec![
                    Value::String("m1".into()),
                    Value::Int(20),
                    Value::Int(2),
                    Value::Int(10)
                ],
                vec![
                    Value::String("m2".into()),
                    Value::Int(40),
                    Value::Int(2),
                    Value::Int(20)
                ],
            ]
        );
    }

    #[test]
    fn cardinality_counts_rows() {
        // `#` (ADR 0031, Decision 9): `#e == fold `+` (|_| 1) e`, so the
        // expansion's two rows count two whatever the column holds.
        let rows = eval(
            r#"view counted {
                 let doubled = machines |> flat_map |_, r| (r, r) |> assume { complete };
                 doubled |> map_bags |_, b| (.n = #b.hours)
               }"#,
            vec![machine("m1", "operational", 10, None)],
        );
        assert_eq!(rows, vec![vec![Value::String("m1".into()), Value::Int(2)]]);
    }

    #[test]
    fn a_qualified_module_reduction_runs_end_to_end() {
        // ADR 0031, Decision 8: `bag.max` is a const binding in a bundled
        // module, so this exercises the whole path at once -- the module's
        // `fold `>>` (|v| v)` eta-expands to a closure at const evaluation,
        // the checker applies it through a `Member` head, lowering
        // beta-reduces the qualified call, and the runtime folds.
        let rows = eval_over(
            MACHINES,
            r#"view summary {
                 let doubled = machines |> flat_map |_, r| (r, r) |> assume { complete };
                 doubled |> map_bags |_, b| (.hottest = bag.max b.hours, .total = bag.sum b.hours)
               }"#,
            &[("machines", vec![machine("m1", "operational", 10, None)])],
        );
        assert_eq!(
            rows,
            vec![vec![
                Value::String("m1".into()),
                Value::Int(10),
                Value::Int(20),
            ]]
        );
    }

    #[test]
    fn fold_reduces_a_projected_bag() {
        // ADR 0031, Decision 4, backed by Stage 1 (`Mensura.foldBag`).
        let rows = eval(
            r#"view folded {
                 let doubled = machines |> flat_map |_, r| (r, r) |> assume { complete };
                 doubled |> map_bags |_, b| (
                   .total = fold `+` (|v| v) b.hours,
                   .biggest = fold `>>` (|v| v) b.hours,
                   .sq = fold `+` (|v| v * v) b.hours
                 )
               }"#,
            vec![machine("m1", "operational", 10, None)],
        );
        assert_eq!(
            rows,
            vec![vec![
                Value::String("m1".into()),
                Value::Int(20),
                Value::Int(10),
                Value::Int(200),
            ]]
        );
    }

    #[test]
    fn fold_over_the_fiber_sees_whole_rows() {
        // ADR 0029's headline example: the mapper's element is a *row*, which
        // needs the fiber transposed out of the columnar group.
        let rows = eval(
            r#"view folded {
                 let doubled = machines |> flat_map |_, r| (r, r) |> assume { complete };
                 doubled |> map_bags |_, b| (.total = fold `+` (|r| r.hours * 2) b)
               }"#,
            vec![machine("m1", "operational", 10, None)],
        );
        assert_eq!(rows, vec![vec![Value::String("m1".into()), Value::Int(40)]]);
    }

    #[test]
    fn map_projects_and_computes() {
        // Decision 3: `map (|r| r.x) b` is `b.x`, and a computed bag is a
        // window value, so the result is one row per member.
        let rows = eval(
            r#"view mapped {
                 let doubled = machines |> flat_map |_, r| (r, r);
                 doubled |> map_bags |_, b| (.h = map (|r| r.hours * 3) b)
               }"#,
            vec![machine("m1", "operational", 10, None)],
        );
        assert_eq!(
            rows,
            vec![
                vec![Value::String("m1".into()), Value::Int(30)],
                vec![Value::String("m1".into()), Value::Int(30)],
            ]
        );
    }

    #[test]
    fn cardinality_of_the_fiber_counts_the_groups_rows() {
        // ADR 0031, Decision 1's headline: `#b` is the group's row count,
        // where today one writes `#b.x` and arbitrarily picks a column.
        // It agrees with every projection, since projection preserves
        // cardinality.
        let rows = eval(
            r#"view counted {
                 let doubled = machines |> flat_map |_, r| (r, r) |> assume { complete };
                 doubled |> map_bags |_, b| (.rows = #b, .via_col = #b.hours)
               }"#,
            vec![machine("m1", "operational", 10, None)],
        );
        assert_eq!(
            rows,
            vec![vec![
                Value::String("m1".into()),
                Value::Int(2),
                Value::Int(2)
            ]]
        );
    }

    #[test]
    fn cardinality_counts_a_missing_value_row() {
        // The mapper discards the element, so a row whose column is missing
        // still counts.  This is the one place `#` differs from the value
        // reductions, which demand a total bag.
        let rows = eval(
            r#"view counted {
                 machines |> assume { complete } |> map_bags |_, b| (.n = #b.last_service)
               }"#,
            vec![machine("m1", "operational", 10, None)],
        );
        assert_eq!(rows, vec![vec![Value::String("m1".into()), Value::Int(1)]]);
    }

    #[test]
    fn min_max_and_the_tacks_evaluate() {
        // ADR 0031, Decision 6.  `<<`/`>>` return an operand under the same
        // total order the comparisons use; the tacks return their named side.
        let rows = eval(
            r#"view clamped {
                 machines |> flat_map |_, r| ((
                   .lo = r.hours << 15,
                   .hi = r.hours >> 15,
                   .left = r.hours <: 99,
                   .right = r.hours :> 99
                 ))
               }"#,
            vec![machine("m1", "operational", 10, None)],
        );
        assert_eq!(
            rows,
            vec![vec![
                Value::String("m1".into()),
                Value::Int(10),
                Value::Int(15),
                Value::Int(10),
                Value::Int(99),
            ]]
        );
    }

    #[test]
    fn map_bags_windows_one_row_per_member() {
        let rows = eval(
            r#"view windowed {
                 let doubled = machines |> flat_map |_, r| (r, r);
                 doubled |> map_bags |_, b| (.h = to_real b.hours)
               }"#,
            vec![machine("m1", "operational", 10, None)],
        );
        assert_eq!(
            rows,
            vec![
                vec![Value::String("m1".into()), Value::Real(10.0)],
                vec![Value::String("m1".into()), Value::Real(10.0)],
            ]
        );
    }

    #[test]
    fn split_and_bind_reconstruct_the_table() {
        let rows = eval(
            r#"view roundtrip {
                 let parts = machines |> split |k| k.id == "m1";
                 parts |> union
               }"#,
            vec![
                machine("m1", "operational", 10, None),
                machine("m2", "degraded", 20, None),
                machine("m3", "failure", 30, None),
            ],
        );
        // The matching side comes first, then the rest, in input order.
        let ids: Vec<&Value> = rows.iter().map(|r| &r[0]).collect();
        assert_eq!(
            ids,
            vec![
                &Value::String("m1".into()),
                &Value::String("m2".into()),
                &Value::String("m3".into()),
            ]
        );
        assert_eq!(rows.len(), 3);
    }

    const FLEET: &str = r#"
        unit Machine { id: string }
        unit Site { code: string }
        store sites {
          unit { Site }
          attr { region: string }
        }
        store machines {
          unit { Machine }
          attr {
            site: string
            hours: int
          }
        }
    "#;

    fn fleet_rows() -> Vec<(&'static str, Vec<Row>)> {
        vec![
            (
                "sites",
                vec![vec![
                    Value::String("s1".into()),
                    Value::String("north".into()),
                ]],
            ),
            (
                "machines",
                vec![
                    vec![
                        Value::String("m1".into()),
                        Value::String("s1".into()),
                        Value::Int(10),
                    ],
                    vec![
                        Value::String("m2".into()),
                        Value::String("sX".into()),
                        Value::Int(20),
                    ],
                ],
            ),
        ]
    }

    #[test]
    fn lookup_keeps_unmatched_rows_with_missing() {
        let rows = eval_over(
            FLEET,
            r#"view located { machines |> lookup sites (|_, r| r.site) }"#,
            &fleet_rows(),
        );
        assert_eq!(
            rows,
            vec![
                vec![
                    Value::String("m1".into()),
                    Value::String("s1".into()),
                    Value::Int(10),
                    Value::String("north".into()),
                ],
                vec![
                    Value::String("m2".into()),
                    Value::String("sX".into()),
                    Value::Int(20),
                    Value::Missing,
                ],
            ]
        );
    }

    #[test]
    fn lookup_total_drops_unmatched_rows() {
        let rows = eval_over(
            FLEET,
            r#"view located { machines |> lookup_total sites (|_, r| r.site) }"#,
            &fleet_rows(),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::String("m1".into()));
        assert_eq!(rows[0][3], Value::String("north".into()));
    }

    const READINGS: &str = r#"
        unit Slot { ts: int }
        store readings {
          unit { Slot }
          attr {
            temperature: real
            humidity: real
          }
        }
    "#;

    #[test]
    fn unpivot_folds_all_attributes_and_drops_missing_cells() {
        let rows = eval_over(
            SPARSE_READINGS,
            r#"view long { readings |> unpivot metric reading }"#,
            &[(
                "readings",
                vec![
                    vec![Value::Int(1), Value::Real(20.0), Value::Real(30.0)],
                    vec![Value::Int(2), Value::Real(21.0), Value::Missing],
                ],
            )],
        );
        // The missing humidity cell at ts=2 yields no row (drop semantics),
        // so the value column is total.
        assert_eq!(
            rows,
            vec![
                vec![
                    Value::Int(1),
                    Value::Enum("temperature".into()),
                    Value::Real(20.0)
                ],
                vec![
                    Value::Int(1),
                    Value::Enum("humidity".into()),
                    Value::Real(30.0)
                ],
                vec![
                    Value::Int(2),
                    Value::Enum("temperature".into()),
                    Value::Real(21.0)
                ],
            ]
        );
    }

    const SPARSE_READINGS: &str = r#"
        unit Slot { ts: int }
        store readings {
          unit { Slot }
          attr {
            temperature: real
            humidity: real?
          }
        }
    "#;

    #[test]
    fn pivot_inverts_unpivot_including_missing_cells() {
        // The round trip (`pivot_unpivotDrop`): the absent long row at
        // (ts=2, humidity) comes back as the missing cell it encoded.
        let rows = eval_over(
            SPARSE_READINGS,
            r#"view wide { readings |> unpivot metric reading |> pivot metric reading }"#,
            &[(
                "readings",
                vec![
                    vec![Value::Int(1), Value::Real(20.0), Value::Real(30.0)],
                    vec![Value::Int(2), Value::Real(21.0), Value::Missing],
                ],
            )],
        );
        assert_eq!(
            rows,
            vec![
                vec![Value::Int(1), Value::Real(20.0), Value::Real(30.0)],
                vec![Value::Int(2), Value::Real(21.0), Value::Missing],
            ]
        );
    }

    #[test]
    fn pivot_gathers_each_residual_fiber_into_one_wide_row() {
        // Two long rows per slot gather into one wide row; keys come out in
        // key order, like `map_bags`'s groups.
        let rows = eval_over(
            READINGS,
            r#"view wide { readings |> unpivot metric reading |> pivot metric reading }"#,
            &[(
                "readings",
                vec![
                    vec![Value::Int(2), Value::Real(21.0), Value::Real(31.0)],
                    vec![Value::Int(1), Value::Real(20.0), Value::Real(30.0)],
                ],
            )],
        );
        assert_eq!(
            rows,
            vec![
                vec![Value::Int(1), Value::Real(20.0), Value::Real(30.0)],
                vec![Value::Int(2), Value::Real(21.0), Value::Real(31.0)],
            ]
        );
    }

    /// A history keyed by `(machine, taken_at)`: the shape whose grading
    /// survives `demote` (ADR 0024).
    const HISTORY: &str = r#"
        unit Sample {
          machine: string
          taken_at: date
        }
        store history {
          unit { Sample }
          attr { temperature: real }
        }
    "#;

    #[test]
    fn demote_drops_key_columns_to_the_end_in_key_order() {
        // `demote c a` names the columns out of key order; they still re-enter
        // at the end of the attribute list in key order, as the checker's
        // `op_demote` types them (ADR 0024 section 3).
        let rows = eval_over(
            r#"
                unit Triple {
                  a: string
                  b: string
                  c: string
                }
                store t {
                  unit { Triple }
                  attr { x: int }
                }
            "#,
            r#"view coarse { t |> demote c a }"#,
            &[(
                "t",
                vec![vec![
                    Value::String("a1".into()),
                    Value::String("b1".into()),
                    Value::String("c1".into()),
                    Value::Int(7),
                ]],
            )],
        );
        assert_eq!(
            rows,
            vec![vec![
                Value::String("b1".into()),
                Value::Int(7),
                Value::String("a1".into()),
                Value::String("c1".into()),
            ]]
        );
    }

    #[test]
    fn promote_then_demote_restores_the_rows() {
        // The exact round trip is the identity on the rows (`demote_promote`);
        // only the attribute order moves, `hours` re-entering at the end.
        let rows = eval(
            r#"view same {
                 machines |> promote hours |> demote hours
               }"#,
            vec![machine("m1", "operational", 10, None)],
        );
        assert_eq!(
            rows,
            vec![vec![
                Value::String("m1".into()),
                Value::Enum("operational".into()),
                Value::Missing,
                Value::Int(10),
            ]]
        );
    }

    #[test]
    fn demote_then_promote_is_the_identity() {
        // The other round-trip order (`promote_demote`): the demoted column
        // is the last attribute, so promoting it back restores the source
        // exactly, column order included.
        let source = vec![
            vec![
                Value::String("m1".into()),
                Value::Date("2026-01-01".into()),
                Value::Real(300.0),
            ],
            vec![
                Value::String("m1".into()),
                Value::Date("2026-01-02".into()),
                Value::Real(302.5),
            ],
        ];
        let rows = eval_over(
            HISTORY,
            r#"view same { history |> demote taken_at |> promote taken_at }"#,
            &[("history", source.clone())],
        );
        assert_eq!(rows, source);
    }

    /// The stride grid of ADR 0037 decision 1: window starts are the
    /// multiples of `stride` anchored at the domain's zero, and a point
    /// lands in every window `w` with `w <= p < w + size`.  Mirrors
    /// `Mensura.Units.Instant.mem_windowStarts`.
    #[test]
    fn window_starts_follow_the_stride_grid() {
        // Tumbling (`stride == size`): exactly one window per point.
        assert_eq!(window_starts(0, 15, 15), vec![0]);
        assert_eq!(window_starts(7, 15, 15), vec![0]);
        assert_eq!(window_starts(15, 15, 15), vec![15]);
        // Overlapping: `size / stride` windows contain the point.
        assert_eq!(window_starts(15, 15, 5), vec![5, 10, 15]);
        assert_eq!(window_starts(0, 15, 5), vec![-10, -5, 0]);
        // A stride wider than the size leaves gaps, and a point in one
        // lands in no window at all (legal, and occasionally wanted).
        assert_eq!(window_starts(10, 5, 15), Vec::<i64>::new());
        assert_eq!(window_starts(15, 5, 15), vec![15]);
        // The left edge is inclusive and the right edge is exclusive, which
        // is what makes a closed window's boundary row safe.
        assert_eq!(window_starts(14, 15, 15), vec![0]);
    }

    /// Pre-epoch points are the reason the grid uses euclidean division: a
    /// truncating `/` rounds toward zero, which would shift every negative
    /// point's window by a whole stride.
    #[test]
    fn window_starts_do_not_shift_before_the_epoch() {
        assert_eq!(window_starts(-1, 15, 15), vec![-15]);
        assert_eq!(window_starts(-15, 15, 15), vec![-15]);
        assert_eq!(window_starts(-16, 15, 15), vec![-30]);
    }

    /// End to end over instants: the replication adds one key column and
    /// one row per containing window, and the window start is written in
    /// the point's own domain.
    #[test]
    fn window_replicates_rows_onto_the_grid() {
        const READINGS: &str = r#"
            import si
            unit Reading { machine_id: string  taken_at: instant }
            registry readings {
              unit { Reading }
              attr { temperature: real }
              lateness { taken_at: 10.0 * si.minute }
            }
        "#;
        let reading = |m: &str, at: &str, t: f64| -> Row {
            vec![
                Value::String(m.into()),
                Value::Instant(at.into()),
                Value::Real(t),
            ]
        };
        // Quarter-hour tumbling windows: 10:07:31 lands in 10:00 only.
        let rows = eval_over(
            READINGS,
            r#"view w {
                 readings |> window w taken_at (15.0 * si.minute) (15.0 * si.minute)
               }"#,
            &[(
                "readings",
                vec![reading("M-07", "2026-08-10T10:07:31.221Z", 351.2)],
            )],
        );
        assert_eq!(
            rows,
            vec![vec![
                Value::String("M-07".into()),
                Value::Instant("2026-08-10T10:07:31.221Z".into()),
                Value::Instant("2026-08-10T10:00:00.000Z".into()),
                Value::Real(351.2),
            ]]
        );
        // A five-minute stride under a fifteen-minute size puts the same
        // reading in three windows.
        let rows = eval_over(
            READINGS,
            r#"view w {
                 readings |> window w taken_at (15.0 * si.minute) (5.0 * si.minute)
               }"#,
            &[(
                "readings",
                vec![reading("M-07", "2026-08-10T10:07:31.221Z", 351.2)],
            )],
        );
        let starts: Vec<&Value> = rows.iter().map(|r| &r[2]).collect();
        assert_eq!(
            starts,
            vec![
                &Value::Instant("2026-08-10T09:55:00.000Z".into()),
                &Value::Instant("2026-08-10T10:00:00.000Z".into()),
                &Value::Instant("2026-08-10T10:05:00.000Z".into()),
            ]
        );
    }
}
