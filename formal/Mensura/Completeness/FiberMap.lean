/-
Safe completeness, key-preserving fragment: the split-invariant operations
express *exactly* the split-invariant (leakage-safe) transformations.

Main Source:  Chapter 5 of F. A. N. Verri (2026). Data Science Project: An
Inductive Learning Approach. Version v1.0.0. Victoria, British Columbia,
Canada: Leanpub. doi: 10.5281/zenodo.14498010. url: https://leanpub.com/dsp.

The backbone is the structural fact that a key-preserving operation is
split-invariant precisely when it is **fiberwise** -- each output key reads
only its own input key.  We package that fiberwise shape as `fiberMap` and
prove the characterization

    f is split-invariant and key-local  ↔  f = fiberMap Φ for a strict Φ
    (`splitInvariant_keyLocal_iff_fiberMap`)

so `fiberMap` is the *universal* safe key-preserving operation, and `map`
(the per-row, `BindHom` case) and `aggregate` (the whole-bag fold case) are
its two principal generators.  We also prove both hypotheses are necessary
(`fiberMap_nonstrict_not_splitInvariant`, `keySwap_not_keyLocal`), pinning
the boundary exactly.  Together with the separations in
`Mensura.SplitSafety` (`aggregate_not_bindHom`,
`project_not_preservesDisjoint`) and `Mensura.Reshape`
(`pivot_not_splitInvariant`), these form an independence matrix in which no
condition is redundant.

Honest scope: the clean `iff` here is for the key-preserving fragment; the
graded generalization along a reindexing function is
`Mensura.Completeness.Reindex`.  Order-dependent verbs (window functions:
`lag`, `cumsum`, `rank`) are not bag operations; they require lifting from
the bag monad to the list monad (an explicit row-number index) and are out
of scope, as in the chapter's "grouped and arranged" section.
-/

import Mensura.Core.Ops
import Mensura.Rectangle

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {H' : Type _} {σ' : H' → Type}
variable {K' N : Type _}

/-! ## The universal fiberwise operation -/

/-- A *fiber map*: the output bag at each key is a function of the input bag at
the *same* key, and nothing else.  This is the universal shape of a
key-preserving split-invariant operation (`splitInvariant_keyLocal_iff_fiberMap`):
`map` and `aggregate` are both `fiberMap`s. -/
def fiberMap (Φ : K → Multiset (Row H σ) → Multiset (Row H' σ')) (T : Table K H σ) :
    Table K H' σ' :=
  ⟨fun k => Φ k (T.rows k)⟩

/-- A fiber action is *strict* when it sends the empty bag to the empty bag (it
does not fabricate rows for an absent key).  Strictness is exactly what makes a
`fiberMap` split-invariant. -/
def Strict (Φ : K → Multiset (Row H σ) → Multiset (Row H' σ')) : Prop :=
  ∀ k, Φ k 0 = 0

/-- A key-preserving operation is *key-local* when each output key depends only on
the input at that same key.  This is the second ingredient of the
characterization: split-invariance forbids reading across keys *additively*, but
not, on its own, key relabelings (`keySwap_not_keyLocal`); key-locality rules
those out, leaving exactly the fiber maps. -/
def KeyLocal (f : Table K H σ → Table K H' σ') : Prop :=
  ∀ (T T' : Table K H σ) (k : K), T.rows k = T'.rows k → (f T).rows k = (f T').rows k

/-- `map` is a fiber map: per-row `Multiset.bind` of the input bag. -/
theorem map_eq_fiberMap (φ : K → Row H σ → Multiset (Row H' σ')) :
    map φ = fiberMap (fun k m => m.bind (φ k)) := rfl

/-- `aggregate` is a fiber map: the whole-bag fold of the input bag. -/
theorem aggregate_eq_fiberMap (f : K → Multiset (Row H σ) → Row H σ) :
    aggregate f = fiberMap (fun k m => if m.card = 0 then 0 else {f k m}) := rfl

/-! ### A strict fiber map is split-safe -/

/-- A strict `fiberMap` is split-invariant: under disjointness one summand is
empty at each key, and strictness makes folding the union the same as folding the
nonempty side. -/
theorem fiberMap_splitInvariant {Φ : K → Multiset (Row H σ) → Multiset (Row H' σ')}
    (hΦ : Strict Φ) : SplitInvariant (fiberMap Φ) := by
  intro T₀ T₁ hdisj
  apply Table.ext_rows
  intro k
  have hk := hΦ k
  simp only [fiberMap, bind]
  rcases hdisj k with h | h
  · rw [h]; simp [hk]
  · rw [h]; simp [hk]

/-- A strict `fiberMap` preserves disjointness: an empty input fiber stays empty. -/
theorem fiberMap_preservesDisjoint {Φ : K → Multiset (Row H σ) → Multiset (Row H' σ')}
    (hΦ : Strict Φ) : PreservesDisjoint (fiberMap Φ) := by
  intro T₀ T₁ hdisj k
  rcases hdisj k with h | h
  · exact Or.inl (by simp [fiberMap, h, hΦ k])
  · exact Or.inr (by simp [fiberMap, h, hΦ k])

/-- Hence a strict `fiberMap` is `SplitSafe`, so it composes into pipelines that
stay split-invariant. -/
theorem fiberMap_splitSafe {Φ : K → Multiset (Row H σ) → Multiset (Row H' σ')}
    (hΦ : Strict Φ) : SplitSafe (fiberMap Φ) :=
  ⟨fiberMap_preservesDisjoint hΦ, fiberMap_splitInvariant hΦ⟩

/-- Every `fiberMap` is key-local by construction. -/
theorem fiberMap_keyLocal (Φ : K → Multiset (Row H σ) → Multiset (Row H' σ')) :
    KeyLocal (fiberMap Φ) := by
  intro T T' k h
  simp only [fiberMap]
  rw [h]

/-- A presence-preserving fiber action preserves the rectangle fact
(ADR 0020 section 2): strictness keeps absent fibers absent, and the
no-emptying hypothesis keeps present fibers present, so full fibers stay
full.  Both `group_map` shapes satisfy it: the aggregate shape folds a
present fiber to one row (`aggregate_exhaustive` is the special case), and
the window shape emits one output row per input row. -/
theorem fiberMap_exhaustive {Φ : (K × N) → Multiset (Row H σ) → Multiset (Row H' σ')}
    (hΦ0 : Strict Φ) (hΦ : ∀ p m, m ≠ 0 → Φ p m ≠ 0)
    {T : Table (K × N) H σ} (hE : Exhaustive T) : Exhaustive (fiberMap Φ T) := by
  have hiff : ∀ p, (fiberMap Φ T).Present p ↔ T.Present p := by
    intro p
    simp only [Table.Present, fiberMap]
    constructor
    · intro h hrows
      rw [hrows] at h
      exact h (hΦ0 p)
    · exact fun h => hΦ p _ h
  rintro k ⟨n₀, h₀⟩ n
  rw [hiff]
  exact hE k ⟨n₀, (hiff (k, n₀)).mp h₀⟩ n

/-! ### Converse: every key-local split-invariant operation is a strict fiber map -/

/-- The table that holds `m` at key `k` and is empty elsewhere. -/
def pointTable [DecidableEq K] (k : K) (m : Multiset (Row H σ)) : Table K H σ :=
  ⟨fun k' => if k' = k then m else 0⟩

@[simp] theorem pointTable_rows_self [DecidableEq K] (k : K) (m : Multiset (Row H σ)) :
    (pointTable k m).rows k = m := by simp [pointTable]

/-- The fiber action *witnessed* by a key-local operation: run it on a table
supported at the single key `k`, and read off that key. -/
def fiberOf [DecidableEq K] (f : Table K H σ → Table K H' σ')
    (k : K) (m : Multiset (Row H σ)) : Multiset (Row H' σ') :=
  (f (pointTable k m)).rows k

/-- Representation: a key-local `f` *is* the fiber map of its witnessed action.
This direction needs only key-locality -- split-invariance is not used. -/
theorem keyLocal_eq_fiberMap [DecidableEq K] {f : Table K H σ → Table K H' σ'}
    (hf : KeyLocal f) : f = fiberMap (fiberOf f) := by
  funext T
  apply Table.ext_rows
  intro k
  show (f T).rows k = (f (pointTable k (T.rows k))).rows k
  exact hf T (pointTable k (T.rows k)) k (by simp)

/-- `x = x + x` forces `x = 0` in a multiset (compare cardinalities). -/
theorem multiset_self_add {α : Type _} {x : Multiset α} (h : x = x + x) : x = 0 := by
  have hc : Multiset.card x = Multiset.card x + Multiset.card x := by
    have := congrArg Multiset.card h
    rwa [Multiset.card_add] at this
  exact Multiset.card_eq_zero.mp (by omega)

/-- A split-invariant operation maps the empty table to the empty table: feeding
it `∅ = ∅ + ∅` gives `f ∅ = f ∅ + f ∅`, which forces `f ∅` empty.  Stated with a
possibly different output key type, so it serves both the key-preserving
(`fiberOf_strict`) and key-changing (`reindexFiberOf_strict`) cases. -/
theorem splitInvariant_empty {f : Table K H σ → Table K' H' σ'} (hf : SplitInvariant f)
    (k : K') : (f ⟨fun _ => 0⟩).rows k = 0 := by
  have hdis : Disjoint (⟨fun _ => 0⟩ : Table K H σ) ⟨fun _ => 0⟩ := fun _ => Or.inl rfl
  have hb : bind (⟨fun _ => 0⟩ : Table K H σ) ⟨fun _ => 0⟩ = ⟨fun _ => 0⟩ := by
    apply Table.ext_rows; intro k; simp [bind]
  have h := hf ⟨fun _ => 0⟩ ⟨fun _ => 0⟩ hdis
  rw [hb] at h
  have hrow := congrArg (fun U => Table.rows U k) h
  simp only [bind] at hrow
  exact multiset_self_add hrow

/-- Hence the witnessed action of a split-invariant operation is strict. -/
theorem fiberOf_strict [DecidableEq K] {f : Table K H σ → Table K H' σ'}
    (hf : SplitInvariant f) : Strict (fiberOf f) := by
  intro k
  unfold fiberOf
  have hpt : pointTable k (0 : Multiset (Row H σ)) = ⟨fun _ => 0⟩ := by
    apply Table.ext_rows; intro k'; simp [pointTable]
  rw [hpt]
  exact splitInvariant_empty hf k

/-- **Safe completeness (key-preserving fragment).**  A key-preserving operation
is split-invariant and key-local *iff* it is a strict `fiberMap`.  So the
split-invariant, key-local transformations are *exactly* the strict fiber maps:
`fiberMap` is their universal form, and `map`/`aggregate` are the two principal
generators (`map_eq_fiberMap`, `aggregate_eq_fiberMap`).  Nothing safe is missing
and nothing unsafe sneaks in. -/
theorem splitInvariant_keyLocal_iff_fiberMap [DecidableEq K]
    {f : Table K H σ → Table K H' σ'} :
    (SplitInvariant f ∧ KeyLocal f) ↔ ∃ Φ, Strict Φ ∧ f = fiberMap Φ := by
  constructor
  · rintro ⟨hSI, hKL⟩
    exact ⟨fiberOf f, fiberOf_strict hSI, keyLocal_eq_fiberMap hKL⟩
  · rintro ⟨Φ, hΦ, rfl⟩
    exact ⟨fiberMap_splitInvariant hΦ, fiberMap_keyLocal Φ⟩

/-! ## Sharpness: both hypotheses of the characterization are necessary -/

/-- Strictness is necessary: a `fiberMap` whose action fabricates a row for the
empty fiber (here a constant singleton) is not split-invariant. -/
theorem fiberMap_nonstrict_not_splitInvariant :
    ¬ SplitInvariant
        (fiberMap (K := Unit) (H := Unit) (σ := fun _ => Unit)
          (H' := Unit) (σ' := fun _ => Unit)
          (fun _ _ => {fun _ => none})) := by
  intro h
  have hT := h ⟨fun _ => 0⟩ ⟨fun _ => 0⟩ (fun _ => Or.inl rfl)
  apply_fun (fun U => (U.rows ()).card) at hT
  simp [fiberMap, bind] at hT

/-- Swap the two keys of a `Bool`-indexed table. -/
def keySwap (T : Table Bool H σ) : Table Bool H σ := ⟨fun b => T.rows (!b)⟩

/-- `keySwap` *is* split-invariant (it is even a bind-homomorphism): it just
relabels keys, and union is computed key-by-key either way. -/
theorem keySwap_splitInvariant : SplitInvariant (keySwap (H := H) (σ := σ)) := by
  intro T₀ T₁ _
  apply Table.ext_rows
  intro b
  simp [keySwap, bind]

/-- Key-locality is necessary: `keySwap` is split-invariant yet not key-local --
its output at a key reads the *other* key -- so it is not a `fiberMap`.  This is
why the characterization needs key-locality as a separate hypothesis. -/
theorem keySwap_not_keyLocal :
    ¬ KeyLocal (keySwap (H := Unit) (σ := fun _ => Unit)) := by
  intro h
  have hcontra := h ⟨fun b => bif b then 0 else {fun _ => none}⟩ ⟨fun _ => 0⟩ true rfl
  simp [keySwap] at hcontra

end Mensura
