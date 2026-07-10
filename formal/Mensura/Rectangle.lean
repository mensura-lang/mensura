/-
The rectangle fact of ADR 0020
(`docs/decisions/0020-reshape-as-a-true-inverse-pair.md`): `Exhaustive`
(every present residual key carries every name) upgrades `pivot`'s output to
`Total`, and it propagates through the key-preserving operations.

`unpivotDrop` establishes the rectangle from a `Total` wide table
(`unpivotDrop_exhaustive`) and its output is `Minimal` unconditionally
(`unpivotDrop_minimal`).  The rectangle propagates through a non-dropping
`map`, `leftJoin`, `aggregate`, and the `bind` of two carriers, and `split`
destroys it (`split_not_exhaustive`) -- ADR 0020's "destroyed by `split`"
row, the honest price of keeping the name in the key.
-/

import Mensura.Core.Ops
import Mensura.Reshape

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {H' : Type _} {σ' : H' → Type}
variable {U G : Type _} {τ : G → Type}
variable {N : Type _} {V : Type}

/-- Every cell of every present row is known: the table-level reading of
"all columns total" (ADR 0010's default).  Row-wise stronger than
`Substantive`, hence stronger than `Minimal` on a nonempty schema. -/
def Total (T : Table K H σ) : Prop := ∀ k, ∀ f ∈ T.rows k, ∀ h, f h ≠ none

/-- The rectangle fact of ADR 0020: every residual key present in a long
table carries a row for **every** name `n`.  The reference is the name
type's whole domain, not a population -- which is exactly what
distinguishes it from the (unmechanized, population-relative)
`complete_over` and makes it the fact that licenses `pivot`'s totality
upgrade (`pivot_total_of_exhaustive`). -/
def Exhaustive (T : Table (K × N) H σ) : Prop :=
  ∀ k, (∃ n, T.Present (k, n)) → ∀ n, T.Present (k, n)

/-- `unpivotDrop`'s output is minimal unconditionally: every long row it
emits carries a known value, by construction. -/
theorem unpivotDrop_minimal (T : Table K N (fun _ => V)) :
    Minimal (unpivotDrop T) := by
  rintro ⟨k, n⟩ f hf
  simp only [unpivotDrop] at hf
  obtain ⟨g, _, hfg⟩ := Multiset.mem_bind.mp hf
  refine ⟨(), ?_⟩
  cases hgn : g n with
  | none => simp [hgn] at hfg
  | some v =>
      simp only [hgn] at hfg
      simp [Multiset.mem_singleton.mp hfg]

/-- `unpivotDrop` establishes the rectangle by mechanism on a `Total` wide
table (ADR 0020: "every folded column is total"): each source row emits
one long row per name, so a residual key present for one name is present
for all. -/
theorem unpivotDrop_exhaustive {T : Table K N (fun _ => V)} (hTot : Total T) :
    Exhaustive (unpivotDrop T) := by
  rintro k ⟨n₀, hn₀⟩ n
  simp only [Table.Present, unpivotDrop] at hn₀ ⊢
  obtain ⟨x, hx⟩ := Multiset.exists_mem_of_ne_zero hn₀
  obtain ⟨f, hf, _⟩ := Multiset.mem_bind.mp hx
  obtain ⟨v, hv⟩ := Option.ne_none_iff_exists'.mp (hTot k f hf n)
  refine Multiset.card_pos.mp (Multiset.card_pos_iff_exists_mem.mpr
    ⟨fun _ => some v, Multiset.mem_bind.mpr ⟨f, hf, ?_⟩⟩)
  simp [hv]

/-- The totality upgrade of ADR 0020, semantically: pivoting an exhaustive,
minimal long table yields only known cells.  `Exhaustive` supplies the row
at every `(k, n)` and `Minimal` supplies the value in it; no
population-relative completeness fact appears, which is the content of the
ADR's `exhaustive` / `complete_over` distinction. -/
theorem pivot_total_of_exhaustive [Fintype N] {L : Table (K × N) Unit (fun _ => V)}
    (hE : Exhaustive L) (hM : Minimal L) : Total (pivot L) := by
  intro k f hf n
  by_cases hg : ∀ n', (L.rows (k, n')).card = 0
  · simp [pivot, hg] at hf
  · have hfF : f = fun n' => cellOf (L.rows (k, n')) := by
      simp only [pivot] at hf
      rw [if_neg hg] at hf
      exact Multiset.mem_singleton.mp hf
    obtain ⟨n₀, hn₀⟩ := not_forall.mp hg
    have hpres : L.rows (k, n) ≠ 0 := by
      refine hE k ⟨n₀, fun h0 => hn₀ ?_⟩ n
      simp [h0]
    have htl : (L.rows (k, n)).toList ≠ [] := by
      simpa [Multiset.toList_eq_nil] using hpres
    obtain ⟨g, l, hl⟩ := List.exists_cons_of_ne_nil htl
    have hgmem : g ∈ L.rows (k, n) := by
      rw [← Multiset.mem_toList, hl]
      exact List.mem_cons_self
    obtain ⟨u, hu⟩ := hM (k, n) g hgmem
    have hu' : g () ≠ none := by cases u; exact hu
    have hcell : cellOf (L.rows (k, n)) = g () := by
      simp [cellOf, hl]
    rw [hfF]
    simpa [hcell] using hu'

/-- A single-variant name axis is exhaustive trivially: a present residual
key has a row for some name, and there is only one name to have.  This
backs the checker's single-variant refinement of the totality upgrade. -/
theorem exhaustive_of_subsingleton [Subsingleton N] (T : Table (K × N) H σ) :
    Exhaustive T := by
  rintro k ⟨n₀, h₀⟩ n
  rwa [Subsingleton.elim n n₀]

/-! ### Propagation of the rectangle fact

The key-preserving rows of ADR 0020 section 2's conservative table: a
non-dropping `map` preserves `Exhaustive` (hence so does `leftJoin`, whose
body never drops), `aggregate` preserves it (one output row per present
key), and the `bind` of two carriers preserves it.  `split` destroys it
(`split_not_exhaustive`): a predicate can read the name axis and cut a
fiber, which is the honest price of keeping the name in the key --
contrast `pivotAttr_splitSafe`, where the whole bag rides with its
residual key. -/

/-- A non-dropping `map` (every row yields at least one output row)
preserves row presence in both directions. -/
theorem map_present_iff {φ : K → Row H σ → Multiset (Row H' σ')}
    (hφ : ∀ k f, φ k f ≠ 0) (T : Table K H σ) (k : K) :
    (map φ T).Present k ↔ T.Present k := by
  simp only [Table.Present, map]
  constructor
  · intro h hrows
    exact h (by simp [hrows])
  · intro h h0
    obtain ⟨f, hf⟩ := Multiset.exists_mem_of_ne_zero h
    obtain ⟨y, hy⟩ := Multiset.exists_mem_of_ne_zero (hφ k f)
    have hmem : y ∈ (T.rows k).bind (φ k) := Multiset.mem_bind.mpr ⟨f, hf, hy⟩
    rw [h0] at hmem
    exact Multiset.notMem_zero _ hmem

/-- A non-dropping `map` preserves the rectangle: presence is preserved in
both directions, so full fibers stay full.  A `map` that can drop (a
filter) is excluded, and rightly so: it can empty one name of a fiber. -/
theorem map_exhaustive {φ : (K × N) → Row H σ → Multiset (Row H' σ')}
    (hφ : ∀ p f, φ p f ≠ 0) {T : Table (K × N) H σ} (hE : Exhaustive T) :
    Exhaustive (map φ T) := by
  rintro k ⟨n₀, h₀⟩ n
  rw [map_present_iff hφ]
  exact hE k ⟨n₀, (map_present_iff hφ T (k, n₀)).mp h₀⟩ n

/-- `leftJoin` preserves the rectangle: its body emits the unmatched row
with missing right columns rather than dropping it, so it is a
non-dropping `map`. -/
theorem leftJoin_exhaustive (key : (K × N) → U) (right : Table U G τ)
    {T : Table (K × N) H σ} (hE : Exhaustive T) :
    Exhaustive (leftJoin key right T) := by
  refine map_exhaustive (fun p f => ?_) hE
  by_cases hR : (right.rows (key p)).card = 0
  · simp only [if_pos hR]
    intro h0
    simpa using congrArg Multiset.card h0
  · simp only [if_neg hR]
    intro h0
    exact hR (by simpa using congrArg Multiset.card h0)

/-- `aggregate` preserves the rectangle: it collapses each present fiber
to one row and leaves absent fibers absent, so presence is untouched. -/
theorem aggregate_exhaustive (f : (K × N) → Multiset (Row H σ) → Row H σ)
    {T : Table (K × N) H σ} (hE : Exhaustive T) :
    Exhaustive (aggregate f T) := by
  have hiff : ∀ p, (aggregate f T).Present p ↔ T.Present p := by
    intro p
    simp only [Table.Present, aggregate]
    constructor
    · intro h hrows
      exact h (by simp [hrows])
    · intro h
      rw [if_neg (fun hc => h (Multiset.card_eq_zero.mp hc))]
      intro h0
      simpa using congrArg Multiset.card h0
  rintro k ⟨n₀, h₀⟩ n
  rw [hiff]
  exact hE k ⟨n₀, (hiff (k, n₀)).mp h₀⟩ n

/-- The `bind` of two carriers of the rectangle carries it: a fiber present
in the union is present on one side, whose fullness fills the union. -/
theorem bind_exhaustive {T₀ T₁ : Table (K × N) H σ}
    (h₀ : Exhaustive T₀) (h₁ : Exhaustive T₁) : Exhaustive (bind T₀ T₁) := by
  have hiff : ∀ p, (bind T₀ T₁).Present p ↔ T₀.Present p ∨ T₁.Present p := by
    intro p
    simp only [Table.Present, bind]
    have hadd : T₀.rows p + T₁.rows p = 0 ↔ T₀.rows p = 0 ∧ T₁.rows p = 0 := by
      constructor
      · intro hst
        have hc := congrArg Multiset.card hst
        simp only [Multiset.card_add, Multiset.card_zero] at hc
        rw [Nat.add_eq_zero_iff] at hc
        exact ⟨Multiset.card_eq_zero.mp hc.1, Multiset.card_eq_zero.mp hc.2⟩
      · rintro ⟨hs, ht⟩
        simp [hs, ht]
    constructor
    · intro hne
      by_cases h0' : T₀.rows p = 0
      · exact Or.inr fun h1 => hne (hadd.mpr ⟨h0', h1⟩)
      · exact Or.inl h0'
    · rintro (h | h) h0
      · exact h (hadd.mp h0).1
      · exact h (hadd.mp h0).2
  rintro k ⟨n₀, h₀'⟩ n
  rw [hiff]
  rcases (hiff (k, n₀)).mp h₀' with h | h
  · exact Or.inl (h₀ k ⟨n₀, h⟩ n)
  · exact Or.inr (h₁ k ⟨n₀, h⟩ n)

/-- `split` does *not* preserve the rectangle: a predicate that reads the
name axis cuts a fiber, leaving one side with some names of a key but not
others.  This is ADR 0020's "destroyed by `split`" row, and the honest
price of the index formulation; recognizing predicates that provably
ignore the name axis is that ADR's axis-aware open question. -/
theorem split_not_exhaustive :
    ¬ ∀ (s : Unit × Bool → Bool) (T : Table (Unit × Bool) Unit (fun _ => Unit)),
        Exhaustive T → Exhaustive (split s T).1 := by
  intro h
  set T : Table (Unit × Bool) Unit (fun _ => Unit) :=
    ⟨fun _ => {fun _ => some ()}⟩ with hT
  have hTE : Exhaustive T := by
    rintro k - n
    intro h0
    simpa [hT] using congrArg Multiset.card h0
  have hpres : (split (fun p => p.2) T).1.Present ((), false) := by
    intro h0
    simpa [hT, split] using congrArg Multiset.card h0
  have hnot := h (fun p => p.2) T hTE () ⟨false, hpres⟩ true
  simp [Table.Present, split] at hnot

end Mensura
