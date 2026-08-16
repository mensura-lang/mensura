/-
Streaming windows as a derived form (ADR 0037,
`docs/decisions/0037-streaming-windows-and-closedness.md`, decisions 1, 2,
and 4; the formal gates of decision 8).

`window w p size stride` replicates each row into every window that contains
its point and adds the window's start as a fresh key column.  The ADR
specifies the semantics *as a derived form*: a replicating `flatMap` (one
tagged copy per containing window start) followed by `promote` of the start
column.  This file mechanizes exactly that composition, so split-safety and
disjointness come from the existing composition lemmas rather than from a new
argument, and it adds the two statements the ADR owes:

* **the grading extension** (gate 1): the replication is injective on
  (input identity, window start), so a `Functional` table windows to a
  `Functional` table at the extended key.  This is decision 2's "extended,
  not reset" rule for gradings, and it is what keeps a downstream scan's
  tie-freedom derivable inside a window fiber;
* **`closedWindow_stable`** (gate 2): under an append-only extension in
  which every added row's point beats `watermark - lateness`, the fiber of
  any window with `w + size + lateness <= watermark` is unchanged.  This is
  the soundness of `closed`'s establishment given the enforced `lateness`
  contract (decision 4), and the finality invariant the refresh slice
  inherits: rerunning after further ingestion never changes a window already
  emitted as final.

The definition is generic in the placement function `starts` (which window
starts contain a given point).  The gates need only its two structural
properties: placement never duplicates a start (a `Finset`), and membership
implies the interval test's upper bound `p < size + w`.  The concrete
placement on the stride grid anchored at the epoch (ADR 0036 decision 5) is
`Units.Instant.windowStarts` below, whose characterization
`mem_windowStarts` is decision 1's interval test `w <= p < w + size` made
deterministic; it rests on the order-compatibility lemma of
`Mensura.Units.Torsor` (ADR 0036 decision 9), which is why that module lands
before this one.
-/

import Mensura.SplitSafety
import Mensura.Reshape
import Mensura.Units.Torsor

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {P : Type} [DecidableEq P]

/-! ## The derived form (ADR 0037 decision 1) -/

/-- The replicating half of `window`: one tagged copy of each row per
containing window start.  `starts` computes the placement from the row's
point; it returns a `Finset` because placement never lands the same row twice
in one window (decision 2's injectivity on (identity, start), by
construction). -/
def windowMap (starts : P → Finset P) (point : K → Row H σ → P) :
    Table K H σ → Table K (H ⊕ Unit) (Sum.elim σ (fun _ => P)) :=
  flatMap fun k f => (starts (point k f)).val.map fun w => addTag w f

/-- ADR 0037 decision 1: `window` is the replicating `flatMap` followed by
`promote` of the start column, so the window start joins the key with the
point's domain.  Being a composition of two operations the calculus already
proves, it is Tier A by construction (`window_splitSafe`). -/
def window (starts : P → Finset P) (point : K → Row H σ → P) (T : Table K H σ) :
    Table (K × P) H σ :=
  promote (windowMap starts point T)

/-! ## Gate 1: safety by composition -/

/-- **`window` is split-safe**, with no new argument: it is `promote` after a
`flatMap`, and split-safe operations compose. -/
theorem window_splitSafe (starts : P → Finset P) (point : K → Row H σ → P) :
    SplitSafe (window starts point) :=
  promote_splitSafe.comp (flatMap_splitSafe _)

/-- `window` distributes over every union: both halves of the composition
are union-homomorphisms. -/
theorem window_unionHom (starts : P → Finset P) (point : K → Row H σ → P) :
    UnionHom (window starts point) := by
  intro T₀ T₁
  show promote (flatMap _ (union T₀ T₁)) = _
  rw [flatMap_unionHom _ T₀ T₁]
  exact promote_unionHom _ _

/-! ## The fiber characterization

Both remaining gates reduce to one computation: the fiber of `window` at
`(k, w)` holds exactly the input rows of key `k` whose point lands in the
window starting at `w`, each exactly once.  The content of a windowed fiber
is the original rows, which is also why `window` is content-identity in
ADR 0024's sense. -/

/-- Binding a duplicate-free bag against the indicator of one element yields
that element's payload at most once. -/
theorem bind_ite_singleton {α β : Type _} [DecidableEq α] {s : Multiset α}
    (hs : s.Nodup) (w : α) (b : β) :
    (s.bind fun a => if a = w then ({b} : Multiset β) else 0) =
      if w ∈ s then {b} else 0 := by
  induction s using Multiset.induction_on with
  | empty => simp
  | cons a s ih =>
    rw [Multiset.nodup_cons] at hs
    rw [Multiset.cons_bind, ih hs.2]
    by_cases haw : a = w
    · subst haw
      simp [hs.1]
    · simp [haw, Multiset.mem_cons, Ne.symm haw]

/-- **The fiber of a window.**  At `(k, w)` the windowed table holds exactly
the rows of key `k` whose point lands in the window starting at `w`. -/
theorem window_rows (starts : P → Finset P) (point : K → Row H σ → P)
    (T : Table K H σ) (k : K) (w : P) :
    (window starts point T).rows (k, w) =
      (T.rows k).bind fun f => if w ∈ starts (point k f) then {f} else 0 := by
  simp only [window, windowMap, promote, flatMap]
  rw [Multiset.bind_assoc]
  refine congrArg _ (funext fun f => ?_)
  rw [Multiset.bind_map]
  exact bind_ite_singleton (starts (point k f)).nodup w f

/-! ## Gate 1, the new statement: the grading extension -/

/-- **The grading extension** (ADR 0037 decision 2): the replication is
injective on (input identity, window start), so a table functional at its key
windows to a table functional at the window-extended key.  Every tracked
grading therefore extends to the window column rather than resetting, which
is what keeps tie-freedom derivable inside a window fiber after the
subsequent `demote`. -/
theorem window_functional {starts : P → Finset P} {point : K → Row H σ → P}
    {T : Table K H σ} (hT : Functional T) :
    Functional (window starts point T) := by
  rintro ⟨k, w⟩
  rw [window_rows, Multiset.card_bind]
  refine le_trans (Multiset.sum_le_card_nsmul _ 1 ?_) ?_
  · intro x hx
    obtain ⟨f, -, rfl⟩ := Multiset.mem_map.mp hx
    by_cases h : w ∈ starts (point k f) <;> simp [h]
  · simpa using hT k

/-! ## Gate 2: closed windows are final -/

section Closedness

variable {G : Type _} [AddCommMonoid G] [Preorder P] [AddAction G P]
variable [CovariantClass G P (· +ᵥ ·) (· < ·)]

/-- **Closed windows are final** (ADR 0037 decision 8, gate 2).  Extend a
table by appended rows every one of whose points is no older than
`watermark - lateness`, stated action-side as `watermark <= lateness + p`:
exactly the rows the `lateness` contract still admits through the intake
(decision 4, where "older than" is strict, so the boundary point is
admissible).  Then the fiber of any window with
`w + size + lateness <= watermark`, any window `closed` keeps, is
unchanged; the boundary row is harmless because the window interval's
upper bound is strict.

This is the soundness of `closed`'s establishment: given the enforced
contract, "no row of this window can still arrive" is a theorem about the
intake, and rerunning after further ingestion never changes a window already
emitted as final.  The placement function enters only through the interval
test's upper bound (`hspec`), which is where the order-compatibility lemma of
ADR 0036 decision 9 does its work.

**The watermark is indexed by a grain** (ADR 0041 decision 2): a function
`grain : K → Γ` partitions the keys, and one watermark serves each part.
The global watermark of ADR 0037 is the constant grain, and the per-entity
watermark of ADR 0041 is the residual key.  What the statement will not let
you write is a *mixed* design: `hlate` (what the intake admits) and
`hclosed` (what may be declared final) read the same `watermark (grain k)`,
so admitting rows at one grain while closing windows at another is not
expressible here, which is ADR 0041 decision 1.  It is worth seeing that
this costs nothing in the proof: the body already used the hypothesis only
at the conclusion's own key. -/
theorem closedWindow_stable {Γ : Type _} {starts : P → Finset P} {size : G}
    (hspec : ∀ ⦃w p : P⦄, w ∈ starts p → p < size +ᵥ w)
    (point : K → Row H σ → P) {T A : Table K H σ} {grain : K → Γ}
    {watermark : Γ → P} {lateness : G}
    (hlate : ∀ k, ∀ f ∈ A.rows k, watermark (grain k) ≤ lateness +ᵥ point k f)
    {w : P} (k : K) (hclosed : (size + lateness) +ᵥ w ≤ watermark (grain k)) :
    (window starts point (union T A)).rows (k, w) =
      (window starts point T).rows (k, w) := by
  rw [window_rows, window_rows]
  show (T.rows k + A.rows k).bind _ = _
  rw [Multiset.add_bind]
  have hA : ∀ f ∈ A.rows k,
      (if w ∈ starts (point k f) then ({f} : Multiset (Row H σ)) else 0) = 0 := by
    intro f hf
    rw [if_neg]
    intro hmem
    have hp : lateness +ᵥ point k f < (size + lateness) +ᵥ w := by
      have hlt : lateness +ᵥ point k f < lateness +ᵥ (size +ᵥ w) :=
        CovariantClass.elim lateness (hspec hmem)
      rwa [← add_vadd, add_comm lateness size] at hlt
    exact absurd (lt_of_lt_of_le hp hclosed) (not_lt_of_ge (hlate k f hf))
  rw [Multiset.bind_congr hA, Multiset.bind_zero, add_zero]

end Closedness

/-! ### The lift through `demote` (ADR 0041 decision 6, item 2)

`closedWindow_stable` speaks of a fiber at one key.  The surface pipeline
coarsens before it closes (`window w p ... |> demote p |> closed`), so the
fact the checker actually cites is about the *coarse* fiber.  The transport
is immediate and worth stating rather than assuming: `demote` reads each
coarse key as a finite sum over the dropped component, so fibers that agree
one by one sum to fibers that agree. -/

/-- Fiber-wise agreement transports through `demote`.  This is the lift
ADR 0041 decision 6 owes: per-key stability becomes stability at the
coarsened key, because a coarse fiber is a finite union of fine ones. -/
theorem demote_congr {D : Type _} [Fintype D] {T U : Table (K × D) H σ} {k : K}
    (h : ∀ d : D, T.rows (k, d) = U.rows (k, d)) :
    (demote T).rows k = (demote U).rows k := by
  simp only [demote]
  exact Finset.sum_congr rfl fun d _ => by rw [h d]

/-! ## The concrete grid at `Instant`

ADR 0037 decision 1 fixes the placement: window starts lie on the stride grid
anchored at the domain's zero, which for `instant` is the epoch of ADR 0036
decision 5, and a row with point `p` lands in every window `w` with
`w <= p < w + size`.  The definition below computes that placement, and
`mem_windowStarts` proves it is exactly the interval test on the grid: no
declaration, no data dependence, deterministic. -/

namespace Units.Instant

open Units

/-- The window starts whose window contains the point `p`: the integer
multiples of `stride` (milliseconds from the epoch) in the half-open interval
`(p - size, p]`.

`noncomputable` is a mathlib artifact (this snapshot's `Finset.Ioc` on `ℤ`
compiles through a noncomputable instance), not a property of the placement,
which is two integer divisions; like `arrange`, this definition is for
reasoning, and the executor is the toolchain's job. -/
noncomputable def windowStarts (size stride : Duration) (p : Instant) : Finset Instant :=
  (Finset.Ioc ((p.msSinceEpoch - size.ms) / stride.ms) (p.msSinceEpoch / stride.ms)).image
    fun n => ⟨n * stride.ms⟩

/-- **Placement is the interval test on the grid** (ADR 0037 decision 1): for
a positive stride, `w` is a start containing `p` iff `w` is a grid multiple
of `stride` from the epoch and `w <= p < w + size`. -/
theorem mem_windowStarts {size stride : Duration} (hstride : 0 < stride) {p w : Instant} :
    w ∈ windowStarts size stride p ↔
      (∃ n : ℤ, w.msSinceEpoch = n * stride.ms) ∧ w ≤ p ∧ p < size +ᵥ w := by
  have hs : (0 : ℤ) < stride.ms := hstride
  constructor
  · intro hmem
    rw [windowStarts, Finset.mem_image] at hmem
    obtain ⟨n, hn, rfl⟩ := hmem
    rw [Finset.mem_Ioc] at hn
    refine ⟨⟨n, rfl⟩, ?_, ?_⟩
    · exact le_iff_ms_le.mpr ((Int.le_ediv_iff_mul_le hs).mp hn.2)
    · have h1 := (Int.ediv_lt_iff_lt_mul hs).mp hn.1
      exact lt_iff_ms_lt.mpr (show p.msSinceEpoch < size.ms + n * stride.ms by omega)
  · rintro ⟨⟨n, hw⟩, hle, hlt⟩
    rw [windowStarts, Finset.mem_image]
    have hle' : n * stride.ms ≤ p.msSinceEpoch := by
      have := le_iff_ms_le.mp hle
      omega
    have hlt' : p.msSinceEpoch - size.ms < n * stride.ms := by
      have := lt_iff_ms_lt.mp hlt
      rw [msSinceEpoch_vadd] at this
      omega
    refine ⟨n, ?_, Instant.ext hw.symm⟩
    rw [Finset.mem_Ioc]
    exact ⟨(Int.ediv_lt_iff_lt_mul hs).mpr hlt', (Int.le_ediv_iff_mul_le hs).mpr hle'⟩

/-- The half of the placement `closedWindow_stable` consumes: membership
implies the interval test's upper bound. -/
theorem lt_vadd_of_mem_windowStarts {size stride : Duration} (hstride : 0 < stride)
    {p w : Instant} (h : w ∈ windowStarts size stride p) : p < size +ᵥ w :=
  ((mem_windowStarts hstride).mp h).2.2

/-- `closedWindow_stable` at the concrete grid: the statement the checker's
`closed` stage cites for `instant`-pointed windows.  The covariance instance
of `Mensura.Units.Torsor` (ADR 0036 decision 9's order compatibility)
discharges the abstract theorem's order hypothesis, and `windowStarts`
discharges its placement hypothesis.

The watermark is grained (ADR 0041): `grain` is the intake's partition of
the keys, which the implementation reads off the declared key with the
contracted column removed, and `watermark` assigns the effective point to
each part.  The effective point is the maximum of what the intake has
accepted in that part and the declared floor; neither half appears here,
because both enter as one value and the theorem asks nothing more of it. -/
theorem closedWindow_stable {Γ : Type _} {size stride : Duration}
    (hstride : 0 < stride) (point : K → Row H σ → Instant) {T A : Table K H σ}
    {grain : K → Γ} {watermark : Γ → Instant} {lateness : Duration}
    (hlate : ∀ k, ∀ f ∈ A.rows k, watermark (grain k) ≤ lateness +ᵥ point k f)
    {w : Instant} (k : K) (hclosed : (size + lateness) +ᵥ w ≤ watermark (grain k)) :
    (window (windowStarts size stride) point (union T A)).rows (k, w) =
      (window (windowStarts size stride) point T).rows (k, w) :=
  Mensura.closedWindow_stable (fun _ _ h => lt_vadd_of_mem_windowStarts hstride h)
    point hlate k hclosed

end Units.Instant
