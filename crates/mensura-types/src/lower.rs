//! Constant lowering: substitute const names in view bodies with their
//! evaluated literals, and beta-reduce const function applications
//! (`docs/language/12-modules-and-imports.md`, ADR 0030).
//!
//! Runs after type checking, on the [`crate::model::ViewPlan`] body the
//! runtime re-walks.  Every reference to a top-level const, an intrinsic
//! base unit, or an imported module member is replaced by a literal node
//! carrying the original span, and every application of a const function
//! is replaced by its substituted body (`add1 r.x` becomes `r.x + 1`), so
//! the runtime evaluator never sees a const name or a function.  This is
//! sound and total because only a const binding can create a function
//! (ADR 0030, Decision 5), so every closure at every call site is
//! statically known.  Only the positions the runtime actually evaluates
//! are rewritten: lambda bodies (and the value expressions inside them).
//! Op selector arguments (column and source names), op blocks (`assume`,
//! `completeness_check`; runtime identities), and builtin spine heads
//! (`sum`, `to_real`, the pipeline ops) are left untouched, mirroring how
//! the checker resolves those positions.

use std::collections::BTreeMap;
use std::sync::Arc;

use mensura_syntax::{BinOp, Block, Expr, ExprKind, Stmt};

use crate::consts::{Closure, ConstValue};
use crate::modules::ModuleEnv;

/// The substitution: const names and module members to literal nodes, and
/// const function names to their closures (ADR 0030).
pub(crate) struct Subst {
    consts: BTreeMap<String, ExprKind>,
    modules: BTreeMap<String, BTreeMap<String, ExprKind>>,
    closures: BTreeMap<String, Arc<Closure>>,
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
        let mut closures = BTreeMap::new();
        // A scalar binding folds to its literal; a function binding has no
        // literal and is recorded for beta-reduction at its application
        // sites instead (ADR 0030, Decision 5).
        for (n, v) in consts {
            match v {
                ConstValue::Closure(c) => {
                    closures.insert(n.clone(), c.clone());
                }
                _ => {
                    all_consts.extend(v.literal().map(|l| (n.clone(), l)));
                }
            }
        }
        Subst {
            consts: all_consts,
            // No bundled module exports a function (ADR 0030, Decision 8),
            // so module members are scalars only.
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
            closures,
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
            let mut env: Vec<Binding> = params
                .iter()
                .map(|p| Binding::Shadow(p.name.clone()))
                .collect();
            let mut fresh = 0;
            reduce(body, s, &mut env, 0, &mut fresh);
        }
    }
}

/// One lexical entry during reduction: a name is either **shadowed** (bound
/// at runtime by a pipeline-lambda parameter, an inner lambda, or a block
/// `let`; never substituted) or **substituted** by an expression (a bound
/// const-function parameter or a captured value, ADR 0030).
enum Binding {
    Shadow(String),
    Subst(String, Expr),
}

impl Binding {
    fn name(&self) -> &str {
        match self {
            Binding::Shadow(n) | Binding::Subst(n, _) => n,
        }
    }
}

/// The reduction depth backstop.  The checker's own depth guard has
/// already accepted the program, so the chain terminates; past this cap
/// the spine is left unreduced (the runtime then reports it) rather than
/// recursing without bound.
const MAX_REDUCE_DEPTH: u32 = 512;

/// Rewrite a value expression in place: substitute const names and module
/// members with their literals, and beta-reduce const function
/// applications (ADR 0030, Decision 5).
fn reduce(e: &mut Expr, s: &Subst, env: &mut Vec<Binding>, depth: u32, fresh: &mut u32) {
    match &e.kind {
        ExprKind::Name(n) => {
            match env.iter().rev().find(|b| b.name() == n) {
                Some(Binding::Shadow(_)) => {}
                Some(Binding::Subst(_, x)) => {
                    // The payload was reduced before it was bound; do not
                    // re-reduce it.
                    *e = x.clone();
                }
                None => {
                    if let Some(kind) = s.consts.get(n) {
                        e.kind = kind.clone();
                    } else if let Some(c) = s.closures.get(n) {
                        // A function passed as an argument (`twice add3 x`)
                        // reifies to its lambda; a function in head
                        // position is consumed by the `App` arm instead.
                        *e = reify(&c.clone(), s, e.span, depth, fresh);
                    }
                }
            }
            return;
        }
        ExprKind::Member(base, field) => {
            // A module member (`si.km`) folds to its literal when the base
            // is an unshadowed module name; anything else recurses.
            if let ExprKind::Name(module) = &base.kind
                && !env.iter().any(|b| b.name() == module)
                && let Some(kind) = s.modules.get(module).and_then(|m| m.get(&field.name))
            {
                e.kind = kind.clone();
                return;
            }
        }
        _ => {}
    }
    match &mut e.kind {
        ExprKind::Member(base, _) => reduce(base, s, env, depth, fresh),
        ExprKind::App(..) => reduce_spine(e, s, env, depth, fresh, None),
        ExprKind::Binary(BinOp::Pipe, lhs, rhs) => {
            // `x |> f a` is `f a x` (ADR 0018).  When `f` is a const
            // function the whole pipe reduces with the input as the
            // trailing argument; a builtin pipe (`x |> sum`) keeps its
            // shape and only the operands are reduced.
            reduce(lhs, s, env, depth, fresh);
            if spine_head_is_function(rhs, s, env) {
                let input = take(lhs);
                let mut spine = take(rhs);
                reduce_spine(&mut spine, s, env, depth, fresh, Some(input));
                *e = spine;
            } else if let ExprKind::App(..) = &rhs.kind {
                reduce_spine(rhs, s, env, depth, fresh, None);
            }
        }
        ExprKind::Binary(_, lhs, rhs) => {
            reduce(lhs, s, env, depth, fresh);
            reduce(rhs, s, env, depth, fresh);
        }
        ExprKind::Unary(_, operand) => reduce(operand, s, env, depth, fresh),
        ExprKind::Presence(base, _) => reduce(base, s, env, depth, fresh),
        ExprKind::If { cond, then, els } => {
            reduce(cond, s, env, depth, fresh);
            reduce(then, s, env, depth, fresh);
            reduce(els, s, env, depth, fresh);
        }
        ExprKind::Tuple(items) => {
            for item in items {
                reduce(item, s, env, depth, fresh);
            }
        }
        ExprKind::Record(fields) => {
            for field in fields {
                reduce(&mut field.value, s, env, depth, fresh);
            }
        }
        ExprKind::Lambda { params, body, .. } => {
            let base = env.len();
            shadow_params(params, env, fresh);
            reduce(body, s, env, depth, fresh);
            env.truncate(base);
        }
        ExprKind::Block(block) => {
            let base = env.len();
            for stmt in &mut block.stmts {
                match stmt {
                    Stmt::Let { name, value, .. } => {
                        reduce(value, s, env, depth, fresh);
                        env.push(Binding::Shadow(name.name.clone()));
                    }
                    Stmt::Assert(inner) | Stmt::Expr(inner) => reduce(inner, s, env, depth, fresh),
                }
            }
            env.truncate(base);
        }
        _ => {}
    }
}

/// Shadow a binder's parameters, alpha-renaming any that would capture a
/// free name of an active substitution payload: substituting `r.col` under
/// a `|r| ...` binder must not capture the caller's `r`, so the binder is
/// renamed (via the substitution mechanism itself) and the payload's `r`
/// stays free.
fn shadow_params(params: &mut [mensura_syntax::Ident], env: &mut Vec<Binding>, fresh: &mut u32) {
    for p in params {
        let captures = env.iter().any(|b| match b {
            Binding::Subst(_, payload) => mentions_name(payload, &p.name),
            Binding::Shadow(_) => false,
        });
        if captures {
            *fresh += 1;
            let renamed = format!("{}__{fresh}", p.name);
            env.push(Binding::Subst(
                p.name.clone(),
                Expr {
                    kind: ExprKind::Name(renamed.clone()),
                    span: p.span,
                },
            ));
            env.push(Binding::Shadow(renamed.clone()));
            p.name = renamed;
        } else {
            env.push(Binding::Shadow(p.name.clone()));
        }
    }
}

/// Whether `name` occurs as a free-ish name anywhere in `e` (conservative:
/// any `Name` occurrence counts, binders included).
fn mentions_name(e: &Expr, name: &str) -> bool {
    match &e.kind {
        ExprKind::Name(n) => n == name,
        ExprKind::Member(base, _) => mentions_name(base, name),
        ExprKind::App(f, a) => mentions_name(f, name) || mentions_name(a, name),
        ExprKind::Binary(_, l, r) => mentions_name(l, name) || mentions_name(r, name),
        ExprKind::Unary(_, x) | ExprKind::Presence(x, _) => mentions_name(x, name),
        ExprKind::If { cond, then, els } => {
            mentions_name(cond, name) || mentions_name(then, name) || mentions_name(els, name)
        }
        ExprKind::Tuple(items) => items.iter().any(|i| mentions_name(i, name)),
        ExprKind::Record(fields) => fields.iter().any(|f| mentions_name(&f.value, name)),
        ExprKind::Lambda { body, .. } => mentions_name(body, name),
        ExprKind::Block(block) => block.stmts.iter().any(|stmt| match stmt {
            Stmt::Let { value, .. } | Stmt::Assert(value) | Stmt::Expr(value) => {
                mentions_name(value, name)
            }
        }),
        _ => false,
    }
}

/// Whether an expression's application-spine head is a const function (an
/// env-bound lambda payload or a top-level closure), i.e. whether a pipe
/// into it merges into a reduction.
fn spine_head_is_function(e: &Expr, s: &Subst, env: &[Binding]) -> bool {
    let mut cur = e;
    while let ExprKind::App(f, _) = &cur.kind {
        cur = f;
    }
    match &cur.kind {
        ExprKind::Name(n) => match env.iter().rev().find(|b| b.name() == n) {
            Some(Binding::Subst(_, payload)) => matches!(payload.kind, ExprKind::Lambda { .. }),
            Some(Binding::Shadow(_)) => false,
            None => s.closures.contains_key(n),
        },
        ExprKind::Lambda { .. } => true,
        _ => false,
    }
}

/// Reduce an application spine in place.  The head resolves to a function
/// (an env-bound lambda, a top-level const function, or a lambda literal
/// produced by a curried step); a builtin head (`sum`, `to_real`, the
/// pipeline ops) is left in place with its arguments reduced.  `piped`
/// carries a `|>` input as the trailing argument (ADR 0018).
fn reduce_spine(
    e: &mut Expr,
    s: &Subst,
    env: &mut Vec<Binding>,
    depth: u32,
    fresh: &mut u32,
    piped: Option<Expr>,
) {
    let spine = take(e);
    let (mut head, mut args) = flatten_owned(spine);
    for arg in &mut args {
        reduce(arg, s, env, depth, fresh);
    }
    if let Some(input) = piped {
        args.push(input); // already reduced by the caller
    }
    // Resolve the head to a lambda, if it is a const function.  A builtin
    // name stays a name; the checker has already vetted it.
    let fun: Option<Expr> = match &head.kind {
        ExprKind::Name(n) => match env.iter().rev().find(|b| b.name() == n) {
            Some(Binding::Subst(_, payload)) if matches!(payload.kind, ExprKind::Lambda { .. }) => {
                Some(payload.clone())
            }
            Some(_) => None,
            None => s
                .closures
                .get(n)
                .map(|c| reify(&c.clone(), s, head.span, depth, fresh)),
        },
        ExprKind::Lambda { .. } => {
            reduce(&mut head, s, env, depth, fresh);
            Some(head.clone())
        }
        _ => None,
    };
    match fun {
        Some(lambda) if depth < MAX_REDUCE_DEPTH => {
            *e = apply_fun(lambda, args, s, depth + 1, fresh);
        }
        _ => *e = rebuild_app(head, args),
    }
}

/// Apply a lambda expression to already-reduced arguments, one saturated
/// step at a time (a curried function's step yields the next lambda).  The
/// checker accepted the program, so a non-lambda intermediate or a tuple
/// mismatch is unreachable; both fall back to an unreduced rebuild that
/// the runtime reports.
fn apply_fun(fun: Expr, args: Vec<Expr>, s: &Subst, depth: u32, fresh: &mut u32) -> Expr {
    let mut fun = fun;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let ExprKind::Lambda { params, body, .. } = fun.kind else {
            return rebuild_app(fun, std::iter::once(arg).chain(args).collect());
        };
        let mut env: Vec<Binding> = Vec::new();
        if params.len() == 1 {
            env.push(Binding::Subst(params[0].name.clone(), arg));
        } else if let ExprKind::Tuple(items) = arg.kind {
            for (p, item) in params.iter().zip(items) {
                env.push(Binding::Subst(p.name.clone(), item));
            }
        } else {
            let restored = Expr {
                kind: ExprKind::Lambda {
                    params,
                    ret: None,
                    body,
                },
                span: fun.span,
            };
            return rebuild_app(restored, args.collect());
        }
        let mut next = *body;
        reduce(&mut next, s, &mut env, depth, fresh);
        fun = next;
    }
    fun
}

/// A const closure as a lambda expression: the captured environment is
/// substituted into the body (scalars as literals, captured functions
/// reified recursively), which is total because captures are the values of
/// completed, non-recursive bindings.
fn reify(c: &Closure, s: &Subst, span: mensura_syntax::Span, depth: u32, fresh: &mut u32) -> Expr {
    let mut env: Vec<Binding> = c
        .env
        .iter()
        .map(|(n, v)| {
            let payload = match v {
                ConstValue::Closure(inner) => reify(inner, s, span, depth, fresh),
                scalar => Expr {
                    kind: scalar.literal().expect("a non-closure const is a scalar"),
                    span,
                },
            };
            Binding::Subst(n.clone(), payload)
        })
        .collect();
    let mut params = c.params.clone();
    let base = env.len();
    shadow_params(&mut params, &mut env, fresh);
    let mut body = c.body.clone();
    reduce(&mut body, s, &mut env, depth, fresh);
    env.truncate(base);
    Expr {
        kind: ExprKind::Lambda {
            params,
            ret: None,
            body: Box::new(body),
        },
        span,
    }
}

/// Detach an expression, leaving an inert placeholder behind.
fn take(e: &mut Expr) -> Expr {
    let span = e.span;
    std::mem::replace(
        e,
        Expr {
            kind: ExprKind::Bool(false),
            span,
        },
    )
}

/// Flatten an owned application spine: `f x y` yields `(f, [x, y])`.
fn flatten_owned(e: Expr) -> (Expr, Vec<Expr>) {
    let mut args = Vec::new();
    let mut cur = e;
    while let ExprKind::App(f, a) = cur.kind {
        args.push(*a);
        cur = *f;
    }
    args.reverse();
    (cur, args)
}

/// Rebuild an application spine from a head and arguments.
fn rebuild_app(head: Expr, args: Vec<Expr>) -> Expr {
    let mut cur = head;
    for arg in args {
        let span = mensura_syntax::Span::new(cur.span.start, arg.span.end);
        cur = Expr {
            kind: ExprKind::App(Box::new(cur), Box::new(arg)),
            span,
        };
    }
    cur
}
