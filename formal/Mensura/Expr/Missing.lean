/-
Missing-aware expressions (ADR 0039,
`docs/decisions/0039-missing-aware-expressions.md`, decision 5).

A value inside a row is known or missing, never both and never more
(the `Cell = Option` axis of `Mensura/Core/Defs.lean`, ADR 0010).
ADR 0039 lifts every scalar operator over that axis: if any operand is
absent the result is absent, and on present values the lifted operator
agrees with the unlifted one.  The `??` operator is the only discharge.

This module states the laws the checker's lifted typing rules rest on,
over *abstract* operations, so no per-operator proof is ever needed:

* absence absorbs (`lift2_none_left`, `lift2_none_right`),
* the present case agrees with the unlifted operator
  (`lift2_some_some`, `lift1_some`),
* lifting distributes over composition (`lift1_comp`, `lift1_lift2`),
  which is why a compound lifted expression is absent exactly when one
  of its leaves is,
* the discharge laws for `??` (`coalesce_some`, `coalesce_none`) and
  its chain (`coalesceOpt_assoc`, `coalesce_coalesceOpt`): a chain
  discharges at its first present value and is total exactly when its
  final default is.

The torsor and dimension operations of ADR 0026/0036 instantiate the
abstract `f` here; nothing in this module depends on what the scalars
are.  The decision boundaries (ADR 0039 decision 3: `if` conditions,
fold and scan accumulators, keys) are typing demands, not lifted
operations, so they have no statement here.
-/

import Mathlib.Tactic

namespace Mensura.Expr

variable {α β γ δ : Type*}

/-- A unary scalar operation lifted over the missing axis: absent in,
absent out. -/
def lift1 (f : α → β) : Option α → Option β := Option.map f

/-- A binary scalar operation lifted over the missing axis: the result
is present exactly when both operands are, and is then the unlifted
application. -/
def lift2 (f : α → β → γ) : Option α → Option β → Option γ :=
  fun a b => a.bind fun x => b.map (f x)

/-- The discharge `e ?? d` with a total default: the present value, or
the default. -/
def coalesce (e : Option α) (d : α) : α := e.getD d

/-- One step of a discharge chain, `e ?? d` with `d` itself optional:
the result stays optional until a total default ends the chain. -/
def coalesceOpt (e d : Option α) : Option α := e.orElse fun () => d

/-! ### Absence absorbs -/

@[simp] theorem lift1_none (f : α → β) : lift1 f none = none := rfl

@[simp] theorem lift2_none_left (f : α → β → γ) (b : Option β) :
    lift2 f none b = none := rfl

@[simp] theorem lift2_none_right (f : α → β → γ) (a : Option α) :
    lift2 f a none = none := by cases a <;> rfl

/-! ### The present case agrees with the unlifted operator -/

@[simp] theorem lift1_some (f : α → β) (a : α) : lift1 f (some a) = some (f a) := rfl

@[simp] theorem lift2_some_some (f : α → β → γ) (a : α) (b : β) :
    lift2 f (some a) (some b) = some (f a b) := rfl

/-- A lifted binary application is present iff both operands are: the
two absorption laws and the present case, packaged as the statement the
checker's optionality join implements (`join_opt`). -/
theorem lift2_isSome (f : α → β → γ) (a : Option α) (b : Option β) :
    (lift2 f a b).isSome ↔ a.isSome ∧ b.isSome := by
  cases a <;> cases b <;> simp [lift2]

/-! ### Lifting distributes over composition -/

theorem lift1_comp (f : α → β) (g : β → γ) :
    lift1 (g ∘ f) = lift1 g ∘ lift1 f := by
  funext a; cases a <;> rfl

/-- Post-composing a lifted binary application is lifting the composed
operation: a compound lifted expression needs no per-shape rule, its
absence is decided by its leaves. -/
theorem lift1_lift2 (f : α → β → γ) (g : γ → δ) (a : Option α) (b : Option β) :
    lift1 g (lift2 f a b) = lift2 (fun x y => g (f x y)) a b := by
  cases a <;> cases b <;> rfl

/-! ### The discharge laws -/

@[simp] theorem coalesce_some (a : α) (d : α) : coalesce (some a) d = a := rfl

@[simp] theorem coalesce_none (d : α) : coalesce none d = d := rfl

@[simp] theorem coalesceOpt_some (a : α) (d : Option α) :
    coalesceOpt (some a) d = some a := rfl

@[simp] theorem coalesceOpt_none (d : Option α) : coalesceOpt none d = d := rfl

/-- A discharge chain re-associates freely, so `a ?? b ?? c` needs no
grouping rule beyond the parser's right associativity. -/
theorem coalesceOpt_assoc (a b c : Option α) :
    coalesceOpt (coalesceOpt a b) c = coalesceOpt a (coalesceOpt b c) := by
  cases a <;> rfl

/-- Ending a chain with a total default discharges at the first present
value: the optional-default step composes with the final `??`.  That
the composite is total is not a theorem but a type: `coalesce` returns
`α`, never `Option α`, which is decision 2's "total exactly when the
last default is". -/
theorem coalesce_coalesceOpt (a b : Option α) (d : α) :
    coalesce (coalesceOpt a b) d = coalesce a (coalesce b d) := by
  cases a <;> rfl

end Mensura.Expr
