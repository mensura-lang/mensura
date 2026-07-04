//! The batch evaluator of the processing layer
//! (`docs/toolkit/04-processing-layer.md`).
//!
//! Evaluates a checked view body ([`mensura_types::ViewPlan`]) over batches
//! of typed rows.  The checker has already established that the body is
//! well-typed, so evaluation cannot fail on shape; every "internal" error
//! here marks a case the frontend is supposed to have ruled out.  This
//! slice executes `map` (which subsumes filtering, ADR 0015); the remaining
//! Tier A operations land incrementally within M2.

use std::collections::BTreeMap;

use mensura_syntax::{BinOp, Expr, ExprKind, Presence, Stmt, UnOp};
use mensura_types::{ColumnRole, ResolvedProgram, Schema, ViewPlan};

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

/// A table value flowing through a pipeline: its key/value column split and
/// its rows (index values first, then attributes, positionally).
#[derive(Clone, Debug, PartialEq)]
pub struct SourceTable {
    index: Vec<String>,
    attrs: Vec<String>,
    rows: Vec<Row>,
}

impl SourceTable {
    /// Present a store's scanned rows to the evaluator.  `rows` are in the
    /// schema's column order, as [`StorageBackend::scan`] returns them.
    pub fn from_store(schema: &Schema, rows: Vec<Row>) -> SourceTable {
        let mut index = Vec::new();
        let mut attrs = Vec::new();
        for col in &schema.columns {
            match col.role {
                ColumnRole::Index => index.push(col.name.clone()),
                ColumnRole::Attr => attrs.push(col.name.clone()),
            }
        }
        SourceTable { index, attrs, rows }
    }
}

/// One output row of a `map` body: named values in output-column order.
type NamedRow = Vec<(String, Value)>;

/// A scalar-expression value: a single [`Value`] or a row/key record.
#[derive(Clone, Debug)]
enum RtVal {
    V(Value),
    Rec(BTreeMap<String, Value>),
}

/// Evaluate a view plan over its sources, returning the materialized rows in
/// the plan's column order (index columns, then attributes).
pub fn eval_view(
    plan: &ViewPlan,
    sources: &BTreeMap<String, SourceTable>,
) -> Result<Vec<Row>, EvalError> {
    let mut env = sources.clone();
    let mut result: Option<SourceTable> = None;
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
    align(plan, table)
}

/// Reorder a table's rows into the plan's output column order.  The checker
/// derived the plan's columns from the same body, so this is normally the
/// identity; a name mismatch is an internal error.
fn align(plan: &ViewPlan, table: SourceTable) -> Result<Vec<Row>, EvalError> {
    if table.rows.is_empty() {
        return Ok(Vec::new());
    }
    let actual: Vec<&String> = table.index.iter().chain(table.attrs.iter()).collect();
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
fn eval_pipeline(
    env: &BTreeMap<String, SourceTable>,
    expr: &Expr,
) -> Result<SourceTable, EvalError> {
    match &expr.kind {
        ExprKind::Name(name) => match env.get(name) {
            Some(table) => Ok(table.clone()),
            None => internal(format!("unknown source `{name}`")),
        },
        ExprKind::Binary(BinOp::Pipe, lhs, rhs) => {
            let input = eval_pipeline(env, lhs)?;
            apply_op(rhs, input)
        }
        ExprKind::App(..) => {
            let (head, mut args) = flatten_app(expr);
            let Some(last) = args.pop() else {
                return internal("pipeline application without an input");
            };
            let input = eval_pipeline(env, last)?;
            apply_args(head, &args, input)
        }
        ExprKind::Tuple(_) => err(
            "`split`/`bind` pairs are not yet executable; this slice runs `map` \
             only (docs/toolkit/04-processing-layer.md)",
        ),
        _ => internal("expression is not a pipeline"),
    }
}

/// Apply a pipeline stage (the right side of a `|>`) to its input.
fn apply_op(op_expr: &Expr, input: SourceTable) -> Result<SourceTable, EvalError> {
    let (head, args) = flatten_app(op_expr);
    apply_args(head, &args, input)
}

fn apply_args(head: &Expr, args: &[&Expr], input: SourceTable) -> Result<SourceTable, EvalError> {
    let ExprKind::Name(op) = &head.kind else {
        return internal("pipeline stage without an operation name");
    };
    match op.as_str() {
        "map" => eval_map(input, args),
        other => err(format!(
            "`{other}` is not yet executable; this slice runs `map` only \
             (docs/toolkit/04-processing-layer.md)"
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

/// `map |k, r| collection` (ADR 0015): evaluate the body once per input row;
/// each yielded value row becomes one output row under the same key, and
/// `( )` yields none.
fn eval_map(input: SourceTable, args: &[&Expr]) -> Result<SourceTable, EvalError> {
    let [lambda] = args else {
        return internal("`map` expects exactly a lambda argument");
    };
    let ExprKind::Lambda { params, body, .. } = &lambda.kind else {
        return internal("`map` expects a lambda argument");
    };
    let [kparam, rparam] = params.as_slice() else {
        return internal("a `map` lambda takes two parameters");
    };

    let nkeys = input.index.len();
    let mut attrs: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    for row in &input.rows {
        let (key, vals) = row.split_at(nkeys);
        let mut scope: BTreeMap<String, RtVal> = BTreeMap::new();
        if kparam.name != "_" {
            scope.insert(kparam.name.clone(), record(&input.index, key));
        }
        if rparam.name != "_" {
            scope.insert(rparam.name.clone(), record(&input.attrs, vals));
        }
        for named in eval_rows(&scope, body)? {
            let names: Vec<String> = named.iter().map(|(n, _)| n.clone()).collect();
            match &attrs {
                None => attrs = Some(names),
                Some(prev) if *prev == names => {}
                Some(_) => return internal("`map` rows with differing schemas"),
            }
            let mut out: Row = key.to_vec();
            out.extend(named.into_iter().map(|(_, v)| v));
            rows.push(out);
        }
    }
    Ok(SourceTable {
        index: input.index,
        attrs: attrs.unwrap_or_default(),
        rows,
    })
}

fn record(names: &[String], values: &[Value]) -> RtVal {
    RtVal::Rec(names.iter().cloned().zip(values.iter().cloned()).collect())
}

/// Evaluate a `map` body as a collection of value rows (ADR 0015): `( )` is
/// empty, `(a, b)` expands, an `if` filters or branches, and any other body
/// is a single row.
fn eval_rows(scope: &BTreeMap<String, RtVal>, body: &Expr) -> Result<Vec<NamedRow>, EvalError> {
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
fn eval_row(scope: &BTreeMap<String, RtVal>, expr: &Expr) -> Result<NamedRow, EvalError> {
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
            RtVal::Rec(fields) => Ok(fields.into_iter().collect()),
            RtVal::V(_) => internal("a `map` body row is not a record"),
        },
    }
}

/// Evaluate a scalar expression to a single [`Value`].
fn eval_value(scope: &BTreeMap<String, RtVal>, expr: &Expr) -> Result<Value, EvalError> {
    match eval_scalar(scope, expr)? {
        RtVal::V(v) => Ok(v),
        RtVal::Rec(_) => internal("expected a single value, found a row"),
    }
}

/// Evaluate a boolean expression (an `if` condition, a predicate).
fn eval_bool(scope: &BTreeMap<String, RtVal>, expr: &Expr) -> Result<bool, EvalError> {
    match eval_value(scope, expr)? {
        Value::Bool(b) => Ok(b),
        _ => internal("condition did not evaluate to a boolean"),
    }
}

/// Evaluate a scalar expression (`docs/language/06-expressions.md`) over the
/// lambda scope.  The checker has enforced the domain rules, so each
/// operator is implemented only on the variants it can meet.
fn eval_scalar(scope: &BTreeMap<String, RtVal>, expr: &Expr) -> Result<RtVal, EvalError> {
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
                Some(v) => Ok(RtVal::V(v.clone())),
                None => internal(format!("unknown column `{}`", field.name)),
            },
            RtVal::V(_) => internal("member access on a non-record value"),
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

/// The value-level builtins: `to_real` today (`x |> to_real` and `to_real x`
/// converge here, ADR 0018).  The aggregates need a group context, which
/// `group_map` brings when it becomes executable.
fn apply_value_fn(
    scope: &BTreeMap<String, RtVal>,
    head: &Expr,
    args: &[&Expr],
) -> Result<RtVal, EvalError> {
    let ExprKind::Name(name) = &head.kind else {
        return internal("application of a non-name");
    };
    let [arg] = args else {
        return internal(format!("`{name}` expects one argument"));
    };
    match name.as_str() {
        "to_real" => match eval_value(scope, arg)? {
            Value::Int(i) => Ok(RtVal::V(Value::Real(i as f64))),
            _ => internal("`to_real` on a non-int value"),
        },
        other => err(format!(
            "`{other}` is not yet executable in this slice \
             (docs/toolkit/04-processing-layer.md)"
        )),
    }
}

fn eval_unary(
    scope: &BTreeMap<String, RtVal>,
    op: UnOp,
    operand: &Expr,
) -> Result<RtVal, EvalError> {
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

fn eval_binary(
    scope: &BTreeMap<String, RtVal>,
    op: BinOp,
    lhs: &Expr,
    rhs: &Expr,
) -> Result<RtVal, EvalError> {
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
            return err("`in` is not yet executable in this slice \
             (docs/toolkit/04-processing-layer.md)");
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
        enum MachineStatus { "operational", "degraded", "failure" }
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

    fn eval(src_view: &str, rows: Vec<Row>) -> Vec<Row> {
        let src = format!("{MACHINES}\n{src_view}");
        let program = program(&src);
        let plan = &program.views[0];
        let mut sources = BTreeMap::new();
        sources.insert(
            "machines".to_string(),
            SourceTable::from_store(&program.schemas[0], rows),
        );
        eval_view(plan, &sources).expect("should evaluate")
    }

    #[test]
    fn map_filters_by_returning_the_empty_collection() {
        let rows = eval(
            r#"view attention_needed {
                 machines |> map |_, r| if r.status == "degraded" then r else ()
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
                 machines |> map |_, r| (.twice = r.hours * 2, .flagged = r.status != "operational")
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
                 let flagged = machines |> map |_, r| if r.last_service is missing then r else ();
                 flagged |> map |_, r| (.hours = r.hours)
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
                 machines |> map |_, r| if r.status == "degraded" then r else ()
               }"#,
            Vec::new(),
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn an_unimplemented_operation_reports_not_executable() {
        let src = format!("{MACHINES}\nview keyed {{ machines |> extend_key hours }}");
        let program = program(&src);
        let plan = &program.views[0];
        let mut sources = BTreeMap::new();
        sources.insert(
            "machines".to_string(),
            SourceTable::from_store(&program.schemas[0], Vec::new()),
        );
        let err = eval_view(plan, &sources).expect_err("should not execute");
        assert!(err.message.contains("not yet executable"), "{err}");
    }
}
