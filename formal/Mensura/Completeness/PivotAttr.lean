/-
A split-safe pivot: spreading a non-index column.

`Mensura.Reshape`'s `pivot` spreads a column that is part of the *index*
(`Table (K × N) Unit → Table K N`), and is not split-invariant
(`pivot_not_splitInvariant`): a split can cut the spread column.  The same
reshape becomes split-safe once the spread column `name` is a non-index
*attribute*: the long table is indexed by the residual key `K`, with the
`(value, name)` pairs carried in the bag at each key (here `value = Sum.inl ()`,
`name = Sum.inr ()`).  Then pivoting reads only one key's bag, so it is a strict
`fiberMap`, hence `SplitSafe`.  This is the formal counterpart of the book's
relaxed pivot: safe when `name` is an attribute, unsafe (a `project`) when `name`
is an index.

Reversibility is stated, not reproved here: recovering the long table from a
wide one re-introduces `name` into the index via the existing `unpivot`, and a
`project` (which is *not* split-safe, `project_not_preservesDisjoint`) brings it
back to attribute form, i.e. `T = project (unpivot (pivotAttr T))` on tables that
hold exactly one row per name.  The lone non-split-safe step is that `project`.
-/

import Mensura.Core.Ops
import Mensura.Reshape
import Mensura.Completeness.FiberMap

namespace Mensura

variable {K : Type _}
variable {N : Type _} {V : Type}

open Classical in
/-- The value sub-bag of `m` at name `n`: the long rows whose `name` cell is
`some n`, projected to single-column value rows so `cellOf` can read them.  The
filter predicate is decided classically (this development is proof-only). -/
noncomputable def valuesAt (n : N)
    (m : Multiset (Row (Unit ⊕ Unit) (Sum.elim (fun _ => V) (fun _ => N)))) :
    Multiset (Row Unit (fun _ => V)) :=
  (m.filter (fun r => (r (Sum.inr ()) : Cell N) = some n)).map
    (fun r => fun _ => (r (Sum.inl ()) : Cell V))

/-- The relaxed, split-safe pivot.  `name` (`Sum.inr ()`) is a non-index column;
each key's whole `(value, name)` bag is collapsed into one wide row `n ↦` the
value stored at name `n`.  `noncomputable` because it reads values through
`cellOf`. -/
noncomputable def pivotAttr
    (T : Table K (Unit ⊕ Unit) (Sum.elim (fun _ => V) (fun _ => N))) :
    Table K N (fun _ => V) :=
  ⟨fun k => if (T.rows k).card = 0 then 0 else {fun n => cellOf (valuesAt n (T.rows k))}⟩

/-- `pivotAttr` is a `fiberMap`: it reads only the bag at its own key. -/
theorem pivotAttr_eq_fiberMap :
    pivotAttr (K := K) (V := V) (N := N)
      = fiberMap (fun (_k : K)
          (m : Multiset (Row (Unit ⊕ Unit) (Sum.elim (fun _ => V) (fun _ => N)))) =>
            if m.card = 0 then 0 else {fun n => cellOf (valuesAt n m)}) := rfl

/-- Its fiber action is strict: the empty bag pivots to the empty table. -/
theorem pivotAttr_strict :
    Strict (fun (_k : K)
      (m : Multiset (Row (Unit ⊕ Unit) (Sum.elim (fun _ => V) (fun _ => N)))) =>
        if m.card = 0 then 0 else {fun n => cellOf (valuesAt n m)}) := by
  intro k
  simp

/-- Hence the relaxed pivot is `SplitSafe` -- in contrast to the index-column
`pivot`, which is not even split-invariant (`pivot_not_splitInvariant`). -/
theorem pivotAttr_splitSafe :
    SplitSafe (pivotAttr (K := K) (V := V) (N := N)) := by
  rw [pivotAttr_eq_fiberMap]
  exact fiberMap_splitSafe pivotAttr_strict

/-! ### Reversibility of the relaxed pivot

`pivotAttr` is inverted by re-introducing `name` into the index with the existing
`unpivot` and taking it back out with `project`: `project (unpivot (pivotAttr T)) = T`
on tables that hold exactly one row per name.  Of the three steps only `project`
is not split safe (`project_not_preservesDisjoint`), so it is the lone unsafe
step of the round trip. -/

/-- A long row carrying value `v` (column `Sum.inl ()`) and name `n` (`Sum.inr ()`). -/
def longRow (v : Cell V) (n : N) :
    Row (Unit ⊕ Unit) (Sum.elim (fun _ => V) (fun _ => N)) :=
  Row.elim (fun _ => v) (fun _ => some n)

@[simp] theorem longRow_inl (v : Cell V) (n : N) : longRow v n (Sum.inl ()) = v := rfl
@[simp] theorem longRow_inr (v : Cell V) (n : N) : longRow v n (Sum.inr ()) = some n := rfl

/-- The name-`n` value sub-bag of a fully-spread, functional long table is the
single value stored there. -/
theorem valuesAt_image [Fintype N] [DecidableEq N] (n : N) (g : N → Cell V) :
    valuesAt n ((Finset.univ.val : Multiset N).map (fun m => longRow (g m) m))
      = {fun _ => g n} := by
  simp only [valuesAt, Multiset.filter_map, Function.comp_def, longRow_inr, longRow_inl,
    Option.some.injEq, Multiset.filter_eq',
    Multiset.count_eq_one_of_mem Finset.univ.nodup (Finset.mem_univ n),
    Multiset.replicate_one, Multiset.map_singleton]

/-- Sum of singletons over any multiset is the map of that multiset. -/
private theorem msum_map_singleton {α β : Type _} (s : Multiset α) (x : α → β) :
    (s.map (fun a => ({x a} : Multiset β))).sum = s.map x := by
  induction s using Multiset.induction with
  | empty => simp
  | cons a s ih => simp [ih]

/-- Hence a sum of singletons over a fintype is the map of its universe multiset. -/
theorem sum_univ_singleton [Fintype N] {β : Type _} (x : N → β) :
    (∑ n : N, ({x n} : Multiset β)) = (Finset.univ.val : Multiset N).map x := by
  rw [Finset.sum]
  exact msum_map_singleton _ x

/-- A long table is *name-total* when each present key is the spread of one wide
row: exactly one row per name `n`, with that name present. -/
def NameTotal [Fintype N]
    (T : Table K (Unit ⊕ Unit) (Sum.elim (fun _ => V) (fun _ => N))) : Prop :=
  ∀ k, T.rows k = 0 ∨ ∃ g : N → Cell V,
    T.rows k = (Finset.univ.val : Multiset N).map (fun m => longRow (g m) m)

/-- Reversibility: on name-total tables, `project ∘ unpivot ∘ pivotAttr` is the
identity.  Only the final `project` is not split safe, so it is the one unsafe
step of the round trip. -/
theorem pivotAttr_reversible [Fintype N] [DecidableEq N] [Nonempty N]
    {T : Table K (Unit ⊕ Unit) (Sum.elim (fun _ => V) (fun _ => N))}
    (hT : NameTotal T) :
    project (unpivot (pivotAttr T)) = T := by
  apply Table.ext_rows
  intro k
  rcases hT k with h0 | ⟨g, hk⟩
  · simp [project, unpivot, pivotAttr, h0]
  · have hcard : ¬ (T.rows k).card = 0 := by
      rw [hk, Multiset.card_map]
      change ¬ Finset.univ.card = 0
      rw [Finset.card_univ]
      exact Fintype.card_ne_zero
    have hpiv : (pivotAttr T).rows k = {g} := by
      simp only [pivotAttr, if_neg hcard]
      rw [Multiset.singleton_inj]
      funext n
      rw [hk, valuesAt_image, cellOf_singleton]
    show (project (unpivot (pivotAttr T))).rows k = T.rows k
    rw [hk]
    simp only [project, unpivot, hpiv, Multiset.map_singleton, longRow]
    rw [sum_univ_singleton (fun n => Row.elim (fun _ => g n) (fun (_ : Unit) => some n))]

end Mensura
