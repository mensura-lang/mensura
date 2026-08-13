/-
Torsor arithmetic for temporal point domains (ADR 0036,
`docs/decisions/0036-temporal-domains-and-torsor-arithmetic.md`, decision 9).

A point domain `P` with difference group `G` is an additive torsor: a free and
transitive action of `G` on `P`.  The two operation families of ADR 0036
decision 4 are the action and its inverse (`+ᵥ` is translation, `-ᵥ` is
difference), and the laws the checker relies on *follow* from the structure
rather than being stipulated:

* `t + (u - t) = u` (`Torsor.translate_vsub`),
* `(t - u) + (u - v) = t - v` (`Torsor.vsub_add_vsub`),
* `t - t = 0` (`Torsor.vsub_self`),
* uniqueness of the difference (`Torsor.vsub_unique`).

The structure is stated over an abstract difference group, so neither a later
exact backing (ADR 0026 decision 9) nor the deferred civil instantiation
(`diff(date)`, ADR 0036 decision 4) reopens the proofs.

The concrete instantiation is `Instant`, the absolute temporal point on the
exact millisecond grid, over `Duration`, the exact millisecond count whose
physical dimension is the `time` axis of the dimension group (ADR 0026,
`Mensura.Units.Dimension`).  The checker-level difference type is
`time[real]`; this model keeps the exact grid, so the laws hold with no
floating-point caveat, and the bridge to the binary64 backing is the
implementation obligation ADR 0036 decision 6 states (the round-trip bound),
not a formal target.

Order compatibility (decision 9 item 3) is the load-bearing extra:
translation by a positive duration is strictly increasing.  ADR 0037's
interval test `w <= p < w + size` and its `closedWindow_stable` gate both
rest on it, which is why this module lands before the window lemmas.
-/

import Mathlib.Algebra.Torsor.Defs
import Mathlib.Order.Basic
import Mathlib.Tactic
import Mensura.Units.Dimension

namespace Mensura.Units

/-! ### The abstract torsor laws (ADR 0036 decision 4)

Mathlib's `AddTorsor G P` *is* the structure decision 9 item 1 names: a free
and transitive additive action of the difference group on the point domain.
The four laws below are the ADR's reading of the typing rules; each is a
one-line consequence of the structure, which is what makes "torsor" the right
word in the ADR's title rather than decoration. -/

namespace Torsor

variable {G : Type*} {P : Type*} [AddGroup G] [AddTorsor G P]

/-- Translating `t` by the difference `u - t` reaches `u`: the round-trip
property ADR 0036 decision 6 requires of the implementation
(`t + (u - t) = u`). -/
theorem translate_vsub (t u : P) : (u -ᵥ t : G) +ᵥ t = u := vsub_vadd u t

/-- Differences compose: `(t - u) + (u - v) = t - v`. -/
theorem vsub_add_vsub (t u v : P) : (t -ᵥ u : G) + (u -ᵥ v) = t -ᵥ v :=
  vsub_add_vsub_cancel t u v

/-- A point's difference with itself is the zero duration: `t - t = 0`. -/
theorem vsub_self (t : P) : (t -ᵥ t : G) = 0 := _root_.vsub_self t

/-- Uniqueness of the difference: `u - t` is the *only* translation carrying
`t` to `u`.  This is freeness and transitivity in one statement, and it is
what makes `-ᵥ` well defined as the inverse of `+ᵥ`. -/
theorem vsub_unique {g : G} {t u : P} (h : g +ᵥ t = u) : g = u -ᵥ t := by
  rw [← h, vadd_vsub]

end Torsor

/-! ### The millisecond grid (ADR 0036 decision 9 item 2) -/

/-- A duration on the exact millisecond grid: an integer count of
milliseconds, the difference group of `Instant`.  Its physical dimension is
the `time` axis (`Duration.dimension`), which is how the torsor extends the
dimension group of ADR 0026 rather than sitting beside it. -/
def Duration : Type := ℤ

namespace Duration

instance : AddCommGroup Duration := inferInstanceAs (AddCommGroup ℤ)

instance : LinearOrder Duration := inferInstanceAs (LinearOrder ℤ)

instance : Inhabited Duration := ⟨(0 : ℤ)⟩

/-- The millisecond count of a duration. -/
def ms (d : Duration) : ℤ := d

/-- The duration of `n` milliseconds. -/
def ofMs (n : ℤ) : Duration := n

@[simp] theorem ms_ofMs (n : ℤ) : (ofMs n).ms = n := rfl

@[simp] theorem ms_zero : (0 : Duration).ms = 0 := rfl

@[simp] theorem ms_add (d e : Duration) : (d + e).ms = d.ms + e.ms := rfl

@[simp] theorem ms_neg (d : Duration) : (-d).ms = -d.ms := rfl

@[simp] theorem ms_sub (d e : Duration) : (d - e).ms = d.ms - e.ms := rfl

theorem lt_iff_ms_lt {d e : Duration} : d < e ↔ d.ms < e.ms := Iff.rfl

theorem pos_iff_ms_pos {d : Duration} : 0 < d ↔ 0 < d.ms := Iff.rfl

theorem ms_injective : Function.Injective ms := fun _ _ h ↦ h

/-- The physical dimension a duration carries at the type level: `time`.
The checker's difference type for `instant` is `time[real]` (ADR 0036
decision 4); this constant is the hook from the exact grid into the dimension
group of ADR 0026. -/
def dimension : Dimension := Dimension.base Base.time

end Duration

/-- An absolute temporal point on the exact millisecond grid (ADR 0036
decision 2): the count of milliseconds from the epoch of decision 5
(`1970-01-01T00:00:00.000Z`).  The epoch is a grid anchor and nothing else.

A structure rather than an integer synonym, deliberately: points are not
quantities (decision 3), so no numeric operation is available on `Instant`
itself, and every operation flows through the torsor interface below. -/
@[ext]
structure Instant where
  /-- Milliseconds since the epoch. -/
  msSinceEpoch : ℤ
  deriving DecidableEq

namespace Instant

instance : Inhabited Instant := ⟨⟨0⟩⟩

instance : LinearOrder Instant :=
  LinearOrder.lift' msSinceEpoch fun _ _ h ↦ Instant.ext h

theorem le_iff_ms_le {t u : Instant} : t ≤ u ↔ t.msSinceEpoch ≤ u.msSinceEpoch := Iff.rfl

theorem lt_iff_ms_lt {t u : Instant} : t < u ↔ t.msSinceEpoch < u.msSinceEpoch := Iff.rfl

/-- Translation: a duration moves an instant along the grid. -/
instance : VAdd Duration Instant := ⟨fun d t ↦ ⟨d.ms + t.msSinceEpoch⟩⟩

/-- Difference: two instants subtract to the duration between them. -/
instance : VSub Duration Instant := ⟨fun t u ↦ Duration.ofMs (t.msSinceEpoch - u.msSinceEpoch)⟩

@[simp] theorem msSinceEpoch_vadd (d : Duration) (t : Instant) :
    (d +ᵥ t).msSinceEpoch = d.ms + t.msSinceEpoch := rfl

@[simp] theorem ms_vsub (t u : Instant) :
    (t -ᵥ u : Duration).ms = t.msSinceEpoch - u.msSinceEpoch := rfl

/-- `Instant` is an additive torsor over the millisecond durations: the free
and transitive action decision 9 item 2 requires, proved on the exact grid.
Every law of `Mensura.Units.Torsor` above therefore holds at `Instant`, and
none of the proofs assumed the backing is `real`. -/
instance addTorsor : AddTorsor Duration Instant where
  vadd := (· +ᵥ ·)
  zero_vadd t := by ext; simp
  add_vadd d e t := by ext; simp [add_assoc]
  vsub := (· -ᵥ ·)
  vsub_vadd' t u := by ext; simp
  vadd_vsub' d t := Duration.ms_injective (by simp)

/-! ### Order compatibility (ADR 0036 decision 9 item 3)

Not bookkeeping: ADR 0037's interval test `w <= p < w + size` and its
`closedWindow_stable` gate, which reasons about `w + size + lateness` against
a watermark, both need translation to be monotone in the duration and in the
point. -/

/-- Translation by a fixed duration preserves the strict order of points. -/
theorem vadd_lt_vadd_iff_right (d : Duration) {t u : Instant} : d +ᵥ t < d +ᵥ u ↔ t < u := by
  simp [lt_iff_ms_lt]

/-- Translating a fixed point by a strictly larger duration lands strictly
later. -/
theorem vadd_lt_vadd_iff_left {d e : Duration} (t : Instant) : d +ᵥ t < e +ᵥ t ↔ d < e := by
  simp [lt_iff_ms_lt, Duration.lt_iff_ms_lt]

/-- Translation by a positive duration is strictly increasing: the
order-compatibility law itself. -/
theorem lt_vadd_of_pos {d : Duration} (h : 0 < d) (t : Instant) : t < d +ᵥ t := by
  have hd : 0 < d.ms := Duration.pos_iff_ms_pos.mp h
  simp only [lt_iff_ms_lt, msSinceEpoch_vadd]
  omega

/-- Translation by a fixed duration is a strictly monotone map of points. -/
theorem strictMono_vadd (d : Duration) : StrictMono (fun t : Instant ↦ d +ᵥ t) :=
  fun _ _ h ↦ (vadd_lt_vadd_iff_right d).mpr h

/-- Order compatibility packaged for typeclass consumers: translation is
covariant in the point under the strict order.  ADR 0037's
`closedWindow_stable` gate consumes the lemma through this instance. -/
instance : CovariantClass Duration Instant (· +ᵥ ·) (· < ·) :=
  ⟨fun d _ _ h ↦ (vadd_lt_vadd_iff_right d).mpr h⟩

/-- The non-strict companion. -/
instance : CovariantClass Duration Instant (· +ᵥ ·) (· ≤ ·) :=
  ⟨fun d t u h ↦ by
    simp only [le_iff_ms_le, msSinceEpoch_vadd] at h ⊢
    omega⟩

end Instant

/-- ADR 0036 decision 4's degenerate case: for the numeric domains the point
and the difference coincide (`diff(int) = int`), which is the standard fact
that an additive group is a torsor over itself.  Count-based windows over an
`int` order key (ADR 0037 decision 3) need no further structure. -/
example : AddTorsor ℤ ℤ := inferInstance

end Mensura.Units
