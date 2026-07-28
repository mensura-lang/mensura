/-
The monoid-parameterized bag fold: the well-behaved subclass of `aggregate`
that a closed table of combiners can implement soundly (ADR 0029 Stage 1,
ADR 0031).

Main Source:  Chapter 5 of F. A. N. Verri (2026). Data Science Project: An
Inductive Learning Approach. Version v1.0.0. Victoria, British Columbia,
Canada: Leanpub. doi: 10.5281/zenodo.14498010. url: https://leanpub.com/dsp.

## Why a *parameterized* fold, when `aggregate` already exists

`Mensura.aggregate` takes an arbitrary whole-bag function
`f : K → Multiset (Row H σ) → Row H σ`, so it is strictly **more** general
than anything here.  This file therefore *carves out* a subclass rather than
generalizing: the folds whose combiner is associative and commutative, which
is what makes the result independent of the bag's arbitrary order and what
licenses folding shards in parallel and combining the partial results.

That restriction is the whole point of the surface design.  Associativity and
commutativity are laws no checker can verify on a user-supplied lambda, so the
combiner is drawn from a closed table (ADR 0029 Decision 2, ADR 0031
Decision 6) whose rows are exactly the instances proved here, while the
*mapper* stays open because its obligation is a type check.

## The two shapes, and where the identity lives

A combiner with an identity (`+`, `*`, `or`, `and`) folds through
`foldBag`, and the identity is the honest answer for the empty bag.  A
combiner without one (binary minimum and maximum, `<<` and `>>`) has no such
answer -- there is no smallest element of nothing -- so it folds through
`foldBagOpt` into `Option`, and `foldBagOpt_isSome_of_ne_zero` is the lemma
that licenses the total surface type on a non-empty bag (ADR 0029
Decision 4).

The identity is in the accumulator, never a user-supplied seed: a seed is
counted once per shard, so `foldBag_shards` is false for a seed that is not
the identity.  That theorem is the reason the surface has no seed at all
(ADR 0029 Alternative 3, re-rejected in ADR 0031 Decision 5).
-/

import Mensura.Completeness.FiberMap

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {α β : Type _}

/-! ## The fold with an identity -/

/-- Fold a bag with an associative-commutative `op`, starting from `e`.  The
`Multiset` quotient is what makes this well defined without choosing an order:
`op`'s two laws are precisely the hypotheses `Multiset.fold` demands. -/
def foldBag (op : α → α → α) [Std.Commutative op] [Std.Associative op]
    (e : α) (m : Multiset α) : α :=
  m.fold op e

variable (op : α → α → α) [Std.Commutative op] [Std.Associative op]

@[simp] theorem foldBag_zero (e : α) : foldBag op e 0 = e := rfl

@[simp] theorem foldBag_singleton (e : α) (a : α) :
    foldBag op e {a} = op a e :=
  Multiset.fold_singleton op e a

/-- **The shard lemma.**  Folding two shards and combining the partial results
equals folding the union, *provided* the accumulator is the combiner's
identity.  This is what licenses partial and parallel folding, and it is the
theorem that forces the identity to live in the accumulator rather than in a
user-supplied seed (ADR 0029 Decision 4). -/
theorem foldBag_add (e : α) (he : ∀ a, op e a = a) (m₁ m₂ : Multiset α) :
    foldBag op e (m₁ + m₂) = op (foldBag op e m₁) (foldBag op e m₂) := by
  have h := Multiset.fold_add op e e m₁ m₂
  rwa [he e] at h

/-- The shard lemma over *arbitrarily many* shards, including empty ones: fold
each shard, then fold the partial results.  Empty shards are harmless exactly
because they contribute the identity. -/
theorem foldBag_shards (e : α) (he : ∀ a, op e a = a)
    (shards : Multiset (Multiset α)) :
    foldBag op e (shards.sum) = foldBag op e (shards.map (foldBag op e)) := by
  induction shards using Multiset.induction_on with
  | empty => simp
  | cons a s ih =>
    rw [Multiset.sum_cons, foldBag_add op e he, ih, Multiset.map_cons]
    exact (Multiset.fold_cons_left op e _ _).symm

/-- A seed that is *not* the identity breaks the shard lemma: it is counted
once per shard.  Stated on `Nat` addition with seed `1` and two empty shards,
where the left side is `1` and the right side `1 + 1`.  This is the concrete
witness behind ADR 0029 Alternative 3's rejection of `fold start op`. -/
theorem foldBag_add_seed_counterexample :
    foldBag (HAdd.hAdd : ℕ → ℕ → ℕ) 1 ((0 : Multiset ℕ) + 0) ≠
      foldBag (HAdd.hAdd : ℕ → ℕ → ℕ) 1 (0 : Multiset ℕ) +
        foldBag (HAdd.hAdd : ℕ → ℕ → ℕ) 1 (0 : Multiset ℕ) := by
  decide

/-! ## The fold without an identity: the `Option` completion

A semigroup has no empty-bag answer, so the accumulator carries the absence.
`optionLift op` is the free completion: `none` acts as the identity that the
domain itself does not contain.  It is associative and commutative whenever
`op` is, so it feeds the very same `foldBag`. -/

/-- Extend a binary operation to `Option`, letting `none` act as an identity.
This is the "accumulator `Option`" of ADR 0029 Decision 4. -/
def optionLift (op : α → α → α) : Option α → Option α → Option α
  | none, y => y
  | some x, none => some x
  | some x, some y => some (op x y)

-- The three reduction lemmas hold for *any* `op`: `none` is an identity by
-- construction, before any law about `op` is assumed.
omit [Std.Commutative op] [Std.Associative op] in
@[simp] theorem optionLift_none_left (y : Option α) : optionLift op none y = y := by
  cases y <;> rfl

omit [Std.Commutative op] [Std.Associative op] in
@[simp] theorem optionLift_none_right (x : Option α) : optionLift op x none = x := by
  cases x <;> rfl

omit [Std.Commutative op] [Std.Associative op] in
@[simp] theorem optionLift_some (x y : α) :
    optionLift op (some x) (some y) = some (op x y) := rfl

instance : Std.Commutative (optionLift op) where
  comm x y := by
    cases x <;> cases y <;> simp [Std.Commutative.comm (op := op)]

instance : Std.Associative (optionLift op) where
  assoc x y z := by
    cases x <;> cases y <;> cases z <;>
      simp [Std.Associative.assoc (op := op)]

/-- Fold a bag with an associative-commutative `op` that has **no** identity in
its own domain: the empty bag folds to `none`, and any other bag to `some`.
This is what `<<` and `>>` (binary minimum and maximum) fold through. -/
def foldBagOpt (op : α → α → α) [Std.Commutative op] [Std.Associative op]
    (m : Multiset α) : Option α :=
  foldBag (optionLift op) none (m.map some)

@[simp] theorem foldBagOpt_zero : foldBagOpt op (0 : Multiset α) = none := rfl

@[simp] theorem foldBagOpt_singleton (a : α) : foldBagOpt op {a} = some a := by
  simp [foldBagOpt]

/-- The shard lemma for the completed fold, with no side condition: `none` is
an identity for `optionLift op` by construction, so the hypothesis
`foldBag_add` needs is discharged here rather than assumed. -/
theorem foldBagOpt_add (m₁ m₂ : Multiset α) :
    foldBagOpt op (m₁ + m₂) = optionLift op (foldBagOpt op m₁) (foldBagOpt op m₂) := by
  unfold foldBagOpt
  rw [Multiset.map_add]
  exact foldBag_add (optionLift op) none (optionLift_none_left op) _ _

/-- **The presence lemma.**  A non-empty bag folds to a present value.  This is
what licenses the *total* surface type of `min` and `max` over a non-empty bag
(ADR 0029 Decision 4): the `Option` is an artefact of the empty case, so it
never surfaces where the checker has proved the fiber present. -/
theorem foldBagOpt_isSome_of_ne_zero {m : Multiset α} (hm : m ≠ 0) :
    (foldBagOpt op m).isSome := by
  induction m using Multiset.induction_on with
  | empty => exact absurd rfl hm
  | cons a s ih =>
    rcases eq_or_ne s 0 with hs | hs
    · subst hs; simp
    · rw [← Multiset.singleton_add, foldBagOpt_add]
      rcases Option.isSome_iff_exists.mp (ih hs) with ⟨v, hv⟩
      simp [hv]

/-- Dually, the empty bag is the *only* one that folds to `none`, so a missing
result is exactly an absent fiber and never a data-dependent surprise. -/
theorem foldBagOpt_eq_none_iff {m : Multiset α} :
    foldBagOpt op m = none ↔ m = 0 := by
  refine ⟨fun h => ?_, fun h => by subst h; simp⟩
  by_contra hm
  exact Option.isSome_iff_ne_none.mp (foldBagOpt_isSome_of_ne_zero op hm) h

/-! ## The bridge to the algebra: a monoid fold is a safe fiber action

The surface's reducing `map_bags` is a `fiberMap` at a fold, so it inherits
split-invariance and the rectangle facts from
`Mensura.Completeness.FiberMap` instead of re-proving them.  Everything in
this section is about *placing* the fold in the algebra; the fold's own
content is above. -/

/-- The fiber action of a monoid fold: read each row with `f`, fold the results
with `op`, and write the answer into the single output row via `out`.  An
absent fiber stays absent, which is `foldFiber_strict`. -/
def foldFiber (op : α → α → α) [Std.Commutative op] [Std.Associative op]
    (e : α) (f : K → Row H σ → α) (out : K → α → Row H σ)
    (k : K) (m : Multiset (Row H σ)) : Multiset (Row H σ) :=
  if m.card = 0 then 0 else {out k (foldBag op e (m.map (f k)))}

/-- A monoid fold never fabricates a row for an absent key. -/
theorem foldFiber_strict (e : α) (f : K → Row H σ → α) (out : K → α → Row H σ) :
    Strict (foldFiber op e f out) := by
  intro k
  simp [foldFiber]

/-- **A monoid fold is an `aggregate`**, hence a `fiberMap`: the parameterized
fold is the well-behaved subclass of the algebra's whole-bag operation, not a
new operation.  The analogue of `aggregate_eq_fiberMap` that ADR 0029 Stage 1
demands. -/
theorem foldFiber_eq_aggregate (e : α) (f : K → Row H σ → α)
    (out : K → α → Row H σ) :
    fiberMap (foldFiber op e f out) =
      aggregate (fun k m => out k (foldBag op e (m.map (f k)))) := by
  apply funext
  intro T
  apply Table.ext_rows
  intro k
  simp [fiberMap, foldFiber, aggregate, Multiset.card_eq_zero]

/-- Hence a monoid fold is `SplitSafe`: it composes into pipelines that stay
split-invariant.  A split routes a key's *whole* bag to one side
(`Mensura.split`), so the bag a fold sees is never torn, which is why the
reducing `map_bags` is Tier A (ADR 0023). -/
theorem foldFiber_splitSafe (e : α) (f : K → Row H σ → α)
    (out : K → α → Row H σ) :
    SplitSafe (fiberMap (foldFiber op e f out)) :=
  fiberMap_splitSafe (foldFiber_strict op e f out)

/-- A monoid fold preserves the rectangle: a present fiber folds to exactly one
row, so no fiber loses its presence (ADR 0020 section 2). -/
theorem foldFiber_exhaustive {N : Type _} (e : α) (f : K × N → Row H σ → α)
    (out : K × N → α → Row H σ) {T : Table (K × N) H σ} (hT : Exhaustive T) :
    Exhaustive (fiberMap (foldFiber op e f out) T) :=
  fiberMap_exhaustive (foldFiber_strict op e f out)
    (fun _ _ hm => by simp [foldFiber, hm]) hT

end Mensura
