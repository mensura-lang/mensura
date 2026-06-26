//! Value-expression type checking (`docs/language/09-typing-reference.md`
//! section 5, `06-expressions.md`).
//!
//! Types an expression against a row/group context derived from a [`TableType`].
//! Mirrors `resolve`'s contract: it collects all diagnostics rather than failing
//! on the first. Scope is the value sublanguage; the `|>` pipe, general
//! application, lambdas, record/tuple literals, and `is known` narrowing are
//! deferred to later rounds.

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
    /// A bag at one key, consumable only by combinators (section 5.4).
    Bag { domain: ColumnType },
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

/// The bag builtins in scope (section 5.4). Non-aggregate builtins (`now`,
/// `env`, `lookup`, `prev`) are deferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agg {
    Sum,
    Mean,
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
        let mut names = BTreeMap::new();
        names.insert(param.to_string(), row_record(table));
        Context {
            names,
            aggregates: builtin_aggregates(),
        }
    }

    /// Bind a group lambda's parameter (e.g. `g`) to a record whose fields are
    /// the table's columns as bags (section 5.4).
    pub fn group(param: &str, table: &TableType) -> Context {
        let mut names = BTreeMap::new();
        names.insert(param.to_string(), group_record(table));
        Context {
            names,
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

fn builtin_aggregates() -> BTreeMap<String, Agg> {
    [
        ("sum", Agg::Sum),
        ("mean", Agg::Mean),
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
        let opt = if table.qualifiers.totality.is_total(&col.name) {
            Optionality::Total
        } else {
            Optionality::Optional
        };
        fields.insert(
            col.name.clone(),
            Ty::Value {
                domain: col.domain.clone(),
                opt,
            },
        );
    }
    Ty::Record(fields)
}

/// A group view of a table: every cell is a bag (section 5.4, "`|g|` sees the
/// whole bag at a key").
fn group_record(table: &TableType) -> Ty {
    let mut fields = BTreeMap::new();
    for col in table.content.index.iter().chain(&table.content.columns) {
        fields.insert(
            col.name.clone(),
            Ty::Bag {
                domain: col.domain.clone(),
            },
        );
    }
    Ty::Record(fields)
}

/// Type an expression, collecting all diagnostics (parallels
/// `resolve(&Program) -> Result<Vec<Schema>, Vec<ResolveError>>`).
pub fn type_expr(ctx: &Context, expr: &Expr) -> Result<Ty, Vec<TypeError>> {
    match &expr.kind {
        ExprKind::Int(_) | ExprKind::Float(_) => Ok(Ty::Value {
            domain: ColumnType::Number,
            opt: Optionality::Total,
        }),
        ExprKind::Str(_) => Ok(Ty::Value {
            domain: ColumnType::String,
            opt: Optionality::Total,
        }),
        ExprKind::Bool(_) => Ok(Ty::Bool),
        ExprKind::Name(name) => type_name(ctx, name, expr.span),
        ExprKind::Member(base, field) => type_member(ctx, base, field),
        ExprKind::Binary(op, lhs, rhs) => type_binary(ctx, *op, lhs, rhs, expr.span),
        ExprKind::Unary(op, operand) => type_unary(ctx, *op, operand, expr.span),
        ExprKind::App(func, arg) => type_app(ctx, func, arg, expr.span),
        ExprKind::Presence(base, _) => type_presence(ctx, base, expr.span),
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

/// Application. The only form typed this increment is an aggregate applied to a
/// bag (section 5.4); any other application is deferred.
fn type_app(ctx: &Context, func: &Expr, arg: &Expr, span: Span) -> Result<Ty, Vec<TypeError>> {
    let aggregate = match &func.kind {
        ExprKind::Name(name) => ctx.aggregate(name).map(|agg| (agg, name.as_str())),
        _ => None,
    };
    match aggregate {
        Some((agg, name)) => type_aggregate(ctx, agg, name, arg),
        None => Err(vec![TypeError::new("unsupported in this increment", span)]),
    }
}

/// A bag combinator: it reduces or summarizes a bag to a single known value
/// (section 5.4). The Total result is what lets the value feed a scalar operator
/// (section 5.5).
fn type_aggregate(ctx: &Context, agg: Agg, name: &str, arg: &Expr) -> Result<Ty, Vec<TypeError>> {
    let domain = match type_expr(ctx, arg)? {
        Ty::Bag { domain } => domain,
        _ => {
            return Err(vec![TypeError::new(
                format!("`{name}` expects a bag"),
                arg.span,
            )]);
        }
    };
    match agg {
        Agg::Sum | Agg::Mean | Agg::Min | Agg::Max => {
            if domain == ColumnType::Number {
                Ok(number_total())
            } else {
                Err(vec![TypeError::new(
                    format!(
                        "`{name}` expects a numeric bag, found a bag of {}",
                        domain_name(&domain)
                    ),
                    arg.span,
                )])
            }
        }
        Agg::Count => Ok(number_total()),
        Agg::Any | Agg::All => {
            if domain == ColumnType::Bool {
                Ok(Ty::Value {
                    domain: ColumnType::Bool,
                    opt: Optionality::Total,
                })
            } else {
                Err(vec![TypeError::new(
                    format!(
                        "`{name}` expects a bag of booleans, found a bag of {}",
                        domain_name(&domain)
                    ),
                    arg.span,
                )])
            }
        }
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
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
            let mut errs = require_value(ctx, lhs, &ColumnType::Number, "an arithmetic operator");
            errs.extend(require_value(
                ctx,
                rhs,
                &ColumnType::Number,
                "an arithmetic operator",
            ));
            if errs.is_empty() {
                Ok(number_total())
            } else {
                Err(errs)
            }
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let mut errs = require_value(ctx, lhs, &ColumnType::Number, "a comparison");
            errs.extend(require_value(ctx, rhs, &ColumnType::Number, "a comparison"));
            if errs.is_empty() {
                Ok(Ty::Bool)
            } else {
                Err(errs)
            }
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
        _ => Err(vec![TypeError::new("unsupported in this increment", span)]),
    }
}

/// `value in bag` (section 5.4): the left side must be a known value matching
/// the bag's element domain; the right side must be a bag. Returns `Bool`.
fn type_membership(ctx: &Context, lhs: &Expr, rhs: &Expr) -> Result<Ty, Vec<TypeError>> {
    let mut errs = Vec::new();
    let lt = collect_ty(type_expr(ctx, lhs), &mut errs);
    let rt = collect_ty(type_expr(ctx, rhs), &mut errs);
    let (Some(lt), Some(rt)) = (lt, rt) else {
        return Err(errs);
    };
    let elem = match rt {
        Ty::Bag { domain } => domain,
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

fn type_unary(ctx: &Context, op: UnOp, operand: &Expr, _span: Span) -> Result<Ty, Vec<TypeError>> {
    match op {
        UnOp::Neg => {
            let errs = require_value(ctx, operand, &ColumnType::Number, "negation");
            if errs.is_empty() {
                Ok(number_total())
            } else {
                Err(errs)
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

/// `==` / `!=`: both sides must be known values of the same domain, with the
/// enum-vs-string-literal exception of section 5.6.
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

    match (
        as_known_value(&lt, "a comparison", lhs.span),
        as_known_value(&rt, "a comparison", rhs.span),
    ) {
        (Ok(ld), Ok(rd)) if ld == rd => Ok(Ty::Bool),
        (Ok(ld), Ok(rd)) => Err(vec![TypeError::new(
            format!(
                "a comparison expects matching domains, found a {} and a {}",
                domain_name(&ld),
                domain_name(&rd)
            ),
            lhs.span,
        )]),
        (ld, rd) => {
            if let Err(e) = ld {
                errs.extend(e);
            }
            if let Err(e) = rd {
                errs.extend(e);
            }
            Err(errs)
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

fn number_total() -> Ty {
    Ty::Value {
        domain: ColumnType::Number,
        opt: Optionality::Total,
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

/// Require `operand` to be a single known value of `want` (section 5.3).
fn require_value(ctx: &Context, operand: &Expr, want: &ColumnType, what: &str) -> Vec<TypeError> {
    let ty = match type_expr(ctx, operand) {
        Ok(ty) => ty,
        Err(errs) => return errs,
    };
    match as_known_value(&ty, what, operand.span) {
        Err(errs) => errs,
        Ok(domain) if &domain == want => Vec::new(),
        Ok(domain) => vec![TypeError::new(
            format!(
                "{what} expects a {}, found a {}",
                domain_name(want),
                domain_name(&domain)
            ),
            operand.span,
        )],
    }
}

/// Require `operand` to be a known boolean (section 5.3). Accepts both a
/// predicate result and a total `bool` column read.
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
        ColumnType::Number => "number",
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
                scol("temperature", ColumnType::Number, ColumnRole::Var, false),
                scol("note", ColumnType::String, ColumnRole::Var, true),
                scol("peak", ColumnType::Number, ColumnRole::Var, true),
                scol(
                    "status",
                    ColumnType::Enum {
                        name: "Status".to_string(),
                        variants: vec!["active".to_string(), "closed".to_string()],
                    },
                    ColumnRole::Var,
                    false,
                ),
                scol("readings", ColumnType::Number, ColumnRole::Var, false),
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

    fn num_total() -> Ty {
        Ty::Value {
            domain: ColumnType::Number,
            opt: Optionality::Total,
        }
    }

    #[test]
    fn literals_have_value_types() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "42"), Ok(num_total()));
        assert_eq!(ty_of(&ctx, "3.5"), Ok(num_total()));
        assert_eq!(
            ty_of(&ctx, "\"hi\""),
            Ok(Ty::Value {
                domain: ColumnType::String,
                opt: Optionality::Total
            })
        );
        assert_eq!(ty_of(&ctx, "true"), Ok(Ty::Bool));
    }

    #[test]
    fn param_binds_to_a_row_record() {
        let ctx = row_ctx();
        let Ok(Ty::Record(fields)) = ty_of(&ctx, "r") else {
            panic!("r should be a record");
        };
        assert_eq!(fields["temperature"], num_total());
        assert_eq!(
            fields["note"],
            Ty::Value {
                domain: ColumnType::String,
                opt: Optionality::Optional
            }
        );
    }

    #[test]
    fn unknown_name_errors() {
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "ghost").expect_err("unknown name");
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("unknown name `ghost`"));
    }

    #[test]
    fn member_access_reads_columns() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.temperature"), Ok(num_total()));
        assert_eq!(
            ty_of(&ctx, "r.note"),
            Ok(Ty::Value {
                domain: ColumnType::String,
                opt: Optionality::Optional
            })
        );
    }

    #[test]
    fn unknown_column_errors() {
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "r.missing").expect_err("unknown column");
        assert!(errs[0].message.contains("unknown column `missing`"));
    }

    #[test]
    fn member_on_non_record_errors() {
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "r.temperature.x").expect_err("not a record");
        assert!(errs[0].message.contains("non-record"));
    }

    #[test]
    fn scalar_arithmetic_on_known_numbers() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.temperature + 1"), Ok(num_total()));
        assert_eq!(ty_of(&ctx, "2 ^ 3"), Ok(num_total()));
        assert_eq!(ty_of(&ctx, "-r.temperature"), Ok(num_total()));
    }

    #[test]
    fn scalar_on_optional_is_rejected() {
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "r.peak + 1").expect_err("optional operand");
        assert!(errs[0].message.contains("known value"));
    }

    #[test]
    fn scalar_on_wrong_domain_is_rejected() {
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "r.machine + 1").expect_err("domain mismatch");
        assert!(errs[0].message.contains("number"));
    }

    #[test]
    fn comparisons_and_bools_are_bool() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.temperature > 30"), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "r.status == \"active\""), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "true and false"), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "not true"), Ok(Ty::Bool));
    }

    #[test]
    fn enum_literal_typo_is_rejected() {
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "r.status == \"activ\"").expect_err("bad variant");
        assert!(errs[0].message.contains("not a variant of `Status`"));
    }

    #[test]
    fn comparison_on_optional_is_rejected() {
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "r.peak > 30").expect_err("optional");
        assert!(errs[0].message.contains("known value"));
    }

    #[test]
    fn ordering_is_number_only() {
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "r.machine < \"z\"").expect_err("string ordering");
        assert!(errs[0].message.contains("number"));
    }

    #[test]
    fn group_columns_are_bags() {
        let ctx = group_ctx();
        assert_eq!(
            ty_of(&ctx, "g.readings"),
            Ok(Ty::Bag {
                domain: ColumnType::Number
            })
        );
    }

    #[test]
    fn reducing_aggregates_return_total_values() {
        let ctx = group_ctx();
        assert_eq!(ty_of(&ctx, "mean g.readings"), Ok(num_total()));
        assert_eq!(ty_of(&ctx, "count g.note"), Ok(num_total()));
        // The canonical accept (09 §5.4): an aggregate yields a known value, so
        // the scalar comparison is well-typed.
        assert_eq!(ty_of(&ctx, "mean g.readings > 30"), Ok(Ty::Bool));
    }

    #[test]
    fn any_all_consume_bool_bags() {
        let ctx = group_ctx();
        let bool_total = Ty::Value {
            domain: ColumnType::Bool,
            opt: Optionality::Total,
        };
        assert_eq!(ty_of(&ctx, "any g.flag"), Ok(bool_total.clone()));
        assert_eq!(ty_of(&ctx, "all g.flag"), Ok(bool_total));
    }

    #[test]
    fn scalar_on_a_bag_is_rejected() {
        let ctx = group_ctx();
        let errs = ty_of(&ctx, "g.readings + 1").expect_err("scalar on bag");
        assert!(errs[0].message.contains("found a bag"));
    }

    #[test]
    fn aggregate_domain_is_checked() {
        let ctx = group_ctx();
        let errs = ty_of(&ctx, "mean g.note").expect_err("non-numeric bag");
        assert!(errs[0].message.contains("numeric bag"));
        let errs = ty_of(&ctx, "any g.readings").expect_err("non-bool bag");
        assert!(errs[0].message.contains("booleans"));
    }

    #[test]
    fn membership_tests_a_bag() {
        let ctx = group_ctx();
        assert_eq!(ty_of(&ctx, "30 in g.readings"), Ok(Ty::Bool));
    }

    #[test]
    fn membership_requires_a_bag_and_matching_domain() {
        let ctx = group_ctx();
        let errs = ty_of(&ctx, "\"x\" in g.readings").expect_err("domain mismatch");
        assert!(errs[0].message.contains("`in`"));
        let errs = ty_of(&ctx, "30 in 40").expect_err("rhs not a bag");
        assert!(errs[0].message.contains("expects a bag"));
    }

    #[test]
    fn presence_tests_are_bool() {
        let ctx = row_ctx();
        assert_eq!(ty_of(&ctx, "r.note is missing"), Ok(Ty::Bool));
        assert_eq!(ty_of(&ctx, "r.temperature is known"), Ok(Ty::Bool));
    }

    #[test]
    fn presence_on_a_row_is_rejected() {
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "r is known").expect_err("presence on a row");
        assert!(!errs.is_empty());
    }

    #[test]
    fn deferred_nodes_error_without_panicking() {
        let ctx = row_ctx();
        for src in ["x |> y", "(1, 2)", "(.a = 1)", "{ 1 }", "f r"] {
            let errs = ty_of(&ctx, src).expect_err("deferred node");
            assert!(
                errs[0].message.contains("unsupported"),
                "expected `unsupported` for {src:?}, got {:?}",
                errs[0].message
            );
        }
    }

    #[test]
    fn binary_collects_both_operand_errors() {
        // Like `resolve`, type_expr reports all diagnostics, not just the first:
        // an unknown column on the left and an optional value on the right.
        let ctx = row_ctx();
        let errs = ty_of(&ctx, "r.bogus + r.note").expect_err("two errors");
        assert_eq!(errs.len(), 2);
    }
}
