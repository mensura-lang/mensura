//! The batch evaluator of the processing layer
//! (`docs/toolkit/04-processing-layer.md`).
//!
//! Evaluates a checked view body ([`mensura_types::ViewPlan`]) over batches
//! of typed rows.  The checker has already established that the body is
//! well-typed, so evaluation cannot fail on shape; every "internal" error
//! here marks a case the frontend is supposed to have ruled out.  All Tier A
//! operations execute (`flat_map`, `map_bag`, `promote`, the joins,
//! `split`/`union`, `unpivot`), plus `pivot` (Tier B only for its lineage
//! effect, ADR 0020; batch evaluation is unaffected), and the establish
//! stages (`assume`, `completeness_check`) are identities: their facts are
//! proven at compile time and trusted at runtime (`ROADMAP.md` M2).
//! `demote` is not yet executable.

use std::collections::BTreeMap;

use mensura_syntax::{BinOp, Expr, ExprKind, Presence, Stmt, UnOp};
use mensura_types::{ColumnRole, ColumnType, ResolvedProgram, Schema, ViewPlan};

use crate::backend::{StorageBackend, StorageError};
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
        Value::String(s) | Value::Date(s) | Value::Enum(s) => Ok(KeyVal::Text(s.clone())),
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
        "map_bag" => Ok(TableVal::Table(eval_map_bag(expect_table(input)?, args)?)),
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
        "demote" => err("`demote` is Tier B and not yet executable \
             (docs/toolkit/04-processing-layer.md)"),
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
    RtVal::Rec(
        cols.iter()
            .map(|c| c.name.clone())
            .zip(values.iter().cloned().map(RtVal::V))
            .collect(),
    )
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
            RtVal::Rec(fields) => fields
                .into_iter()
                .map(|(name, v)| match v {
                    RtVal::V(v) => Ok((name, v)),
                    _ => internal("a row field that is not a single value"),
                })
                .collect(),
            _ => internal("a `flat_map` body row is not a record"),
        },
    }
}

// ---------------------------------------------------------------------------
// map_bag

/// `map_bag |k, b| record` (section 6.2): group rows by key and transform
/// each group.  All single-valued fields: one aggregate row per key.  All
/// bag-valued fields: one window row per input row of the group.
fn eval_map_bag(input: SourceTable, args: &[&Expr]) -> Result<SourceTable, EvalError> {
    let (params, body) = lambda_parts(args, 2)?;
    let ExprKind::Record(fields) = &body.kind else {
        return internal("`map_bag` body is not a record");
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
            let bags: BTreeMap<String, RtVal> = input
                .attrs
                .iter()
                .enumerate()
                .map(|(a, c)| {
                    let column: Vec<Value> = members
                        .iter()
                        .map(|&i| input.rows[i][nkeys + a].clone())
                        .collect();
                    (c.name.clone(), RtVal::Bag(column))
                })
                .collect();
            scope.insert(params[1].to_string(), RtVal::Rec(bags));
        }

        let mut aggregates: Vec<Value> = Vec::new();
        let mut windows: Vec<Vec<Value>> = Vec::new();
        for field in fields {
            match eval_scalar(&scope, &field.value)? {
                RtVal::V(v) => aggregates.push(v),
                RtVal::Bag(vs) => windows.push(vs),
                RtVal::Rec(_) => return internal("a `map_bag` field yielded a row"),
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
            _ => return internal("a mixed `map_bag` record (the checker rejects this)"),
        }
    }
    Ok(SourceTable {
        key: input.key,
        attrs,
        rows,
    })
}

/// The statically known domain of a `map_bag` field: a window copies its
/// column, the aggregates have fixed or copied domains.
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
    let applied = |head: &Expr, arg: &Expr| {
        let ExprKind::Name(name) = &head.kind else {
            return None;
        };
        match name.as_str() {
            "count" => Some(ColumnType::Int),
            "any" | "all" => Some(ColumnType::Bool),
            "to_real" => Some(ColumnType::Real),
            "sum" | "min" | "max" => member_domain(arg),
            _ => None,
        }
    };
    match &expr.kind {
        ExprKind::Member(..) => member_domain(expr),
        ExprKind::App(..) => {
            let (head, args) = flatten_app(expr);
            match args[..] {
                [arg] => applied(head, arg),
                _ => None,
            }
        }
        ExprKind::Binary(BinOp::Pipe, lhs, rhs) => applied(rhs, lhs),
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
/// missing cell.  Residual keys come out in key order, like `map_bag`'s
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

/// The value-level builtins: `to_real` and the bag aggregates (`x |> op`
/// and `op x` converge here, ADR 0018).
fn apply_value_fn(scope: &Scope, head: &Expr, args: &[&Expr]) -> Result<RtVal, EvalError> {
    let ExprKind::Name(name) = &head.kind else {
        return internal("application of a non-name");
    };
    let [arg] = args else {
        return internal(format!("`{name}` expects one argument"));
    };
    match (name.as_str(), eval_scalar(scope, arg)?) {
        ("to_real", RtVal::V(Value::Int(i))) => Ok(RtVal::V(Value::Real(i as f64))),
        ("to_real", RtVal::Bag(vs)) => Ok(RtVal::Bag(
            vs.into_iter()
                .map(|v| match v {
                    Value::Int(i) => Ok(Value::Real(i as f64)),
                    _ => internal("`to_real` on a non-int bag element"),
                })
                .collect::<Result<_, _>>()?,
        )),
        (agg, RtVal::Bag(vs)) => Ok(RtVal::V(aggregate(agg, &vs)?)),
        _ => internal(format!("`{name}` applied to an unsupported value")),
    }
}

/// A bag aggregate (section 5.4, ADR 0014).  Groups are never empty (they
/// arise from rows), and the checker requires a total bag, so elements are
/// never missing.
fn aggregate(name: &str, values: &[Value]) -> Result<Value, EvalError> {
    let [first, rest @ ..] = values else {
        return internal(format!("`{name}` over an empty bag"));
    };
    match name {
        "count" => Ok(Value::Int(values.len() as i64)),
        "sum" => rest.iter().try_fold(first.clone(), |acc, v| {
            arithmetic(BinOp::Add, acc, v.clone())
        }),
        "min" | "max" => {
            let mut best = first.clone();
            for v in rest {
                let ord = compare(v, &best)?;
                if (name == "min" && ord.is_lt()) || (name == "max" && ord.is_gt()) {
                    best = v.clone();
                }
            }
            Ok(best)
        }
        "any" | "all" => {
            let mut acc = name == "all";
            for v in values {
                let Value::Bool(b) = v else {
                    return internal(format!("`{name}` on a non-boolean bag"));
                };
                acc = if name == "any" { acc || *b } else { acc && *b };
            }
            Ok(Value::Bool(acc))
        }
        other => err(format!(
            "`{other}` is not yet executable (docs/toolkit/04-processing-layer.md)"
        )),
    }
}

fn eval_unary(scope: &Scope, op: UnOp, operand: &Expr) -> Result<RtVal, EvalError> {
    let v = eval_value(scope, operand)?;
    let out = match (op, v) {
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
    match op {
        // Short-circuit before evaluating the right side.
        BinOp::And => {
            let out = eval_bool(scope, lhs)? && eval_bool(scope, rhs)?;
            return Ok(RtVal::V(Value::Bool(out)));
        }
        BinOp::Or => {
            let out = eval_bool(scope, lhs)? || eval_bool(scope, rhs)?;
            return Ok(RtVal::V(Value::Bool(out)));
        }
        // `x |> op` is `op x` (ADR 0018).
        BinOp::Pipe => {
            let (head, mut args) = flatten_app(rhs);
            args.push(lhs);
            return apply_value_fn(scope, head, &args);
        }
        _ => {}
    }

    let a = eval_value(scope, lhs)?;
    let b = eval_value(scope, rhs)?;
    let out = match op {
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
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => arithmetic(op, a, b)?,
        BinOp::In => {
            return err("`in` is not yet executable (docs/toolkit/04-processing-layer.md)");
        }
        BinOp::And | BinOp::Or | BinOp::Pipe => unreachable!("handled above"),
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
        (Value::Enum(x), Value::Enum(y)) => Ok(x == y),
        (Value::Enum(x), Value::String(y)) | (Value::String(x), Value::Enum(y)) => Ok(x == y),
        _ => internal("`==` on values the checker should have rejected"),
    }
}

/// Ordering on orderable domains: `int`, `real`, and `date` (ISO 8601 text
/// orders chronologically).
fn compare(a: &Value, b: &Value) -> Result<std::cmp::Ordering, EvalError> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(x.cmp(y)),
        (Value::Real(x), Value::Real(y)) => match x.partial_cmp(y) {
            Some(ord) => Ok(ord),
            None => err("comparison with a NaN value"),
        },
        (Value::Date(x), Value::Date(y)) => Ok(x.cmp(y)),
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

    /// Resolve a source and return its program (the test frontend).
    fn program(src: &str) -> ResolvedProgram {
        let tokens = mensura_syntax::tokenize(src).expect("should lex");
        let parsed = mensura_syntax::parse(&tokens).expect("should parse");
        mensura_types::resolve(&parsed).expect("should resolve")
    }

    const MACHINES: &str = r#"
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
    fn map_bag_aggregates_one_row_per_key() {
        // `flat_map` expands each row to two, then `map_bag` folds each group
        // back to one aggregate row.  The expansion is complete by
        // construction, a fact the checker cannot yet derive, so the reducer's
        // ADR 0023 obligation is discharged with `assume`.
        let rows = eval(
            r#"view stats {
                 let doubled = machines |> flat_map |_, r| (r, r) |> assume { complete };
                 doubled |> map_bag |_, b| (.total = sum b.hours, .n = count b.hours, .worst = max b.hours)
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
    fn map_bag_windows_one_row_per_member() {
        let rows = eval(
            r#"view windowed {
                 let doubled = machines |> flat_map |_, r| (r, r);
                 doubled |> map_bag |_, b| (.h = to_real b.hours)
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
        // key order, like `map_bag`'s groups.
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

    #[test]
    fn tier_b_demote_reports_not_executable() {
        let err = try_eval_over(
            MACHINES,
            r#"view coarse {
                 machines |> promote hours |> assume { complete } |> demote hours
               }"#,
            &[("machines", Vec::new())],
        )
        .expect_err("Tier B must not execute");
        assert!(err.message.contains("demote"), "{err}");
        assert!(err.message.contains("not yet executable"), "{err}");
    }
}
