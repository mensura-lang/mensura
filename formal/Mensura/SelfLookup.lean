import Mensura.Completeness.BinaryJoins
import Mensura.Completeness.Reindex
import Mensura.Leak

/-!
Self-derived lookups: joining a table against a reference computed from that same
table, and the splits on which doing so before the split is leak-free.

`lookup_splitSafe` assumes a *fixed* right table.  A pipeline can instead build
the right table from its own input (a per-group statistic, say) and join it back:
`T ↦ lookup key (g T) T`.  Run before a `split`, each side then carries values
computed from the other side's rows unless the split respects the groups.  The
leak-free property is that the side of the result *is* the result of running the
whole pipeline on that side alone:

  (split s (lookup key (g T) T)).i = lookup key (g (split s T).i) (split s T).i

This holds whenever `g` commutes with the splits aligned along `key`
(`CommutesAligned`), the split is aligned (`Aligned key s sR`: the predicate
reads only the lookup key), and the join is a jointly strict binary fibre map
(`selfJoin_split`).  `lookup` and `lookupTotal` are such maps
(`lookup_self_split`, `lookupTotal_self_split`).

The reference statistics a program builds are aligned-commuting: every strict
`gatherMap` along `Prod.fst` (`gatherMap_commutesAligned`), hence `demote`
followed by any strict per-key reduction, the surface `demote d |> map_bags`
(`demote_then_aggregate_commutesAligned`).  The headline instance is
`lookup_demote_aggregate_split`.

Alignment cannot be dropped: `joinMean_not_splitInvariant` (`Mensura.Leak`) is
the same composite on an unaligned split.

Results:

- `CommutesAligned`, `fiberMap_strict_split`, `CommutesAligned.fiberMap`
- `selfJoin_split`, `lookup_self_split`, `lookupTotal_self_split`
- `selfJoin_union_split`: the diagonal form, whole = union of the per-side runs
- `gatherMap_commutesAligned`, `demote_then_aggregate_commutesAligned`
- `lookup_demote_aggregate_split`
-/

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {H' : Type _} {σ' : H' → Type}
variable {U G : Type _} {τ : G → Type}
variable {G' : Type _} {τ' : G' → Type}

/-- `g` commutes with every split aligned along `key`: building the reference
from one side of the input gives that side of the reference built from the
whole. -/
def CommutesAligned (key : K → U) (g : Table K H σ → Table U G τ) : Prop :=
  ∀ (s : K → Bool) (sR : U → Bool), Aligned key s sR → ∀ T,
    g (split s T).1 = (split sR (g T)).1 ∧ g (split s T).2 = (split sR (g T)).2

/-- A strict fibre map commutes with every split. -/
theorem fiberMap_strict_split {Φ : U → Multiset (Row G τ) → Multiset (Row G' τ')}
    (hΦ : Strict Φ) (sR : U → Bool) (R : Table U G τ) :
    fiberMap Φ (split sR R).1 = (split sR (fiberMap Φ R)).1 ∧
      fiberMap Φ (split sR R).2 = (split sR (fiberMap Φ R)).2 := by
  constructor <;>
  · apply Table.ext_rows
    intro u
    simp only [fiberMap, split]
    cases sR u <;> simp [hΦ u]

/-- Post-composing a strict per-key reduction keeps a reference aligned-commuting. -/
theorem CommutesAligned.fiberMap {key : K → U} {g : Table K H σ → Table U G τ}
    (hg : CommutesAligned key g) {Φ : U → Multiset (Row G τ) → Multiset (Row G' τ')}
    (hΦ : Strict Φ) : CommutesAligned key (fun T => Mensura.fiberMap Φ (g T)) := by
  intro s sR hal T
  obtain ⟨h₁, h₂⟩ := hg s sR hal T
  obtain ⟨e₁, e₂⟩ := fiberMap_strict_split hΦ sR (g T)
  exact ⟨by beta_reduce; rw [h₁, e₁], by beta_reduce; rw [h₂, e₂]⟩

/-- **The self-join is leak-free on aligned splits.**  For a jointly strict binary
fibre map and an aligned-commuting reference, each side of the joined table is the
join computed on that side alone. -/
theorem selfJoin_split (key : K → U)
    {Φ : K → Multiset (Row H σ) → Multiset (Row G τ) → Multiset (Row H' σ')}
    (hΦ : JointlyStrict Φ) {g : Table K H σ → Table U G τ} (hg : CommutesAligned key g)
    {s : K → Bool} {sR : U → Bool} (hal : Aligned key s sR) (T : Table K H σ) :
    split s (biFiberMap key Φ T (g T))
      = (biFiberMap key Φ (split s T).1 (g (split s T).1),
         biFiberMap key Φ (split s T).2 (g (split s T).2)) := by
  obtain ⟨h₁, h₂⟩ := hg s sR hal T
  rw [h₁, h₂]
  ext1 <;>
  · apply Table.ext_rows
    intro k
    have := hal k
    simp only [biFiberMap, split]
    cases hs : s k <;> rw [hs] at this <;> rw [← this] <;> simp [hΦ k]

/-- The diagonal form: running the self-join on the whole equals the union of the
runs on the two sides of an aligned split. -/
theorem selfJoin_union_split (key : K → U)
    {Φ : K → Multiset (Row H σ) → Multiset (Row G τ) → Multiset (Row H' σ')}
    (hΦ : JointlyStrict Φ) {g : Table K H σ → Table U G τ} (hg : CommutesAligned key g)
    {s : K → Bool} {sR : U → Bool} (hal : Aligned key s sR) (T : Table K H σ) :
    biFiberMap key Φ T (g T)
      = union (biFiberMap key Φ (split s T).1 (g (split s T).1))
          (biFiberMap key Φ (split s T).2 (g (split s T).2)) := by
  have e := selfJoin_split key hΦ hg hal T
  rw [← union_split s (biFiberMap key Φ T (g T)), e]

/-- `lookup` against a reference built from its own input is leak-free on aligned
splits. -/
theorem lookup_self_split (key : K → U) {g : Table K H σ → Table U G τ}
    (hg : CommutesAligned key g) {s : K → Bool} {sR : U → Bool} (hal : Aligned key s sR)
    (T : Table K H σ) :
    split s (lookup key (g T) T)
      = (lookup key (g (split s T).1) (split s T).1,
         lookup key (g (split s T).2) (split s T).2) :=
  selfJoin_split key (Φ := fun _ m n => m.bind (fun f =>
      if n.card = 0 then {f.elim (fun _ => none)} else n.map (fun r => f.elim r)))
    (fun k => by simp) hg hal T

/-- Likewise for `lookupTotal`. -/
theorem lookupTotal_self_split (key : K → U) {g : Table K H σ → Table U G τ}
    (hg : CommutesAligned key g) {s : K → Bool} {sR : U → Bool} (hal : Aligned key s sR)
    (T : Table K H σ) :
    split s (lookupTotal key (g T) T)
      = (lookupTotal key (g (split s T).1) (split s T).1,
         lookupTotal key (g (split s T).2) (split s T).2) :=
  selfJoin_split key (Φ := fun _ m n => m.bind (fun f => n.map (fun r => f.elim r)))
    (fun k => by simp) hg hal T

/-! ### The references a program builds: gather along the coarse key -/

section Gather

variable {D : Type _} [Fintype D]

/-- A strict `gatherMap` along `Prod.fst` commutes with the aligned splits: a split
of `K × D` that reads only the `K` component sends each output fiber's whole input
to one side. -/
theorem gatherMap_commutesAligned
    {Ψ : K → (D → Multiset (Row H σ)) → Multiset (Row H' σ')}
    (hΨ : ∀ k, Ψ k (fun _ => 0) = 0) :
    CommutesAligned (Prod.fst : K × D → K) (gatherMap Ψ) := by
  intro s sR hal T
  constructor <;>
  · apply Table.ext_rows
    intro k
    simp only [gatherMap, split]
    have hk : ∀ d, s (k, d) = sR k := fun d => hal (k, d)
    simp only [hk]
    cases sR k <;> simp [hΨ k]

/-- `demote` is a strict gather, so it commutes with the aligned splits. -/
theorem demote_commutesAligned :
    CommutesAligned (Prod.fst : K × D → K) (demote (H := H) (σ := σ) (D := D)) := by
  rw [demote_eq_gatherMap]
  exact gatherMap_commutesAligned (fun k => by simp)

/-- `demote` followed by a per-key reduction (the surface `demote d |> map_bags`)
commutes with the aligned splits. -/
theorem demote_then_aggregate_commutesAligned
    (f : K → Multiset (Row (H ⊕ Unit) (Sum.elim σ (fun _ => D))) →
      Row (H ⊕ Unit) (Sum.elim σ (fun _ => D))) :
    CommutesAligned (Prod.fst : K × D → K)
      (fun T : Table (K × D) H σ => aggregate f (demote T)) := by
  have hstrict : Strict (fun k (m : Multiset (Row (H ⊕ Unit) (Sum.elim σ (fun _ => D)))) =>
      if m.card = 0 then (0 : Multiset _) else {f k m}) := fun k => by simp
  have := demote_commutesAligned (H := H) (σ := σ) (D := D) |>.fiberMap hstrict
  simpa [aggregate_eq_fiberMap] using this

/-- **Headline.**  Joining each row back to a per-group statistic of its own table
(`demote` to the group key, reduce, `lookup` on the group key) and then splitting
on a predicate of the group key is leak-free: each side is exactly the pipeline
run on that side alone. -/
theorem lookup_demote_aggregate_split
    (f : K → Multiset (Row (H ⊕ Unit) (Sum.elim σ (fun _ => D))) →
      Row (H ⊕ Unit) (Sum.elim σ (fun _ => D)))
    {s : K × D → Bool} {sR : K → Bool} (hal : Aligned Prod.fst s sR)
    (T : Table (K × D) H σ) :
    split s (lookup Prod.fst (aggregate f (demote T)) T)
      = (lookup Prod.fst (aggregate f (demote (split s T).1)) (split s T).1,
         lookup Prod.fst (aggregate f (demote (split s T).2)) (split s T).2) :=
  lookup_self_split Prod.fst (demote_then_aggregate_commutesAligned f) hal T

end Gather

end Mensura
