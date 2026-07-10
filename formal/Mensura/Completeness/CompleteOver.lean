/-
Completeness propagation for the key-merging boundary (ADR 0023,
`docs/decisions/0023-completeness-consumed-by-the-reducer.md`).

The `Exhaustive` fact of `Mensura.Rectangle` is domain-relative and fully
mechanized; the ADR 0017 `complete_over` fact is *population*-relative and was
left unmechanized (see the doc comment on `Mensura.Exhaustive`).  This file
gives it the honest mechanization: completeness is relative to a **reference
table** `R` that stands for the intended full population, and a table `T` is
complete when it has a row wherever the reference does.

The headline result seeds ADR 0023: `project` (the algebra's `shrink_key`,
`Mensura.project`) **propagates** completeness from the fine key `K × D` to
the coarse key `K`.  It neither demands nor invents the fact; it carries it
across the coarsening, which is why the ADR moves the *demand* onto the
reducing `group_map` downstream and leaves `shrink_key` responsible only for
its lineage break (`project_not_preservesDisjoint`).
-/

import Mensura.Core.Ops

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

end Mensura
