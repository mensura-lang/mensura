/-
Equational laws: the rewrite-rule seeds.

Per ADR 0008 (`docs/decisions/0008-formalize-algebra-in-lean.md`), algebraic
laws are stated in equational form (`lhs = rhs` under named side conditions)
so they translate directly into rewrite rules once the processing layer grows
an optimizing plan IR.  The laws below are the prototypical plan rewrites:
`empty` completes `bind`'s commutative monoid (unit laws); `map` carries
identity and fusion laws (fusion subsumes filter/filter, filter/mutate, and
select fusion, since all are `map`s, ADR 0015); the joins absorb a preceding
`map` and commute with a left-column `filter` (pushdown); and `split`/`bind`
cancel in both directions (`bind_split` in `Mensura.Core.Defs`, `split_bind`
below).
-/

import Mensura.Core.Ops

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {K' H' : Type _} {σ' : H' → Type}
variable {K'' H'' : Type _} {σ'' : H'' → Type}
variable {U G : Type _} {τ : G → Type}

/-- The empty table: no rows at any key.  The identity of `bind`
(`bind_empty`, `empty_bind`), completing the commutative monoid that
`bind_comm` and `bind_assoc` establish. -/
def empty : Table K H σ := ⟨fun _ => 0⟩

/-- Right unit: `bind T empty = T`. -/
@[simp] theorem bind_empty (T : Table K H σ) : bind T empty = T := by
  apply Table.ext_rows
  intro k
  simp [bind, empty]

/-- Left unit: `bind empty T = T`. -/
@[simp] theorem empty_bind (T : Table K H σ) : bind empty T = T := by
  apply Table.ext_rows
  intro k
  simp [bind, empty]

/-- `map` annihilates `empty`: mapping over no rows yields no rows. -/
@[simp] theorem map_empty (φ : K → Row H σ → Multiset (Row H' σ')) :
    map φ (empty : Table K H σ) = empty := by
  apply Table.ext_rows
  intro k
  simp [map, empty]

/-- Identity law: mapping each row to its singleton is the identity. -/
@[simp] theorem map_id (T : Table K H σ) : map (fun _ f => {f}) T = T := by
  apply Table.ext_rows
  intro k
  simp [map, bind_singleton_id]

/-- Fusion law: two consecutive `map`s collapse into one whose body binds
the second body over the first's output rows.  The prototypical plan
rewrite; with `map_id` it makes `map` bodies a monoid under Kleisli-style
composition. -/
theorem map_map (ψ : K → Row H' σ' → Multiset (Row H'' σ''))
    (φ : K → Row H σ → Multiset (Row H' σ')) (T : Table K H σ) :
    map ψ (map φ T) = map (fun k f => (φ k f).bind (ψ k)) T := by
  apply Table.ext_rows
  intro k
  simp only [map]
  exact Multiset.bind_assoc

/-- def:filtering as a named operation: keep each row iff `p` holds.  A
`map`, so every `map` law applies; named so the pushdown laws below read
the way the optimizer will use them. -/
def filter (p : K → Row H σ → Bool) : Table K H σ → Table K H σ :=
  map (fun k f => bif p k f then {f} else 0)

/-- Join pushdown, fusion form: a `map` feeding `innerJoin` fuses into one
`map` whose body joins each of the `map`'s output rows.  An instance of
`map_map`, stated separately because a plan IR carries the join as a node,
not as its `map` body. -/
theorem innerJoin_map (key : K → U) (right : Table U G τ)
    (φ : K → Row H σ → Multiset (Row H' σ')) (T : Table K H σ) :
    innerJoin key right (map φ T)
      = map (fun k f => (φ k f).bind
          (fun g => (right.rows (key k)).map (fun r => g.elim r))) T :=
  map_map _ φ T

/-- Join pushdown, fusion form, for `leftJoin`. -/
theorem leftJoin_map (key : K → U) (right : Table U G τ)
    (φ : K → Row H σ → Multiset (Row H' σ')) (T : Table K H σ) :
    leftJoin key right (map φ T)
      = map (fun k f => (φ k f).bind (fun g =>
          let R := right.rows (key k)
          if R.card = 0 then {g.elim (fun _ => none)}
          else R.map (fun r => g.elim r))) T :=
  map_map _ φ T

/-- Filter pushdown through `innerJoin`: a filter that reads only the left
columns (syntactically, only `Sum.inl` columns of the joined row) commutes
below the join.  The optimizer's directed use is left to right: filter
before joining. -/
theorem innerJoin_filter_pushdown (key : K → U) (right : Table U G τ)
    (p : K → Row H σ → Bool) (T : Table K H σ) :
    filter (fun k g => p k (fun h => g (Sum.inl h))) (innerJoin key right T)
      = innerJoin key right (filter p T) := by
  simp only [filter, innerJoin, map_map]
  apply Table.ext_rows
  intro k
  simp only [map]
  refine Multiset.bind_congr (fun f _ => ?_)
  cases hp : p k f <;>
    simp [hp, Multiset.bind_map, Multiset.bind_singleton]

/-- Filter pushdown through `leftJoin`: valid because a left row's copies
(matched or the missing-padded survivor) all agree on the left columns, so
the filter keeps or drops them together. -/
theorem leftJoin_filter_pushdown (key : K → U) (right : Table U G τ)
    (p : K → Row H σ → Bool) (T : Table K H σ) :
    filter (fun k g => p k (fun h => g (Sum.inl h))) (leftJoin key right T)
      = leftJoin key right (filter p T) := by
  simp only [filter, leftJoin, map_map]
  apply Table.ext_rows
  intro k
  simp only [map]
  refine Multiset.bind_congr (fun f _ => ?_)
  cases hp : p k f <;>
    by_cases hR : (right.rows (key k)).card = 0 <;>
    simp [hp, hR, Multiset.bind_map, Multiset.bind_singleton]

/-- Split undoes bind, the cancellation dual of `bind_split`: when `T₀`
lives on the keys `s` routes left and `T₁` on the keys `s` routes right,
splitting their bind recovers the pair.  The hypotheses are the routing
side conditions a rewrite must check. -/
theorem split_bind (s : K → Bool) {T₀ T₁ : Table K H σ}
    (h₀ : ∀ k, s k = true → T₀.rows k = 0)
    (h₁ : ∀ k, s k = false → T₁.rows k = 0) :
    split s (bind T₀ T₁) = (T₀, T₁) := by
  have e₁ : (split s (bind T₀ T₁)).1 = T₀ := by
    apply Table.ext_rows
    intro k
    simp only [split, bind]
    cases hs : s k
    · simp [h₁ k hs]
    · simp [h₀ k hs]
  have e₂ : (split s (bind T₀ T₁)).2 = T₁ := by
    apply Table.ext_rows
    intro k
    simp only [split, bind]
    cases hs : s k
    · simp [h₁ k hs]
    · simp [h₀ k hs]
  exact Prod.ext e₁ e₂

end Mensura
