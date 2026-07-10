/-
Indexed tables over multisets, with per-column typed domains: the core data
structure of the algebra, `split`/`bind`, and the three safety properties.

Main Source:  Chapter 5, section "Formal structured data" and "Split-invariant
operations", of F. A. N. Verri (2026). Data Science Project: An Inductive Learning
Approach. Version v1.0.0. Victoria, British Columbia, Canada: Leanpub. doi:
10.5281/zenodo.14498010. url: https://leanpub.com/dsp.

## Representation: a row's content is a multiset of nested rows

A schema is a column-name type `H` together with a per-column domain
`σ : H → Type`.  A nested row is a *dependent* function

    Row H σ = (h : H) → Cell (σ h)

so each column carries its own value type (a genuine schema), and `Cell` makes
each value optionally missing.  The content at a key is a `Multiset (Row H σ)`,
a bag of nested rows.  Three advantages over the chapter's column-major aligned
tuples carry over:

* **Order is honest.**  The chapter calls the order of nested rows "arbitrary
  but fixed"; a multiset is exactly that (lists up to permutation), so we model
  it instead of asserting an order we never use.

* **Associations cannot desync.**  Each nested row is one function, so its
  column values are bound together structurally; the chapter's positional
  alignment invariant is unrepresentable.

* **`bind` is a real commutative monoid.**  Multiset union is commutative,
  associative, total, and bias-free, so the row-wise operations are
  bind-homomorphisms unconditionally, hence split-invariant for free.

`card(r)` is the multiset's cardinality; `card(r) = 0` is an absent row.

Three properties are formalized, in increasing/orthogonal strength:
* `SplitInvariant` -- distributes over the bind of *disjoint* tables (the
  chapter's def:split-invariance, the per-operation guarantee);
* `BindHom` -- distributes over *every* bind (strictly stronger);
* `SplitSafe` -- split-invariant *and* disjointness-preserving, the class
  closed under composition, so an entire *pipeline* stays split-invariant.

The inventory of results and their dependencies lives in the blueprint
(`formal/blueprint/`), not here.
-/

import Mathlib.Data.Multiset.Bind
import Mathlib.Tactic

namespace Mensura

/-- The missing marker `?` from the chapter: a cell value may be absent. -/
abbrev Cell (β : Type _) := Option β

/-- A nested row over a schema: column names `H` with per-column domain `σ h`.
Each column may carry a different value type, and every column is bound together
inside the one (dependent) function -- so cross-column associations are
intrinsic, not positional. -/
abbrev Row (H : Type _) (σ : H → Type) := (h : H) → Cell (σ h)

/-- An indexed table over schema `(H, σ)`.

`rows k` is the multiset of nested rows sharing key `k`; its cardinality is the
chapter's `card(r)`, and `0` means the row is absent. -/
@[ext]
structure Table (K H : Type _) (σ : H → Type) where
  rows : K → Multiset (Row H σ)

variable {K H : Type _} {σ : H → Type}
variable {K' H' : Type _} {σ' : H' → Type}
variable {K'' H'' : Type _} {σ'' : H'' → Type}
variable {G : Type _} {τ : G → Type}

/-- Combine a left row and a right row into a row over the disjoint-union schema
`Sum.elim σ τ`.  This is the dependent counterpart of `Sum.elim`: at `Sum.inl h`
it has type `Cell (σ h)`, at `Sum.inr g` type `Cell (τ g)`. -/
def Row.elim (f : Row H σ) (r : Row G τ) : Row (H ⊕ G) (Sum.elim σ τ) :=
  fun c => match c with
    | Sum.inl h => f h
    | Sum.inr g => r g

@[simp] theorem Row.elim_inl (f : Row H σ) (r : Row G τ) (h : H) :
    f.elim r (Sum.inl h) = f h := rfl

@[simp] theorem Row.elim_inr (f : Row H σ) (r : Row G τ) (g : G) :
    f.elim r (Sum.inr g) = r g := rfl

/-- A row is present when it has positive cardinality. -/
def Table.Present (T : Table K H σ) (k : K) : Prop := T.rows k ≠ 0

/-- Two tables are equal when they agree key-by-key. -/
theorem Table.ext_rows {T U : Table K H σ} (h : ∀ k, T.rows k = U.rows k) : T = U := by
  obtain ⟨r₀⟩ := T
  obtain ⟨r₁⟩ := U
  simp only [Table.mk.injEq]
  funext k
  exact h k

/-- def:split.  An indicator `s` routes each key's whole multiset of rows to one
side, leaving the other empty. -/
def split (s : K → Bool) (T : Table K H σ) : Table K H σ × Table K H σ :=
  (⟨fun k => bif s k then 0 else T.rows k⟩,
   ⟨fun k => bif s k then T.rows k else 0⟩)

/-- def:bind.  Multiset union of the two tables' rows at each key: the chapter's
cell concatenation made order-free, commutative, associative, total, bias-free. -/
def bind (T₀ T₁ : Table K H σ) : Table K H σ :=
  ⟨fun k => T₀.rows k + T₁.rows k⟩

/-- def:disjoint-tables.  At every key, at least one table is empty.  This makes
`split` a partition (so `bind ∘ split = id`), and it is the hypothesis of
`SplitInvariant`. -/
def Disjoint (T₀ T₁ : Table K H σ) : Prop :=
  ∀ k, T₀.rows k = 0 ∨ T₁.rows k = 0

/-- def:split-invariance, faithful to the chapter: `f` distributes over the
`bind` of *disjoint* tables -- exactly what a `split` produces (`split_disjoint`).
`f` may change the schema and key type; disjointness is asked of the inputs.

**This is the property Mensura tracks and enforces.**  The disjointness
hypothesis is load-bearing: a `split` never divides a key's multiset, so
row-collapsing operations like `aggregate` stay invariant
(`aggregate_splitInvariant`).  Drop the hypothesis and it strengthens to
`BindHom`, which `aggregate` fails. -/
def SplitInvariant (f : Table K H σ → Table K' H' σ') : Prop :=
  ∀ T₀ T₁ : Table K H σ, Disjoint T₀ T₁ → f (bind T₀ T₁) = bind (f T₀) (f T₁)

/-- `f` distributes over *every* `bind`: a full commutative-monoid homomorphism,
strictly stronger than `SplitInvariant` (`BindHom.splitInvariant`).  The row-wise
operations satisfy it because they act on each nested row independently and
multiset union distributes (`Multiset.add_bind`). -/
def BindHom (f : Table K H σ → Table K' H' σ') : Prop :=
  ∀ T₀ T₁ : Table K H σ, f (bind T₀ T₁) = bind (f T₀) (f T₁)

/-- Every bind-homomorphism is split-invariant: split-invariance asks for the
equation only on disjoint binds, a special case. -/
theorem BindHom.splitInvariant {f : Table K H σ → Table K' H' σ'} (h : BindHom f) :
    SplitInvariant f := by
  intro T₀ T₁ _
  exact h T₀ T₁

/-- `f` sends disjoint tables to disjoint tables.  This is the missing
ingredient that makes split-invariance *compositional*: `SplitInvariant` alone
is not closed under composition, because `g`'s split-invariance needs its inputs
`f T₀, f T₁` disjoint, which only holds if `f` preserves disjointness. -/
def PreservesDisjoint (f : Table K H σ → Table K' H' σ') : Prop :=
  ∀ T₀ T₁ : Table K H σ, Disjoint T₀ T₁ → Disjoint (f T₀) (f T₁)

/-- The class of operations safe to put in a pipeline between a split and a
bind: split-invariant *and* disjointness-preserving.  Unlike bare
`SplitInvariant`, this is closed under composition (`SplitSafe.comp`) and
contains the identity (`SplitSafe.id`), so a whole pipeline of `SplitSafe`
operations is split-invariant -- applying it between split and bind equals
applying it to the full table.  `project` is split-invariant but *not* here
(it does not preserve disjointness), which is exactly why pipelines through it,
such as `aggregate ∘ project`, can disagree with the full-table result. -/
def SplitSafe (f : Table K H σ → Table K' H' σ') : Prop :=
  PreservesDisjoint f ∧ SplitInvariant f

/-- The identity is split-safe. -/
theorem SplitSafe.id : SplitSafe (id : Table K H σ → Table K H σ) :=
  ⟨fun _ _ h => h, fun _ _ _ => rfl⟩

/-- Split-safe operations are closed under composition.  This is the payoff: the
disjointness `f` preserves is exactly what feeds `g`'s split-invariance, so the
equation threads through the whole pipeline. -/
theorem SplitSafe.comp {f : Table K H σ → Table K' H' σ'}
    {g : Table K' H' σ' → Table K'' H'' σ''} (hg : SplitSafe g) (hf : SplitSafe f) :
    SplitSafe (g ∘ f) := by
  obtain ⟨hfP, hfS⟩ := hf
  obtain ⟨hgP, hgS⟩ := hg
  refine ⟨fun T₀ T₁ h => hgP _ _ (hfP _ _ h), fun T₀ T₁ h => ?_⟩
  show g (f (bind T₀ T₁)) = bind (g (f T₀)) (g (f T₁))
  rw [hfS T₀ T₁ h]
  exact hgS (f T₀) (f T₁) (hfP T₀ T₁ h)

/-- The two halves of a split are disjoint. -/
theorem split_disjoint (s : K → Bool) (T : Table K H σ) :
    Disjoint (split s T).1 (split s T).2 := by
  intro k
  simp only [split]
  cases s k <;> simp

/-- Bind undoes split: split and bind are mutual inverses (one direction). -/
theorem bind_split (s : K → Bool) (T : Table K H σ) :
    bind (split s T).1 (split s T).2 = T := by
  apply Table.ext_rows
  intro k
  simp only [bind, split]
  cases s k <;> simp

/-- `bind` is commutative -- unconditionally. -/
theorem bind_comm (T₀ T₁ : Table K H σ) : bind T₀ T₁ = bind T₁ T₀ := by
  apply Table.ext_rows
  intro k
  simp only [bind]
  exact add_comm _ _

/-- `bind` is associative. -/
theorem bind_assoc (T₀ T₁ T₂ : Table K H σ) :
    bind (bind T₀ T₁) T₂ = bind T₀ (bind T₁ T₂) := by
  apply Table.ext_rows
  intro k
  simp only [bind]
  exact add_assoc _ _ _

/-- `bind` weakens disjointness: a merge is disjoint from a third table iff
*both* of its parts are.  Multiset union can only grow a table's support, so
binding can only *lose* a disjointness fact (binding in a table that overlaps
`T₂` destroys `Disjoint _ T₂`).  This backs the `bind` propagation rule in
`docs/language/08-lineage.md`. -/
theorem bind_disjoint_iff (T₀ T₁ T₂ : Table K H σ) :
    Disjoint (bind T₀ T₁) T₂ ↔ Disjoint T₀ T₂ ∧ Disjoint T₁ T₂ := by
  -- A multiset sum is empty exactly when both summands are.
  have hadd : ∀ s t : Multiset (Row H σ), s + t = 0 ↔ s = 0 ∧ t = 0 := by
    intro s t
    constructor
    · intro hst
      have hc : Multiset.card s + Multiset.card t = 0 := by
        rw [← Multiset.card_add, hst, Multiset.card_zero]
      rw [Nat.add_eq_zero_iff] at hc
      exact ⟨Multiset.card_eq_zero.mp hc.1, Multiset.card_eq_zero.mp hc.2⟩
    · rintro ⟨hs, ht⟩
      simp [hs, ht]
  constructor
  · intro h
    refine ⟨fun k => ?_, fun k => ?_⟩
    · rcases h k with hk | hk
      · exact Or.inl ((hadd _ _).mp hk).1
      · exact Or.inr hk
    · rcases h k with hk | hk
      · exact Or.inl ((hadd _ _).mp hk).2
      · exact Or.inr hk
  · rintro ⟨h₀, h₁⟩ k
    rcases h₀ k with hk₀ | hk₀
    · rcases h₁ k with hk₁ | hk₁
      · exact Or.inl ((hadd _ _).mpr ⟨hk₀, hk₁⟩)
      · exact Or.inr hk₁
    · exact Or.inr hk₀

/-! ### Minimality (the chapter's no-all-missing-row assumption) -/

/-- A nested row is *substantive* when at least one column is present; the
chapter's minimality assumption forbids all-missing nested rows. -/
def Substantive (f : Row H σ) : Prop := ∃ h, f h ≠ none

/-- A table is minimal when every nested row is substantive (the chapter's
standing well-formedness assumption, so `card` counts only real rows). -/
def Minimal (T : Table K H σ) : Prop := ∀ k, ∀ f ∈ T.rows k, Substantive f

/-- `bind` preserves minimality: a row of the union is a row of one summand. -/
theorem Minimal.bind {T₀ T₁ : Table K H σ} (h₀ : Minimal T₀) (h₁ : Minimal T₁) :
    Minimal (bind T₀ T₁) := by
  intro k f hf
  rcases Multiset.mem_add.mp hf with hf | hf
  · exact h₀ k f hf
  · exact h₁ k f hf

/-- `split` preserves minimality: each half's rows are a subset of `T`'s. -/
theorem Minimal.split (s : K → Bool) {T : Table K H σ} (h : Minimal T) :
    Minimal (split s T).1 ∧ Minimal (split s T).2 := by
  have hle₁ : ∀ k, (Mensura.split s T).1.rows k ≤ T.rows k := by
    intro k; simp only [Mensura.split]; cases s k <;> simp
  have hle₂ : ∀ k, (Mensura.split s T).2.rows k ≤ T.rows k := by
    intro k; simp only [Mensura.split]; cases s k <;> simp
  exact ⟨fun k f hf => h k f (Multiset.mem_of_le (hle₁ k) hf),
         fun k f hf => h k f (Multiset.mem_of_le (hle₂ k) hf)⟩

end Mensura
