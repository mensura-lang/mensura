import Mensura.SplitSafety
import Mensura.Completeness.FiberMap

/-!
Binary joins: both arguments are tables, and either may be split.

`lookup` and `lookupTotal` are split-safe with the right table *fixed*
(`lookup_splitSafe`, `lookupTotal_splitSafe`).  This module lets the right table
vary too.  Here the binary forms are `fun T R => lookup key R T` and
`fun T R => lookupTotal key R T`.

The four-term statement (`FourTerm`) puts the cross terms `f T₀ R₁` on the right
side, which is exactly the leakage the algebra rules out; the statement that
matches the semantics of a split is the *aligned diagonal law* (`AlignedDiagonal`),
where a split of the universe routes each entity, with its rows in every table, to
one side.

Results:

- `lookupTotal_bihom`: the inner join is bilinear, hence satisfies the four-term
  statement unconditionally.
- `lookup_not_splitInvariant_right`, `lookup_not_fourTerm`: the left join is
  invariant in the left argument only.
- `lookupTotal_alignedDiagonal`, `lookup_alignedDiagonal`: under alignment both
  joins satisfy the diagonal law; `lookupTotal_not_alignedDiagonal`,
  `lookup_not_alignedDiagonal` show alignment is necessary.
- `alignedDiagonal_biKeyLocal_iff_biFiberMap`: binary safe completeness.

Ported from the Lean development of the paper "Split Safety for Tabular Data
Handling: An Algebra and a Completeness Theorem" (Zeferino, Verri, Quiles), with
its `innerJoin key T R` / `leftJoin key T R` renamed to this development's
`lookupTotal key R T` / `lookup key R T`.
-/

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {H' : Type _} {σ' : H' → Type}
variable {U G : Type _} {τ : G → Type}

/-! ## The four-term statement -/

/-- The natural statement: on disjoint halves in both arguments, the join of the
unions is the union of the four joins of the halves. -/
def FourTerm {K' : Type _} (f : Table K H σ → Table U G τ → Table K' H' σ') : Prop :=
  ∀ T₀ T₁ R₀ R₁, Disjoint T₀ T₁ → Disjoint R₀ R₁ →
    f (union T₀ T₁) (union R₀ R₁)
      = union (union (f T₀ R₀) (f T₀ R₁)) (union (f T₁ R₀) (f T₁ R₁))

/-- A bihomomorphism satisfies the four-term statement with no disjointness. -/
def BiHom {K' : Type _} (f : Table K H σ → Table U G τ → Table K' H' σ') : Prop :=
  ∀ T₀ T₁ R₀ R₁,
    f (union T₀ T₁) (union R₀ R₁)
      = union (union (f T₀ R₀) (f T₀ R₁)) (union (f T₁ R₀) (f T₁ R₁))

theorem BiHom.fourTerm {K' : Type _} {f : Table K H σ → Table U G τ → Table K' H' σ'}
    (h : BiHom f) : FourTerm f :=
  fun T₀ T₁ R₀ R₁ _ _ => h T₀ T₁ R₀ R₁

/-- The inner join is bilinear: the per-key cross product distributes over the
multiset sum in each argument. -/
theorem lookupTotal_bihom (key : K → U) :
    BiHom (fun (T : Table K H σ) (R : Table U G τ) => lookupTotal key R T) := by
  intro T₀ T₁ R₀ R₁
  apply Table.ext_rows
  intro k
  simp only [lookupTotal, flatMap, union, Multiset.add_bind,
    Multiset.map_add, Multiset.bind_add]
  abel

/-- `lookupTotal` is a union homomorphism in the right argument. -/
theorem lookupTotal_unionHom_right (key : K → U) (T : Table K H σ) :
    UnionHom (fun R : Table U G τ => lookupTotal key R T) := by
  intro R₀ R₁
  apply Table.ext_rows
  intro k
  simp only [lookupTotal, flatMap, union, Multiset.map_add, Multiset.bind_add]

section LookupCounterexample

/-- The unit row. -/
private def unitRow : Row Unit (fun _ : Unit => Unit) := fun _ => some ()

private def oneRow : Table Unit Unit (fun _ : Unit => Unit) := ⟨fun _ => {unitRow}⟩

private def zeroTab : Table Unit Unit (fun _ : Unit => Unit) := ⟨fun _ => 0⟩

/-- `lookup` is not split invariant in its right argument: a left row matched
in one half of the reference and unmatched in the other receives both a value and
a missing marker. -/
theorem lookup_not_splitInvariant_right :
    ∃ (T : Table Unit Unit (fun _ : Unit => Unit)),
      ¬ SplitInvariant (fun R : Table Unit Unit (fun _ : Unit => Unit) =>
        lookup (id : Unit → Unit) R T) := by
  refine ⟨oneRow, fun h => ?_⟩
  have hd : Disjoint oneRow zeroTab := fun _ => Or.inr rfl
  have e := h oneRow zeroTab hd
  have hu : union oneRow zeroTab = oneRow := by
    apply Table.ext_rows; intro k; simp [union, oneRow, zeroTab]
  rw [hu] at e
  have := congrArg (fun X => (X.rows ()).card) e
  simp [lookup, flatMap, union, oneRow, zeroTab] at this

/-- `lookup` fails the four-term statement. -/
theorem lookup_not_fourTerm :
    ¬ FourTerm (fun (T : Table Unit Unit (fun _ : Unit => Unit))
      (R : Table Unit Unit (fun _ : Unit => Unit)) => lookup (id : Unit → Unit) R T) := by
  intro h
  have hd₂ : Disjoint oneRow zeroTab := fun _ => Or.inr rfl
  have e := h oneRow zeroTab oneRow zeroTab hd₂ hd₂
  have hu : union oneRow zeroTab = oneRow := by
    apply Table.ext_rows; intro k; simp [union, oneRow, zeroTab]
  rw [hu] at e
  have := congrArg (fun X => (X.rows ()).card) e
  simp [lookup, flatMap, union, oneRow, zeroTab] at this

end LookupCounterexample

/-! ## Aligned splits and the diagonal law -/

/-- Indicators on the left and right keys are aligned when the right indicator is
the left one composed with `key`: an entity goes to the same side in both tables. -/
def Aligned (key : K → U) (s : K → Bool) (sR : U → Bool) : Prop :=
  ∀ k, s k = sR (key k)

/-- The aligned diagonal law: for every aligned split, the join of the whole is the
union of the joins of the matching halves. -/
def AlignedDiagonal {K' : Type _} (key : K → U)
    (f : Table K H σ → Table U G τ → Table K' H' σ') : Prop :=
  ∀ (s : K → Bool) (sR : U → Bool), Aligned key s sR → ∀ T R,
    f T R = union (f (split s T).1 (split sR R).1) (f (split s T).2 (split sR R).2)

/-- Binary fibre maps: the output bag at `k` depends on the left bag at `k` and the
right bag at `key k`. -/
def biFiberMap (key : K → U)
    (Φ : K → Multiset (Row H σ) → Multiset (Row G τ) → Multiset (Row H' σ'))
    (T : Table K H σ) (R : Table U G τ) : Table K H' σ' :=
  ⟨fun k => Φ k (T.rows k) (R.rows (key k))⟩

/-- Joint strictness: no rows are fabricated when both fibres are empty. -/
def JointlyStrict (Φ : K → Multiset (Row H σ) → Multiset (Row G τ) → Multiset (Row H' σ')) :
    Prop :=
  ∀ k, Φ k 0 0 = 0

/-- Binary key-locality. -/
def BiKeyLocal (key : K → U) (f : Table K H σ → Table U G τ → Table K H' σ') : Prop :=
  ∀ (T T' : Table K H σ) (R R' : Table U G τ) (k : K),
    T.rows k = T'.rows k → R.rows (key k) = R'.rows (key k) →
      (f T R).rows k = (f T' R').rows k

theorem biFiberMap_biKeyLocal (key : K → U)
    (Φ : K → Multiset (Row H σ) → Multiset (Row G τ) → Multiset (Row H' σ')) :
    BiKeyLocal key (biFiberMap key Φ) := by
  intro T T' R R' k hT hR
  simp only [biFiberMap, hT, hR]

/-- Splitting the zero table gives zero tables. -/
private theorem split_zero (s : K → Bool) :
    split s (⟨fun _ => 0⟩ : Table K H σ) = (⟨fun _ => 0⟩, ⟨fun _ => 0⟩) := by
  simp only [split, Prod.mk.injEq]
  constructor <;> (apply Table.ext_rows; intro k; cases s k <;> simp)

/-- The diagonal law forces the join of empty tables to be empty. -/
theorem AlignedDiagonal.map_zero {key : K → U}
    {f : Table K H σ → Table U G τ → Table K H' σ'} (hf : AlignedDiagonal key f) (k : K) :
    (f ⟨fun _ => 0⟩ ⟨fun _ => 0⟩).rows k = 0 := by
  have e := hf (fun _ => false) (fun _ => false) (fun _ => rfl) ⟨fun _ => 0⟩ ⟨fun _ => 0⟩
  rw [split_zero, split_zero] at e
  have hrow := congrArg (fun X => X.rows k) e
  simp only [union] at hrow
  exact multiset_self_add (by simpa using hrow)

/-- A jointly strict binary fibre map satisfies the aligned diagonal law. -/
theorem biFiberMap_alignedDiagonal (key : K → U)
    {Φ : K → Multiset (Row H σ) → Multiset (Row G τ) → Multiset (Row H' σ')}
    (hΦ : JointlyStrict Φ) : AlignedDiagonal key (biFiberMap key Φ) := by
  intro s sR hal T R
  apply Table.ext_rows
  intro k
  have := hal k
  simp only [biFiberMap, union, split]
  cases hs : s k <;> rw [hs] at this <;> rw [← this] <;> simp [hΦ k]

/-- Under alignment the two halves of the result are disjoint, so the binary
operation is split safe. -/
theorem biFiberMap_aligned_disjoint (key : K → U)
    {Φ : K → Multiset (Row H σ) → Multiset (Row G τ) → Multiset (Row H' σ')}
    (hΦ : JointlyStrict Φ) {s : K → Bool} {sR : U → Bool} (hal : Aligned key s sR)
    (T : Table K H σ) (R : Table U G τ) :
    Disjoint (biFiberMap key Φ (split s T).1 (split sR R).1)
      (biFiberMap key Φ (split s T).2 (split sR R).2) := by
  intro k
  have := hal k
  simp only [biFiberMap, split]
  cases hs : s k <;> rw [hs] at this <;> rw [← this] <;> simp [hΦ k]

/-- **Binary safe completeness.**  A binary operation is bi-key-local and satisfies
the aligned diagonal law iff it is a jointly strict binary fibre map. -/
theorem alignedDiagonal_biKeyLocal_iff_biFiberMap [DecidableEq K] [DecidableEq U]
    {key : K → U} {f : Table K H σ → Table U G τ → Table K H' σ'} :
    (AlignedDiagonal key f ∧ BiKeyLocal key f) ↔
      ∃ Φ, JointlyStrict Φ ∧ f = biFiberMap key Φ := by
  constructor
  · rintro ⟨hAD, hKL⟩
    refine ⟨fun k m n => (f (pointTable k m) (pointTable (key k) n)).rows k, ?_, ?_⟩
    · intro k
      have hT : pointTable k (0 : Multiset (Row H σ)) = ⟨fun _ => 0⟩ := by
        apply Table.ext_rows; intro k'; simp [pointTable]
      have hR : pointTable (key k) (0 : Multiset (Row G τ)) = ⟨fun _ => 0⟩ := by
        apply Table.ext_rows; intro k'; simp [pointTable]
      simp only [hT, hR]
      exact hAD.map_zero k
    · funext T R
      apply Table.ext_rows
      intro k
      exact hKL T (pointTable k (T.rows k)) R (pointTable (key k) (R.rows (key k))) k
        (by simp) (by simp)
  · rintro ⟨Φ, hΦ, rfl⟩
    exact ⟨biFiberMap_alignedDiagonal key hΦ, biFiberMap_biKeyLocal key Φ⟩

/-! ## The joins as binary fibre maps -/

theorem lookupTotal_eq_biFiberMap (key : K → U) :
    (fun (T : Table K H σ) (R : Table U G τ) => lookupTotal key R T)
      = biFiberMap key (fun _ m n => m.bind (fun f => n.map (fun r => f.elim r))) := rfl

theorem lookup_eq_biFiberMap (key : K → U) :
    (fun (T : Table K H σ) (R : Table U G τ) => lookup key R T)
      = biFiberMap key (fun _ m n => m.bind (fun f =>
          if n.card = 0 then {f.elim (fun _ => none)} else n.map (fun r => f.elim r))) := rfl

theorem lookupTotal_alignedDiagonal (key : K → U) :
    AlignedDiagonal key (fun (T : Table K H σ) (R : Table U G τ) => lookupTotal key R T) := by
  rw [lookupTotal_eq_biFiberMap]
  exact biFiberMap_alignedDiagonal key (fun k => by simp)

theorem lookup_alignedDiagonal (key : K → U) :
    AlignedDiagonal key (fun (T : Table K H σ) (R : Table U G τ) => lookup key R T) := by
  rw [lookup_eq_biFiberMap]
  exact biFiberMap_alignedDiagonal key (fun k => by simp)

/-! ## Alignment is necessary -/

section Unaligned

private def uRow : Row Unit (fun _ : Unit => Unit) := fun _ => some ()

private def uOne : Table Unit Unit (fun _ : Unit => Unit) := ⟨fun _ => {uRow}⟩

/-- Without alignment (left entity to side 0, reference row to side 1)
`lookupTotal` fails the diagonal law. -/
theorem lookupTotal_not_alignedDiagonal :
    ∃ (s : Unit → Bool) (sR : Unit → Bool), ¬ Aligned (id : Unit → Unit) s sR ∧
      ∃ (T R : Table Unit Unit (fun _ : Unit => Unit)), lookupTotal id R T
        ≠ union (lookupTotal id (split sR R).1 (split s T).1)
                (lookupTotal id (split sR R).2 (split s T).2) := by
  refine ⟨fun _ => false, fun _ => true, fun h => by simpa using h (), uOne, uOne, fun e => ?_⟩
  have := congrArg (fun X => (X.rows ()).card) e
  simp [lookupTotal, flatMap, union, split, uOne] at this

/-- Likewise for `lookup`. -/
theorem lookup_not_alignedDiagonal :
    ∃ (s : Unit → Bool) (sR : Unit → Bool), ¬ Aligned (id : Unit → Unit) s sR ∧
      ∃ (T R : Table Unit Unit (fun _ : Unit => Unit)), lookup id R T
        ≠ union (lookup id (split sR R).1 (split s T).1)
                (lookup id (split sR R).2 (split s T).2) := by
  refine ⟨fun _ => false, fun _ => true, fun h => by simpa using h (), uOne, uOne, fun e => ?_⟩
  have := congrArg (fun X => X.rows ()) e
  simp [lookup, flatMap, union, split, uOne] at this
  have := congrFun this (Sum.inr ())
  simp [Row.elim, uRow] at this

end Unaligned

end Mensura
