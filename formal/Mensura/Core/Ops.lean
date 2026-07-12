/-
The operations of the data-handling algebra over indexed tables: `flatMap` (the
single row-wise primitive), the fixed-right joins, `aggregate`, `promote`,
`demote`, and the tagged union/split pair with its reversibility.

Main Source:  Chapter 5 of F. A. N. Verri (2026). Data Science Project: An
Inductive Learning Approach. Version v1.0.0. Victoria, British Columbia,
Canada: Leanpub. doi: 10.5281/zenodo.14498010. url: https://leanpub.com/dsp.

`flatMap` subsumes def:selection, def:mutating, def:filtering, and the
row-expanding direction of def:grouping; the reshape pair (`unpivot`/`pivot`)
lives in `Mensura.Reshape`, and the split-safety of everything here is proved
in `Mensura.SplitSafety`.
-/

import Mensura.Core.Defs

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {K' H' : Type _} {σ' : H' → Type}
variable {U G : Type _} {τ : G → Type}
variable {D : Type _}
variable {S : Type}

/-- The single row-wise primitive (def:selection + def:mutating + def:filtering,
and the row-expanding direction of def:grouping).  `φ k f` maps a nested row to a
multiset of output rows: `0` drops it, a singleton keeps or transforms it, and
several rows expand it.  Being `Multiset.bind`-shaped over a commutative union,
it is a union-homomorphism (hence split-invariant) with no disjointness needed. -/
def flatMap (φ : K → Row H σ → Multiset (Row H' σ')) (T : Table K H σ) :
    Table K H' σ' :=
  ⟨fun k => (T.rows k).bind (φ k)⟩

/-- def:left-join against a fixed right table, sharing index columns `U` and
adding columns `G` (disjoint from `H` via `⊕`, with the combined schema
`Sum.elim σ τ`).  Each present left row is combined with every matching right
row, or kept once with missing right columns when there is no match (the "left"
guarantee).  Being a `flatMap`, it is split-invariant. -/
def lookup (key : K → U) (right : Table U G τ) (T : Table K H σ) :
    Table K (H ⊕ G) (Sum.elim σ τ) :=
  flatMap (fun k f =>
    let R := right.rows (key k)
    if R.card = 0 then {f.elim (fun _ => none)}
    else R.map (fun r => f.elim r)) T

/-- def:inner-join against a fixed right table.  Like `lookup`, but a left row
with no match is dropped (empty cross product) rather than kept with missing
columns.  Still a `flatMap`, so split-invariant.

The chapter leaves split-invariance of the inner join open, noting only that the
*binary* join can erase rows from either side.  In the unary, fixed-right form
the only effect is dropping unmatched left rows -- a `flatMap` -- so it is. -/
def lookupTotal (key : K → U) (right : Table U G τ) (T : Table K H σ) :
    Table K (H ⊕ G) (Sum.elim σ τ) :=
  flatMap (fun k f => (right.rows (key k)).map (fun r => f.elim r)) T

/-- def:aggregating.  Collapse each key's whole bag of nested rows to a single
row via `f` (empty stays empty).  Unlike `flatMap`, `f` sees the *entire* multiset at
a key, so it is a sibling of `flatMap` under a more general "whole-bag per key"
operation, not a special case.  That whole-bag access is why it is not a
union-homomorphism (`aggregate_not_unionHom`), though it remains split-invariant
(`aggregate_splitInvariant`): a split never merges a key's bag. -/
def aggregate (f : K → Multiset (Row H σ) → Row H σ) (T : Table K H σ) :
    Table K H σ :=
  ⟨fun k => if (T.rows k).card = 0 then 0 else {f k (T.rows k)}⟩

/-- def:grouping (promote).  Turn the distinguished column `Sum.inr ()` (domain
`β`) into part of the key: the new key is `K × β`, and at `(k, v)` we keep the
nested rows of key `k` whose promoteed column held `some v`, dropping that
column.  An arbitrary column is reached by `flatMap`-reorder then promote; a row
whose promoteed column is missing matches no `v` and is dropped (the chapter
requires that column total).  Being `Multiset.bind`-shaped per output key over a
single input key, it is split-invariant. -/
def promote {β : Type} [DecidableEq β]
    (T : Table K (H ⊕ Unit) (Sum.elim σ (fun _ => β))) : Table (K × β) H σ :=
  ⟨fun p => (T.rows p.1).bind (fun f =>
    let v : Cell β := f (Sum.inr ())
    match v with
    | some w => if w = p.2 then {fun h => f (Sum.inl h)} else 0
    | none => 0)⟩

/-- def:projection.  Drop the index component `D` from the key, turning it into a
new column (`Sum.inr ()`, domain `D`): the rows of every dropped key `(k, d)` are
*merged* into the single output key `k`, each tagged with its `d`.  Needs `D`
finite to sum over.  This *changes the observational unit*, and -- crucially --
it does not preserve disjointness (`demote_not_preservesDisjoint`): two input
rows that a split separates can share an output key, so `demote` is not
`SplitSafe` even though it is a `UnionHom` (`demote_unionHom`). -/
def demote [Fintype D] (T : Table (K × D) H σ) :
    Table K (H ⊕ Unit) (Sum.elim σ (fun _ => D)) :=
  ⟨fun k => ∑ d : D, (T.rows (k, d)).map (fun f => Row.elim f (fun _ => some d))⟩

/-! ### Tagged union / split -/

/-- Tag a row with a source value `s` in a fresh column `Sum.inr ()` (domain `S`). -/
def addTag (s : S) (f : Row H σ) : Row (H ⊕ Unit) (Sum.elim σ (fun _ => S)) :=
  Row.elim f (fun _ => some s)

/-- Drop the tag column, projecting back to the original columns. -/
def dropTag (f : Row (H ⊕ Unit) (Sum.elim σ (fun _ => S))) : Row H σ :=
  fun h => f (Sum.inl h)

@[simp] theorem addTag_inr (s : S) (f : Row H σ) :
    addTag s f (Sum.inr ()) = some s := rfl

@[simp] theorem dropTag_addTag (s : S) (f : Row H σ) : dropTag (addTag s f) = f := rfl

/-- def:tagged-union.  Bind two tables, recording each row's source in a new
column: `T₀`'s rows are tagged `s₀`, `T₁`'s `s₁`.  It is `union` of two
tag-`flatMap`s, so its content is the plain union plus the source column. -/
def taggedBind (s₀ s₁ : S) (T₀ T₁ : Table K H σ) :
    Table K (H ⊕ Unit) (Sum.elim σ (fun _ => S)) :=
  union (flatMap (fun _ f => {addTag s₀ f}) T₀) (flatMap (fun _ f => {addTag s₁ f}) T₁)

/-- def:tagged-split.  Recover the rows of source `s`: keep those whose tag
column is `some s`, dropping the tag.  A `flatMap`, hence split-safe; it inverts
`taggedBind` (`taggedSplit_taggedBind_left`). -/
def taggedSplit [DecidableEq S]
    (T : Table K (H ⊕ Unit) (Sum.elim σ (fun _ => S))) (s : S) : Table K H σ :=
  flatMap (fun _ f =>
    let v : Cell S := f (Sum.inr ())
    match v with
    | some w => if w = s then {dropTag f} else 0
    | none => 0) T

/-! ### Tagged union / split: reversibility -/

/-- `Multiset.bind` with `singleton` is the identity (the monad return law). -/
theorem bind_singleton_id {α : Type _} (s : Multiset α) : s.bind (fun a => {a}) = s := by
  have := Multiset.bind_singleton (f := id) (s := s)
  simpa using this

/-- `taggedSplit` inverts `taggedBind`: recovering source `s₀` (with distinct
tags) gives back `T₀`.  `T₀`'s rows, tagged `s₀`, are kept and untagged;
`T₁`'s rows, tagged `s₁ ≠ s₀`, are filtered out. -/
theorem taggedSplit_taggedBind_left [DecidableEq S] {s₀ s₁ : S} (hne : s₀ ≠ s₁)
    (T₀ T₁ : Table K H σ) :
    taggedSplit (taggedBind s₀ s₁ T₀ T₁) s₀ = T₀ := by
  apply Table.ext_rows
  intro k
  simp [taggedSplit, taggedBind, flatMap, union, Multiset.add_bind, Multiset.bind_map,
        Multiset.bind_singleton, bind_singleton_id, Multiset.bind_zero, Ne.symm hne]

/-- Symmetrically, recovering source `s₁` gives back `T₁`. -/
theorem taggedSplit_taggedBind_right [DecidableEq S] {s₀ s₁ : S} (hne : s₀ ≠ s₁)
    (T₀ T₁ : Table K H σ) :
    taggedSplit (taggedBind s₀ s₁ T₀ T₁) s₁ = T₁ := by
  apply Table.ext_rows
  intro k
  simp [taggedSplit, taggedBind, flatMap, union, Multiset.add_bind, Multiset.bind_map,
        Multiset.bind_singleton, bind_singleton_id, Multiset.bind_zero, hne]

end Mensura
