//! Constant lowering: substitute const names in view bodies with their
//! evaluated literals (`docs/language/12-modules-and-imports.md`).
//!
//! Runs after type checking, on the [`crate::model::ViewPlan`] body the
//! runtime re-walks.  Every reference to a top-level const, an intrinsic
//! base unit, or an imported module member is replaced by a literal node
//! carrying the original span, so the runtime evaluator never needs the
//! names in scope.  Only the positions the runtime actually evaluates are
//! rewritten: lambda bodies (and the value expressions inside them).  Op
//! selector arguments (column and source names), op blocks (`assume`,
//! `completeness_check`; runtime identities), and application-spine heads
//! (`sum`, `to_real`, the pipeline ops) are left untouched, mirroring how
//! the checker resolves those positions.

use std::collections::BTreeMap;

use mensura_syntax::{BinOp, Block, Expr, ExprKind, Stmt};

use crate::consts::ConstValue;
use crate::modules::ModuleEnv;

/// The substitution: const names and module members to literal nodes.
pub(crate) struct Subst {
    consts: BTreeMap<String, ExprKind>,
    modules: BTreeMap<String, BTreeMap<String, ExprKind>>,
}

impl Subst {
    pub(crate) fn new(
        consts: &BTreeMap<String, ConstValue>,
        modules: &BTreeMap<String, &'static ModuleEnv>,
    ) -> Subst {
        let mut all_consts: BTreeMap<String, ExprKind> = crate::units::BASE_UNITS
            .iter()
            .map(|u| (u.to_string(), ExprKind::Float(1.0)))
            .collect();
        // A function binding has no literal (ADR 0030): it is skipped here
        // and beta-reduced at its application sites once lowering learns
        // closures; until then a function name survives lowering unchanged.
        all_consts.extend(
            consts
                .iter()
                .filter_map(|(n, v)| Some((n.clone(), v.literal()?))),
        );
        Subst {
            consts: all_consts,
            modules: modules
                .iter()
                .map(|(name, env)| {
                    let members = env
                        .values
                        .iter()
                        .filter_map(|(n, v)| Some((n.clone(), v.literal()?)))
                        .collect();
                    (name.clone(), members)
                })
                .collect(),
        }
    }
}

/// Lower one view body in place.
pub(crate) fn lower_view_body(block: &mut Block, subst: &Subst) {
    for stmt in &mut block.stmts {
        match stmt {
            Stmt::Let { value, .. } | Stmt::Expr(value) | Stmt::Assert(value) => {
                lower_pipeline(value, subst);
            }
        }
    }
}

/// A pipeline-position expression: sources stay names; only operation
/// applications carry expressions to lower.
fn lower_pipeline(e: &mut Expr, s: &Subst) {
    match &mut e.kind {
        ExprKind::Tuple(items) => {
            for item in items {
                lower_pipeline(item, s);
            }
        }
        ExprKind::Binary(BinOp::Pipe, lhs, rhs) => {
            lower_pipeline(lhs, s);
            lower_op(rhs, s);
        }
        ExprKind::App(..) => lower_op(e, s),
        _ => {}
    }
}

/// An operation application spine: the head and the selector/source/block
/// arguments stay; a lambda argument's body is a value expression the
/// runtime evaluates, so it is lowered.
fn lower_op(e: &mut Expr, s: &Subst) {
    if let ExprKind::App(f, arg) = &mut e.kind {
        lower_op(f, s);
        if let ExprKind::Lambda { params, body, .. } = &mut arg.kind {
            let mut shadow: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
            lower_expr(body, s, &mut shadow);
        }
    }
}

/// Rewrite a value expression in place.  `shadow` carries the lexically
/// bound names (lambda parameters, block `let`s): a shadowed name is never
/// substituted, matching the checker's scoping.
fn lower_expr(e: &mut Expr, s: &Subst, shadow: &mut Vec<String>) {
    if let Some(kind) = substitute(e, s, shadow) {
        e.kind = kind;
        return;
    }
    match &mut e.kind {
        ExprKind::Member(base, _) => lower_expr(base, s, shadow),
        ExprKind::App(..) => lower_app(e, s, shadow),
        ExprKind::Binary(BinOp::Pipe, lhs, rhs) => {
            lower_expr(lhs, s, shadow);
            lower_app(rhs, s, shadow);
        }
        ExprKind::Binary(_, lhs, rhs) => {
            lower_expr(lhs, s, shadow);
            lower_expr(rhs, s, shadow);
        }
        ExprKind::Unary(_, operand) => lower_expr(operand, s, shadow),
        ExprKind::Presence(base, _) => lower_expr(base, s, shadow),
        ExprKind::If { cond, then, els } => {
            lower_expr(cond, s, shadow);
            lower_expr(then, s, shadow);
            lower_expr(els, s, shadow);
        }
        ExprKind::Tuple(items) => {
            for item in items {
                lower_expr(item, s, shadow);
            }
        }
        ExprKind::Record(fields) => {
            for field in fields {
                lower_expr(&mut field.value, s, shadow);
            }
        }
        ExprKind::Lambda { params, body, .. } => {
            let depth = shadow.len();
            shadow.extend(params.iter().map(|p| p.name.clone()));
            lower_expr(body, s, shadow);
            shadow.truncate(depth);
        }
        ExprKind::Block(block) => {
            let depth = shadow.len();
            for stmt in &mut block.stmts {
                match stmt {
                    Stmt::Let { name, value, .. } => {
                        lower_expr(value, s, shadow);
                        shadow.push(name.name.clone());
                    }
                    Stmt::Assert(e) | Stmt::Expr(e) => lower_expr(e, s, shadow),
                }
            }
            shadow.truncate(depth);
        }
        _ => {}
    }
}

/// An application spine inside a value expression: the head is a builtin
/// (`sum`, `to_real`, ...) resolved by name at that position, never a
/// const, so it is protected; arguments are ordinary value expressions.
fn lower_app(e: &mut Expr, s: &Subst, shadow: &mut Vec<String>) {
    if let ExprKind::App(f, arg) = &mut e.kind {
        lower_app(f, s, shadow);
        lower_expr(arg, s, shadow);
    }
}

/// The literal a node rewrites to, if it is an unshadowed const name or
/// module member.
fn substitute(e: &Expr, s: &Subst, shadow: &[String]) -> Option<ExprKind> {
    let shadowed = |n: &str| shadow.iter().any(|x| x == n);
    match &e.kind {
        ExprKind::Name(n) if !shadowed(n) => s.consts.get(n).cloned(),
        ExprKind::Member(base, field) => {
            let ExprKind::Name(module) = &base.kind else {
                return None;
            };
            if shadowed(module) {
                return None;
            }
            s.modules.get(module)?.get(&field.name).cloned()
        }
        _ => None,
    }
}
