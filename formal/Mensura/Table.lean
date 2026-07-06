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
`aggregate` (def:aggregating), `project` (def:projection), `unpivot` / `pivot`
(def:pivot-w2l / def:pivot-l2w), and the tagged variants `taggedBind` /
`taggedSplit` (def:tagged-bind / def:tagged-split).  Proved: split yields
disjoint tables, bind undoes split, bind is commutative and associative,
`map`/`leftJoin`/`innerJoin`/`ungroup`/`unpivot` are bind-homomorphisms,
`aggregate` is split-invariant but not a bind-homomorphism, `project` is a
bind-homomorphism but not disjointness-preserving, `pivot` is *not even*
split-invariant (a name-separating split breaks it) though it inverts `unpivot`
on functional tables (`pivot_unpivot`), and `taggedSplit` inverts `taggedBind`
(`taggedSplit_taggedBind_left`/`_right`).  The drop-variant `unpivotDrop`
(ADR 0020: a missing cell yields no long row) is split-safe
(`unpivotDrop_splitSafe`) and forms a *mutually* inverse pair with `pivot`
on functional minimal tables (`pivot_unpivotDrop`, `unpivotDrop_pivot`),
the long-to-wide-to-long direction carrying no completeness side condition.
`Exhaustive` (every present residual key carries every name) is the
rectangle fact that upgrades `pivot`'s output to `Total`
(`pivot_total_of_exhaustive`); `unpivotDrop` establishes it from a `Total`
wide table (`unpivotDrop_exhaustive`) and its output is `Minimal`
unconditionally (`unpivotDrop_minimal`).  All bind-homomorphisms plus
`aggregate` are `SplitSafe` and compose; `project` and `pivot` are not.  The
chapter's minimality assumption is `Minimal`, preserved by `bind` and `split`.
The `bind` unit laws over `empty`, the `map` identity/fusion laws, the join
pushdowns (`map` fusion into both joins, left-column `filter` past both
joins), and the `split`/`bind` cancellations are stated as bare equations,
seeding the future rewrite-rule set (ADR 0008).

This completes the operations of the chapter's data-handling algebra.  Future
directions: the grouped/arranged (window) operations of the chapter's "Other
operations" section, which are deliberately not split-invariant, and a
completeness/expressiveness result for the algebra as a whole.
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
variable {N : Type _} {V : Type}
variable {S : Type}

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

/-- Read the one value out of a card-≤1 bag of single-column rows (`none` when
empty).  `noncomputable` (uses `Multiset.toList`), which is fine for this
proof-only development; execution lives in the runtime layer. -/
noncomputable def cellOf (m : Multiset (Row Unit (fun _ => V))) : Cell V :=
  (m.toList.head?).elim none (fun g => g ())

@[simp] theorem cellOf_zero :
    cellOf (0 : Multiset (Row Unit (fun _ => V))) = none := by
  simp [cellOf, Multiset.toList_zero]

@[simp] theorem cellOf_singleton (g : Row Unit (fun _ => V)) :
    cellOf {g} = g () := by
  simp [cellOf, Multiset.toList_singleton]

/-- def:pivot-w2l (unpivot, wide-to-long).  Spread each name-column `n` of a wide
row into its own output key `(k, n)`, carrying that column's value.  A map-like
operation, hence a `BindHom` and `SplitSafe` -- the safe reshape direction. -/
def unpivot (T : Table K N (fun _ => V)) : Table (K × N) Unit (fun _ => V) :=
  ⟨fun p => (T.rows p.1).map (fun f => fun _ => f p.2)⟩

/-- def:pivot-l2w (pivot, long-to-wide).  Gather the value at each name `n` into
one wide row per key `k`; a key whose names are all empty stays absent (matching
the chapter's minimality).  A canonical value exists only at card ≤ 1 per
`(k, n)` (the chapter's "card constant"), so `cellOf` is used.  Unlike `unpivot`,
`pivot` is *not* split-invariant (`pivot_not_splitInvariant`). -/
noncomputable def pivot [Fintype N] (T : Table (K × N) Unit (fun _ => V)) :
    Table K N (fun _ => V) :=
  ⟨fun k => if (∀ n, (T.rows (k, n)).card = 0) then 0
            else {fun n => cellOf (T.rows (k, n))}⟩

/-- A table is functional when every key holds at most one nested row. -/
def Functional (T : Table K H σ) : Prop := ∀ k, (T.rows k).card ≤ 1

/-- Tag a row with a source value `s` in a fresh column `Sum.inr ()` (domain `S`). -/
def addTag (s : S) (f : Row H σ) : Row (H ⊕ Unit) (Sum.elim σ (fun _ => S)) :=
  Row.elim f (fun _ => some s)

/-- Drop the tag column, projecting back to the original columns. -/
def dropTag (f : Row (H ⊕ Unit) (Sum.elim σ (fun _ => S))) : Row H σ :=
  fun h => f (Sum.inl h)

@[simp] theorem addTag_inr (s : S) (f : Row H σ) :
    addTag s f (Sum.inr ()) = some s := rfl

@[simp] theorem dropTag_addTag (s : S) (f : Row H σ) : dropTag (addTag s f) = f := rfl

/-- def:tagged-bind.  Bind two tables, recording each row's source in a new
column: `T₀`'s rows are tagged `s₀`, `T₁`'s `s₁`.  It is `bind` of two
tag-`map`s, so its content is the plain bind plus the source column. -/
def taggedBind (s₀ s₁ : S) (T₀ T₁ : Table K H σ) :
    Table K (H ⊕ Unit) (Sum.elim σ (fun _ => S)) :=
  bind (map (fun _ f => {addTag s₀ f}) T₀) (map (fun _ f => {addTag s₁ f}) T₁)

/-- def:tagged-split.  Recover the rows of source `s`: keep those whose tag
column is `some s`, dropping the tag.  A `map`, hence split-safe; it inverts
`taggedBind` (`taggedSplit_taggedBind_left`). -/
def taggedSplit [DecidableEq S]
    (T : Table K (H ⊕ Unit) (Sum.elim σ (fun _ => S))) (s : S) : Table K H σ :=
  map (fun _ f =>
    let v : Cell S := f (Sum.inr ())
    match v with
    | some w => if w = s then {dropTag f} else 0
    | none => 0) T

/-- A nested row is *substantive* when at least one column is present; the
chapter's minimality assumption forbids all-missing nested rows. -/
def Substantive (f : Row H σ) : Prop := ∃ h, f h ≠ none

/-- A table is minimal when every nested row is substantive (the chapter's
standing well-formedness assumption, so `card` counts only real rows). -/
def Minimal (T : Table K H σ) : Prop := ∀ k, ∀ f ∈ T.rows k, Substantive f

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

/-! ### Reshape: unpivot is split-safe, pivot is not, and they are inverses -/

/-- `unpivot` is a bind-homomorphism (it is map-like over the input key). -/
theorem unpivot_bindHom : BindHom (unpivot (K := K) (N := N) (V := V)) := by
  intro T₀ T₁
  apply Table.ext_rows
  rintro ⟨k, n⟩
  simp only [unpivot, bind]
  exact Multiset.map_add _ _ _

theorem unpivot_preservesDisjoint :
    PreservesDisjoint (unpivot (K := K) (N := N) (V := V)) := by
  intro T₀ T₁ hdisj
  rintro ⟨k, n⟩
  rcases hdisj k with h | h
  · exact Or.inl (by simp [unpivot, h])
  · exact Or.inr (by simp [unpivot, h])

/-- Hence `unpivot` is split-safe -- the safe reshape direction. -/
theorem unpivot_splitSafe : SplitSafe (unpivot (K := K) (N := N) (V := V)) :=
  ⟨unpivot_preservesDisjoint, unpivot_bindHom.splitInvariant⟩

/-- `pivot` is *not* split-invariant: a split that separates the names of one key
yields two complementary partial rows that union-`bind` keeps apart (card 1 on
the left, card 2 on the right).  This refines the book, whose pivot
split-invariance relies on cell-wise-merge bind over ragged cells -- which this
total-row / union-bind model deliberately does not have. -/
theorem pivot_not_splitInvariant :
    ¬ SplitInvariant (pivot (K := Unit) (N := Bool) (V := Unit)) := by
  intro h
  have hd := h
    ⟨fun p => if p.2 then 0 else {fun _ => none}⟩
    ⟨fun p => if p.2 then {fun _ => none} else 0⟩
    (by intro p; cases hp : p.2 <;> simp [hp])
  apply_fun (fun U => (U.rows ()).card) at hd
  simp [pivot, bind, Bool.forall_bool] at hd

/-- def:pivot inverts def:pivot-w2l on functional tables (the "card constant"
case): pivoting an unpivoted wide table recovers it.  This reversibility is the
reason pivot is useful despite not being split-invariant. -/
theorem pivot_unpivot [Fintype N] [Nonempty N] {T : Table K N (fun _ => V)}
    (hT : Functional T) : pivot (unpivot T) = T := by
  apply Table.ext_rows
  intro k
  rcases Nat.lt_or_ge (T.rows k).card 1 with hc | hc
  · have h0 : T.rows k = 0 := Multiset.card_eq_zero.mp (by omega)
    simp [pivot, unpivot, h0]
  · have hc1 : (T.rows k).card = 1 := le_antisymm (hT k) hc
    obtain ⟨f, hf⟩ := Multiset.card_eq_one.mp hc1
    obtain ⟨n₀⟩ := (inferInstance : Nonempty N)
    have hguard : ¬ (∀ n, ((unpivot T).rows (k, n)).card = 0) := by
      intro hall
      have h1 := hall n₀
      simp [unpivot, hf] at h1
    simp only [pivot]
    rw [if_neg hguard]
    simp [unpivot, hf, Multiset.map_singleton]

/-! ### The drop-variant unpivot: a truly inverse pair (ADR 0020)

`unpivot` above *reifies* a missing wide cell as a long row holding `none`.
That asymmetry is what blocks the long-to-wide-to-long round trip on sparse
tables: `pivot` sends an absent row to a missing cell, and the reify
variant sends the missing cell back to a *present* row, fabricating rows
the sparse table never had.  The drop variant
(`docs/decisions/0020-reshape-as-a-true-inverse-pair.md`) removes the
asymmetry: a missing cell yields no long row, so value-missing in the wide
table and row-absent in the long table carry the same information, and the
pair becomes mutually inverse on functional, minimal tables -- with no
completeness or saturation side condition in either direction. -/

/-- The drop-variant of def:pivot-w2l (ADR 0020): spread each name-column
`n` of a wide row into its own output key `(k, n)`, emitting a long row
only when the cell is present.  The long value column is total by
construction.  Being `Multiset.bind`-shaped per output key over a single
input key (compare `ungroup`), it is a bind-homomorphism, hence
split-safe. -/
def unpivotDrop (T : Table K N (fun _ => V)) : Table (K × N) Unit (fun _ => V) :=
  ⟨fun p => (T.rows p.1).bind (fun f =>
    match f p.2 with
    | some v => {fun _ => some v}
    | none => 0)⟩

/-- `unpivotDrop` is a bind-homomorphism: the drop is decided per input
row, so it distributes over every bind. -/
theorem unpivotDrop_bindHom : BindHom (unpivotDrop (K := K) (N := N) (V := V)) := by
  intro T₀ T₁
  apply Table.ext_rows
  rintro ⟨k, n⟩
  simp only [unpivotDrop, bind]
  exact Multiset.add_bind _ _ _

theorem unpivotDrop_preservesDisjoint :
    PreservesDisjoint (unpivotDrop (K := K) (N := N) (V := V)) := by
  intro T₀ T₁ hdisj
  rintro ⟨k, n⟩
  rcases hdisj k with h | h
  · exact Or.inl (by simp [unpivotDrop, h])
  · exact Or.inr (by simp [unpivotDrop, h])

/-- `unpivotDrop` is split-safe, like the reify variant: dropping a missing
cell is a per-row decision. -/
theorem unpivotDrop_splitSafe : SplitSafe (unpivotDrop (K := K) (N := N) (V := V)) :=
  ⟨unpivotDrop_preservesDisjoint, unpivotDrop_bindHom.splitInvariant⟩

/-- `pivot` inverts `unpivotDrop` on functional, **minimal** wide tables.
Minimality is load-bearing where it was not for the reify variant
(`pivot_unpivot`): a wide row whose cells are all missing yields no long
rows, so its key would vanish; the chapter's standing minimality
assumption rules exactly those rows out (ADR 0020's "at least one total
folded column" is the surface approximation of it). -/
theorem pivot_unpivotDrop [Fintype N] {T : Table K N (fun _ => V)}
    (hT : Functional T) (hM : Minimal T) : pivot (unpivotDrop T) = T := by
  apply Table.ext_rows
  intro k
  rcases Nat.lt_or_ge (T.rows k).card 1 with hc | hc
  · have h0 : T.rows k = 0 := Multiset.card_eq_zero.mp (by omega)
    simp [pivot, unpivotDrop, h0]
  · have hc1 : (T.rows k).card = 1 := le_antisymm (hT k) hc
    obtain ⟨f, hf⟩ := Multiset.card_eq_one.mp hc1
    obtain ⟨n₀, hn₀⟩ := hM k f (by rw [hf]; exact Multiset.mem_singleton_self f)
    obtain ⟨v₀, hv₀⟩ := Option.ne_none_iff_exists'.mp hn₀
    have hguard : ¬ (∀ n, ((unpivotDrop T).rows (k, n)).card = 0) := by
      intro hall
      have h1 := hall n₀
      simp [unpivotDrop, hf, hv₀] at h1
    simp only [pivot]
    rw [if_neg hguard, hf, Multiset.singleton_inj]
    funext n
    cases hfn : f n <;> simp [unpivotDrop, hf, hfn]

/-- `unpivotDrop` inverts `pivot` on functional, minimal long tables --
with **no completeness or saturation side condition**: an absent `(k, n)`
row pivots to a missing cell, which the drop variant sends back to an
absent row, so a sparse long table round-trips as it is.  This direction
is not statable for the reify variant, and it is the mechanized content of
ADR 0020's inverse contract.  Minimality here says every long row's value
is known, which is the invariant `unpivotDrop`'s own output satisfies by
construction. -/
theorem unpivotDrop_pivot [Fintype N] {L : Table (K × N) Unit (fun _ => V)}
    (hL : Functional L) (hM : Minimal L) : unpivotDrop (pivot L) = L := by
  apply Table.ext_rows
  rintro ⟨k, n⟩
  by_cases hg : ∀ n', (L.rows (k, n')).card = 0
  · -- Every fiber at `k` is empty: both sides are empty at `(k, n)`.
    have h0 : L.rows (k, n) = 0 := Multiset.card_eq_zero.mp (hg n)
    simp [unpivotDrop, pivot, hg, h0]
  · -- Some fiber is present: the one wide row reads back cell by cell.
    rcases Nat.lt_or_ge (L.rows (k, n)).card 1 with hc | hc
    · have h0 : L.rows (k, n) = 0 := Multiset.card_eq_zero.mp (by omega)
      simp only [unpivotDrop, pivot]
      rw [if_neg hg]
      simp [h0]
    · have hc1 : (L.rows (k, n)).card = 1 := le_antisymm (hL (k, n)) hc
      obtain ⟨g, hgrow⟩ := Multiset.card_eq_one.mp hc1
      obtain ⟨u, hu⟩ := hM (k, n) g (by rw [hgrow]; exact Multiset.mem_singleton_self g)
      have hu' : g () ≠ none := by cases u; exact hu
      obtain ⟨v, hv⟩ := Option.ne_none_iff_exists'.mp hu'
      simp only [unpivotDrop, pivot]
      rw [if_neg hg]
      have hcell : cellOf (L.rows (k, n)) = some v := by
        rw [hgrow, cellOf_singleton, hv]
      simp only [Multiset.singleton_bind, hcell]
      rw [hgrow, Multiset.singleton_inj]
      funext u'
      cases u'
      exact hv.symm

/-- Every cell of every present row is known: the table-level reading of
"all columns total" (ADR 0010's default).  Row-wise stronger than
`Substantive`, hence stronger than `Minimal` on a nonempty schema. -/
def Total (T : Table K H σ) : Prop := ∀ k, ∀ f ∈ T.rows k, ∀ h, f h ≠ none

/-- The rectangle fact of ADR 0020: every residual key present in a long
table carries a row for **every** name `n`.  The reference is the name
type's whole domain, not a population -- which is exactly what
distinguishes it from the (unmechanized, population-relative)
`complete_over` and makes it the fact that licenses `pivot`'s totality
upgrade (`pivot_total_of_exhaustive`). -/
def Exhaustive (T : Table (K × N) H σ) : Prop :=
  ∀ k, (∃ n, T.Present (k, n)) → ∀ n, T.Present (k, n)

/-- `unpivotDrop`'s output is minimal unconditionally: every long row it
emits carries a known value, by construction. -/
theorem unpivotDrop_minimal (T : Table K N (fun _ => V)) :
    Minimal (unpivotDrop T) := by
  rintro ⟨k, n⟩ f hf
  simp only [unpivotDrop] at hf
  obtain ⟨g, _, hfg⟩ := Multiset.mem_bind.mp hf
  refine ⟨(), ?_⟩
  cases hgn : g n with
  | none => simp [hgn] at hfg
  | some v =>
      simp only [hgn] at hfg
      simp [Multiset.mem_singleton.mp hfg]

/-- `unpivotDrop` establishes the rectangle by mechanism on a `Total` wide
table (ADR 0020: "every folded column is total"): each source row emits
one long row per name, so a residual key present for one name is present
for all. -/
theorem unpivotDrop_exhaustive {T : Table K N (fun _ => V)} (hTot : Total T) :
    Exhaustive (unpivotDrop T) := by
  rintro k ⟨n₀, hn₀⟩ n
  simp only [Table.Present, unpivotDrop] at hn₀ ⊢
  obtain ⟨x, hx⟩ := Multiset.exists_mem_of_ne_zero hn₀
  obtain ⟨f, hf, _⟩ := Multiset.mem_bind.mp hx
  obtain ⟨v, hv⟩ := Option.ne_none_iff_exists'.mp (hTot k f hf n)
  refine Multiset.card_pos.mp (Multiset.card_pos_iff_exists_mem.mpr
    ⟨fun _ => some v, Multiset.mem_bind.mpr ⟨f, hf, ?_⟩⟩)
  simp [hv]

/-- The totality upgrade of ADR 0020, semantically: pivoting an exhaustive,
minimal long table yields only known cells.  `Exhaustive` supplies the row
at every `(k, n)` and `Minimal` supplies the value in it; no
population-relative completeness fact appears, which is the content of the
ADR's `exhaustive` / `complete_over` distinction. -/
theorem pivot_total_of_exhaustive [Fintype N] {L : Table (K × N) Unit (fun _ => V)}
    (hE : Exhaustive L) (hM : Minimal L) : Total (pivot L) := by
  intro k f hf n
  by_cases hg : ∀ n', (L.rows (k, n')).card = 0
  · simp [pivot, hg] at hf
  · have hfF : f = fun n' => cellOf (L.rows (k, n')) := by
      simp only [pivot] at hf
      rw [if_neg hg] at hf
      exact Multiset.mem_singleton.mp hf
    obtain ⟨n₀, hn₀⟩ := not_forall.mp hg
    have hpres : L.rows (k, n) ≠ 0 := by
      refine hE k ⟨n₀, fun h0 => hn₀ ?_⟩ n
      simp [h0]
    have htl : (L.rows (k, n)).toList ≠ [] := by
      simpa [Multiset.toList_eq_nil] using hpres
    obtain ⟨g, l, hl⟩ := List.exists_cons_of_ne_nil htl
    have hgmem : g ∈ L.rows (k, n) := by
      rw [← Multiset.mem_toList, hl]
      exact List.mem_cons_self
    obtain ⟨u, hu⟩ := hM (k, n) g hgmem
    have hu' : g () ≠ none := by cases u; exact hu
    have hcell : cellOf (L.rows (k, n)) = g () := by
      simp [cellOf, hl]
    rw [hfF]
    simpa [hcell] using hu'

/-! ### Tagged bind / split: reversibility -/

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
  simp [taggedSplit, taggedBind, map, bind, Multiset.add_bind, Multiset.bind_map,
        Multiset.bind_singleton, bind_singleton_id, Multiset.bind_zero, Ne.symm hne]

/-- Symmetrically, recovering source `s₁` gives back `T₁`. -/
theorem taggedSplit_taggedBind_right [DecidableEq S] {s₀ s₁ : S} (hne : s₀ ≠ s₁)
    (T₀ T₁ : Table K H σ) :
    taggedSplit (taggedBind s₀ s₁ T₀ T₁) s₁ = T₁ := by
  apply Table.ext_rows
  intro k
  simp [taggedSplit, taggedBind, map, bind, Multiset.add_bind, Multiset.bind_map,
        Multiset.bind_singleton, bind_singleton_id, Multiset.bind_zero, hne]

/-! ### Equational laws: the rewrite-rule seeds

Per ADR 0008, algebraic laws are stated in equational form (`lhs = rhs`
under named side conditions) so they translate directly into rewrite rules
once the processing layer grows an optimizing plan IR.  The laws below are
the prototypical plan rewrites: `empty` completes `bind`'s commutative
monoid (unit laws); `map` carries identity and fusion laws (fusion subsumes
filter/filter, filter/mutate, and select fusion, since all are `map`s, ADR
0015); the joins absorb a preceding `map` and commute with a left-column
`filter` (pushdown); and `split`/`bind` cancel in both directions
(`bind_split` above, `split_bind` below). -/

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

/-! ### Minimality (the chapter's no-all-missing-row assumption) -/

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
