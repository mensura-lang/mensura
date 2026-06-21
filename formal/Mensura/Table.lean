/-
Indexed tables: the core data structure of the algebra, plus the first
operations (split, bind, filter) and the split-invariance result.

Main Source:  Chapter 5, section "Formal structured data" and "Split-invariant
operations", of F. A. N. Verri (2026). Data Science Project: An Inductive Learning
Approach. Version v1.0.0. Victoria, British Columbia, Canada: Leanpub. doi:
10.5281/zenodo.14498010. url: https://leanpub.com/dsp.

## Simplifying assumption (initial version)

The chapter lets a row have any cardinality `card(r) ≥ 0`, with each cell
holding a tuple of that length (def:itable, def:vmatrix).  We start from the
restriction

    card(r) ∈ {0, 1}

so a row is either *absent* or *present exactly once*.  A cell then holds at
most one (possibly missing) value instead of a tuple, the value matrix
collapses to a single nested row, and multisets disappear entirely.  We also
use a single value type `α` for every column rather than per-column domains.
The general `card(r) ≥ 2` case (needing `Multiset`, and the home of `ungroup`
and `aggregate`) is where the Mathlib *library* starts to earn its keep; for
now we only borrow Mathlib's tactics.

Done here: def:split, def:bind, def:disjoint-tables, def:split-invariance,
def:filtering, and the proofs that a split yields disjoint tables, that bind
undoes split, and that `filter` is split-invariant.

Next, in dependency order: def:selection, def:mutating, def:left-join, then the
tagged variants (def:tagged-bind / def:tagged-split).
-/

import Mathlib.Tactic

namespace Mensura

/-- The missing marker `?` from the chapter: a cell value may be absent. -/
abbrev Cell (α : Type _) := Option α

/-- An indexed table under the `card(r) ∈ {0, 1}` assumption.

`K` is the type of row keys (the tuple of index-column values identifying a
row); `H` is the type of non-index column names.

`row k` is `none` when row `k` is absent (cardinality 0) and `some f` when it
is present (cardinality 1); then `f h` is the value held in column `h`, itself
possibly missing (`Cell`). -/
@[ext]
structure Table (K H α : Type _) where
  row : K → Option (H → Cell α)

variable {K H α : Type _}

/-- A row is present when it has cardinality 1. -/
def Table.Present (T : Table K H α) (k : K) : Prop := T.row k ≠ none

/-- Two tables are equal when they agree row-by-row.  Unlike the `ext` tactic,
this stops at the `Option`-valued rows instead of descending into them. -/
theorem Table.ext_row {T U : Table K H α} (h : ∀ k, T.row k = U.row k) : T = U := by
  obtain ⟨r₀⟩ := T
  obtain ⟨r₁⟩ := U
  simp only [Table.mk.injEq]
  funext k
  exact h k

/-- def:split.  An indicator `s` sends each key to one of the two output
tables; the row is kept verbatim in its table and made absent in the other. -/
def split (s : K → Bool) (T : Table K H α) : Table K H α × Table K H α :=
  (⟨fun k => bif s k then none else T.row k⟩,
   ⟨fun k => bif s k then T.row k else none⟩)

/-- def:bind.  At each key, take the present row if there is one.  On disjoint
tables (def:disjoint-tables) at most one side is present, so this is exactly
the chapter's cell concatenation. -/
def bind (T₀ T₁ : Table K H α) : Table K H α :=
  ⟨fun k =>
    match T₀.row k with
    | some f => some f
    | none => T₁.row k⟩

/-- def:disjoint-tables.  Phrased as "every key is absent in at least one
table", which under `card ∈ {0,1}` is equivalent to "present in one ⇒ absent
in the other". -/
def Disjoint (T₀ T₁ : Table K H α) : Prop :=
  ∀ k, T₀.row k = none ∨ T₁.row k = none

/-- def:split-invariance, for a unary operation.  `f` distributes over `bind`
of disjoint tables. -/
def SplitInvariant (f : Table K H α → Table K H α) : Prop :=
  ∀ T₀ T₁ : Table K H α, Disjoint T₀ T₁ → f (bind T₀ T₁) = bind (f T₀) (f T₁)

/-- def:filtering.  Keep a row iff the predicate holds on its key and nested
row; rows are treated independently. -/
def filter (p : K → (H → Cell α) → Bool) (T : Table K H α) : Table K H α :=
  ⟨fun k =>
    match T.row k with
    | some f => bif p k f then some f else none
    | none => none⟩

/-- The two halves of a split are disjoint. -/
theorem split_disjoint (s : K → Bool) (T : Table K H α) :
    Disjoint (split s T).1 (split s T).2 := by
  intro k
  simp only [split]
  cases s k <;> simp

/-- Bind undoes split: split and bind are mutual inverses (one direction). -/
theorem bind_split (s : K → Bool) (T : Table K H α) :
    bind (split s T).1 (split s T).2 = T := by
  refine Table.ext_row fun k => ?_
  simp only [bind, split]
  cases s k <;> cases T.row k <;> rfl

/-- `filter` is split-invariant.  Disjointness is essential: were a row present
in both tables, filtering it out of one but not the other would break the
equation. -/
theorem filter_splitInvariant (p : K → (H → Cell α) → Bool) :
    SplitInvariant (filter p) := by
  intro T₀ T₁ hdisj
  refine Table.ext_row fun k => ?_
  simp only [filter, bind]
  rcases hdisj k with h | h
  · simp only [h]
  · cases T₀.row k with
    | none => simp only [h]
    | some f =>
      simp only [h]
      cases p k f <;> rfl

end Mensura
