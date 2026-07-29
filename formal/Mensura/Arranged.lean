/-
The arranged structure: fiber content lifted from the bag monad to the list
monad, the inclusive and exclusive scans over it, and their coherence with the
fold (ADR 0029 Stage 2, ADR 0031 Decision 7).

Main Source:  Chapter 5, the "grouped and arranged" section, of F. A. N. Verri
(2026). Data Science Project: An Inductive Learning Approach. Version v1.0.0.
Victoria, British Columbia, Canada: Leanpub. doi: 10.5281/zenodo.14498010.
url: https://leanpub.com/dsp.

## Why new structure is needed at all

`Mensura.Core.Defs` argues *for* multisets: the chapter calls the order of
nested rows "arbitrary but fixed", and a multiset is exactly that, so the model
declines to assert an order it never uses.  That argument is right, and it is
precisely why neither a scan nor a positional map is expressible over a
`Multiset`: both read the order.  So the ordered verbs need an arrangement, and
this file supplies it without weakening the substrate: a `Table`'s content stays
a `Multiset`, and an arrangement is a *relation* between that content and a list
of the same elements.

## Why the arrangement is a relation, not a sort

The obvious route is `Multiset.sort`, and it does not work.  `Multiset.sort`
demands `Std.Antisymm r` as an *instance* on the element type, and for a
key-induced order (`r a b := key a <= key b`) antisymmetry is exactly the
statement that the key is injective.  ADR 0029's Tier 1 supplies that only
*within a fiber* (two rows in different fibers routinely share a key), and a
per-fiber fact cannot produce a global instance.

`IsArrangement` therefore states the property and splits the two obligations
that `Multiset.sort` conflates:

* **existence** needs no hypothesis at all, because `List.pairwise_mergeSort'`
  asks only for totality and transitivity (`exists_isArrangement`);
* **uniqueness** is Tier 1, and it goes through `List.Perm.eq_of_pairwise`,
  whose antisymmetry argument is a *pointwise hypothesis on the members* rather
  than an instance.  That is what per-fiber injectivity discharges
  (`IsArrangement.unique`).

Splitting them is not a workaround.  It is the honest shape of the surface
obligation: a scan is always *defined*, and it is *deterministic* exactly when
the key has no ties.

## One `scanl`, two scans

`List.scanl f e l` has length `l.length + 1` and carries the seed at position
zero, so it is natively "exclusive plus a final total".  Both surface forms are
slices of it:

    scanBag    op e l = (l.scanl op e).tail       -- inclusive: 1..i
    prescanBag op e l = (l.scanl op e).dropLast   -- exclusive: 1..i-1

So ADR 0031's demand that the two forms cohere is list slicing rather than a
second induction, and `lag`'s missing first row *falls out* of `dropLast` at the
`Option` completion: the exclusive scan's first entry folds the empty prefix,
which for a combiner with no identity is absent.  The missingness is not
designed.

## Which coherence claims exist, and for which combiners

`Multiset.fold` requires `Std.Commutative`, so the fold a scan could cohere
*with* does not exist for every row of the surface's combiner table:

* commutative monoid (`+`, `*`, `or`, `and`): full coherence, `scanBag`'s last
  element is `foldBag` (`scanBag_getLast_eq_foldBag`);
* commutative semigroup (binary `<<` and `>>`): full coherence through the
  `Option` completion (`scanBag_getLast_eq_foldBagOpt`);
* the associative-only tacks (`<:`, `:>`): **no `foldBag` exists**, so there is
  no coherence theorem to state, only the intrinsic `getLast = foldl` fact.
  This is the formal content of the surface rule that a tack is admitted under
  `scan` and rejected under `fold` (ADR 0031 Decision 6): it is not a taste, it
  is that `Multiset.fold` does not typecheck.

Note what coherence does *not* need: injectivity.  Ties may permute the
arrangement, but an associative-commutative combiner cannot observe the
difference, so the *total* is determined even when the intermediates are not.
That asymmetry is exactly why `fold` needs no order and `scan` does.
-/

import Mensura.Fold
import Mensura.Reshape

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {α ω : Type _}

/-! ## The arrangement of a fiber -/

/-- A list `l` *arranges* the bag `m` by `key` when it holds exactly `m`'s
elements and its keys are non-decreasing.  Stated as a relation rather than as a
chosen sort, because existence and uniqueness have different hypotheses: every
fiber can be arranged, but only a tie-free key arranges it uniquely. -/
def IsArrangement [LinearOrder ω] (key : α → ω) (m : Multiset α) (l : List α) :
    Prop :=
  (l : Multiset α) = m ∧ l.Pairwise (key ⁻¹'o (· ≤ ·))

/-- **Every fiber can be arranged**, with no hypothesis on the key.  Merge sort
at the key-induced order supplies the witness: `List.pairwise_mergeSort'` asks
only for totality and transitivity, both of which a `LinearOrder` gives through
`Order.Preimage`, and neither of which mentions ties.  This is the half of
`Multiset.sort` that survives without antisymmetry. -/
theorem exists_isArrangement [LinearOrder ω] (key : α → ω) (m : Multiset α) :
    ∃ l, IsArrangement key m l := by
  induction m using Quot.inductionOn with
  | _ l =>
    refine ⟨l.mergeSort fun a b => decide (key a ≤ key b), ?_, ?_⟩
    · exact Quot.sound (List.mergeSort_perm _ _)
    · exact List.pairwise_mergeSort' (key ⁻¹'o (· ≤ ·)) l

/-- **The Tier 1 determinism theorem.**  A key that is injective *on the fiber*
arranges it uniquely.  The proof goes through `List.Perm.eq_of_pairwise`, whose
antisymmetry argument is a pointwise hypothesis about the members rather than an
instance on the type: that is what makes a per-fiber fact usable, and it is why
`Multiset.sort` (which wants `Std.Antisymm` as an instance) cannot state this.

Two rows with equal keys would be interchangeable in the arrangement, so
injectivity is not merely sufficient here, it is what the conclusion is about
(ADR 0029 Decision 11's tie model, tier 1). -/
theorem IsArrangement.unique [LinearOrder ω] (key : α → ω) {m : Multiset α}
    (hinj : Set.InjOn key {a | a ∈ m}) {l₁ l₂ : List α}
    (h₁ : IsArrangement key m l₁) (h₂ : IsArrangement key m l₂) : l₁ = l₂ := by
  have hperm : l₁.Perm l₂ := Multiset.coe_eq_coe.mp (h₁.1.trans h₂.1.symm)
  refine List.Perm.eq_of_pairwise (fun a b ha hb hab hba => ?_) h₁.2 h₂.2 hperm
  have ham : a ∈ m := by rw [← h₁.1]; exact ha
  have hbm : b ∈ m := by rw [← h₂.1]; exact hb
  exact hinj ham hbm (le_antisymm hab hba)

/-- The canonical arrangement, chosen once so downstream statements need not
carry a witness.  `Classical.choice` is inside the axiom gate (ADR 0021), and
`arrangeList` below is the computable companion for the executable story. -/
noncomputable def arrange [LinearOrder ω] (key : α → ω) (m : Multiset α) :
    List α :=
  (exists_isArrangement key m).choose

theorem arrange_spec [LinearOrder ω] (key : α → ω) (m : Multiset α) :
    IsArrangement key m (arrange key m) :=
  (exists_isArrangement key m).choose_spec

/-- Under Tier 1 the canonical choice is the *only* choice, so `arrange` is
deterministic and any arrangement a reader exhibits is that one. -/
theorem arrange_eq_of_injOn [LinearOrder ω] (key : α → ω) {m : Multiset α}
    (hinj : Set.InjOn key {a | a ∈ m}) {l : List α}
    (h : IsArrangement key m l) : arrange key m = l :=
  IsArrangement.unique key hinj (arrange_spec key m) h

@[simp] theorem arrange_coe [LinearOrder ω] (key : α → ω) (m : Multiset α) :
    ((arrange key m : List α) : Multiset α) = m := (arrange_spec key m).1

@[simp] theorem arrange_length [LinearOrder ω] (key : α → ω) (m : Multiset α) :
    (arrange key m).length = Multiset.card m := by
  conv_rhs => rw [← arrange_coe key m]
  exact (Multiset.coe_card _).symm

/-- The computable arrangement of a list: merge sort at the key-induced order.
`arrange` is for reasoning (it picks a witness classically); this is what an
executor runs, and `arrangeList_isArrangement` says they agree on the property. -/
def arrangeList [LinearOrder ω] (key : α → ω) (l : List α) : List α :=
  l.mergeSort fun a b => decide (key a ≤ key b)

theorem arrangeList_isArrangement [LinearOrder ω] (key : α → ω) (l : List α) :
    IsArrangement key (l : Multiset α) (arrangeList key l) :=
  ⟨Quot.sound (List.mergeSort_perm _ _),
   List.pairwise_mergeSort' (key ⁻¹'o (· ≤ ·)) l⟩

/-! ## The two scans, as slices of one `List.scanl`

`List.scanl op e l` has length `l.length + 1`: the seed at position zero, then
one entry per element.  Dropping the seed gives the inclusive scan, dropping the
final total gives the exclusive one.  Everything about how the two relate is
therefore slicing, not induction. -/

/-- The **inclusive** scan: position `i` carries the fold of elements `1..i`.
No typeclass is required to define it; the laws enter only where a theorem needs
them. -/
def scanBag (op : α → α → α) (e : α) (l : List α) : List α :=
  (l.scanl op e).tail

/-- The **exclusive** scan (Blelloch's prescan): position `i` carries the fold of
the proper prefix `1..i-1`, so the first entry folds the empty prefix.  For a
combiner with an identity that is the identity; for one without, it is absent,
which is where `lag`'s missing first row comes from. -/
def prescanBag (op : α → α → α) (e : α) (l : List α) : List α :=
  (l.scanl op e).dropLast

@[simp] theorem scanBag_nil (op : α → α → α) (e : α) : scanBag op e [] = [] := rfl

@[simp] theorem prescanBag_nil (op : α → α → α) (e : α) :
    prescanBag op e [] = [] := rfl

@[simp] theorem length_scanBag (op : α → α → α) (e : α) (l : List α) :
    (scanBag op e l).length = l.length := by
  simp [scanBag]

@[simp] theorem length_prescanBag (op : α → α → α) (e : α) (l : List α) :
    (prescanBag op e l).length = l.length := by
  simp [prescanBag]

/-- **The two forms cohere**: the exclusive scan is the inclusive one shifted,
which is ADR 0031's "drop the last element of the inclusive scan, prepend the
identity" read off the shared `scanl`.  Both sides are slices of one list, so
this is bookkeeping rather than a second induction. -/
theorem prescanBag_eq_dropLast_scanl (op : α → α → α) (e : α) (l : List α) :
    prescanBag op e l = (l.scanl op e).dropLast := rfl

theorem scanBag_eq_tail_scanl (op : α → α → α) (e : α) (l : List α) :
    scanBag op e l = (l.scanl op e).tail := rfl

/-! ## Coherence with the fold

"Same combiner, two variants" is a theorem, not a slogan: the scan's last
element is the fold.  It needs commutativity and associativity, because that is
what `Multiset.fold` needs, and it needs *no* injectivity, because a
commutative-associative combiner cannot observe a tie's permutation. -/

/-- **The coherence theorem.**  The last entry of the inclusive scan over any
arrangement of a bag equals the bag's fold.  This is what licenses the surface's
claim that `fold` and `scan` share one combiner table: the ordered variant
refines the unordered one rather than departing from it.

The arrangement is arbitrary, so ties are harmless here: only the *total* is
claimed, and it is order-independent by `foldBag`'s own laws. -/
theorem scanl_getLast_eq_foldBag [LinearOrder ω] (key : α → ω)
    (op : α → α → α) [Std.Commutative op] [Std.Associative op] (e : α)
    {m : Multiset α} {l : List α} (h : IsArrangement key m l)
    (hne : l.scanl op e ≠ []) :
    (l.scanl op e).getLast hne = foldBag op e m := by
  rw [List.getLast_scanl]
  unfold foldBag
  rw [← h.1, Multiset.coe_fold_l]

/-- The coherence theorem for a combiner with no identity in its own domain: the
scan runs in the `Option` completion and its last entry is `foldBagOpt`.  This
is the pair (`<<`, `>>`) whose surface type is total only on a non-empty bag
(`foldBagOpt_isSome_of_ne_zero`). -/
theorem scanl_getLast_eq_foldBagOpt [LinearOrder ω] (key : α → ω)
    (op : α → α → α) [Std.Commutative op] [Std.Associative op]
    {m : Multiset α} {l : List α} (h : IsArrangement key m l)
    (hne : (l.map some).scanl (optionLift op) none ≠ []) :
    ((l.map some).scanl (optionLift op) none).getLast hne = foldBagOpt op m := by
  rw [List.getLast_scanl]
  unfold foldBagOpt foldBag
  rw [← h.1]
  rw [show ((l : Multiset α).map some) = ((l.map some : List (Option α)) : Multiset (Option α)) from
      (Multiset.map_coe some l)]
  rw [Multiset.coe_fold_l]

/-- **The prefix decomposition**, which licenses a parallel scan: scanning a
concatenation is the first part's scan followed by the second part's scan seeded
at the first part's total.  Stage 2's analogue of `foldBag_shards`, and note it
needs no laws at all, because a list scan asserts no order-independence. -/
theorem scanl_append_decomp (op : α → α → α) (e : α) (l₁ l₂ : List α) :
    (l₁ ++ l₂).scanl op e =
      l₁.scanl op e ++ (l₂.scanl op (l₁.foldl op e)).tail :=
  List.scanl_append

/-! ## Tier 1 as a table property

ADR 0029 asks the determinism hypothesis to compose with `Mensura.Functional`
rather than restate it.  It does compose, but the composition is *vacuous*, and
saying so is more useful than implying the ADR's letter and spirit coincide:
`Functional` bounds a key's fiber at one row, so there are no two rows to tie
and any scan over such a fiber is a one-element list.  The substantive
hypothesis is per-fiber injectivity of the order key, below; `Functional`
implies it, and that implication is a compatibility bridge rather than the
content. -/

/-- The order key is injective on every fiber: ADR 0029 Decision 11's tier 1,
as a property of a table rather than of a bare bag. -/
def KeyInjOn [LinearOrder ω] (key : K → Row H σ → ω) (T : Table K H σ) : Prop :=
  ∀ k, Set.InjOn (key k) {r | r ∈ T.rows k}

/-- A bag of at most one row has no two distinct members, so any function is
injective on it. -/
theorem injOn_of_card_le_one {m : Multiset α} (hm : Multiset.card m ≤ 1)
    (f : α → ω) : Set.InjOn f {a | a ∈ m} := by
  intro a ha b hb _
  -- `card <= 1` leaves only the empty bag and a singleton; the empty case
  -- contradicts membership, and in a singleton both members are its element.
  interval_cases h : Multiset.card m
  · exact absurd (Multiset.card_eq_zero.mp h ▸ ha) (by simp)
  · obtain ⟨c, rfl⟩ := Multiset.card_eq_one.mp h
    simp only [Set.mem_setOf_eq, Multiset.mem_singleton] at ha hb
    rw [ha, hb]

/-- `Functional` discharges Tier 1, vacuously: a functional table's fibers hold
at most one row, so no key can tie.  Useful as a compatibility bridge (a
singleton-cardinality store needs no tie ceremony), but it is not evidence that
a scan over a real bag is deterministic. -/
theorem keyInjOn_of_functional [LinearOrder ω] (key : K → Row H σ → ω)
    {T : Table K H σ} (hT : Functional T) : KeyInjOn key T :=
  fun k => injOn_of_card_le_one (hT k) (key k)

/-! ## The bridge to the algebra: an arranged verb is a safe fiber action

The blueprint's reserved node called arranged verbs "deliberately not
split-invariant".  That concerns lifting to the list monad *in general*, where an
order across keys would be asserted.  It is not true of these operations under
this `split`: `Mensura.split` routes each key's *whole* multiset to one side, so
a fiber is never torn and an arranged verb sorts an intact bag.  The results
agree whichever side the key lands on, and the proof is Stage 1's, reused
verbatim (ADR 0029, "the demand is a hypothesis to pin, not a contradiction to
resolve"). -/

/-- The fiber action of a scan: arrange the fiber by the key, scan the mapped
elements, and write one output row per input row.  Unlike `foldFiber` this is a
*window* shape (the output bag has the input's cardinality), which is the second
`map_bags` shape `fiberMap_exhaustive` already anticipated. -/
noncomputable def scanFiber [LinearOrder ω] (key : K → Row H σ → ω)
    (op : α → α → α) (e : α) (f : K → Row H σ → α)
    (out : K → Row H σ → α → Row H σ) (k : K) (m : Multiset (Row H σ)) :
    Multiset (Row H σ) :=
  let l := arrange (key k) m
  ((l.zip (scanBag op e (l.map (f k)))).map fun p => out k p.1 p.2 : List _)

/-- A scan never fabricates a row for an absent key: the empty bag arranges to
the empty list, whose scan is empty. -/
theorem scanFiber_strict [LinearOrder ω] (key : K → Row H σ → ω)
    (op : α → α → α) (e : α) (f : K → Row H σ → α)
    (out : K → Row H σ → α → Row H σ) :
    Strict (scanFiber key op e f out) := by
  intro k
  have h : arrange (key k) (0 : Multiset (Row H σ)) = [] :=
    List.length_eq_zero_iff.mp (by simp)
  simp [scanFiber, h]

/-- **A scan is split-safe.**  It is a strict `fiberMap`, so Stage 1's lemma
applies unchanged: the arrangement is computed inside one key's fiber, which a
`split` never tears.  This is the lemma ADR 0029 Stage 2 owes, and it is one
line because the substrate was already right. -/
theorem scanFiber_splitSafe [LinearOrder ω] (key : K → Row H σ → ω)
    (op : α → α → α) (e : α) (f : K → Row H σ → α)
    (out : K → Row H σ → α → Row H σ) :
    SplitSafe (fiberMap (scanFiber key op e f out)) :=
  fiberMap_splitSafe (scanFiber_strict key op e f out)

/-- A scan preserves the rectangle: it emits one output row per input row, so a
present fiber stays present (ADR 0020 section 2).  This is the window-shape half
of `fiberMap_exhaustive`'s docstring, now discharged. -/
theorem scanFiber_exhaustive {N : Type _} [LinearOrder ω]
    (key : K × N → Row H σ → ω) (op : α → α → α) (e : α)
    (f : K × N → Row H σ → α) (out : K × N → Row H σ → α → Row H σ)
    {T : Table (K × N) H σ} (hT : Exhaustive T) :
    Exhaustive (fiberMap (scanFiber key op e f out) T) := by
  refine fiberMap_exhaustive (scanFiber_strict key op e f out) ?_ hT
  intro p m hm
  have hlen : (arrange (key p) m).length = Multiset.card m := arrange_length _ _
  have hpos : 0 < Multiset.card m := Multiset.card_pos.mpr hm
  have : arrange (key p) m ≠ [] := by
    intro h
    rw [h] at hlen
    simp at hlen
    omega
  simp only [scanFiber, ne_eq, Multiset.coe_eq_zero]
  intro hnil
  rw [List.map_eq_nil_iff, List.zip_eq_nil_iff] at hnil
  rcases hnil with h | h
  · exact this h
  · rw [List.eq_nil_iff_length_eq_zero, length_scanBag] at h
    simp only [List.length_map] at h
    exact this (List.eq_nil_iff_length_eq_zero.mpr h)

end Mensura
