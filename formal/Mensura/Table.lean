/-
Indexed tables over multisets: the core data structure of the algebra, the
operations (split, bind, map, leftJoin), and the split-invariance results.

Main Source:  Chapter 5, section "Formal structured data" and "Split-invariant
operations", of F. A. N. Verri (2026). Data Science Project: An Inductive Learning
Approach. Version v1.0.0. Victoria, British Columbia, Canada: Leanpub. doi:
10.5281/zenodo.14498010. url: https://leanpub.com/dsp.

## Representation: a row's content is a multiset of nested rows

The chapter stores a row's content *column-major*: each cell `c(r,h)` is a
tuple of length `card(r)`, and a nested row is recovered by reading position `i`
across all columns.  We store it *row-major* instead: the content at a key is a

    Multiset (H → Cell α)

a bag of nested rows, each nested row a single function binding every column to
its (possibly missing) value.  This is better than the chapter's encoding on
three counts:

* **Order is honest.**  The chapter calls the order of nested rows "arbitrary
  but fixed".  A sequence whose order carries no meaning is a multiset (lists
  up to permutation), so we model exactly that, instead of asserting an order
  we then promise never to use.  Operations that *do* need an order (window
  functions, `sort by`) must impose it explicitly from a data column -- never
  from storage.

* **Associations cannot desync.**  The chapter's cross-column correlation
  (which value of one column goes with which of another) is *positional*: it
  holds only while every column's tuple stays aligned, an invariant nothing
  enforces.  Here each nested row is one function, so `f h₁` and `f h₂` are
  bound together structurally; the alignment invariant is unrepresentable.

* **`bind` is a real commutative monoid.**  Multiset union is commutative,
  associative, total, and bias-free, so `bind` needs no tie-break and the
  row-wise operations are split-invariant *unconditionally* (contrast the
  `card ∈ {0,1}` model, where `Option` cannot hold `some ⊎ some`, forcing a
  left-biased `bind` and a disjointness hypothesis on split-invariance).

`card(r)` is the multiset's cardinality; `card(r) = 0` is an absent row.

Done here: def:split, def:bind, def:disjoint-tables, def:split-invariance,
`map` (a single row-wise primitive subsuming def:selection, def:mutating,
def:filtering, and the row-expanding direction of def:grouping), and `leftJoin`
(def:left-join).  Proved: split yields disjoint tables, bind undoes split, bind
is commutative and associative (now *unconditionally*), and `map` and
`leftJoin` are split-invariant (also unconditionally).

Next: aggregate and project (which collapse/merge rows, so they are *not*
homomorphisms and correctly fail split-invariance), the index-changing form of
ungroup, the tagged variants (def:tagged-bind / def:tagged-split), the
minimality side-condition (no all-missing nested row), and per-column typed
domains.
-/

import Mathlib.Data.Multiset.Bind
import Mathlib.Tactic

namespace Mensura

/-- The missing marker `?` from the chapter: a cell value may be absent. -/
abbrev Cell (α : Type _) := Option α

/-- An indexed table.

A nested row is a function `H → Cell α` giving each column its (possibly
missing) value, with all of a row's columns bound together inside the one
function -- so cross-column associations are intrinsic, not positional.

`rows k` is the multiset of nested rows sharing key `k`; its cardinality is the
chapter's `card(r)`, and `0` means the row is absent.  See the module comment
for why a multiset improves on the chapter's column-major aligned tuples. -/
@[ext]
structure Table (K H α : Type _) where
  rows : K → Multiset (H → Cell α)

variable {K H α : Type _}
variable {H' α' : Type _}
variable {U G : Type _}

/-- A row is present when it has positive cardinality. -/
def Table.Present (T : Table K H α) (k : K) : Prop := T.rows k ≠ 0

/-- Two tables are equal when they agree key-by-key. -/
theorem Table.ext_rows {T U : Table K H α} (h : ∀ k, T.rows k = U.rows k) : T = U := by
  obtain ⟨r₀⟩ := T
  obtain ⟨r₁⟩ := U
  simp only [Table.mk.injEq]
  funext k
  exact h k

/-- def:split.  An indicator `s` routes each key's whole multiset of rows to one
side, leaving the other empty. -/
def split (s : K → Bool) (T : Table K H α) : Table K H α × Table K H α :=
  (⟨fun k => bif s k then 0 else T.rows k⟩,
   ⟨fun k => bif s k then T.rows k else 0⟩)

/-- def:bind.  Multiset union of the two tables' rows at each key.  This is the
chapter's cell concatenation made order-free: the union is commutative,
associative, total, and bias-free -- no disjointness needed for it to be
well-defined, and no tie-break to invent (unlike the `card ∈ {0,1}` model). -/
def bind (T₀ T₁ : Table K H α) : Table K H α :=
  ⟨fun k => T₀.rows k + T₁.rows k⟩

/-- def:disjoint-tables.  At every key, at least one table is empty.  Still
meaningful -- it makes `split` a partition, so `bind ∘ split = id` -- but no
longer needed for split-invariance. -/
def Disjoint (T₀ T₁ : Table K H α) : Prop :=
  ∀ k, T₀.rows k = 0 ∨ T₁.rows k = 0

/-- def:split-invariance.  `f` distributes over `bind`.  Because `bind` is now a
genuine commutative-monoid union, we drop the chapter's disjointness hypothesis:
the row-wise operations distribute over *every* bind, not only over splits.
Operations that merge rows (aggregate, project) are not homomorphisms and are
correctly excluded. -/
def SplitInvariant (f : Table K H α → Table K H' α') : Prop :=
  ∀ T₀ T₁ : Table K H α, f (bind T₀ T₁) = bind (f T₀) (f T₁)

/-- The single row-wise primitive (def:selection + def:mutating + def:filtering,
and the row-expanding direction of def:grouping).  `φ k f` maps a nested row to
a multiset of output rows: `0` drops it, a singleton keeps or transforms it,
and several rows expand it.  Being `Multiset.bind`-shaped over a commutative
union, it is split-invariant with no disjointness (`Multiset.add_bind`), unlike
the `card ∈ {0,1}` model where dropping a row forced the hypothesis. -/
def map (φ : K → (H → Cell α) → Multiset (H' → Cell α')) (T : Table K H α) :
    Table K H' α' :=
  ⟨fun k => (T.rows k).bind (φ k)⟩

/-- def:left-join against a fixed right table, which shares the index columns
`U` with the left and adds columns `G` (kept apart from `H` by `⊕`).  Each
present left row is combined with every matching right row -- the general join
cardinality, now expressible since a row may expand -- or kept once with missing
right columns when there is no match (the "left" guarantee).  Being a `map`, it
is split-invariant. -/
def leftJoin (key : K → U) (right : Table U G α) (T : Table K H α) :
    Table K (H ⊕ G) α :=
  map (fun k f =>
    let R := right.rows (key k)
    if R.card = 0 then {Sum.elim f (fun _ => none)}
    else R.map (fun r => Sum.elim f r)) T

/-- The two halves of a split are disjoint. -/
theorem split_disjoint (s : K → Bool) (T : Table K H α) :
    Disjoint (split s T).1 (split s T).2 := by
  intro k
  simp only [split]
  cases s k <;> simp

/-- Bind undoes split: split and bind are mutual inverses (one direction). -/
theorem bind_split (s : K → Bool) (T : Table K H α) :
    bind (split s T).1 (split s T).2 = T := by
  apply Table.ext_rows
  intro k
  simp only [bind, split]
  cases s k <;> simp

/-- `bind` is commutative -- unconditionally.  Multiset union has no preferred
side, so the left-bias of the `card ∈ {0,1}` model is gone and no disjointness
is needed. -/
theorem bind_comm (T₀ T₁ : Table K H α) : bind T₀ T₁ = bind T₁ T₀ := by
  apply Table.ext_rows
  intro k
  simp only [bind]
  exact add_comm _ _

/-- `bind` is associative. -/
theorem bind_assoc (T₀ T₁ T₂ : Table K H α) :
    bind (bind T₀ T₁) T₂ = bind T₀ (bind T₁ T₂) := by
  apply Table.ext_rows
  intro k
  simp only [bind]
  exact add_assoc _ _ _

/-- `map` is split-invariant -- with no disjointness, since `Multiset.bind`
distributes over union (`Multiset.add_bind`). -/
theorem map_splitInvariant (φ : K → (H → Cell α) → Multiset (H' → Cell α')) :
    SplitInvariant (map φ) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [map, bind]
  exact Multiset.add_bind _ _ _

/-- `leftJoin` against a fixed table is split-invariant: it is a `map`. -/
theorem leftJoin_splitInvariant (key : K → U) (right : Table U G α) :
    SplitInvariant (leftJoin (H := H) key right) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [leftJoin, map, bind]
  exact Multiset.add_bind _ _ _

end Mensura
