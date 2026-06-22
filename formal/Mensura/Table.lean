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
  row-wise operations are bind-homomorphisms *unconditionally*, hence
  split-invariant for free (contrast the `card ∈ {0,1}` model, where `Option`
  cannot hold `some ⊎ some`, forcing a left-biased `bind` and a disjointness
  hypothesis even on the row-wise proofs).

`card(r)` is the multiset's cardinality; `card(r) = 0` is an absent row.

Two properties are formalized: `SplitInvariant` (the chapter's def:split-invariance,
distributivity over the bind of *disjoint* tables, which is what Mensura tracks
and enforces) and `BindHom` (distributivity over *every* bind, strictly
stronger).  `BindHom.splitInvariant` bridges them.

Done here: def:split, def:bind, def:disjoint-tables, the two properties above,
`map` (subsuming def:selection, def:mutating, def:filtering, and the
row-expanding direction of def:grouping), `leftJoin` and `innerJoin`
(def:left-join and def:join, fixed-right form), `ungroup` (def:grouping), and
`aggregate` (def:aggregating).  Proved: split yields disjoint tables, bind
undoes split, bind is commutative and associative (unconditionally),
`map`/`leftJoin`/`innerJoin`/`ungroup` are bind-homomorphisms (hence
split-invariant), and -- the safe/unsafe boundary, both machine-checked --
`aggregate` *is* split-invariant (matching the book) yet *not* a bind-homomorphism.
Aggregate is precisely the operation that separates the two notions.

Next, needing per-column *typed* domains -- different columns, source tags,
index values, and column names carry different types, which the single `α` here
cannot express: def:projection, def:pivot-l2w / def:pivot-w2l, and the tagged
variants (def:tagged-bind / def:tagged-split).  `project` and `pivot` also need
finite domains (a sum/gather over a column's values).  Also deferred: the
minimality side-condition (no all-missing nested row).
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
variable {K' H' α' : Type _}
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

/-- def:disjoint-tables.  At every key, at least one table is empty.  This makes
`split` a partition (so `bind ∘ split = id`), and it is the hypothesis of
`SplitInvariant`: it is exactly what a split guarantees, and what lets
row-collapsing operations like `aggregate` qualify (`aggregate_splitInvariant`). -/
def Disjoint (T₀ T₁ : Table K H α) : Prop :=
  ∀ k, T₀.rows k = 0 ∨ T₁.rows k = 0

/-- def:split-invariance, faithful to the chapter: `f` distributes over the
`bind` of *disjoint* tables.  That is exactly what `split` produces -- its two
halves are key-disjoint (`split_disjoint`) -- so this quantifies over precisely
the binds that undo a split, and no others.  `f` may change the key type (e.g.
`ungroup` sends `K` to `K × α`); the right-hand `bind` is then over the new key,
and disjointness is asked of the *input* tables.

**This is the property Mensura tracks and enforces.**  The disjointness
hypothesis is load-bearing, not incidental: it is what lets row-collapsing
operations qualify.  A `split` never divides a single key's multiset, so
`aggregate` -- which folds the bag at each key -- never sees a merged bag and
stays invariant, matching the book (`aggregate_splitInvariant`).  Drop the
hypothesis and the property silently strengthens to `BindHom`, which `aggregate`
fails. -/
def SplitInvariant (f : Table K H α → Table K' H' α') : Prop :=
  ∀ T₀ T₁ : Table K H α, Disjoint T₀ T₁ → f (bind T₀ T₁) = bind (f T₀) (f T₁)

/-- `f` distributes over *every* `bind`, disjoint or not: a full
commutative-monoid homomorphism, strictly stronger than `SplitInvariant`
(`BindHom.splitInvariant`).  The row-wise operations satisfy it because they act
on each nested row independently and multiset union distributes
(`Multiset.add_bind`); that is the real reason they are split-invariant.
Row-collapsing operations are the dividing line: `aggregate` is split-invariant
yet not a `BindHom` (`aggregate_not_bindHom`). -/
def BindHom (f : Table K H α → Table K' H' α') : Prop :=
  ∀ T₀ T₁ : Table K H α, f (bind T₀ T₁) = bind (f T₀) (f T₁)

/-- Every bind-homomorphism is split-invariant: split-invariance asks for the
equation only on disjoint binds, a special case. -/
theorem BindHom.splitInvariant {f : Table K H α → Table K' H' α'} (h : BindHom f) :
    SplitInvariant f := by
  intro T₀ T₁ _
  exact h T₀ T₁

/-- The single row-wise primitive (def:selection + def:mutating + def:filtering,
and the row-expanding direction of def:grouping).  `φ k f` maps a nested row to
a multiset of output rows: `0` drops it, a singleton keeps or transforms it,
and several rows expand it.  Being `Multiset.bind`-shaped over a commutative
union, it is a bind-homomorphism (hence split-invariant) with no disjointness
needed (`Multiset.add_bind`), unlike the `card ∈ {0,1}` model where dropping a
row forced the hypothesis. -/
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

/-- def:inner-join against a fixed right table.  Like `leftJoin`, but a left row
with no match is *dropped* (empty cross product) rather than kept with missing
columns.  It is still a `map` -- the per-row function returns `0` on no match --
so it is split-invariant.

The chapter leaves split-invariance of the inner join open, noting only that the
*binary* join can erase rows from either side.  In the unary, fixed-right form
the only effect is dropping unmatched left rows, which is a `map`; so here the
unary inner join *is* split-invariant. -/
def innerJoin (key : K → U) (right : Table U G α) (T : Table K H α) :
    Table K (H ⊕ G) α :=
  map (fun k f => (right.rows (key k)).map (fun r => Sum.elim f r)) T

/-- def:aggregating.  Collapse each key's whole bag of nested rows to a single
row via `f` (empty stays empty).  Unlike `map`, `f` sees the *entire* multiset
at a key, not one row at a time -- so it is a sibling of `map` under a more
general "whole-bag per key" operation, not a special case of it.  That whole-bag
access is why it is not a bind-homomorphism (`aggregate_not_bindHom`), though it
remains split-invariant (`aggregate_splitInvariant`): a split never merges a
key's bag. -/
def aggregate (f : K → Multiset (H → Cell α) → (H → Cell α)) (T : Table K H α) :
    Table K H α :=
  ⟨fun k => if (T.rows k).card = 0 then 0 else {f k (T.rows k)}⟩

/-- def:grouping (ungroup).  Turn the distinguished column `Sum.inr ()` into part
of the key: the new key is `K × α`, and at `(k, v)` we keep the nested rows of
key `k` whose ungrouped column held `some v`, dropping that column.  Reaching an
arbitrary column is `map`-reorder then ungroup.  A row whose ungrouped column is
missing matches no `v` and is dropped (the chapter requires that column total).
Being `Multiset.bind`-shaped per output key over a single input key, it is
split-invariant. -/
def ungroup [DecidableEq α] (T : Table K (H ⊕ Unit) α) : Table (K × α) H α :=
  ⟨fun p => (T.rows p.1).bind (fun f =>
    if f (Sum.inr ()) = some p.2 then {f ∘ Sum.inl} else 0)⟩

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

/-- `map` is a bind-homomorphism -- with no disjointness, since `Multiset.bind`
distributes over union (`Multiset.add_bind`). -/
theorem map_bindHom (φ : K → (H → Cell α) → Multiset (H' → Cell α')) :
    BindHom (map φ) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [map, bind]
  exact Multiset.add_bind _ _ _

/-- Hence `map` is split-invariant, the property Mensura enforces. -/
theorem map_splitInvariant (φ : K → (H → Cell α) → Multiset (H' → Cell α')) :
    SplitInvariant (map φ) := (map_bindHom φ).splitInvariant

/-- `leftJoin` against a fixed table is a bind-homomorphism: it is a `map`. -/
theorem leftJoin_bindHom (key : K → U) (right : Table U G α) :
    BindHom (leftJoin (H := H) key right) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [leftJoin, map, bind]
  exact Multiset.add_bind _ _ _

/-- Hence `leftJoin` is split-invariant. -/
theorem leftJoin_splitInvariant (key : K → U) (right : Table U G α) :
    SplitInvariant (leftJoin (H := H) key right) :=
  (leftJoin_bindHom key right).splitInvariant

/-- The unary, fixed-right `innerJoin` is a bind-homomorphism: it is a `map`. -/
theorem innerJoin_bindHom (key : K → U) (right : Table U G α) :
    BindHom (innerJoin (H := H) key right) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [innerJoin, map, bind]
  exact Multiset.add_bind _ _ _

/-- Hence the unary, fixed-right `innerJoin` is split-invariant. -/
theorem innerJoin_splitInvariant (key : K → U) (right : Table U G α) :
    SplitInvariant (innerJoin (H := H) key right) :=
  (innerJoin_bindHom key right).splitInvariant

/-- `ungroup` is a bind-homomorphism.  Each output key `(k, v)` reads only from
input key `k`, where the operation is `Multiset.bind`, which distributes over `+`. -/
theorem ungroup_bindHom [DecidableEq α] :
    BindHom (ungroup (K := K) (H := H) (α := α)) := by
  intro T₀ T₁
  apply Table.ext_rows
  rintro ⟨k, v⟩
  simp only [ungroup, bind]
  exact Multiset.add_bind _ _ _

/-- Hence `ungroup` is split-invariant. -/
theorem ungroup_splitInvariant [DecidableEq α] :
    SplitInvariant (ungroup (K := K) (H := H) (α := α)) :=
  ungroup_bindHom.splitInvariant

/-- `aggregate` *is* split-invariant -- the property Mensura enforces, and the
book's claim.  A `split` routes each key's whole bag to one side, so under
disjointness aggregate never folds a merged bag: at every key one summand is
empty, and folding the other is the same as folding the whole. -/
theorem aggregate_splitInvariant (f : K → Multiset (H → Cell α) → (H → Cell α)) :
    SplitInvariant (aggregate f) := by
  intro T₀ T₁ hdisj
  apply Table.ext_rows
  intro k
  simp only [aggregate, bind]
  rcases hdisj k with h | h
  · rw [h, zero_add]; simp
  · rw [h, add_zero]; simp

/-- `aggregate` is *not* a bind-homomorphism: on a key present in both summands
it folds the merged bag to one row on the left but binds two aggregated rows on
the right.  This is the safe/unsafe boundary -- `aggregate` is the operation that
separates `SplitInvariant` from the strictly stronger `BindHom`. -/
theorem aggregate_not_bindHom :
    ¬ BindHom
        (aggregate (fun (_ : Unit) (_ : Multiset (Unit → Cell Unit)) => fun _ => none)) := by
  intro h
  have hT := h ⟨fun _ => {fun _ => none}⟩ ⟨fun _ => {fun _ => none}⟩
  apply_fun (fun U => (U.rows ()).card) at hT
  simp [aggregate, bind] at hT

end Mensura
