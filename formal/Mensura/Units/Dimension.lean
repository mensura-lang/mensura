/-
Physical dimensions: the free abelian group over the seven SI base dimensions
(ADR 0026, `docs/decisions/0026-dimensional-physical-units.md`).

A dimension is an integer exponent vector over the seven base dimensions,
written multiplicatively: dimensions *multiply* (`length * length = length^2`)
while their exponent vectors *add*, and the group identity is the dimensionless
quantity.  Two facts back the checker:

* the commutative-group structure makes `*`, `/`, and integer powers of
  dimensions well defined (the operations the expression checker performs when
  it multiplies, divides, or raises measured columns);
* dimension equality is decidable, which is what turns "a unit mismatch is a
  compile error" into a decision procedure rather than a heuristic.

The seven base dimensions are proved pairwise distinct, so the group is
genuinely of rank seven and no two axes collapse.

This module is standalone: it does not depend on, or change, the table algebra
in `Core/`.  The blueprint chapter "Physical dimensions" records the nodes.
-/

import Mathlib.Algebra.Group.TypeTags.Basic
import Mathlib.Algebra.Group.Pi.Basic
import Mathlib.Data.Fintype.Pi
import Mathlib.Tactic

namespace Mensura.Units

/-- The seven SI base dimensions (ADR 0026).  `luminosity` names luminous
intensity. -/
inductive Base
  | time | length | mass | current | temperature | amount | luminosity
  deriving DecidableEq, Fintype, Repr

/-- A physical dimension: an element of the free abelian group over the seven
base dimensions, written multiplicatively.  Concretely an integer exponent
vector `Base → ℤ`, with dimension multiplication realized as exponent
addition. -/
def Dimension : Type := Multiplicative (Base → ℤ)

namespace Dimension

/-- Dimensions form a commutative group: `*` (compose), `⁻¹` (invert), `/`
(cancel), and integer powers are all well defined.  This is the algebra the
expression checker relies on when combining measured columns. -/
instance : CommGroup Dimension :=
  inferInstanceAs (CommGroup (Multiplicative (Base → ℤ)))

/-- Dimension equality is decidable: the unit-mismatch check is a decision
procedure. -/
instance : DecidableEq Dimension :=
  inferInstanceAs (DecidableEq (Base → ℤ))

instance : Inhabited Dimension := ⟨1⟩

/-- The exponent vector of a dimension: its integer exponent on each base
dimension.  The dimensionless quantity is the constant-zero vector (`1` in the
group). -/
def exponents (d : Dimension) : Base → ℤ := Multiplicative.toAdd d

/-- The base dimension `b` as a `Dimension`: exponent `1` on `b`, `0`
elsewhere. -/
def base (b : Base) : Dimension := Multiplicative.ofAdd (Pi.single b 1)

@[simp] theorem exponents_one (b : Base) : exponents 1 b = 0 := rfl

@[simp] theorem exponents_mul (d e : Dimension) (b : Base) :
    exponents (d * e) b = exponents d b + exponents e b := rfl

@[simp] theorem exponents_inv (d : Dimension) (b : Base) :
    exponents d⁻¹ b = -exponents d b := rfl

@[simp] theorem exponents_div (d e : Dimension) (b : Base) :
    exponents (d / e) b = exponents d b - exponents e b := by
  rw [div_eq_mul_inv, exponents_mul, exponents_inv, sub_eq_add_neg]

theorem exponents_base_self (b : Base) : exponents (base b) b = 1 :=
  Pi.single_eq_same b 1

theorem exponents_base_of_ne {b c : Base} (h : c ≠ b) :
    exponents (base b) c = 0 :=
  Pi.single_eq_of_ne h 1

/-- The seven base dimensions are pairwise distinct, so the group is genuinely
of rank seven: no two axes collapse. -/
theorem base_injective : Function.Injective base := by
  intro b c h
  by_contra hbc
  have hb : exponents (base b) b = 1 := exponents_base_self b
  have hc : exponents (base c) b = 0 := exponents_base_of_ne hbc
  rw [h] at hb
  exact absurd (hb.symm.trans hc) (by norm_num)

/-- A dimension mismatch is decidable, e.g. length is not time. -/
example : base Base.length ≠ base Base.time :=
  fun h => absurd (base_injective h) (by decide)

/-- Derived dimensions compose from the seven.  Acceleration is length per time
squared (time squared written as `time * time`, so the worked exponent facts
below stay inside the multiplication/division lemma set). -/
def acceleration : Dimension :=
  base Base.length / (base Base.time * base Base.time)

/-- Force is mass times acceleration. -/
def force : Dimension := base Base.mass * acceleration

/-- Energy is force times length (`mass * length^2 / time^2`). -/
def energy : Dimension := force * base Base.length

/-- The exponent vector is the canonical form: acceleration is `+1` on length. -/
example : exponents acceleration Base.length = 1 := by
  simp [acceleration, exponents_base_self,
    exponents_base_of_ne (show Base.length ≠ Base.time by decide)]

/-- Acceleration is `-2` on time. -/
example : exponents acceleration Base.time = -2 := by
  simp [acceleration, exponents_base_self,
    exponents_base_of_ne (show Base.time ≠ Base.length by decide)]

end Dimension

end Mensura.Units
