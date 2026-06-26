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

use mensura_syntax::{BinOp, Expr, ExprKind, Span};

use crate::expr_check::TypeError;
use crate::table::TableType;

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
    tables: BTreeMap<String, TableType>,
}

impl Sources {
    pub fn new() -> Self {
        Sources::default()
    }

    pub fn with(mut self, name: &str, table: TableType) -> Self {
        self.tables.insert(name.to_string(), table);
        self
    }

    fn get(&self, name: &str) -> Option<&TableType> {
        self.tables.get(name)
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
            Some(table) => Ok(PipeTy::Table(table.clone())),
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
fn apply_op(_sources: &Sources, input: PipeTy, op_expr: &Expr) -> Result<PipeTy, Vec<TypeError>> {
    let (head, args) = flatten_app(op_expr);
    let ExprKind::Name(op) = &head.kind else {
        return Err(error("expected a pipeline operation", op_expr.span));
    };
    match op.as_str() {
        "extend_key" => op_extend_key(input, &args, head.span),
        _ => Err(error(format!("unsupported operation `{op}`"), head.span)),
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
                scol("ts", ColumnType::Number, ColumnRole::Index, false),
                scol("machine", ColumnType::String, ColumnRole::Var, false),
                scol("temperature", ColumnType::Number, ColumnRole::Var, false),
                scol("peak", ColumnType::Number, ColumnRole::Var, true),
                scol("flag", ColumnType::Bool, ColumnRole::Var, false),
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
        let errs = pipe_ty(&s, "readings |> extend_key peak").expect_err("optional");
        assert!(errs[0].message.contains("to be total"));
    }

    #[test]
    fn extend_key_unknown_column_errors() {
        let s = sample_sources();
        let errs = pipe_ty(&s, "readings |> extend_key bogus").expect_err("unknown column");
        assert!(errs[0].message.contains("unknown column `bogus`"));
    }
}
