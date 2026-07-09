/-
Split-safety of the operations: every operation of `Mensura.Core.Ops` except
`project` is `SplitSafe`, hence composes into pipelines that stay
split-invariant.  `aggregate` separates `SplitInvariant` from `BindHom`
(split-invariant, not a hom); `project` separates `SplitInvariant` from
`SplitSafe` (a hom, hence split-invariant, yet not disjointness-preserving,
so unsafe to pipeline).

Main Source:  Chapter 5, section "Split-invariant operations", of
F. A. N. Verri (2026). Data Science Project: An Inductive Learning Approach.
Version v1.0.0. Victoria, British Columbia, Canada: Leanpub. doi:
10.5281/zenodo.14498010. url: https://leanpub.com/dsp.
-/

import Mensura.Core.Ops

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {K' H' : Type _} {σ' : H' → Type}
variable {U G : Type _} {τ : G → Type}
variable {D : Type _}

/-- `map` is a bind-homomorphism, since `Multiset.bind` distributes over union. -/
theorem map_bindHom (φ : K → Row H σ → Multiset (Row H' σ')) :
    BindHom (map φ) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [map, bind]
  exact Multiset.add_bind _ _ _

/-- Hence `map` is split-invariant, the property Mensura enforces. -/
theorem map_splitInvariant (φ : K → Row H σ → Multiset (Row H' σ')) :
    SplitInvariant (map φ) := (map_bindHom φ).splitInvariant

/-- `leftJoin` against a fixed table is a bind-homomorphism: it is a `map`. -/
theorem leftJoin_bindHom (key : K → U) (right : Table U G τ) :
    BindHom (leftJoin (σ := σ) key right) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [leftJoin, map, bind]
  exact Multiset.add_bind _ _ _

/-- Hence `leftJoin` is split-invariant. -/
theorem leftJoin_splitInvariant (key : K → U) (right : Table U G τ) :
    SplitInvariant (leftJoin (σ := σ) key right) :=
  (leftJoin_bindHom key right).splitInvariant

/-- The unary, fixed-right `innerJoin` is a bind-homomorphism: it is a `map`. -/
theorem innerJoin_bindHom (key : K → U) (right : Table U G τ) :
    BindHom (innerJoin (σ := σ) key right) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [innerJoin, map, bind]
  exact Multiset.add_bind _ _ _

/-- Hence the unary, fixed-right `innerJoin` is split-invariant. -/
theorem innerJoin_splitInvariant (key : K → U) (right : Table U G τ) :
    SplitInvariant (innerJoin (σ := σ) key right) :=
  (innerJoin_bindHom key right).splitInvariant

/-- `ungroup` is a bind-homomorphism.  Each output key `(k, v)` reads only from
input key `k`, where the operation is `Multiset.bind`, which distributes over `+`. -/
theorem ungroup_bindHom {β : Type} [DecidableEq β] :
    BindHom (ungroup (K := K) (H := H) (σ := σ) (β := β)) := by
  intro T₀ T₁
  apply Table.ext_rows
  rintro ⟨k, v⟩
  simp only [ungroup, bind]
  exact Multiset.add_bind _ _ _

/-- Hence `ungroup` is split-invariant. -/
theorem ungroup_splitInvariant {β : Type} [DecidableEq β] :
    SplitInvariant (ungroup (K := K) (H := H) (σ := σ) (β := β)) :=
  ungroup_bindHom.splitInvariant

/-- `aggregate` *is* split-invariant -- the property Mensura enforces, and the
book's claim.  Under disjointness, at every key one summand is empty, so folding
the union is the same as folding the nonempty side. -/
theorem aggregate_splitInvariant (f : K → Multiset (Row H σ) → Row H σ) :
    SplitInvariant (aggregate f) := by
  intro T₀ T₁ hdisj
  apply Table.ext_rows
  intro k
  simp only [aggregate, bind]
  rcases hdisj k with h | h
  · rw [h, zero_add]; simp
  · rw [h, add_zero]; simp

/-- `aggregate` is *not* a bind-homomorphism: on a key present in both summands
it folds the merged bag to one row on the left but binds two aggregated rows on
the right.  This is the operation that separates `SplitInvariant` from the
strictly stronger `BindHom`. -/
theorem aggregate_not_bindHom :
    ¬ BindHom
        (aggregate (fun (_ : Unit) (_ : Multiset (Row Unit (fun _ => Unit))) =>
          fun _ => none)) := by
  intro h
  have hT := h ⟨fun _ => {fun _ => none}⟩ ⟨fun _ => {fun _ => none}⟩
  apply_fun (fun U => (U.rows ()).card) at hT
  simp [aggregate, bind] at hT

/-- `project` *is* a bind-homomorphism (hence split-invariant): it sums each
dropped key's mapped rows, and both `Multiset.map` and `Finset.sum` distribute
over `+`.  So by the bare equation alone, `project` looks safe. -/
theorem project_bindHom [Fintype D] :
    BindHom (project (K := K) (H := H) (σ := σ) (D := D)) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [project, bind, Multiset.map_add]
  rw [Finset.sum_add_distrib]

/-- ...yet `project` does *not* preserve disjointness, so it is **not**
`SplitSafe`.  Counterexample: two single-row tables that a split separates by the
dropped index (`d = false` vs `d = true`) both land on the same output key once
`d` is dropped.  This is precisely why a pipeline through `project` -- e.g.
`aggregate ∘ project`, the averaging case -- can disagree with the full-table
result: `aggregate` is then handed overlapping tables its guarantee does not
cover. -/
theorem project_not_preservesDisjoint :
    ¬ PreservesDisjoint
        (project (K := Unit) (σ := fun (_ : Unit) => Unit) (D := Bool)) := by
  intro h
  have hd := h
    ⟨fun p => if p.2 then 0 else {fun _ => none}⟩
    ⟨fun p => if p.2 then {fun _ => none} else 0⟩
    (by intro p; cases hp : p.2 <;> simp [hp]) ()
  simp [project] at hd

/-! ### Split-safety of the operations

Every operation defined above preserves disjointness, hence is `SplitSafe`, hence
can be freely composed into pipelines that stay split-invariant.  Each acts at a
key (or refines the key, for `ungroup`) using `Multiset.bind`/the `if`, both of
which send the empty multiset to the empty multiset. -/

theorem map_preservesDisjoint (φ : K → Row H σ → Multiset (Row H' σ')) :
    PreservesDisjoint (map φ) := by
  intro T₀ T₁ hdisj k
  rcases hdisj k with h | h
  · exact Or.inl (by simp [map, h])
  · exact Or.inr (by simp [map, h])

theorem map_splitSafe (φ : K → Row H σ → Multiset (Row H' σ')) :
    SplitSafe (map φ) := ⟨map_preservesDisjoint φ, map_splitInvariant φ⟩

theorem aggregate_preservesDisjoint (f : K → Multiset (Row H σ) → Row H σ) :
    PreservesDisjoint (aggregate f) := by
  intro T₀ T₁ hdisj k
  rcases hdisj k with h | h
  · exact Or.inl (by simp [aggregate, h])
  · exact Or.inr (by simp [aggregate, h])

theorem aggregate_splitSafe (f : K → Multiset (Row H σ) → Row H σ) :
    SplitSafe (aggregate f) := ⟨aggregate_preservesDisjoint f, aggregate_splitInvariant f⟩

theorem leftJoin_preservesDisjoint (key : K → U) (right : Table U G τ) :
    PreservesDisjoint (leftJoin (σ := σ) key right) := by
  intro T₀ T₁ hdisj k
  rcases hdisj k with h | h
  · exact Or.inl (by simp [leftJoin, map, h])
  · exact Or.inr (by simp [leftJoin, map, h])

theorem leftJoin_splitSafe (key : K → U) (right : Table U G τ) :
    SplitSafe (leftJoin (σ := σ) key right) :=
  ⟨leftJoin_preservesDisjoint key right, leftJoin_splitInvariant key right⟩

theorem innerJoin_preservesDisjoint (key : K → U) (right : Table U G τ) :
    PreservesDisjoint (innerJoin (σ := σ) key right) := by
  intro T₀ T₁ hdisj k
  rcases hdisj k with h | h
  · exact Or.inl (by simp [innerJoin, map, h])
  · exact Or.inr (by simp [innerJoin, map, h])

theorem innerJoin_splitSafe (key : K → U) (right : Table U G τ) :
    SplitSafe (innerJoin (σ := σ) key right) :=
  ⟨innerJoin_preservesDisjoint key right, innerJoin_splitInvariant key right⟩

theorem ungroup_preservesDisjoint {β : Type} [DecidableEq β] :
    PreservesDisjoint (ungroup (K := K) (H := H) (σ := σ) (β := β)) := by
  intro T₀ T₁ hdisj
  rintro ⟨k, v⟩
  rcases hdisj k with h | h
  · exact Or.inl (by simp [ungroup, h])
  · exact Or.inr (by simp [ungroup, h])

theorem ungroup_splitSafe {β : Type} [DecidableEq β] :
    SplitSafe (ungroup (K := K) (H := H) (σ := σ) (β := β)) :=
  ⟨ungroup_preservesDisjoint, ungroup_splitInvariant⟩

/-- A pipeline of split-safe operations is split-safe, hence split-invariant --
applying it between split and bind equals applying it to the full table.  Here:
`map`, then `leftJoin`, then `aggregate`. -/
example (φ : K → Row H σ → Multiset (Row H' σ')) (key : K → U) (right : Table U G τ)
    (g : K → Multiset (Row (H' ⊕ G) (Sum.elim σ' τ)) → Row (H' ⊕ G) (Sum.elim σ' τ)) :
    SplitSafe (aggregate g ∘ leftJoin (σ := σ') key right ∘ map φ) :=
  (aggregate_splitSafe g).comp ((leftJoin_splitSafe key right).comp (map_splitSafe φ))

end Mensura
