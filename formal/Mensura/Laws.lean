/-
Equational laws: the rewrite-rule seeds.

Per ADR 0008 (`docs/decisions/0008-formalize-algebra-in-lean.md`), algebraic
laws are stated in equational form (`lhs = rhs` under named side conditions)
so they translate directly into rewrite rules once the processing layer grows
an optimizing plan IR.  The laws below are the prototypical plan rewrites:
`empty` completes `union`'s commutative monoid (unit laws); `flatMap` carries
identity and fusion laws (fusion subsumes filter/filter, filter/mutate, and
select fusion, since all are `flatMap`s, ADR 0015); the joins absorb a preceding
`flatMap` and commute with a left-column `filter` (pushdown); `split`/`union`
cancel in both directions (`union_split` in `Mensura.Core.Defs`, `split_bind`
below); and `promote`/`demote` -- the algebra's `promote`/`demote` --
cancel in both directions on the domain the checker enforces
(`demote_ungroup`, `promote_project`, ADR 0024), with `promote`
preserving functionality (`promote_functional`, the checker's
"`promote` keeps `singletons`" row).
-/

import Mensura.Core.Ops
import Mensura.Reshape

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {K' H' : Type _} {σ' : H' → Type}
variable {K'' H'' : Type _} {σ'' : H'' → Type}
variable {U G : Type _} {τ : G → Type}

/-- The empty table: no rows at any key.  The identity of `union`
(`union_empty`, `empty_union`), completing the commutative monoid that
`union_comm` and `union_assoc` establish. -/
def empty : Table K H σ := ⟨fun _ => 0⟩

/-- Right unit: `union T empty = T`. -/
@[simp] theorem union_empty (T : Table K H σ) : union T empty = T := by
  apply Table.ext_rows
  intro k
  simp [union, empty]

/-- Left unit: `union empty T = T`. -/
@[simp] theorem empty_union (T : Table K H σ) : union empty T = T := by
  apply Table.ext_rows
  intro k
  simp [union, empty]

/-- `flatMap` annihilates `empty`: mapping over no rows yields no rows. -/
@[simp] theorem flatMap_empty (φ : K → Row H σ → Multiset (Row H' σ')) :
    flatMap φ (empty : Table K H σ) = empty := by
  apply Table.ext_rows
  intro k
  simp [flatMap, empty]

/-- Identity law: mapping each row to its singleton is the identity. -/
@[simp] theorem flatMap_id (T : Table K H σ) : flatMap (fun _ f => {f}) T = T := by
  apply Table.ext_rows
  intro k
  simp [flatMap, bind_singleton_id]

/-- Fusion law: two consecutive `flatMap`s collapse into one whose body binds
the second body over the first's output rows.  The prototypical plan
rewrite; with `flatMap_id` it makes `flatMap` bodies a monoid under Kleisli-style
composition. -/
theorem flatMap_flatMap (ψ : K → Row H' σ' → Multiset (Row H'' σ''))
    (φ : K → Row H σ → Multiset (Row H' σ')) (T : Table K H σ) :
    flatMap ψ (flatMap φ T) = flatMap (fun k f => (φ k f).bind (ψ k)) T := by
  apply Table.ext_rows
  intro k
  simp only [flatMap]
  exact Multiset.bind_assoc

/-- def:filtering as a named operation: keep each row iff `p` holds.  A
`flatMap`, so every `flatMap` law applies; named so the pushdown laws below read
the way the optimizer will use them. -/
def filter (p : K → Row H σ → Bool) : Table K H σ → Table K H σ :=
  flatMap (fun k f => bif p k f then {f} else 0)

/-- Join pushdown, fusion form: a `flatMap` feeding `lookupTotal` fuses into one
`flatMap` whose body joins each of the `flatMap`'s output rows.  An instance of
`flatMap_flatMap`, stated separately because a plan IR carries the join as a node,
not as its `flatMap` body. -/
theorem lookupTotal_map (key : K → U) (right : Table U G τ)
    (φ : K → Row H σ → Multiset (Row H' σ')) (T : Table K H σ) :
    lookupTotal key right (flatMap φ T)
      = flatMap (fun k f => (φ k f).bind
          (fun g => (right.rows (key k)).map (fun r => g.elim r))) T :=
  flatMap_flatMap _ φ T

/-- Join pushdown, fusion form, for `lookup`. -/
theorem lookup_map (key : K → U) (right : Table U G τ)
    (φ : K → Row H σ → Multiset (Row H' σ')) (T : Table K H σ) :
    lookup key right (flatMap φ T)
      = flatMap (fun k f => (φ k f).bind (fun g =>
          let R := right.rows (key k)
          if R.card = 0 then {g.elim (fun _ => none)}
          else R.map (fun r => g.elim r))) T :=
  flatMap_flatMap _ φ T

/-- Filter pushdown through `lookupTotal`: a filter that reads only the left
columns (syntactically, only `Sum.inl` columns of the joined row) commutes
below the join.  The optimizer's directed use is left to right: filter
before joining. -/
theorem lookupTotal_filter_pushdown (key : K → U) (right : Table U G τ)
    (p : K → Row H σ → Bool) (T : Table K H σ) :
    filter (fun k g => p k (fun h => g (Sum.inl h))) (lookupTotal key right T)
      = lookupTotal key right (filter p T) := by
  simp only [filter, lookupTotal, flatMap_flatMap]
  apply Table.ext_rows
  intro k
  simp only [flatMap]
  refine Multiset.bind_congr (fun f _ => ?_)
  cases hp : p k f <;>
    simp [hp, Multiset.bind_map, Multiset.bind_singleton]

/-- Filter pushdown through `lookup`: valid because a left row's copies
(matched or the missing-padded survivor) all agree on the left columns, so
the filter keeps or drops them together. -/
theorem lookup_filter_pushdown (key : K → U) (right : Table U G τ)
    (p : K → Row H σ → Bool) (T : Table K H σ) :
    filter (fun k g => p k (fun h => g (Sum.inl h))) (lookup key right T)
      = lookup key right (filter p T) := by
  simp only [filter, lookup, flatMap_flatMap]
  apply Table.ext_rows
  intro k
  simp only [flatMap]
  refine Multiset.bind_congr (fun f _ => ?_)
  cases hp : p k f <;>
    by_cases hR : (right.rows (key k)).card = 0 <;>
    simp [hp, hR, Multiset.bind_map, Multiset.bind_singleton]

/-- Split undoes union, the cancellation dual of `union_split`: when `T₀`
lives on the keys `s` routes left and `T₁` on the keys `s` routes right,
splitting their union recovers the pair.  The hypotheses are the routing
side conditions a rewrite must check. -/
theorem split_bind (s : K → Bool) {T₀ T₁ : Table K H σ}
    (h₀ : ∀ k, s k = true → T₀.rows k = 0)
    (h₁ : ∀ k, s k = false → T₁.rows k = 0) :
    split s (union T₀ T₁) = (T₀, T₁) := by
  have e₁ : (split s (union T₀ T₁)).1 = T₀ := by
    apply Table.ext_rows
    intro k
    simp only [split, union]
    cases hs : s k
    · simp [h₁ k hs]
    · simp [h₀ k hs]
  have e₂ : (split s (union T₀ T₁)).2 = T₁ := by
    apply Table.ext_rows
    intro k
    simp only [split, union]
    cases hs : s k
    · simp [h₁ k hs]
    · simp [h₀ k hs]
  exact Prod.ext e₁ e₂

/-! ### `promote` / `demote`: the key-move cancellation (ADR 0024)

`promote` is the algebra's `promote` (promote the distinguished column
into the key) and `demote` its `demote` (demote a key component into
a column).  On the domain the checker already enforces -- `promote`
requires the promoted column total -- they cancel in both directions, so
the key moves form a true inverse pair like the reshape pair of ADR 0020.
The cancellation is what entitles the checker to treat an exact
`promote c |> demote c` round trip as the identity in *all*
tracked facts: the composite is literally `id`, which is `SplitSafe` and
preserves every property by rewriting. -/

/-- A finite sum of binds of one multiset is the union of the pointwise
sum: `Multiset.bind` is additive in its function argument
(`Multiset.bind_add`), so it commutes with `∑`. -/
theorem sum_bind {α β γ : Type _} [Fintype γ] (m : Multiset α)
    (F : γ → α → Multiset β) :
    ∑ c : γ, m.bind (F c) = m.bind (fun a => ∑ c : γ, F c a) := by
  refine Multiset.induction_on m (by simp) (fun a m ih => ?_)
  simp [Multiset.cons_bind, Finset.sum_add_distrib, ih]

/-- The union of a finite sum of multisets is the sum of the binds:
`Multiset.bind` is additive in its multiset argument
(`Multiset.add_bind`), so it commutes with `∑`. -/
theorem bind_sum {α β γ : Type _} (s : Finset γ) (m : γ → Multiset α)
    (ψ : α → Multiset β) :
    (∑ c ∈ s, m c).bind ψ = ∑ c ∈ s, (m c).bind ψ := by
  classical
  refine Finset.induction_on s (by simp) (fun {a} {t} ha ih => ?_)
  simp [Finset.sum_insert ha, Multiset.add_bind, ih]

/-- **`demote` undoes `promote` (ADR 0024).**  `promote` promotes
the distinguished column into the key, dropping any row whose value there
is missing; when that column is **total** -- exactly the gate `promote`
enforces ("narrow it first") -- no row drops, and `demote` re-tags each
row with the key component it was filed under, rebuilding the original
row.  The hypothesis is the pair's inverse-domain side condition, enforced
by the checker at `promote`. -/
theorem demote_ungroup {D : Type} [Fintype D] [DecidableEq D]
    {T : Table K (H ⊕ Unit) (Sum.elim σ (fun _ => D))}
    (htot : ∀ k, ∀ f ∈ T.rows k, f (Sum.inr ()) ≠ none) :
    demote (promote T) = T := by
  apply Table.ext_rows
  intro k
  simp only [demote, promote, Multiset.map_bind]
  rw [sum_bind]
  conv_rhs => rw [← bind_singleton_id (T.rows k)]
  refine Multiset.bind_congr (fun f hf => ?_)
  obtain ⟨d₀, hd₀⟩ := Option.ne_none_iff_exists'.mp (htot k f hf)
  have hrow : Row.elim (fun h => f (Sum.inl h)) (fun _ => some d₀) = f := by
    funext c
    cases c with
    | inl h => rfl
    | inr u => cases u; exact hd₀.symm
  simp [hd₀, apply_ite, Finset.sum_ite_eq]
  exact hrow

/-- **`promote` undoes `demote` (ADR 0024)** -- with no side
condition: `demote` tags every demoted row with its own key component,
so the demoted column is total by construction, and `promote` files every
row back under exactly the key it came from. -/
theorem promote_project {D : Type} [Fintype D] [DecidableEq D]
    (T : Table (K × D) H σ) : promote (demote T) = T := by
  apply Table.ext_rows
  rintro ⟨k, d⟩
  simp only [promote, demote, bind_sum, Multiset.bind_map, Row.elim_inr, Row.elim_inl]
  calc ∑ d' : D, (T.rows (k, d')).bind (fun f => if d' = d then {f} else 0)
      = ∑ d' : D, (if d' = d then T.rows (k, d') else 0) :=
        Finset.sum_congr rfl (fun d' _ => by
          by_cases h : d' = d <;> simp [h, bind_singleton_id])
    _ = T.rows (k, d) := by simp

/-- **`promote` preserves `singletons` (ADR 0024).**  `promote` files
each row of a key's group under the finer key its promoted column names,
so a fiber of the output is a filtered subset of the input group: at most
one row in, at most one row out.  This is the propagation row behind
re-deriving `singletons` after a promotion; consuming a *grading* needs no
lemma of its own, since the grading is by definition the statement
`Functional (promote T)` for the promoted columns (ADR 0024). -/
theorem promote_functional {β : Type} [DecidableEq β]
    {T : Table K (H ⊕ Unit) (Sum.elim σ (fun _ => β))}
    (hT : Functional T) : Functional (promote T) := by
  rintro ⟨k, b⟩
  rcases Nat.le_one_iff_eq_zero_or_eq_one.mp (hT k) with h0 | h1
  · simp [promote, Multiset.card_eq_zero.mp h0]
  · obtain ⟨f, hf⟩ := Multiset.card_eq_one.mp h1
    simp only [promote, hf, Multiset.singleton_bind]
    cases f (Sum.inr ()) with
    | none => simp
    | some w => split <;> (try split) <;> simp

end Mensura
