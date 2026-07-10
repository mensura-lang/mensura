/-
Safe completeness, key-changing fragment: the characterization of
`Mensura.Completeness.FiberMap` lifted along a reindexing function.

A `fiberMap` keeps the key.  The safe *key-changing* operations (`ungroup`,
`unpivot`) still read each output key from a *single* input key, related by a
function `r : K' → K` from output keys to input keys.  We capture that as
`reindexMap` and lift the whole characterization to it; `fiberMap` is the
`r = id` case.  `project` is different in kind -- one output key reads a whole
*fiber* of input keys -- so it gets the dual `gatherMap` form, which is
split-invariant when its combiner is additive but is exactly the form that
fails to preserve disjointness.  Together these place every operation of the
algebra.
-/

import Mensura.Core.Ops
import Mensura.Reshape
import Mensura.Completeness.FiberMap

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {H' : Type _} {σ' : H' → Type}
variable {K' D : Type _}
variable {N : Type _} {V : Type}

/-! ### Pullback form: `reindexMap` (safe key-changing operations) -/

/-- A *reindexing fiber map* along `r : K' → K`: the output bag at key `k'` is a
strict-ready function of the input bag at the single key `r k'`.  This is the
universal shape of a split-invariant operation that reads each output key from
one input key.  `fiberMap` is the `r = id` case (`fiberMap_eq_reindexMap`);
`ungroup` and `unpivot` are the `r = Prod.fst` cases. -/
def reindexMap (r : K' → K) (Φ : K' → Multiset (Row H σ) → Multiset (Row H' σ'))
    (T : Table K H σ) : Table K' H' σ' :=
  ⟨fun k' => Φ k' (T.rows (r k'))⟩

/-- `fiberMap` is the identity-reindexing case. -/
theorem fiberMap_eq_reindexMap (Φ : K → Multiset (Row H σ) → Multiset (Row H' σ')) :
    fiberMap Φ = reindexMap id Φ := rfl

/-- A strict `reindexMap` is split-invariant: at each output key `k'` the relevant
input key `r k'` has one empty summand under disjointness, and strictness makes
folding the union the same as folding the nonempty side. -/
theorem reindexMap_splitInvariant (r : K' → K)
    {Φ : K' → Multiset (Row H σ) → Multiset (Row H' σ')} (hΦ : Strict Φ) :
    SplitInvariant (reindexMap r Φ) := by
  intro T₀ T₁ hdisj
  apply Table.ext_rows
  intro k'
  have hk := hΦ k'
  simp only [reindexMap, bind]
  rcases hdisj (r k') with h | h
  · rw [h]; simp [hk]
  · rw [h]; simp [hk]

/-- A strict `reindexMap` preserves disjointness: an empty input fiber at `r k'`
yields an empty output fiber at `k'`. -/
theorem reindexMap_preservesDisjoint (r : K' → K)
    {Φ : K' → Multiset (Row H σ) → Multiset (Row H' σ')} (hΦ : Strict Φ) :
    PreservesDisjoint (reindexMap r Φ) := by
  intro T₀ T₁ hdisj k'
  rcases hdisj (r k') with h | h
  · exact Or.inl (by simp [reindexMap, h, hΦ k'])
  · exact Or.inr (by simp [reindexMap, h, hΦ k'])

/-- Hence a strict `reindexMap` is `SplitSafe`: the safe key-changing operations
compose into split-invariant pipelines just like the key-preserving ones. -/
theorem reindexMap_splitSafe (r : K' → K)
    {Φ : K' → Multiset (Row H σ) → Multiset (Row H' σ')} (hΦ : Strict Φ) :
    SplitSafe (reindexMap r Φ) :=
  ⟨reindexMap_preservesDisjoint r hΦ, reindexMap_splitInvariant r hΦ⟩

/-- Locality along `r`: each output key `k'` depends only on the input at `r k'`.
For `r = id` this is exactly `KeyLocal`. -/
def ReindexLocal (r : K' → K) (f : Table K H σ → Table K' H' σ') : Prop :=
  ∀ (T T' : Table K H σ) (k' : K'),
    T.rows (r k') = T'.rows (r k') → (f T).rows k' = (f T').rows k'

/-- The fiber action witnessed by an `r`-local operation: run it on a table
supported at the single input key `r k'`, and read off the output key `k'`. -/
def reindexFiberOf [DecidableEq K] (r : K' → K) (f : Table K H σ → Table K' H' σ')
    (k' : K') (m : Multiset (Row H σ)) : Multiset (Row H' σ') :=
  (f (pointTable (r k') m)).rows k'

/-- Representation: an `r`-local operation *is* the `reindexMap` of its witnessed
action (needs only `r`-locality). -/
theorem reindexLocal_eq_reindexMap [DecidableEq K] (r : K' → K)
    {f : Table K H σ → Table K' H' σ'} (hf : ReindexLocal r f) :
    f = reindexMap r (reindexFiberOf r f) := by
  funext T
  apply Table.ext_rows
  intro k'
  show (f T).rows k' = (f (pointTable (r k') (T.rows (r k')))).rows k'
  exact hf T (pointTable (r k') (T.rows (r k'))) k' (by simp)

/-- The witnessed action of a split-invariant operation is strict. -/
theorem reindexFiberOf_strict [DecidableEq K] (r : K' → K)
    {f : Table K H σ → Table K' H' σ'} (hf : SplitInvariant f) :
    Strict (reindexFiberOf r f) := by
  intro k'
  unfold reindexFiberOf
  have hpt : pointTable (r k') (0 : Multiset (Row H σ)) = ⟨fun _ => 0⟩ := by
    apply Table.ext_rows; intro k''; simp [pointTable]
  rw [hpt]
  exact splitInvariant_empty hf k'

/-- **Safe completeness (key-changing fragment).**  Along any `r : K' → K`, an
operation is split-invariant and `r`-local *iff* it is a strict `reindexMap` over
`r`.  This is the full generalization of `splitInvariant_keyLocal_iff_fiberMap`
(its `r = id` instance): the split-invariant, single-source operations are
exactly the strict reindex maps, with `ungroup` and `unpivot` as instances
(`ungroup_eq_reindexMap`, `unpivot_eq_reindexMap`). -/
theorem splitInvariant_reindexLocal_iff_reindexMap [DecidableEq K] (r : K' → K)
    {f : Table K H σ → Table K' H' σ'} :
    (SplitInvariant f ∧ ReindexLocal r f) ↔ ∃ Φ, Strict Φ ∧ f = reindexMap r Φ := by
  constructor
  · rintro ⟨hSI, hRL⟩
    exact ⟨reindexFiberOf r f, reindexFiberOf_strict r hSI, reindexLocal_eq_reindexMap r hRL⟩
  · rintro ⟨Φ, hΦ, rfl⟩
    refine ⟨reindexMap_splitInvariant r hΦ, ?_⟩
    intro T T' k' hk
    simp only [reindexMap]
    rw [hk]

/-! ### `ungroup` and `unpivot` are reindex maps

Both read each output key `(k, ·)` from the single input key `k`, so each is a
`reindexMap` along `Prod.fst`.  This gives a second, uniform proof of their
split-safety straight from `reindexMap_splitSafe`. -/

theorem unpivot_eq_reindexMap :
    unpivot (K := K) (N := N) (V := V)
      = reindexMap Prod.fst (fun (p : K × N) m => m.map (fun f => fun _ => f p.2)) := rfl

example : SplitSafe (unpivot (K := K) (N := N) (V := V)) := by
  rw [unpivot_eq_reindexMap]
  exact reindexMap_splitSafe Prod.fst (by intro _; simp)

theorem ungroup_eq_reindexMap {β : Type} [DecidableEq β] :
    ungroup (K := K) (H := H) (σ := σ) (β := β)
      = reindexMap Prod.fst (fun (p : K × β) m => m.bind (fun f =>
          let v : Cell β := f (Sum.inr ())
          match v with
          | some w => if w = p.2 then {fun h => f (Sum.inl h)} else 0
          | none => 0)) := rfl

example {β : Type} [DecidableEq β] :
    SplitSafe (ungroup (K := K) (H := H) (σ := σ) (β := β)) := by
  rw [ungroup_eq_reindexMap]
  exact reindexMap_splitSafe Prod.fst (by intro _; simp)

/-! ### Merge form: `gatherMap` (the key-merging boundary, `project`)

`project` does not fit `reindexMap`: one output key `k` reads the whole fiber
`{(k, d) : d}` of input keys.  We capture that as `gatherMap`.  When its combiner
is additive in the fiber it is a `BindHom` (hence split-invariant), which is why
`project` is split-invariant -- but it is precisely this many-to-one shape that
fails to preserve disjointness (two rows a split separates by `d` collapse onto
the same output key), so `gatherMap` is *not* `SplitSafe`.  This is the formal
boundary between the safe key-changing operations (`reindexMap`) and the unsafe
key-merging one (`project`), the latter already witnessed by
`project_not_preservesDisjoint`. -/

/-- An output key `k` reads and combines the whole fiber `{T.rows (k, d) : d}`. -/
def gatherMap [Fintype D] (Ψ : K → (D → Multiset (Row H σ)) → Multiset (Row H' σ'))
    (T : Table (K × D) H σ) : Table K H' σ' :=
  ⟨fun k => Ψ k (fun d => T.rows (k, d))⟩

/-- The combiner is additive when it distributes over pointwise union of fibers. -/
def AdditiveGather [Fintype D]
    (Ψ : K → (D → Multiset (Row H σ)) → Multiset (Row H' σ')) : Prop :=
  ∀ (k : K) (fam fam' : D → Multiset (Row H σ)),
    Ψ k (fun d => fam d + fam' d) = Ψ k fam + Ψ k fam'

/-- An additive `gatherMap` is a bind-homomorphism, hence split-invariant. -/
theorem gatherMap_bindHom [Fintype D]
    {Ψ : K → (D → Multiset (Row H σ)) → Multiset (Row H' σ')} (hΨ : AdditiveGather Ψ) :
    BindHom (gatherMap Ψ) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [gatherMap, bind]
  exact hΨ k _ _

/-- `project` is a `gatherMap`: it sums each fiber's mapped rows. -/
theorem project_eq_gatherMap [Fintype D] :
    project (K := K) (H := H) (σ := σ) (D := D)
      = gatherMap (fun (_k : K) (fam : D → Multiset (Row H σ)) =>
          ∑ d : D, (fam d).map (fun f => Row.elim f (fun _ => some d))) := rfl

/-- And its combiner is additive -- so `project` is a `BindHom` by the uniform
`gatherMap` route (compare `project_bindHom`). -/
theorem project_additiveGather [Fintype D] :
    AdditiveGather (fun (_k : K) (fam : D → Multiset (Row H σ)) =>
      ∑ d : D, (fam d).map (fun f => Row.elim f (fun (_ : Unit) => some d))) := by
  intro k fam fam'
  simp only [Multiset.map_add]
  exact Finset.sum_add_distrib

end Mensura
