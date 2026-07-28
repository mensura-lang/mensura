//! Compile-time evaluation of top-level const bindings
//! (`docs/language/12-modules-and-imports.md`, ADR 0027).
//!
//! A top-level `let` names an immutable, pure value.  Bindings are
//! order-independent and non-recursive: evaluation is demand-driven with
//! memoization, and a reference cycle is a diagnostic.  A dimensioned
//! constant (`let km = 1000.0 * meter`) evaluates to its base-unit
//! magnitude plus its dimension, which is how units are ordinary constants
//! (ADR 0026, Decision 7).

use std::collections::BTreeMap;
use std::sync::Arc;

use mensura_syntax::{BinOp, Block, Expr, ExprKind, Ident, Span, Stmt, TypeExpr, UnOp};

use crate::expr_check::{Optionality, Ty};
use crate::model::ColumnType;
use crate::modules::ModuleEnv;
use crate::resolve::ResolveError;
use crate::units::Dimension;

/// A compile-time constant value.  A real carries its magnitude normalized
/// to base units together with its dimension; bare reals carry the group
/// identity.  A closure is a const function (ADR 0030): first-class in the
/// checker, beta-reduced at lowering, never seen by the runtime.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    Int(i64),
    Bool(bool),
    Str(String),
    Real { magnitude: f64, dim: Dimension },
    Closure(Arc<Closure>),
}

/// A const function value (ADR 0030).  The body is owned (cloned out of the
/// AST) because a `ConstValue` outlives the borrowed `Program`: it is
/// returned by value from [`eval_const_bindings`] and, for a bundled
/// module, stored behind a `&'static ModuleEnv`.  The environment is
/// captured **by value** at the point the lambda evaluates, since a
/// closure may escape the block whose local `let`s it references.
///
/// Equality is structural and alpha-sensitive (`|a| a != |b| b`).  No
/// language construct observes const equality; the derive exists for
/// tests.
#[derive(Clone, Debug, PartialEq)]
pub struct Closure {
    /// The parameter list: one name, or n names for the tupled form
    /// `|a, b| e`, which binds a single n-tuple parameter (ADR 0030,
    /// Decision 2).
    pub params: Vec<Ident>,
    pub body: Expr,
    /// The captured locals, innermost last; bound arguments are appended,
    /// so a parameter shadows a captured local (the `Name` lookup searches
    /// innermost-first).
    pub env: Vec<(String, ConstValue)>,
}

impl ConstValue {
    /// The scalar domain of this constant; `None` for a function, which
    /// has no `ColumnType` (functions never enter storage).
    pub fn domain(&self) -> Option<ColumnType> {
        match self {
            ConstValue::Int(_) => Some(ColumnType::Int),
            ConstValue::Bool(_) => Some(ColumnType::Bool),
            ConstValue::Str(_) => Some(ColumnType::String),
            ConstValue::Real { dim, .. } => Some(dim.applied()),
            ConstValue::Closure(_) => None,
        }
    }

    /// A short name for diagnostics: the domain's type name for a scalar,
    /// `function` for a closure.
    pub fn describe(&self) -> String {
        match self.domain() {
            Some(d) => crate::resolve::type_name(&d),
            None => "function".to_string(),
        }
    }

    /// The expression type of this constant: a total single value for a
    /// scalar.  `None` for a function until the checker's function type
    /// lands (ADR 0030); a view body referencing a function binding is an
    /// unknown name meanwhile.
    pub fn ty(&self) -> Option<Ty> {
        Some(Ty::Value {
            domain: self.domain()?,
            opt: Optionality::Total,
        })
    }

    /// The literal expression node this constant folds to
    /// (`docs/language/12-modules-and-imports.md`: const names are
    /// constant-folded before evaluation, so the runtime never sees them).
    /// `None` for a function, which has no literal: a function application
    /// is beta-reduced at lowering instead (ADR 0030, Decision 5).
    pub fn literal(&self) -> Option<ExprKind> {
        match self {
            ConstValue::Int(n) => Some(ExprKind::Int(*n)),
            ConstValue::Bool(b) => Some(ExprKind::Bool(*b)),
            ConstValue::Str(s) => Some(ExprKind::Str(s.clone())),
            ConstValue::Real { magnitude, .. } => Some(ExprKind::Float(*magnitude)),
            ConstValue::Closure(_) => None,
        }
    }
}

/// One top-level value binding awaiting evaluation.  The body is the
/// braced statement block of ADR 0027 Decision 1 (as revised): local
/// `let`s and a trailing result expression.
pub(crate) struct ConstDecl<'a> {
    pub name: &'a Ident,
    pub ty: Option<&'a TypeExpr>,
    pub value: &'a Block,
}

/// Evaluate a set of const bindings against the intrinsics and the imported
/// modules.  `resolve_ascription` resolves a `let name: type = ...`
/// ascription to a domain in the caller's type environment.  Returns every
/// successfully evaluated binding plus all diagnostics.
pub(crate) fn eval_const_bindings(
    decls: &[ConstDecl],
    modules: &BTreeMap<String, &'static ModuleEnv>,
    resolve_ascription: &dyn Fn(&TypeExpr) -> Result<ColumnType, ResolveError>,
) -> (BTreeMap<String, ConstValue>, Vec<ResolveError>) {
    let mut ev = Evaluator {
        decls: decls
            .iter()
            .map(|d| (d.name.name.as_str(), d))
            .collect::<BTreeMap<_, _>>(),
        modules,
        done: BTreeMap::new(),
        in_progress: Vec::new(),
        depth: 0,
        errors: Vec::new(),
    };
    for decl in decls {
        let Some(value) = ev.value_of(&decl.name.name, decl.name.span) else {
            continue;
        };
        // Check the optional ascription against the evaluated domain.  A
        // function binding cannot carry one: the type grammar has no
        // function type to ascribe (ADR 0030).
        if let Some(ty) = decl.ty {
            let Some(domain) = value.domain() else {
                ev.errors.push(ResolveError::new(
                    format!(
                        "`{}` is a function binding and cannot carry a `: type` \
                         ascription (the type grammar has no function type)",
                        decl.name.name,
                    ),
                    ty.span(),
                ));
                continue;
            };
            match resolve_ascription(ty) {
                Ok(declared) if declared == domain => {}
                Ok(declared) => ev.errors.push(ResolveError::new(
                    format!(
                        "`{}` is declared `{}` but its value is `{}`",
                        decl.name.name,
                        crate::resolve::type_name(&declared),
                        crate::resolve::type_name(&domain)
                    ),
                    ty.span(),
                )),
                Err(e) => ev.errors.push(e),
            }
        }
    }
    (ev.done, ev.errors)
}

/// The maximum function-application nesting depth.  The definitional cycle
/// detector cannot catch dynamic recursion (`let f { |x| f x }` evaluates
/// to a closure without touching `f`), and the language has no loop
/// construct, so every const divergence is application-depth divergence.
/// A depth guard is therefore sufficient, and unlike a large step budget
/// it is stack-safe: recursive applications nest `eval_expr` frames, so a
/// budget big enough to be useful would overflow the Rust stack before it
/// fired (ADR 0030, Decision 6).
const MAX_APPLY_DEPTH: u32 = 256;

struct Evaluator<'a> {
    decls: BTreeMap<&'a str, &'a ConstDecl<'a>>,
    modules: &'a BTreeMap<String, &'static ModuleEnv>,
    done: BTreeMap<String, ConstValue>,
    in_progress: Vec<String>,
    /// Current function-application nesting depth; see [`MAX_APPLY_DEPTH`].
    depth: u32,
    errors: Vec<ResolveError>,
}

impl<'a> Evaluator<'a> {
    /// The value of a name: an intrinsic base unit, a memoized binding, or
    /// a binding evaluated on demand (order-independence).  `None` records
    /// a diagnostic.
    fn value_of(&mut self, name: &str, span: Span) -> Option<ConstValue> {
        if let Some(v) = self.done.get(name) {
            return Some(v.clone());
        }
        if let Some(dim) = Dimension::of_base_unit(name) {
            return Some(ConstValue::Real {
                magnitude: 1.0,
                dim,
            });
        }
        let Some(decl) = self.decls.get(name).copied() else {
            if self.modules.contains_key(name) {
                self.errors.push(ResolveError::new(
                    format!("`{name}` is a module, not a value: write `{name}.<member>`"),
                    span,
                ));
            } else {
                self.errors.push(ResolveError::new(
                    format!("unknown name `{name}` in a const expression"),
                    span,
                ));
            }
            return None;
        };
        if self.in_progress.iter().any(|n| n == name) {
            let chain = self.in_progress.join("` -> `");
            self.errors.push(ResolveError::new(
                format!("recursive const binding: `{chain}` -> `{name}`"),
                span,
            ));
            return None;
        }
        self.in_progress.push(name.to_string());
        let value = self.eval_block(decl.value, &mut Vec::new());
        self.in_progress.pop();
        if let Some(v) = &value {
            self.done.insert(name.to_string(), v.clone());
        }
        value
    }

    /// Evaluate a const block: local `let`s (lexically scoped, shadowing
    /// outer names) and a trailing result expression.  `assert` is not yet
    /// supported here (`12-modules-and-imports.md`, "Deferred").
    fn eval_block(
        &mut self,
        block: &Block,
        locals: &mut Vec<(String, ConstValue)>,
    ) -> Option<ConstValue> {
        let depth = locals.len();
        let mut result = None;
        for (i, stmt) in block.stmts.iter().enumerate() {
            let last = i + 1 == block.stmts.len();
            match stmt {
                Stmt::Let { name, value, .. } => {
                    let v = self.eval_expr(value, locals)?;
                    locals.push((name.name.clone(), v));
                }
                Stmt::Assert(e) => {
                    return self.fail(
                        "an `assert` in a const binding is not yet supported",
                        e.span,
                    );
                }
                Stmt::Expr(e) if last => {
                    result = Some(self.eval_expr(e, locals)?);
                }
                Stmt::Expr(e) => {
                    return self.fail(
                        "only the final expression of a const block is its result",
                        e.span,
                    );
                }
            }
        }
        locals.truncate(depth);
        match result {
            Some(v) => Some(v),
            None => self.fail(
                "a const block must end in its result expression",
                block.span,
            ),
        }
    }

    fn eval_expr(
        &mut self,
        e: &Expr,
        locals: &mut Vec<(String, ConstValue)>,
    ) -> Option<ConstValue> {
        match &e.kind {
            ExprKind::Int(n) => Some(ConstValue::Int(*n)),
            ExprKind::Float(x) => Some(ConstValue::Real {
                magnitude: *x,
                dim: Dimension::DIMENSIONLESS,
            }),
            ExprKind::Str(s) => Some(ConstValue::Str(s.clone())),
            ExprKind::Bool(b) => Some(ConstValue::Bool(*b)),
            ExprKind::Name(name) => match locals.iter().rev().find(|(n, _)| n == name) {
                Some((_, v)) => Some(v.clone()),
                None => self.value_of(name, e.span),
            },
            ExprKind::Member(base, field) => self.member(base, field, locals),
            ExprKind::Block(block) => self.eval_block(block, locals),
            ExprKind::Unary(UnOp::Neg, operand) => match self.eval_expr(operand, locals)? {
                ConstValue::Int(n) => match n.checked_neg() {
                    Some(n) => Some(ConstValue::Int(n)),
                    None => self.fail("integer overflow in a const expression", e.span),
                },
                ConstValue::Real { magnitude, dim } => Some(ConstValue::Real {
                    magnitude: -magnitude,
                    dim,
                }),
                _ => self.fail("negation expects a number", operand.span),
            },
            ExprKind::Binary(op, lhs, rhs) => {
                let a = self.eval_expr(lhs, locals);
                let b = self.eval_expr(rhs, locals);
                self.arith(*op, a?, b?, lhs.span, rhs.span)
            }
            ExprKind::Lambda { params, ret, body } => {
                if ret.is_some() {
                    return self.fail(
                        "a return-type ascription on a const lambda is not \
                         supported (the type grammar has no function type)",
                        e.span,
                    );
                }
                if params.is_empty() {
                    return self.fail("a const lambda takes at least one parameter", e.span);
                }
                for (i, p) in params.iter().enumerate() {
                    if params[..i].iter().any(|q| q.name == p.name) {
                        return self
                            .fail(format!("duplicate lambda parameter `{}`", p.name), p.span);
                    }
                }
                Some(ConstValue::Closure(Arc::new(Closure {
                    params: params.clone(),
                    body: (**body).clone(),
                    env: locals.clone(),
                })))
            }
            ExprKind::App(..) => {
                let (head, args) = flatten_app(e);
                let mut f = self.eval_expr(head, locals)?;
                for arg in args {
                    f = self.apply_one(f, arg, head.span, locals)?;
                }
                Some(f)
            }
            _ => self.fail(
                "not a const expression (a literal, a name, a lambda, or \
                 arithmetic and application over them)",
                e.span,
            ),
        }
    }

    /// Apply one argument to a const function (ADR 0030, Decision 3): the
    /// application is saturated or an error.  A one-parameter closure binds
    /// any value; a tupled closure of n parameters requires a syntactic
    /// n-tuple.  The body evaluates in the *captured* environment plus the
    /// bound parameters, never in the caller's locals (lexical scoping).
    fn apply_one(
        &mut self,
        f: ConstValue,
        arg: &Expr,
        head_span: Span,
        locals: &mut Vec<(String, ConstValue)>,
    ) -> Option<ConstValue> {
        let ConstValue::Closure(c) = f else {
            return self.fail(
                format!(
                    "cannot apply a value of type `{}`: it is not a function",
                    f.describe()
                ),
                head_span,
            );
        };
        let mut env = c.env.clone();
        match c.params.len() {
            1 => {
                let v = self.eval_expr(arg, locals)?;
                env.push((c.params[0].name.clone(), v));
            }
            n => match &arg.kind {
                ExprKind::Tuple(items) if items.len() == n => {
                    for (p, item) in c.params.iter().zip(items) {
                        let v = self.eval_expr(item, locals)?;
                        env.push((p.name.clone(), v));
                    }
                }
                ExprKind::Tuple(items) => {
                    return self.fail(
                        format!("expects a tuple of {n} values, found {}", items.len()),
                        arg.span,
                    );
                }
                _ => {
                    return self.fail(
                        format!(
                            "expects a tuple of {n} values (a multi-parameter \
                             lambda is tupled; currying is written `|a| |b| ...`)"
                        ),
                        arg.span,
                    );
                }
            },
        }
        if self.depth >= MAX_APPLY_DEPTH {
            return self.fail(
                "const evaluation exceeded the application depth limit: \
                 a const function may be recursive",
                head_span,
            );
        }
        self.depth += 1;
        let result = self.eval_expr(&c.body, &mut env);
        self.depth -= 1;
        result
    }

    /// `module.member`: the only member access a const expression admits.
    fn member(
        &mut self,
        base: &Expr,
        field: &Ident,
        locals: &[(String, ConstValue)],
    ) -> Option<ConstValue> {
        let ExprKind::Name(module) = &base.kind else {
            return self.fail(
                "member access in a const expression reads a module member (`si.km`)",
                base.span,
            );
        };
        if locals.iter().any(|(n, _)| n == module) {
            return self.fail(
                "member access in a const expression reads a module member (`si.km`)",
                base.span,
            );
        }
        let Some(env) = self.modules.get(module) else {
            return self.fail(
                format!("unknown module `{module}` in a const expression"),
                base.span,
            );
        };
        match env.values.get(&field.name) {
            Some(v) => Some(v.clone()),
            None => self.fail(
                format!("module `{module}` has no member `{}`", field.name),
                field.span,
            ),
        }
    }

    /// The arithmetic rules of the expression checker, mirrored over
    /// values: strict `int`/`real` separation (ADR 0014) and the ADR 0026
    /// dimension rules.  The one liberalization: a dimensioned base may be
    /// raised to any const `int` (not only a literal), since a const
    /// exponent is compile-time known by construction.
    fn arith(
        &mut self,
        op: BinOp,
        a: ConstValue,
        b: ConstValue,
        lspan: Span,
        rspan: Span,
    ) -> Option<ConstValue> {
        use ConstValue::{Int, Real};
        match (op, a, b) {
            (BinOp::Add | BinOp::Sub | BinOp::Mul, Int(x), Int(y)) => {
                let out = match op {
                    BinOp::Add => x.checked_add(y),
                    BinOp::Sub => x.checked_sub(y),
                    _ => x.checked_mul(y),
                };
                match out {
                    Some(v) => Some(Int(v)),
                    None => self.fail("integer overflow in a const expression", lspan),
                }
            }
            (BinOp::Pow, Int(x), Int(y)) => {
                match u32::try_from(y).ok().and_then(|e| x.checked_pow(e)) {
                    Some(v) => Some(Int(v)),
                    None => self.fail(
                        "`^` on const ints requires a non-negative exponent and must not overflow",
                        rspan,
                    ),
                }
            }
            (BinOp::Div, Int(_), Int(_)) => self.fail("`/` is real-only (ADR 0014)", lspan),
            (
                BinOp::Add | BinOp::Sub,
                Real {
                    magnitude: x,
                    dim: dx,
                },
                Real {
                    magnitude: y,
                    dim: dy,
                },
            ) => {
                if dx != dy {
                    return self.fail(
                        format!(
                            "cannot {} `{}` and `{}`: the dimensions differ",
                            if op == BinOp::Add { "add" } else { "subtract" },
                            dx.applied_name(),
                            dy.applied_name(),
                        ),
                        lspan,
                    );
                }
                Some(Real {
                    magnitude: if op == BinOp::Add { x + y } else { x - y },
                    dim: dx,
                })
            }
            (
                BinOp::Mul,
                Real {
                    magnitude: x,
                    dim: dx,
                },
                Real {
                    magnitude: y,
                    dim: dy,
                },
            ) => Some(Real {
                magnitude: x * y,
                dim: dx * dy,
            }),
            (
                BinOp::Div,
                Real {
                    magnitude: x,
                    dim: dx,
                },
                Real {
                    magnitude: y,
                    dim: dy,
                },
            ) => Some(Real {
                magnitude: x / y,
                dim: dx / dy,
            }),
            (BinOp::Pow, Real { magnitude: x, dim }, Int(n)) => {
                let Ok(exp) = i32::try_from(n) else {
                    return self.fail("the exponent of a dimensioned value is out of range", rspan);
                };
                Some(Real {
                    magnitude: x.powi(exp),
                    dim: dim.pow(exp),
                })
            }
            (
                BinOp::Pow,
                Real { magnitude: x, dim },
                Real {
                    magnitude: y,
                    dim: dy,
                },
            ) if dim.is_dimensionless() && dy.is_dimensionless() => Some(Real {
                magnitude: x.powf(y),
                dim,
            }),
            (op, a, b) => self.fail(
                format!(
                    "`{}` is not defined on `{}` and `{}` in a const expression",
                    op_name(op),
                    a.describe(),
                    b.describe(),
                ),
                lspan,
            ),
        }
    }

    fn fail(&mut self, message: impl Into<String>, span: Span) -> Option<ConstValue> {
        self.errors.push(ResolveError::new(message, span));
        None
    }
}

/// Flatten an application spine: `f x y` yields `(f, [x, y])`.  A local
/// mirror of the checker's; the const evaluator and the checker stay
/// decoupled (`docs/toolkit/01-application-checking.md`).
fn flatten_app(e: &Expr) -> (&Expr, Vec<&Expr>) {
    let mut args = Vec::new();
    let mut cur = e;
    while let ExprKind::App(f, a) = &cur.kind {
        args.push(a.as_ref());
        cur = f.as_ref();
    }
    args.reverse();
    (cur, args)
}

fn op_name(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Pow => "^",
        _ => "operator",
    }
}

impl Dimension {
    /// `type_name` for prose diagnostics: `real` when dimensionless.
    fn applied_name(&self) -> String {
        if self.is_dimensionless() {
            "real".to_string()
        } else {
            self.type_name()
        }
    }
}
