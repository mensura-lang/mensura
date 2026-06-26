//! Value-expression type checking (`docs/language/09-typing-reference.md`
//! section 5, `06-expressions.md`).
//!
//! Types an expression against a row/group context derived from a [`TableType`].
//! Mirrors `resolve`'s contract: it collects all diagnostics rather than failing
//! on the first. Scope is the value sublanguage; the `|>` pipe, general
//! application, lambdas, record/tuple literals, and `is known` narrowing are
//! deferred to later rounds.

use std::collections::BTreeMap;

use mensura_syntax::{Expr, ExprKind, Ident, Span};

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
}
