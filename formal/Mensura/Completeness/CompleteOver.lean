/-
Completeness propagation for the key-merging boundary (ADR 0023,
`docs/decisions/0023-completeness-consumed-by-the-reducer.md`).

The `Exhaustive` fact of `Mensura.Rectangle` is domain-relative and fully
mechanized; the ADR 0017 `complete_over` fact is *population*-relative and was
left unmechanized (see the doc comment on `Mensura.Exhaustive`).  This file
gives it the honest mechanization: completeness is relative to a **reference
table** `R` that stands for the intended full population, and a table `T` is
complete when it has a row wherever the reference does.

Two results seed ADR 0023.  First, `project` (the algebra's `shrink_key`,
`Mensura.project`) **propagates** completeness from the fine key `K × D` to
the coarse key `K`.  It neither demands nor invents the fact; it carries it
across the coarsening, which is why the ADR moves the *demand* onto the
reducing `group_map` downstream and leaves `shrink_key` responsible only for
its lineage break (`project_not_preservesDisjoint`).

Second, the **trivial discharge at `card <= 1`**: `CompleteWrt` is key
coverage (no key the population has is absent), but the fact a *fold* needs
is fiber-level (no group it folds is partial), mechanized here as
`FiberCompleteWrt`.  When the population itself is `Functional` (ADR 0001's
identity discipline: at most one observation per identity exists in the
world) and the table holds only genuine observations, every present key
carries its whole fiber (`fiberCompleteWrt_of_functional`): a singleton
group is either absent or whole, never partial.  This is the base case the
checker uses to accept a reducing `group_map` over a `singletons` store's
full key with no establishment step (ADR 0022 / 0023).
-/

import Mensura.Core.Ops
import Mensura.Reshape

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {D : Type _} [Fintype D]

/-- **Population-relative completeness** (ADR 0017 `complete_over`, mechanized).
`T` is complete with respect to a reference table `R` when every key present in
`R` is present in `T`: no row the population has is missing from `T`.  The
reference `R` is what "should be there"; `complete_over(k)` is the instance
where `R` and `T` are keyed by the coarsening `k`. -/
def CompleteWrt (R T : Table K H σ) : Prop := ∀ k, R.Present k → T.Present k

/-- Reflexivity: every table is complete with respect to itself. -/
theorem completeWrt_refl (T : Table K H σ) : CompleteWrt T T := fun _ h => h

/-- **`shrink_key` propagates completeness (ADR 0023).**  If `T` is complete
with respect to a reference `R` at the fine key `K × D`, then `project T` is
complete with respect to `project R` at the coarse key `K`.  Coarsening keeps
"nothing is missing": a coarse group is present exactly when some fine key in
its fibre is present, and completeness carries that fine presence from `R` to
`T`.  So `project` (the algebra's `shrink_key`) transforms the fact rather than
consuming it. -/
theorem project_completeWrt {R T : Table (K × D) H σ}
    (h : CompleteWrt R T) : CompleteWrt (project R) (project T) := by
  intro k hR
  simp only [Table.Present, project] at hR ⊢
  contrapose! hR
  rw [Finset.sum_eq_zero_iff] at hR ⊢
  intro d _
  have hT : (T.rows (k, d)).map (fun f => Row.elim f (fun _ => some d)) = 0 :=
    hR d (Finset.mem_univ d)
  rw [Multiset.map_eq_zero] at hT ⊢
  by_contra hR0
  exact absurd hT (h (k, d) hR0)

/-- **Fiber-level completeness**: every key present in `T` carries its whole
population fiber.  This is the fact a reducing `group_map` needs for the rows
it emits (a fold over a partial group is silently wrong); it is weaker than
`CompleteWrt` in that it says nothing about keys absent from `T` (an absent
key manifests as an absent output row, not a wrong value). -/
def FiberCompleteWrt (R T : Table K H σ) : Prop :=
  ∀ k, T.Present k → T.rows k = R.rows k

/-- **Trivial discharge at `card <= 1` (ADR 0023).**  If the intended
population `R` is `Functional` (ADR 0001's identity discipline as a fact about
the world: an identity is observed once or not at all) and `T` holds only
genuine observations (`T.rows k ≤ R.rows k`), then every present key of `T`
carries its whole fiber: at `card <= 1` there is no middle ground between an
absent group and a whole one.  This backs the checker rule that a reducing
`group_map` over a `singletons` store's full key needs no establishment step.
It does *not* give key coverage (`CompleteWrt R T`): whole keys may still be
absent from `T`, and coarsening converts exactly that absence into a fiber
gap, which is where `project_completeWrt` and the reference take over. -/
theorem fiberCompleteWrt_of_functional {R T : Table K H σ}
    (hR : Functional R) (hsub : ∀ k, T.rows k ≤ R.rows k) :
    FiberCompleteWrt R T := by
  intro k hk
  have hk' : T.rows k ≠ 0 := hk
  have hcard : (R.rows k).card ≤ (T.rows k).card := by
    have h1 : (R.rows k).card ≤ 1 := hR k
    have h2 : 0 < (T.rows k).card := Multiset.card_pos.mpr hk'
    omega
  exact Multiset.eq_of_le_of_card_le (hsub k) hcard

end Mensura
