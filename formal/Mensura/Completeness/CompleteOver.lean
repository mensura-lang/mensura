/-
Completeness propagation for the key-merging boundary (ADR 0023,
`docs/decisions/0023-completeness-consumed-by-the-reducer.md`).

The `Exhaustive` fact of `Mensura.Rectangle` is domain-relative and fully
mechanized; the ADR 0017 `complete_over` fact is *population*-relative and was
left unmechanized (see the doc comment on `Mensura.Exhaustive`).  This file
gives it the honest mechanization: completeness is relative to a **reference
table** `R` that stands for the intended full population, and a table `T` is
complete when it has a row wherever the reference does.

Two results seed ADR 0023.  First, `demote` (the algebra's `demote`,
`Mensura.demote`) **propagates** completeness from the fine key `K × D` to
the coarse key `K`, *relative to a reference that coarsens with the table*
(`demote_completeWrt`).  ADR 0035 sharpens what that does and does not
give: the fact the checker's qualifier tracks is fiber-level completeness
against a **fixed** reference, and that fact does not survive a genuine
coarsening (the fiber-gap counterexample recorded at the end of this
file), which is why the checker clears the qualifier at a coarsening
`demote` and the establishment step sits after it.  The *demand* stays on the reducing `map_bags` downstream,
and `demote` remains responsible only for its lineage break
(`demote_not_preservesDisjoint`).

Second, the **trivial discharge at `card <= 1`**: `CompleteWrt` is key
coverage (no key the population has is absent), but the fact a *fold* needs
is fiber-level (no group it folds is partial), mechanized here as
`FiberCompleteWrt`.  When the population itself is `Functional` (ADR 0001's
identity discipline: at most one observation per identity exists in the
world) and the table holds only genuine observations, every present key
carries its whole fiber (`fiberCompleteWrt_of_functional`): a singleton
group is either absent or whole, never partial.  This is the base case the
checker uses to accept a reducing `map_bags` over a `singletons` store's
full key with no establishment step (ADR 0022 / 0023), and, applied at the
post-move key, it is also what re-derives the qualifier when a key move's
result is graded `singletons` (ADR 0035), keeping `promote`/`demote` a
true inverse pair on the whole qualifier vector.
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

/-- **`demote` propagates reference-relative completeness (ADR 0023).**  If
`T` is complete with respect to a reference `R` at the fine key `K × D`,
then `demote T` is complete with respect to `demote R` at the coarse key
`K`.  Coarsening keeps "nothing is missing": a coarse group is present
exactly when some fine key in its fibre is present, and completeness
carries that fine presence from `R` to `T`.  Note the reference coarsens
**with** the table; this is *not* the fact the checker's qualifier tracks
(fiber-level completeness against a fixed reference, which a coarsening
destroys: the counterexample at the end of this file, ADR 0035). -/
theorem demote_completeWrt {R T : Table (K × D) H σ}
    (h : CompleteWrt R T) : CompleteWrt (demote R) (demote T) := by
  intro k hR
  simp only [Table.Present, demote] at hR ⊢
  contrapose! hR
  rw [Finset.sum_eq_zero_iff] at hR ⊢
  intro d _
  have hT : (T.rows (k, d)).map (fun f => Row.elim f (fun _ => some d)) = 0 :=
    hR d (Finset.mem_univ d)
  rw [Multiset.map_eq_zero] at hT ⊢
  by_contra hR0
  exact absurd hT (h (k, d) hR0)

/-- **Fiber-level completeness**: every key present in `T` carries its whole
population fiber.  This is the fact a reducing `map_bags` needs for the rows
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
`map_bags` over a `singletons` store's full key needs no establishment step.
It does *not* give key coverage (`CompleteWrt R T`): whole keys may still be
absent from `T`, and coarsening converts exactly that absence into a fiber
gap, which is where `demote_completeWrt` and the reference take over. -/
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

/-! **`demote` does not preserve fiber-level completeness (ADR 0035).**
Against a fixed reference, "every present group is whole" is destroyed by
a genuine coarsening.  `FiberCompleteWrt` never constrains a key *absent*
from `T`, and `demote` merges exactly that absence into a coarse fiber,
where it becomes a gap.  Witness: over key `Unit × Bool`, let the
population `R` hold one row at each of `((), false)` and `((), true)`,
and let `T` hold only the `false` row.  `FiberCompleteWrt R T` holds
(the one present key carries its whole fiber), yet after `demote` the
single coarse key `()` is present in `demote T` with a one-row bag where
`demote R`'s has two, so `FiberCompleteWrt (demote R) (demote T)` fails.

Contrast `demote_completeWrt` above, which is true because its reference
coarsens *with* the table.  The checker tracks the fixed-reference fact,
so a coarsening `demote` clears the qualifier and the establishment step
sits after it; the clearing is conservative and needs no lemma
(ADR 0021).  Mechanizing this witness as `demote_not_fiberCompleteWrt`
is recorded in ADR 0035 as the slice's open formal item, alongside the
`promote` preservation row. -/

end Mensura
