//! Value-expression type checking (`docs/language/09-typing-reference.md`
//! section 5, `06-expressions.md`, ADR 0014).
//!
//! Types an expression against a row/group context derived from a [`TableType`].
//! Mirrors `resolve`'s contract: it collects all diagnostics rather than failing
//! on the first. Operators are gated by the scalar domain's properties
//! (equatable / orderable / numeric); typing is strict, with no `int`/`real`
//! coercion. The `|>` pipe, general application, lambdas, record/tuple literals,
//! and `is known` narrowing are deferred to later rounds.

use std::collections::BTreeMap;

use mensura_syntax::{BinOp, Expr, ExprKind, Ident, Span, UnOp};

use crate::model::ColumnType;
use crate::table::TableType;

/// The optional axis of a single value (`09` section 3.3 / 5.3). Distinct from
/// the table-scoped `table::Totality`; here it is a per-value flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Optionality {
    Total,
    Optional,
}

/// The type of an expression value (`09` section 5).
#[derive(Clone, Debug, PartialEq)]
pub enum Ty {
    /// A card-1 value carrying its domain and optionality (section 5.3).
    Value {
        domain: ColumnType,
        opt: Optionality,
    },
    /// A bag at one key, consumable only by combinators (section 5.4). It
    /// carries a totality so an aggregate can demand a total bag (ADR 0014).
    Bag {
        domain: ColumnType,
        opt: Optionality,
    },
    /// A boolean: the result of a predicate, comparison, or presence test.
    Bool,
    /// A row (fields are `Value`) or a group (fields are `Bag`).
    Record(BTreeMap<String, Ty>),
}

impl Ty {
    /// `Some(domain)` iff this is a card-1, not-missing value: the gate the
    /// scalar rule checks (section 5.3).
    pub fn known_value_domain(&self) -> Option<&ColumnType> {
        match self {
            Ty::Value {
                domain,
                opt: Optionality::Total,
            } => Some(domain),
            _ => None,
        }
    }
}

/// A type-checking diagnostic, located by span. Mirrors `ResolveError`.
#[derive(Clone, Debug, PartialEq)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
}

impl TypeError {
    fn new(message: impl Into<String>, span: Span) -> Self {
        TypeError {
            message: message.into(),
            span,
        }
    }
}

/// The bag aggregate builtins in scope (section 5.4). `mean` is not a primitive
/// (ADR 0014): it is `sum(x) / to_real(count(x))`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agg {
    Sum,
    Min,
    Max,
    Count,
    Any,
    All,
}

/// The typing context `Gamma` (section 5.1): the named values in scope and the
/// in-scope builtins. Which builtins are in scope is a property of the context,
/// not the grammar.
pub struct Context {
    names: BTreeMap<String, Ty>,
    aggregates: BTreeMap<String, Agg>,
}

impl Context {
    /// Bind a row lambda's parameter (e.g. `r`) to a record whose fields are the
    /// table's columns as single values carrying their totality (section 5.1).
    pub fn row(param: &str, table: &TableType) -> Context {
        Context {
            names: bind(param, row_record(table)),
            aggregates: builtin_aggregates(),
        }
    }

    /// Bind a group lambda's parameter (e.g. `g`) to a record whose fields are
    /// the table's columns as bags (section 5.4).
    pub fn group(param: &str, table: &TableType) -> Context {
        Context {
            names: bind(param, group_record(table)),
            aggregates: builtin_aggregates(),
        }
    }

    /// Bind a `split` predicate's parameter (e.g. `k`) to a record of the
    /// table's index columns as total values (the key, `09` section 6.5).
    pub fn key(param: &str, table: &TableType) -> Context {
        Context {
            names: bind(param, key_record(table)),
            aggregates: builtin_aggregates(),
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&Ty> {
        self.names.get(name)
    }

    pub fn aggregate(&self, name: &str) -> Option<Agg> {
        self.aggregates.get(name).copied()
    }
}

fn bind(param: &str, ty: Ty) -> BTreeMap<String, Ty> {
    let mut names = BTreeMap::new();
    names.insert(param.to_string(), ty);
    names
}

fn builtin_aggregates() -> BTreeMap<String, Agg> {
    [
        ("sum", Agg::Sum),
        ("min", Agg::Min),
        ("max", Agg::Max),
        ("count", Agg::Count),
        ("any", Agg::Any),
        ("all", Agg::All),
    ]
    .into_iter()
    .map(|(name, agg)| (name.to_string(), agg))
    .collect()
}

/// A row view of a table: each column is a single value; non-index columns carry
/// their totality, index columns are always total.
fn row_record(table: &TableType) -> Ty {
    let mut fields = BTreeMap::new();
    for col in &table.content.index {
        fields.insert(
            col.name.clone(),
            Ty::Value {
                domain: col.domain.clone(),
                opt: Optionality::Total,
            },
        );
    }
    for col in &table.content.columns {
        fields.insert(
            col.name.clone(),
            Ty::Value {
                domain: col.domain.clone(),
                opt: column_opt(table, &col.name),
            },
        );
    }
    Ty::Record(fields)
}

/// A group view of a table: every cell is a bag carrying the column's totality
/// (section 5.4, "`|g|` sees the whole bag at a key").
fn group_record(table: &TableType) -> Ty {
    let mut fields = BTreeMap::new();
    for col in table.content.index.iter().chain(&table.content.columns) {
        fields.insert(
            col.name.clone(),
            Ty::Bag {
                domain: col.domain.clone(),
                opt: column_opt(table, &col.name),
            },
        );
    }
    Ty::Record(fields)
}

/// A key view of a table: the index columns as total values.
fn key_record(table: &TableType) -> Ty {
    let mut fields = BTreeMap::new();
    for col in &table.content.index {
        fields.insert(
            col.name.clone(),
            Ty::Value {
                domain: col.domain.clone(),
                opt: Optionality::Total,
            },
        );
    }
    Ty::Record(fields)
}

fn column_opt(table: &TableType, name: &str) -> Optionality {
    if table.qualifiers.totality.is_total(name) {
        Optionality::Total
    } else {
        Optionality::Optional
    }
}

fn total(domain: ColumnType) -> Ty {
    Ty::Value {
        domain,
        opt: Optionality::Total,
    }
}

/// Type an expression, collecting all diagnostics (parallels
/// `resolve(&Program) -> Result<Vec<Schema>, Vec<ResolveError>>`).
pub fn type_expr(ctx: &Context, expr: &Expr) -> Result<Ty, Vec<TypeError>> {
    match &expr.kind {
        ExprKind::Int(_) => Ok(total(ColumnType::Int)),
        ExprKind::Float(_) => Ok(total(ColumnType::Real)),
        ExprKind::Str(_) => Ok(total(ColumnType::String)),
        ExprKind::Bool(_) => Ok(Ty::Bool),
        ExprKind::Name(name) => type_name(ctx, name, expr.span),
        ExprKind::Member(base, field) => type_member(ctx, base, field),
        ExprKind::Binary(op, lhs, rhs) => type_binary(ctx, *op, lhs, rhs, expr.span),
        ExprKind::Unary(op, operand) => type_unary(ctx, *op, operand),
        ExprKind::App(func, arg) => type_app(ctx, func, arg, expr.span),
        ExprKind::Presence(base, _) => type_presence(ctx, base, expr.span),
        ExprKind::If { cond, then, els } => type_if(ctx, cond, then, els, expr.span),
        _ => Err(vec![TypeError::new(
            "unsupported in this increment",
            expr.span,
        )]),
    }
}

fn type_name(ctx: &Context, name: &str, span: Span) -> Result<Ty, Vec<TypeError>> {
    match ctx.lookup(name) {
        Some(ty) => Ok(ty.clone()),
        None => Err(vec![TypeError::new(format!("unknown name `{name}`"), span)]),
    }
}

fn type_member(ctx: &Context, base: &Expr, field: &Ident) -> Result<Ty, Vec<TypeError>> {
    match type_expr(ctx, base)? {
        Ty::Record(fields) => match fields.get(&field.name) {
            Some(ty) => Ok(ty.clone()),
            None => Err(vec![TypeError::new(
                format!("unknown column `{}`", field.name),
                field.span,
            )]),
        },
        _ => Err(vec![TypeError::new(
            "member access on a non-record value",
            field.span,
        )]),
    }
}

/// Application. The forms typed are the `to_real` conversion and an aggregate
/// applied to a bag (section 5.4, ADR 0014); any other application is deferred.
fn type_app(ctx: &Context, func: &Expr, arg: &Expr, span: Span) -> Result<Ty, Vec<TypeError>> {
    let ExprKind::Name(name) = &func.kind else {
        return Err(vec![TypeError::new("unsupported in this increment", span)]);
    };
    if name == "to_real" {
        return type_to_real(ctx, arg);
    }
    match ctx.aggregate(name) {
        Some(agg) => type_aggregate(ctx, agg, name, arg),
        None => Err(vec![TypeError::new("unsupported in this increment", span)]),
    }
}

/// `to_real` (ADR 0014): `int -> real` on a value, lifted element-wise over a
/// bag (`bag<int> -> bag<real>`); totality is preserved.
fn type_to_real(ctx: &Context, arg: &Expr) -> Result<Ty, Vec<TypeError>> {
    match type_expr(ctx, arg)? {
        Ty::Value {
            domain: ColumnType::Int,
            opt,
        } => Ok(Ty::Value {
            domain: ColumnType::Real,
            opt,
        }),
        Ty::Bag {
            domain: ColumnType::Int,
            opt,
        } => Ok(Ty::Bag {
            domain: ColumnType::Real,
            opt,
        }),
        _ => Err(vec![TypeError::new(
            "`to_real` converts an int value or bag",
            arg.span,
        )]),
    }
}

/// A bag aggregate (section 5.4). Requires a total bag; the result domain is
/// per the aggregate (ADR 0014): `count -> int`; `sum` preserves a numeric
/// domain; `min`/`max` preserve an orderable domain; `any`/`all -> bool`.
fn type_aggregate(ctx: &Context, agg: Agg, name: &str, arg: &Expr) -> Result<Ty, Vec<TypeError>> {
    let (domain, opt) = match type_expr(ctx, arg)? {
        Ty::Bag { domain, opt } => (domain, opt),
        _ => {
            return Err(vec![TypeError::new(
                format!("`{name}` expects a bag"),
                arg.span,
            )]);
        }
    };
    if opt == Optionality::Optional {
        return Err(vec![TypeError::new(
            format!("`{name}` requires a total bag; this column may be missing values"),
            arg.span,
        )]);
    }
    match agg {
        Agg::Count => Ok(total(ColumnType::Int)),
        Agg::Sum if domain.is_numeric() => Ok(total(domain)),
        Agg::Sum => Err(vec![TypeError::new(
            format!(
                "`sum` expects a numeric bag, found a bag of {}",
                domain_name(&domain)
            ),
            arg.span,
        )]),
        Agg::Min | Agg::Max if domain.is_orderable() => Ok(total(domain)),
        Agg::Min | Agg::Max => Err(vec![TypeError::new(
            format!(
                "`{name}` expects an orderable bag (int, real, or date), found a bag of {}",
                domain_name(&domain)
            ),
            arg.span,
        )]),
        Agg::Any | Agg::All if domain == ColumnType::Bool => Ok(total(ColumnType::Bool)),
        Agg::Any | Agg::All => Err(vec![TypeError::new(
            format!(
                "`{name}` expects a bag of booleans, found a bag of {}",
                domain_name(&domain)
            ),
            arg.span,
        )]),
    }
}

fn type_binary(
    ctx: &Context,
    op: BinOp,
    lhs: &Expr,
    rhs: &Expr,
    span: Span,
) -> Result<Ty, Vec<TypeError>> {
    match op {
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Pow => {
            let domain = matching_operands(
                ctx,
                lhs,
                rhs,
                ColumnType::is_numeric,
                "arithmetic",
                "a number",
            )?;
            Ok(total(domain))
        }
        BinOp::Div => {
            let domain = matching_operands(
                ctx,
                lhs,
                rhs,
                |d| matches!(d, ColumnType::Real),
                "`/`",
                "a real (`/` is real only)",
            )?;
            Ok(total(domain))
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            matching_operands(
                ctx,
                lhs,
                rhs,
                ColumnType::is_orderable,
                "a comparison",
                "an orderable value (int, real, or date)",
            )?;
            Ok(Ty::Bool)
        }
        BinOp::Eq | BinOp::Ne => type_equality(ctx, lhs, rhs),
        BinOp::And | BinOp::Or => {
            let mut errs = require_bool(ctx, lhs, "a boolean operator");
            errs.extend(require_bool(ctx, rhs, "a boolean operator"));
            if errs.is_empty() {
                Ok(Ty::Bool)
            } else {
                Err(errs)
            }
        }
        BinOp::In => type_membership(ctx, lhs, rhs),
        BinOp::Pipe => Err(vec![TypeError::new("unsupported in this increment", span)]),
    }
}

/// Type both operands as known values whose domain satisfies `ok` and that match
/// each other; return the common domain. `label`/`expected` shape the messages.
fn matching_operands(
    ctx: &Context,
    lhs: &Expr,
    rhs: &Expr,
    ok: fn(&ColumnType) -> bool,
    label: &str,
    expected: &str,
) -> Result<ColumnType, Vec<TypeError>> {
    let mut errs = Vec::new();
    let ld = operand_domain(ctx, lhs, ok, label, expected, &mut errs);
    let rd = operand_domain(ctx, rhs, ok, label, expected, &mut errs);
    let (Some(ld), Some(rd)) = (ld, rd) else {
        return Err(errs);
    };
    if ld == rd {
        Ok(ld)
    } else {
        Err(vec![TypeError::new(
            format!(
                "{label} expects operands of the same type, found {} and {}",
                domain_name(&ld),
                domain_name(&rd)
            ),
            lhs.span,
        )])
    }
}

fn operand_domain(
    ctx: &Context,
    operand: &Expr,
    ok: fn(&ColumnType) -> bool,
    label: &str,
    expected: &str,
    errs: &mut Vec<TypeError>,
) -> Option<ColumnType> {
    match type_expr(ctx, operand) {
        Err(e) => {
            errs.extend(e);
            None
        }
        Ok(ty) => match as_known_value(&ty, label, operand.span) {
            Err(e) => {
                errs.extend(e);
                None
            }
            Ok(domain) if ok(&domain) => Some(domain),
            Ok(domain) => {
                errs.push(TypeError::new(
                    format!(
                        "{label} expects {expected}, found a {}",
                        domain_name(&domain)
                    ),
                    operand.span,
                ));
                None
            }
        },
    }
}

/// `value in bag` (section 5.4): the left side a known value matching the bag's
/// element domain; the right side a bag. Returns `Bool`.
fn type_membership(ctx: &Context, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    let mut errs = Vec::new();
    let lt = collect_ty(type_expr(ctx, lhs), &mut errs);
    let rt = collect_ty(type_expr(ctx, rhs), &mut errs);
    let (Some(lt), Some(rt)) = (lt, rt) else {
        return Err(errs);
    };
    let elem = match rt {
        Ty::Bag { domain, .. } => domain,
        _ => {
            errs.push(TypeError::new("`in` expects a bag on the right", rhs.span));
            return Err(errs);
        }
    };
    match as_known_value(&lt, "`in`", lhs.span) {
        Ok(domain) if domain == elem => Ok(Ty::Bool),
        Ok(domain) => Err(vec![TypeError::new(
            format!(
                "`in` expects a {} value to match the bag, found a {}",
                domain_name(&elem),
                domain_name(&domain)
            ),
            lhs.span,
        )]),
        Err(e) => {
            errs.extend(e);
            Err(errs)
        }
    }
}

/// `is known` / `is missing` (section 5.5): apply to a value, yield `Bool`.
/// Narrowing is deferred, so `is known` does not change the value's totality.
fn type_presence(ctx: &Context, base: &Expr, span: Span) -> Result<Ty, Vec<TypeError>> {
    match type_expr(ctx, base)? {
        Ty::Value { .. } => Ok(Ty::Bool),
        _ => Err(vec![TypeError::new(
            "`is known` / `is missing` apply to a value",
            span,
        )]),
    }
}

fn type_unary(ctx: &Context, op: UnOp, operand: &Expr) -> Result<Ty, Vec<TypeError>> {
    match op {
        UnOp::Neg => {
            let mut errs = Vec::new();
            let d = operand_domain(
                ctx,
                operand,
                ColumnType::is_numeric,
                "negation",
                "a number",
                &mut errs,
            );
            match d {
                Some(domain) if errs.is_empty() => Ok(total(domain)),
                _ => Err(errs),
            }
        }
        UnOp::Not => {
            let errs = require_bool(ctx, operand, "`not`");
            if errs.is_empty() {
                Ok(Ty::Bool)
            } else {
                Err(errs)
            }
        }
    }
}

/// `==` / `!=` (equatable, ADR 0014): both sides known values of the same
/// equatable domain (so `real` is rejected), with the enum-vs-string-literal
/// exception of section 5.6.
fn type_equality(ctx: &Context, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    let mut errs = Vec::new();
    let lt = collect_ty(type_expr(ctx, lhs), &mut errs);
    let rt = collect_ty(type_expr(ctx, rhs), &mut errs);
    let (Some(lt), Some(rt)) = (lt, rt) else {
        return Err(errs);
    };

    if let Some(res) = enum_vs_literal(&lt, rhs) {
        return res;
    }
    if let Some(res) = enum_vs_literal(&rt, lhs) {
        return res;
    }

    let ld = known_equatable(&lt, lhs.span, &mut errs);
    let rd = known_equatable(&rt, rhs.span, &mut errs);
    let (Some(ld), Some(rd)) = (ld, rd) else {
        return Err(errs);
    };
    if ld == rd {
        Ok(Ty::Bool)
    } else {
        Err(vec![TypeError::new(
            format!(
                "`==`/`!=` expects matching types, found a {} and a {}",
                domain_name(&ld),
                domain_name(&rd)
            ),
            lhs.span,
        )])
    }
}

fn known_equatable(ty: &Ty, span: Span, errs: &mut Vec<TypeError>) -> Option<ColumnType> {
    match as_known_value(ty, "`==`/`!=`", span) {
        Err(e) => {
            errs.extend(e);
            None
        }
        Ok(domain) if domain.is_equatable() => Some(domain),
        Ok(domain) => {
            errs.push(TypeError::new(
                format!("`==`/`!=` is not defined on {}", domain_name(&domain)),
                span,
            ));
            None
        }
    }
}

fn collect_ty(result: Result<Ty, Vec<TypeError>>, errs: &mut Vec<TypeError>) -> Option<Ty> {
    match result {
        Ok(ty) => Some(ty),
        Err(e) => {
            errs.extend(e);
            None
        }
    }
}

/// The section 5.6 exception: an enum value compared to a string literal,
/// validating the literal against the variant set. `None` if `value` is not an
/// enum or `other` is not a string literal.
fn enum_vs_literal(value: &Ty, other: &Expr) -> Option<Result<Ty, Vec<TypeError>>> {
    let Ty::Value {
        domain: ColumnType::Enum { name, variants },
        opt: Optionality::Total,
    } = value
    else {
        return None;
    };
    let ExprKind::Str(lit) = &other.kind else {
        return None;
    };
    if variants.iter().any(|v| v == lit) {
        Some(Ok(Ty::Bool))
    } else {
        Some(Err(vec![TypeError::new(
            format!("`{lit}` is not a variant of `{name}`"),
            other.span,
        )]))
    }
}

/// The domain of `ty` if it is a single known value, else a located error
/// (the scalar rule, section 5.3).
fn as_known_value(ty: &Ty, what: &str, span: Span) -> Result<ColumnType, Vec<TypeError>> {
    match ty {
        Ty::Value {
            domain,
            opt: Optionality::Total,
        } => Ok(domain.clone()),
        Ty::Value {
            opt: Optionality::Optional,
            ..
        } => Err(vec![TypeError::new(
            format!("{what} expects a known value; this value may be missing"),
            span,
        )]),
        Ty::Bag { .. } => Err(vec![TypeError::new(
            format!("{what} expects a single value, found a bag"),
            span,
        )]),
        Ty::Bool => Err(vec![TypeError::new(
            format!("{what} expects a value, found a boolean"),
            span,
        )]),
        Ty::Record(_) => Err(vec![TypeError::new(
            format!("{what} expects a value, found a row"),
            span,
        )]),
    }
}

/// Require `operand` to be a known boolean (section 5.3). Accepts both a
/// predicate result and a total `bool` column read.
/// `if c then a else b` (section 5, ADR 0015): `c` is a known boolean and the
/// two branches unify to one value type, which is the result.
fn type_if(
    ctx: &Context,
    cond: &Expr,
    then: &Expr,
    els: &Expr,
    span: Span,
) -> Result<Ty, Vec<TypeError>> {
    let mut errs = require_bool(ctx, cond, "an `if` condition");
    let then_ty = collect_ty(type_expr(ctx, then), &mut errs);
    let els_ty = collect_ty(type_expr(ctx, els), &mut errs);
    let (Some(then_ty), Some(els_ty)) = (then_ty, els_ty) else {
        return Err(errs);
    };
    if !errs.is_empty() {
        return Err(errs);
    }
    unify_branches(&then_ty, &els_ty, span)
}

/// Unify the two branch types of a conditional. Two values of the same domain
/// merge (the result is optional if either branch is); otherwise the branches
/// must be identical. A mismatch is a located error.
fn unify_branches(then_ty: &Ty, els_ty: &Ty, span: Span) -> Result<Ty, Vec<TypeError>> {
    match (then_ty, els_ty) {
        (
            Ty::Value {
                domain: da,
                opt: oa,
            },
            Ty::Value {
                domain: db,
                opt: ob,
            },
        ) if da == db => Ok(Ty::Value {
            domain: da.clone(),
            opt: join_opt(*oa, *ob),
        }),
        _ if then_ty == els_ty => Ok(then_ty.clone()),
        _ => Err(vec![TypeError::new(
            format!(
                "the `if` branches must have the same type, found {} and {}",
                describe_ty(then_ty),
                describe_ty(els_ty)
            ),
            span,
        )]),
    }
}

/// The optional axis of a merge: optional if either input is.
fn join_opt(a: Optionality, b: Optionality) -> Optionality {
    if a == Optionality::Optional || b == Optionality::Optional {
        Optionality::Optional
    } else {
        Optionality::Total
    }
}

/// A short human description of a `Ty` for diagnostics.
fn describe_ty(ty: &Ty) -> String {
    match ty {
        Ty::Value { domain, .. } => format!("a {}", domain_name(domain)),
        Ty::Bag { domain, .. } => format!("a bag of {}", domain_name(domain)),
        Ty::Bool => "a bool".to_string(),
        Ty::Record(_) => "a record".to_string(),
    }
}

fn require_bool(ctx: &Context, operand: &Expr, what: &str) -> Vec<TypeError> {
    match type_expr(ctx, operand) {
        Err(errs) => errs,
        Ok(Ty::Bool) => Vec::new(),
        Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt: Optionality::Total,
        }) => Vec::new(),
        Ok(Ty::Value {
            domain: ColumnType::Bool,
            opt: Optionality::Optional,
        }) => vec![TypeError::new(
            format!("{what} expects a known value; this value may be missing"),
            operand.span,
        )],
        Ok(_) => vec![TypeError::new(
            format!("{what} expects a boolean"),
            operand.span,
        )],
    }
}

fn domain_name(domain: &ColumnType) -> &'static str {
    match domain {
        ColumnType::String => "string",
        ColumnType::Int => "int",
        ColumnType::Real => "real",
        ColumnType::Bool => "bool",
        ColumnType::Date => "date",
        ColumnType::Enum { .. } => "enum",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Column as StorageColumn, ColumnRole, Schema};

    fn scol(name: &str, ty: ColumnType, role: ColumnRole, optional: bool) -> StorageColumn {
        StorageColumn {
            name: name.to_string(),
            ty,
            role,
            optional,
            span: Span::new(0, 0),
        }
    }

    fn sample_table() -> TableType {
        let schema = Schema {
            store: "readings".to_string(),
            unit: "Machine".to_string(),
            columns: vec![
                scol("machine", ColumnType::String, ColumnRole::Index, false),
                scol("size", ColumnType::Int, ColumnRole::Var, false),
                scol("temperature", ColumnType::Real, ColumnRole::Var, false),
                scol("peak", ColumnType::Real, ColumnRole::Var, true),
                scol("note", ColumnType::String, ColumnRole::Var, true),
                scol("at", ColumnType::Date, ColumnRole::Var, false),
                scol(
                    "status",
                    ColumnType::Enum {
                        name: "Status".to_string(),
                        variants: vec!["active".to_string(), "closed".to_string()],
                    },
                    ColumnRole::Var,
                    false,
                ),
                scol("flag", ColumnType::Bool, ColumnRole::Var, false),
            ],
            span: Span::new(0, 0),
        };
        TableType::from_store(&schema)
    }

    fn ty_of(ctx: &Context, src: &str) -> Result<Ty, Vec<TypeError>> {
        let toks = mensura_syntax::tokenize(src).expect("lex");
        let expr = mensura_syntax::parse_expr(&toks).expect("parse");
        type_expr(ctx, &expr)
    }

    fn row_ctx() -> Context {
        Context::row("r", &sample_table())
    }

    fn group_ctx() -> Context {
        Context::group("g", &sample_table())
    }

    #[test]
    fn literals_split_int_and_real() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "42"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "3.5"), Ok(total(ColumnType::Real)));
        assert_eq!(ty_of(&ctx, "\"hi\""), Ok(total(ColumnType::String)));
        assert_eq!(ty_of(&ctx, "true"), Ok(Ty::Bool));
    }

    #[test]
    fn member_reads_columns_with_totality() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.temperature"), Ok(total(ColumnType::Real)));
        assert_eq!(
            ty_of(&ctx, "r.note"),
            Ok(Ty::Value {
                domain: ColumnType::String,
                opt: Optionality::Optional
            })
        );
        assert!(ty_of(&ctx, "r.missing").is_err());
    }

    #[test]
    fn arithmetic_requires_matching_numeric() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.size + 1"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "2 ^ 3"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "-r.size"), Ok(total(ColumnType::Int)));
        assert_eq!(
            ty_of(&ctx, "r.temperature + 1.0"),
            Ok(total(ColumnType::Real))
        );
        // int mixed with real is a type error (no coercion).
        let errs = ty_of(&ctx, "r.size + 1.0").expect_err("mixed");
        assert!(errs[0].message.contains("same type"));
        // arithmetic on a non-number.
        assert!(ty_of(&ctx, "r.machine + 1").is_err());
        // optional operand.
        assert!(ty_of(&ctx, "r.peak + 1.0").is_err());
    }

    #[test]
    fn division_is_real_only() {
        let ctx = row_ctx();
        assert_eq!(
            ty_of(&ctx, "r.temperature / 2.0"),
            Ok(total(ColumnType::Real))
        );
        let errs = ty_of(&ctx, "r.size / 2").expect_err("int division");
        assert!(errs[0].message.contains("real"));
    }

    #[test]
    fn ordering_is_orderable_including_date() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.temperature > 30.0"), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "r.size < 2"), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "r.at < r.at"), Ok(Ty::Bool)); // date is orderable
        let errs = ty_of(&ctx, "r.machine < \"z\"").expect_err("string not orderable");
        assert!(errs[0].message.contains("orderable"));
    }

    #[test]
    fn equality_excludes_real_and_validates_enum() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.size == 1"), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "r.machine == \"m1\""), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "r.status == \"active\""), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "r.at == r.at"), Ok(Ty::Bool));
        let errs = ty_of(&ctx, "r.temperature == 30.0").expect_err("real equality");
        assert!(errs[0].message.contains("not defined on real"));
        let errs = ty_of(&ctx, "r.status == \"activ\"").expect_err("bad variant");
        assert!(errs[0].message.contains("not a variant of `Status`"));
    }

    #[test]
    fn conditional_unifies_branches() {
        let ctx = row_ctx();
        // Both branches a total int.
        assert_eq!(
            ty_of(&ctx, "if true then 1 else 2"),
            Ok(total(ColumnType::Int))
        );
        // A row predicate as the condition.
        assert_eq!(
            ty_of(&ctx, "if r.flag then 1 else 2"),
            Ok(total(ColumnType::Int))
        );
        // An optional branch makes the whole conditional optional.
        assert_eq!(
            ty_of(&ctx, "if r.flag then r.note else \"x\""),
            Ok(Ty::Value {
                domain: ColumnType::String,
                opt: Optionality::Optional
            })
        );
        // Mismatched branch domains.
        let errs = ty_of(&ctx, "if true then 1 else \"x\"").expect_err("branch mismatch");
        assert!(errs[0].message.contains("same type"));
        // A non-boolean condition.
        let errs = ty_of(&ctx, "if 1 then 1 else 2").expect_err("non-bool condition");
        assert!(errs[0].message.contains("boolean"));
    }

    #[test]
    fn aggregates_have_per_domain_signatures() {
        let ctx = group_ctx();
        assert_eq!(ty_of(&ctx, "count g.size"), Ok(total(ColumnType::Int)));
        assert_eq!(ty_of(&ctx, "sum g.size"), Ok(total(ColumnType::Int)));
        assert_eq!(
            ty_of(&ctx, "sum g.temperature"),
            Ok(total(ColumnType::Real))
        );
        assert_eq!(ty_of(&ctx, "min g.at"), Ok(total(ColumnType::Date))); // date is orderable
        assert_eq!(
            ty_of(&ctx, "max g.temperature"),
            Ok(total(ColumnType::Real))
        );
        assert_eq!(ty_of(&ctx, "any g.flag"), Ok(total(ColumnType::Bool)));
        // sum on a non-numeric bag, min on a non-orderable bag.
        assert!(ty_of(&ctx, "sum g.note").is_err());
        assert!(ty_of(&ctx, "min g.note").is_err());
    }

    #[test]
    fn mean_is_not_a_primitive() {
        let ctx = group_ctx();
        // `mean` is gone; it is recovered from sum/count/to_real.
        assert!(ty_of(&ctx, "mean g.temperature").is_err());
        assert_eq!(
            ty_of(&ctx, "sum g.temperature / to_real (count g.temperature)"),
            Ok(total(ColumnType::Real))
        );
    }

    #[test]
    fn aggregates_require_a_total_bag() {
        let ctx = group_ctx();
        let errs = ty_of(&ctx, "sum g.peak").expect_err("optional bag");
        assert!(errs[0].message.contains("total bag"));
    }

    #[test]
    fn to_real_converts_int_value_and_bag() {
        let row = row_ctx();
        assert_eq!(ty_of(&row, "to_real r.size"), Ok(total(ColumnType::Real)));
        let group = group_ctx();
        assert_eq!(
            ty_of(&group, "to_real g.size"),
            Ok(Ty::Bag {
                domain: ColumnType::Real,
                opt: Optionality::Total
            })
        );
        assert!(ty_of(&row, "to_real r.machine").is_err());
    }

    #[test]
    fn presence_is_bool_and_collects_errors() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.note is missing"), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "r.temperature is known"), Ok(Ty::Bool));
        let errs = ty_of(&ctx, "r.bogus + r.note").expect_err("two errors");
        assert_eq!(errs.len(), 2);
    }
}
