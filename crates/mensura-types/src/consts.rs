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

use mensura_syntax::{BinOp, Expr, ExprKind, Ident, Span, TypeExpr, UnOp};

use crate::expr_check::{Optionality, Ty};
use crate::model::ColumnType;
use crate::modules::ModuleEnv;
use crate::resolve::ResolveError;
use crate::units::Dimension;

/// A compile-time constant value.  A real carries its magnitude normalized
/// to base units together with its dimension; bare reals carry the group
/// identity.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    Int(i64),
    Bool(bool),
    Str(String),
    Real { magnitude: f64, dim: Dimension },
}

impl ConstValue {
    /// The scalar domain of this constant.
    pub fn domain(&self) -> ColumnType {
        match self {
            ConstValue::Int(_) => ColumnType::Int,
            ConstValue::Bool(_) => ColumnType::Bool,
            ConstValue::Str(_) => ColumnType::String,
            ConstValue::Real { dim, .. } => dim.applied(),
        }
    }

    /// The expression type of this constant: a total single value.
    pub fn ty(&self) -> Ty {
        Ty::Value {
            domain: self.domain(),
            opt: Optionality::Total,
        }
    }

    /// The literal expression node this constant folds to
    /// (`docs/language/12-modules-and-imports.md`: const names are
    /// constant-folded before evaluation, so the runtime never sees them).
    pub fn literal(&self) -> ExprKind {
        match self {
            ConstValue::Int(n) => ExprKind::Int(*n),
            ConstValue::Bool(b) => ExprKind::Bool(*b),
            ConstValue::Str(s) => ExprKind::Str(s.clone()),
            ConstValue::Real { magnitude, .. } => ExprKind::Float(*magnitude),
        }
    }
}

/// One top-level value binding awaiting evaluation.
pub(crate) struct ConstDecl<'a> {
    pub name: &'a Ident,
    pub ty: Option<&'a TypeExpr>,
    pub value: &'a Expr,
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
        errors: Vec::new(),
    };
    for decl in decls {
        let Some(value) = ev.value_of(&decl.name.name, decl.name.span) else {
            continue;
        };
        // Check the optional ascription against the evaluated domain.
        if let Some(ty) = decl.ty {
            match resolve_ascription(ty) {
                Ok(declared) if declared == value.domain() => {}
                Ok(declared) => ev.errors.push(ResolveError::new(
                    format!(
                        "`{}` is declared `{}` but its value is `{}`",
                        decl.name.name,
                        crate::resolve::type_name(&declared),
                        crate::resolve::type_name(&value.domain())
                    ),
                    ty.span(),
                )),
                Err(e) => ev.errors.push(e),
            }
        }
    }
    (ev.done, ev.errors)
}

struct Evaluator<'a> {
    decls: BTreeMap<&'a str, &'a ConstDecl<'a>>,
    modules: &'a BTreeMap<String, &'static ModuleEnv>,
    done: BTreeMap<String, ConstValue>,
    in_progress: Vec<String>,
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
        let value = self.eval(decl.value);
        self.in_progress.pop();
        if let Some(v) = &value {
            self.done.insert(name.to_string(), v.clone());
        }
        value
    }

    fn eval(&mut self, e: &Expr) -> Option<ConstValue> {
        match &e.kind {
            ExprKind::Int(n) => Some(ConstValue::Int(*n)),
            ExprKind::Float(x) => Some(ConstValue::Real {
                magnitude: *x,
                dim: Dimension::DIMENSIONLESS,
            }),
            ExprKind::Str(s) => Some(ConstValue::Str(s.clone())),
            ExprKind::Bool(b) => Some(ConstValue::Bool(*b)),
            ExprKind::Name(name) => self.value_of(name, e.span),
            ExprKind::Member(base, field) => self.member(base, field),
            ExprKind::Unary(UnOp::Neg, operand) => match self.eval(operand)? {
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
                let a = self.eval(lhs);
                let b = self.eval(rhs);
                self.arith(*op, a?, b?, lhs.span, rhs.span)
            }
            _ => self.fail(
                "not a const expression (a literal, a name, or arithmetic over them)",
                e.span,
            ),
        }
    }

    /// `module.member`: the only member access a const expression admits.
    fn member(&mut self, base: &Expr, field: &Ident) -> Option<ConstValue> {
        let ExprKind::Name(module) = &base.kind else {
            return self.fail(
                "member access in a const expression reads a module member (`si.km`)",
                base.span,
            );
        };
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
                    crate::resolve::type_name(&a.domain()),
                    crate::resolve::type_name(&b.domain()),
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
