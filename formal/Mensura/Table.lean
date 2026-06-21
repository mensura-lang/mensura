/-
Indexed tables: the core data structure of the algebra, the operations
(split, bind, map), and the split-invariance results.

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

Done here: def:split, def:bind, def:disjoint-tables, def:split-invariance, and
`map` -- a single row-wise primitive that subsumes the chapter's def:selection,
def:mutating, and def:filtering.  Proved: a split yields disjoint tables, bind
undoes split, bind is commutative on disjoint tables, and `map` is
split-invariant.

Next: def:left-join (changes the key type, so it needs the binary / fixed-table
form of split-invariance), then aggregate and ungroup (which need `card ≥ 2`,
i.e. `Multiset`), and the tagged variants (def:tagged-bind / def:tagged-split).
The bias-free `bind` arrives with the `card ≥ 2` lift, where it becomes
multiset union; until then `bind_comm` shows the current left-bias is invisible
on disjoint tables.
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
variable {H' α' : Type _}

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
of disjoint tables.  The operation may change the columns and value type, but
keeps the key type `K`: that is what guarantees the outputs are still disjoint,
so the right-hand `bind` is meaningful.  (A key-changing operation such as
`left_join` needs the binary / fixed-table form, deferred.) -/
def SplitInvariant (f : Table K H α → Table K H' α') : Prop :=
  ∀ T₀ T₁ : Table K H α, Disjoint T₀ T₁ → f (bind T₀ T₁) = bind (f T₀) (f T₁)

/-- The single row-wise primitive (def:selection + def:mutating + def:filtering).
`φ k f` transforms the nested row `f` at key `k`, returning `none` to drop the
row or `some f'` to keep it as `f'`.  It may rename, drop, reorder, or add
columns and change the value type, so every per-row operation of the chapter is
`map φ` for a particular `φ`:

* selection by a reindexing `g : H' → H`:  `map (fun _ f => some (f ∘ g))`;
* mutation adding a column from `m`:
    `map (fun k f => some (Sum.elim f (fun _ => m k f)))`;
* filtering by a predicate `p`:  `map (fun k f => bif p k f then some f else none)`.

`map` is `Option.bind`-shaped: because it can drop a row, its split-invariance
genuinely needs disjointness (a row dropped on one side must not be silently
recovered from the other).  The drop-free fragment (`φ` always `some`) is
split-invariant even without it, but folding `filter` in means the general
statement uses the hypothesis. -/
def map (φ : K → (H → Cell α) → Option (H' → Cell α')) (T : Table K H α) :
    Table K H' α' :=
  ⟨fun k =>
    match T.row k with
    | some f => φ k f
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

/-- `bind` is commutative on disjoint tables.  The definition is left-biased,
but disjointness kills one side at every key, so the bias is unobservable: this
is what "the order of concatenation is not an issue" means under `card ∈ {0,1}`. -/
theorem bind_comm {T₀ T₁ : Table K H α} (h : Disjoint T₀ T₁) :
    bind T₀ T₁ = bind T₁ T₀ := by
  refine Table.ext_row fun k => ?_
  simp only [bind]
  rcases h k with h | h
  · simp only [h]; cases T₁.row k <;> rfl
  · simp only [h]; cases T₀.row k <;> rfl

/-- `map` is split-invariant.  Disjointness is essential because `map` can drop
rows: were a row present in both tables, dropping it on one side but not the
other would break the equation. -/
theorem map_splitInvariant (φ : K → (H → Cell α) → Option (H' → Cell α')) :
    SplitInvariant (map φ) := by
  intro T₀ T₁ hdisj
  refine Table.ext_row fun k => ?_
  simp only [map, bind]
  rcases hdisj k with h | h
  · simp only [h]
  · cases T₀.row k with
    | none => simp only [h]
    | some f =>
      simp only [h]
      cases φ k f <;> rfl

end Mensura
