/-
Indexed tables over multisets, with per-column typed domains: the core data
structure of the algebra, the operations (split, bind, map, leftJoin,
innerJoin, aggregate, ungroup), and the split-invariance results.

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
`BindHom.splitInvariant` and `SplitSafe` package the relationships.  `aggregate`
separates `SplitInvariant` from `BindHom` (split-invariant, not a hom);
`project` separates `SplitInvariant` from `SplitSafe` (a hom, hence
split-invariant, yet not disjointness-preserving, so unsafe to pipeline).

Done here: def:split, def:bind, def:disjoint-tables, the three properties above,
`map` (subsuming def:selection, def:mutating, def:filtering, and the
row-expanding direction of def:grouping), `leftJoin` and `innerJoin`
(def:left-join and def:join, fixed-right form), `ungroup` (def:grouping),
`aggregate` (def:aggregating), and `project` (def:projection).  Proved: split
yields disjoint tables, bind undoes split, bind is commutative and associative,
`map`/`leftJoin`/`innerJoin`/`ungroup` are bind-homomorphisms, `aggregate` is
split-invariant but not a bind-homomorphism, `project` is a bind-homomorphism
but not disjointness-preserving; all of the former are `SplitSafe` and compose,
`project` is not.

Next: def:pivot-l2w / def:pivot-w2l (a gather across keys; clean only when at
most one value per name) and the tagged variants (def:tagged-bind /
def:tagged-split, whose characteristic law is reversibility, not
split-invariance).  Also deferred: the minimality side-condition (no all-missing
nested row).
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
variable {U G : Type _} {τ : G → Type}
variable {D : Type _}

/-- Combine a left row and a right row into a row over the disjoint-union schema
`Sum.elim σ τ`.  This is the dependent counterpart of `Sum.elim`: at `Sum.inl h`
it has type `Cell (σ h)`, at `Sum.inr g` type `Cell (τ g)`. -/
def Row.elim (f : Row H σ) (r : Row G τ) : Row (H ⊕ G) (Sum.elim σ τ) :=
  fun c => match c with
    | Sum.inl h => f h
    | Sum.inr g => r g

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

/-- The single row-wise primitive (def:selection + def:mutating + def:filtering,
and the row-expanding direction of def:grouping).  `φ k f` maps a nested row to a
multiset of output rows: `0` drops it, a singleton keeps or transforms it, and
several rows expand it.  Being `Multiset.bind`-shaped over a commutative union,
it is a bind-homomorphism (hence split-invariant) with no disjointness needed. -/
def map (φ : K → Row H σ → Multiset (Row H' σ')) (T : Table K H σ) :
    Table K H' σ' :=
  ⟨fun k => (T.rows k).bind (φ k)⟩

/-- def:left-join against a fixed right table, sharing index columns `U` and
adding columns `G` (disjoint from `H` via `⊕`, with the combined schema
`Sum.elim σ τ`).  Each present left row is combined with every matching right
row, or kept once with missing right columns when there is no match (the "left"
guarantee).  Being a `map`, it is split-invariant. -/
def leftJoin (key : K → U) (right : Table U G τ) (T : Table K H σ) :
    Table K (H ⊕ G) (Sum.elim σ τ) :=
  map (fun k f =>
    let R := right.rows (key k)
    if R.card = 0 then {f.elim (fun _ => none)}
    else R.map (fun r => f.elim r)) T

/-- def:inner-join against a fixed right table.  Like `leftJoin`, but a left row
with no match is dropped (empty cross product) rather than kept with missing
columns.  Still a `map`, so split-invariant.

The chapter leaves split-invariance of the inner join open, noting only that the
*binary* join can erase rows from either side.  In the unary, fixed-right form
the only effect is dropping unmatched left rows -- a `map` -- so it is. -/
def innerJoin (key : K → U) (right : Table U G τ) (T : Table K H σ) :
    Table K (H ⊕ G) (Sum.elim σ τ) :=
  map (fun k f => (right.rows (key k)).map (fun r => f.elim r)) T

/-- def:aggregating.  Collapse each key's whole bag of nested rows to a single
row via `f` (empty stays empty).  Unlike `map`, `f` sees the *entire* multiset at
a key, so it is a sibling of `map` under a more general "whole-bag per key"
operation, not a special case.  That whole-bag access is why it is not a
bind-homomorphism (`aggregate_not_bindHom`), though it remains split-invariant
(`aggregate_splitInvariant`): a split never merges a key's bag. -/
def aggregate (f : K → Multiset (Row H σ) → Row H σ) (T : Table K H σ) :
    Table K H σ :=
  ⟨fun k => if (T.rows k).card = 0 then 0 else {f k (T.rows k)}⟩

/-- def:grouping (ungroup).  Turn the distinguished column `Sum.inr ()` (domain
`β`) into part of the key: the new key is `K × β`, and at `(k, v)` we keep the
nested rows of key `k` whose ungrouped column held `some v`, dropping that
column.  An arbitrary column is reached by `map`-reorder then ungroup; a row
whose ungrouped column is missing matches no `v` and is dropped (the chapter
requires that column total).  Being `Multiset.bind`-shaped per output key over a
single input key, it is split-invariant. -/
def ungroup {β : Type} [DecidableEq β]
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
it does not preserve disjointness (`project_not_preservesDisjoint`): two input
rows that a split separates can share an output key, so `project` is not
`SplitSafe` even though it is a `BindHom` (`project_bindHom`). -/
def project [Fintype D] (T : Table (K × D) H σ) :
    Table K (H ⊕ Unit) (Sum.elim σ (fun _ => D)) :=
  ⟨fun k => ∑ d : D, (T.rows (k, d)).map (fun f => Row.elim f (fun _ => some d))⟩

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

/-- `map` is a bind-homomorphism, since `Multiset.bind` distributes over union. -/
theorem map_bindHom (φ : K → Row H σ → Multiset (Row H' σ')) :
    BindHom (map φ) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [map, bind]
  exact Multiset.add_bind _ _ _

/-- Hence `map` is split-invariant, the property Mensura enforces. -/
theorem map_splitInvariant (φ : K → Row H σ → Multiset (Row H' σ')) :
    SplitInvariant (map φ) := (map_bindHom φ).splitInvariant

/-- `leftJoin` against a fixed table is a bind-homomorphism: it is a `map`. -/
theorem leftJoin_bindHom (key : K → U) (right : Table U G τ) :
    BindHom (leftJoin (σ := σ) key right) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [leftJoin, map, bind]
  exact Multiset.add_bind _ _ _

/-- Hence `leftJoin` is split-invariant. -/
theorem leftJoin_splitInvariant (key : K → U) (right : Table U G τ) :
    SplitInvariant (leftJoin (σ := σ) key right) :=
  (leftJoin_bindHom key right).splitInvariant

/-- The unary, fixed-right `innerJoin` is a bind-homomorphism: it is a `map`. -/
theorem innerJoin_bindHom (key : K → U) (right : Table U G τ) :
    BindHom (innerJoin (σ := σ) key right) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [innerJoin, map, bind]
  exact Multiset.add_bind _ _ _

/-- Hence the unary, fixed-right `innerJoin` is split-invariant. -/
theorem innerJoin_splitInvariant (key : K → U) (right : Table U G τ) :
    SplitInvariant (innerJoin (σ := σ) key right) :=
  (innerJoin_bindHom key right).splitInvariant

/-- `ungroup` is a bind-homomorphism.  Each output key `(k, v)` reads only from
input key `k`, where the operation is `Multiset.bind`, which distributes over `+`. -/
theorem ungroup_bindHom {β : Type} [DecidableEq β] :
    BindHom (ungroup (K := K) (H := H) (σ := σ) (β := β)) := by
  intro T₀ T₁
  apply Table.ext_rows
  rintro ⟨k, v⟩
  simp only [ungroup, bind]
  exact Multiset.add_bind _ _ _

/-- Hence `ungroup` is split-invariant. -/
theorem ungroup_splitInvariant {β : Type} [DecidableEq β] :
    SplitInvariant (ungroup (K := K) (H := H) (σ := σ) (β := β)) :=
  ungroup_bindHom.splitInvariant

/-- `aggregate` *is* split-invariant -- the property Mensura enforces, and the
book's claim.  Under disjointness, at every key one summand is empty, so folding
the union is the same as folding the nonempty side. -/
theorem aggregate_splitInvariant (f : K → Multiset (Row H σ) → Row H σ) :
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
the right.  This is the operation that separates `SplitInvariant` from the
strictly stronger `BindHom`. -/
theorem aggregate_not_bindHom :
    ¬ BindHom
        (aggregate (fun (_ : Unit) (_ : Multiset (Row Unit (fun _ => Unit))) =>
          fun _ => none)) := by
  intro h
  have hT := h ⟨fun _ => {fun _ => none}⟩ ⟨fun _ => {fun _ => none}⟩
  apply_fun (fun U => (U.rows ()).card) at hT
  simp [aggregate, bind] at hT

/-- `project` *is* a bind-homomorphism (hence split-invariant): it sums each
dropped key's mapped rows, and both `Multiset.map` and `Finset.sum` distribute
over `+`.  So by the bare equation alone, `project` looks safe. -/
theorem project_bindHom [Fintype D] :
    BindHom (project (K := K) (H := H) (σ := σ) (D := D)) := by
  intro T₀ T₁
  apply Table.ext_rows
  intro k
  simp only [project, bind, Multiset.map_add]
  rw [Finset.sum_add_distrib]

/-- ...yet `project` does *not* preserve disjointness, so it is **not**
`SplitSafe`.  Counterexample: two single-row tables that a split separates by the
dropped index (`d = false` vs `d = true`) both land on the same output key once
`d` is dropped.  This is precisely why a pipeline through `project` -- e.g.
`aggregate ∘ project`, the averaging case -- can disagree with the full-table
result: `aggregate` is then handed overlapping tables its guarantee does not
cover. -/
theorem project_not_preservesDisjoint :
    ¬ PreservesDisjoint
        (project (K := Unit) (σ := fun (_ : Unit) => Unit) (D := Bool)) := by
  intro h
  have hd := h
    ⟨fun p => if p.2 then 0 else {fun _ => none}⟩
    ⟨fun p => if p.2 then {fun _ => none} else 0⟩
    (by intro p; cases hp : p.2 <;> simp [hp]) ()
  simp [project] at hd

/-! ### Split-safety of the operations

Every operation defined above preserves disjointness, hence is `SplitSafe`, hence
can be freely composed into pipelines that stay split-invariant.  Each acts at a
key (or refines the key, for `ungroup`) using `Multiset.bind`/the `if`, both of
which send the empty multiset to the empty multiset. -/

theorem map_preservesDisjoint (φ : K → Row H σ → Multiset (Row H' σ')) :
    PreservesDisjoint (map φ) := by
  intro T₀ T₁ hdisj k
  rcases hdisj k with h | h
  · exact Or.inl (by simp [map, h])
  · exact Or.inr (by simp [map, h])

theorem map_splitSafe (φ : K → Row H σ → Multiset (Row H' σ')) :
    SplitSafe (map φ) := ⟨map_preservesDisjoint φ, map_splitInvariant φ⟩

theorem aggregate_preservesDisjoint (f : K → Multiset (Row H σ) → Row H σ) :
    PreservesDisjoint (aggregate f) := by
  intro T₀ T₁ hdisj k
  rcases hdisj k with h | h
  · exact Or.inl (by simp [aggregate, h])
  · exact Or.inr (by simp [aggregate, h])

theorem aggregate_splitSafe (f : K → Multiset (Row H σ) → Row H σ) :
    SplitSafe (aggregate f) := ⟨aggregate_preservesDisjoint f, aggregate_splitInvariant f⟩

theorem leftJoin_preservesDisjoint (key : K → U) (right : Table U G τ) :
    PreservesDisjoint (leftJoin (σ := σ) key right) := by
  intro T₀ T₁ hdisj k
  rcases hdisj k with h | h
  · exact Or.inl (by simp [leftJoin, map, h])
  · exact Or.inr (by simp [leftJoin, map, h])

theorem leftJoin_splitSafe (key : K → U) (right : Table U G τ) :
    SplitSafe (leftJoin (σ := σ) key right) :=
  ⟨leftJoin_preservesDisjoint key right, leftJoin_splitInvariant key right⟩

theorem innerJoin_preservesDisjoint (key : K → U) (right : Table U G τ) :
    PreservesDisjoint (innerJoin (σ := σ) key right) := by
  intro T₀ T₁ hdisj k
  rcases hdisj k with h | h
  · exact Or.inl (by simp [innerJoin, map, h])
  · exact Or.inr (by simp [innerJoin, map, h])

theorem innerJoin_splitSafe (key : K → U) (right : Table U G τ) :
    SplitSafe (innerJoin (σ := σ) key right) :=
  ⟨innerJoin_preservesDisjoint key right, innerJoin_splitInvariant key right⟩

theorem ungroup_preservesDisjoint {β : Type} [DecidableEq β] :
    PreservesDisjoint (ungroup (K := K) (H := H) (σ := σ) (β := β)) := by
  intro T₀ T₁ hdisj
  rintro ⟨k, v⟩
  rcases hdisj k with h | h
  · exact Or.inl (by simp [ungroup, h])
  · exact Or.inr (by simp [ungroup, h])

theorem ungroup_splitSafe {β : Type} [DecidableEq β] :
    SplitSafe (ungroup (K := K) (H := H) (σ := σ) (β := β)) :=
  ⟨ungroup_preservesDisjoint, ungroup_splitInvariant⟩

/-- A pipeline of split-safe operations is split-safe, hence split-invariant --
applying it between split and bind equals applying it to the full table.  Here:
`map`, then `leftJoin`, then `aggregate`. -/
example (φ : K → Row H σ → Multiset (Row H' σ')) (key : K → U) (right : Table U G τ)
    (g : K → Multiset (Row (H' ⊕ G) (Sum.elim σ' τ)) → Row (H' ⊕ G) (Sum.elim σ' τ)) :
    SplitSafe (aggregate g ∘ leftJoin (σ := σ') key right ∘ map φ) :=
  (aggregate_splitSafe g).comp ((leftJoin_splitSafe key right).comp (map_splitSafe φ))

end Mensura
