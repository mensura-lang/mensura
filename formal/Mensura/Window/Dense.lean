/-
Rectangularization over the window grid (ADR 0038,
`docs/decisions/0038-rectangularization-over-the-window-grid.md`: the
theorems its decision 7 owes for the checker-visible rules of decisions 2
and 4).

`window` replicates rows, so a window containing no row is represented by
absence (ADR 0037 decision 1).  That is right for the primitive and wrong
for the question "how many intervals did this entity report nothing",
which is unanswerable while the intervals in question are not rows.
`dense` completes the grid *after* the reduction: it keeps every reduced
row and adds one filled row per grid slot that produced none.

## Why after the reduction, and not before

The obvious design materializes the empty fibers and lets the reduction
handle them.  ADR 0038 decision 1 rejects it, and `Mensura.foldFiber`'s
`if m.card = 0 then 0` guard is why: that guard is ADR 0029 decision 4 in
mechanized form, so a reducing lambda never faces the empty bag, which is
what makes a seedless fold total and what keeps the accumulator's `Option`
out of every surface type.  Filling after the reduction preserves the
guard and still computes the ideal grid's answer, which is the content of
`dense_fiberMap_foldFiber`: the cheap order of operations agrees with the
expensive one.

## What the four results are for

* `dense_fiberMap_foldFiber` and `dense_fiberMap_foldFiberOpt` (decision 7
  item 1): filling from the combiner's identity computes the reduction
  over the completed grid, and a combiner with no identity fills absent,
  which is the same statement in `Option`.  Together they are the backing
  for decision 2's typing rule, where an identity-carrying column stays
  total and every other column goes optional.
* `dense_idem` (item 2): `dense` twice is `dense` once, at the same
  population and bounds.
* `dense_stable_of_closed` (item 3): rerunning after further ingestion
  adds rows for newly closed slots and changes no row already emitted.
  This is ADR 0037's `closedWindow_stable` carried across the fill, and it
  is what makes a rectangularized view safe to serve incrementally.
* `dense_eq_rectangle` and `demote_fiberCompleteWrt_dense` (item 4): after
  the fill the table *is* the ideal rectangle, so the coarse fiber a
  `demote w` produces is complete with respect to it.  This is the theorem
  behind decision 4's single exception to ADR 0035's clearing rule, and it
  is available only because the grid is decidable: stride and origin are
  compile-time constants (ADR 0036 decision 5) and closedness bounds the
  grid above.

## The grid enters as a given finite set of slots

Decision 3 fixes where the grid comes from: stride and origin from the
`window` declaration, the upper bound from `closed`, and the population
and per-entity lower bound from the stage's own arguments, never inferred.
None of that is a hypothesis any theorem here needs, so the slots enter as
`grid : K -> Finset D` over an abstract slot type, the way `Mensura.window`
takes an abstract placement function and leaves the concrete stride grid to
`Units.Instant.windowStarts`.  The abstraction is also what keeps `demote`
available in item 4, which needs a `Fintype` of slots where `Instant` is
not one.
-/

import Mensura.Window.Defs
import Mensura.Fold
import Mensura.Completeness.CompleteOver

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {D : Type _} [DecidableEq D]
variable {α : Type _}

/-! ## The stage -/

/-- **Rectangularization.**  Keep every present fiber, and at a grid slot
that produced no row emit the single filled row `fill k d`.  A slot outside
the grid is left exactly as it was, which is what confines the stage to the
grid the program wrote down (decision 5).

The presence test is on the cardinality rather than on the multiset, so the
definition is decidable for the same reason `Mensura.foldFiber` is. -/
def dense (grid : K → Finset D) (fill : K → D → Row H σ)
    (T : Table (K × D) H σ) : Table (K × D) H σ :=
  ⟨fun p => if (T.rows p).card = 0 ∧ p.2 ∈ grid p.1 then {fill p.1 p.2} else T.rows p⟩

/-- The fiber of `dense`, unfolded at a key.  Every proof below starts here. -/
theorem dense_rows (grid : K → Finset D) (fill : K → D → Row H σ)
    (T : Table (K × D) H σ) (k : K) (d : D) :
    (dense grid fill T).rows (k, d) =
      if (T.rows (k, d)).card = 0 ∧ d ∈ grid k then {fill k d} else T.rows (k, d) :=
  rfl

/-- **Every grid slot is present after the fill**, whatever the data did.
This is the fact the checker's establishment rests on: absence inside the
grid is no longer representable, so the coarse fiber a later `demote`
produces has no holes. -/
theorem dense_present_of_mem_grid {grid : K → Finset D} {fill : K → D → Row H σ}
    {T : Table (K × D) H σ} {k : K} {d : D} (hd : d ∈ grid k) :
    (dense grid fill T).Present (k, d) := by
  rw [Table.Present, dense_rows]
  by_cases hcard : (T.rows (k, d)).card = 0
  · simp [hcard, hd]
  · simpa [hcard] using fun h => hcard (by simp [h])

/-- A slot the data reached keeps its rows: the fill never overwrites. -/
theorem dense_rows_of_present {grid : K → Finset D} {fill : K → D → Row H σ}
    {T : Table (K × D) H σ} {k : K} {d : D} (h : T.Present (k, d)) :
    (dense grid fill T).rows (k, d) = T.rows (k, d) := by
  rw [dense_rows, if_neg]
  exact fun hc => h (Multiset.card_eq_zero.mp hc.1)

/-! ## Item 1: agreement with the ideal, per column

The ideal is the expensive order of operations: complete the grid first,
then reduce every slot, including the empty ones.  `foldBag` on the empty
bag is the identity by `foldBag_zero`, so the ideal's answer at an empty
slot is the identity, which is exactly what `dense` writes there.  The two
orders therefore agree, and decision 1's inversion is licensed. -/

/-- The ideal completed reduction: one reduced row per grid slot and per
slot the data reached, folding whatever bag that slot holds, empty or not.
This is the guard-free counterpart of `Mensura.foldFiber`, and it exists
only inside this statement: no surface operation faces an empty bag. -/
def idealFold (grid : K → Finset D) (op : α → α → α) [Std.Commutative op]
    [Std.Associative op] (e : α) (f : K × D → Row H σ → α)
    (out : K × D → α → Row H σ) (T : Table (K × D) H σ) : Table (K × D) H σ :=
  ⟨fun p =>
    if p.2 ∈ grid p.1 ∨ (T.rows p).card ≠ 0 then
      {out p (foldBag op e ((T.rows p).map (f p)))}
    else 0⟩

/-- **Filling after the reduction computes the reduction over the completed
grid** (decision 7 item 1).  For a column that is a single fold at a
combiner carrying an identity, `dense` at the identity fill agrees with the
ideal everywhere, so the cheap order of operations is not an approximation
of the expensive one but equal to it.

The empty slot is the whole content: there `foldBag_zero` says the ideal
folds to `e`, and the fill writes `out (k, d) e`.  That is also the
statement of ADR 0038 decision 1's conclusion, that a filled row is a
reduced row rather than a fabricated observation: `count` reports zero
because zero rows were reduced, not because a placeholder was invented. -/
theorem dense_fiberMap_foldFiber (grid : K → Finset D) (op : α → α → α)
    [Std.Commutative op] [Std.Associative op] (e : α) (f : K × D → Row H σ → α)
    (out : K × D → α → Row H σ) (T : Table (K × D) H σ) :
    dense grid (fun k d => out (k, d) e) (fiberMap (foldFiber op e f out) T) =
      idealFold grid op e f out T := by
  apply Table.ext_rows
  rintro ⟨k, d⟩
  simp only [dense_rows, idealFold, fiberMap, foldFiber]
  by_cases hcard : (T.rows (k, d)).card = 0
  · have hzero : T.rows (k, d) = 0 := Multiset.card_eq_zero.mp hcard
    by_cases hd : d ∈ grid k <;> simp [hd, hzero]
  · simp [hcard]

/-! ### The same statement in `Option`

A combiner with no identity has no true answer for the empty slot, so the
column is absent there and optional view-wide (ADR 0010, decision 2).  The
fold goes through `Mensura.foldBagOpt`, whose empty case is `none` by
`foldBagOpt_zero`, and the theorem is the previous one with `none` as the
fill. -/

/-- The fiber action of an identity-free fold: the `Option`-completed
counterpart of `Mensura.foldFiber`. -/
def foldFiberOpt (op : α → α → α) [Std.Commutative op] [Std.Associative op]
    (f : K → Row H σ → α) (out : K → Option α → Row H σ)
    (k : K) (m : Multiset (Row H σ)) : Multiset (Row H σ) :=
  if m.card = 0 then 0 else {out k (foldBagOpt op (m.map (f k)))}

/-- The ideal completed reduction for an identity-free combiner. -/
def idealFoldOpt (grid : K → Finset D) (op : α → α → α) [Std.Commutative op]
    [Std.Associative op] (f : K × D → Row H σ → α)
    (out : K × D → Option α → Row H σ) (T : Table (K × D) H σ) :
    Table (K × D) H σ :=
  ⟨fun p =>
    if p.2 ∈ grid p.1 ∨ (T.rows p).card ≠ 0 then
      {out p (foldBagOpt op ((T.rows p).map (f p)))}
    else 0⟩

/-- **The identity-free case** (decision 7 item 1, second half).  With no
identity to write, the fill is `none`, and it agrees with the ideal for the
same reason: `foldBagOpt_zero` says the empty slot's honest answer is
absence.  There is no minimum of nothing, so a sentinel would be a lie and
`Option` is the truth. -/
theorem dense_fiberMap_foldFiberOpt (grid : K → Finset D) (op : α → α → α)
    [Std.Commutative op] [Std.Associative op] (f : K × D → Row H σ → α)
    (out : K × D → Option α → Row H σ) (T : Table (K × D) H σ) :
    dense grid (fun k d => out (k, d) none) (fiberMap (foldFiberOpt op f out) T) =
      idealFoldOpt grid op f out T := by
  apply Table.ext_rows
  rintro ⟨k, d⟩
  simp only [dense_rows, idealFoldOpt, fiberMap, foldFiberOpt]
  by_cases hcard : (T.rows (k, d)).card = 0
  · have hzero : T.rows (k, d) = 0 := Multiset.card_eq_zero.mp hcard
    by_cases hd : d ∈ grid k <;> simp [hd, hzero]
  · simp [hcard]

/-! ## Item 2: idempotence -/

/-- **`dense` twice is `dense` once** (decision 7 item 2), at the same
population and bounds.  The second pass finds every grid slot present, by
`dense_present_of_mem_grid`, so it has nothing left to fill. -/
theorem dense_idem (grid : K → Finset D) (fill : K → D → Row H σ)
    (T : Table (K × D) H σ) :
    dense grid fill (dense grid fill T) = dense grid fill T := by
  apply Table.ext_rows
  rintro ⟨k, d⟩
  by_cases hd : d ∈ grid k
  · exact dense_rows_of_present (dense_present_of_mem_grid hd)
  · rw [dense_rows, dense_rows, if_neg (by simp [hd]), if_neg (by simp [hd])]

/-! ## Item 3: stability under append

A filled row is determined by the grid slot and the bag the slot holds, so
a slot whose bag is stable has a stable row, whether the row was reduced or
filled.  ADR 0037's `closedWindow_stable` supplies the stable bag for every
closed window, and the grid only ever grows as the watermark advances, so a
rerun after further ingestion adds rows for newly closed slots and changes
none of the rows it already emitted.  That is the finality invariant the
refresh slice inherits, now across the fill. -/

/-- Stability transports through a fiber-local reduction and then through
the fill: at a slot both grids contain, an unchanged bag gives an unchanged
row.  The reduction enters only as a `fiberMap`, which is any per-fiber
action, so this covers every reducing `map_bags` rather than one fold. -/
theorem dense_fiberMap_rows_congr {grid grid' : K → Finset D}
    {fill : K → D → Row H σ} {Φ : K × D → Multiset (Row H σ) → Multiset (Row H σ)}
    {T T' : Table (K × D) H σ} {k : K} {d : D}
    (hd : d ∈ grid k) (hd' : d ∈ grid' k)
    (hfib : T'.rows (k, d) = T.rows (k, d)) :
    (dense grid' fill (fiberMap Φ T')).rows (k, d) =
      (dense grid fill (fiberMap Φ T)).rows (k, d) := by
  rw [dense_rows, dense_rows]
  simp only [fiberMap, hfib, hd, hd', and_true]

end Mensura

namespace Mensura.Units.Instant

open Mensura Units

variable {K H : Type _} {σ : H → Type}

/-- **A closed slot's row is final, fill included** (decision 7 item 3).
Append rows the `lateness` contract still admits, rerun, and a slot closed
before the append carries the row it carried, whether the data produced it
or the grid fill did.  `closedWindow_stable` gives the unchanged bag and
`dense_fiberMap_rows_congr` carries it across the reduction and the fill;
`grid` and `grid'` are the grids before and after the append, and the only
thing asked of them is that the slot in question belongs to both, which is
what "the grid only grows" comes to at one slot.

Newly closed slots are exactly what a rerun adds, and this says nothing
about them, which is the honest scope: the ADR's guarantee is that a
rectangularized view grows without retracting. -/
theorem dense_stable_of_closed {Γ : Type _} {size stride : Duration}
    (hstride : 0 < stride) (point : K → Row H σ → Instant) {T A : Table K H σ}
    {grain : K → Γ} {watermark : Γ → Instant} {lateness : Duration}
    (hlate : ∀ k, ∀ f ∈ A.rows k, watermark (grain k) ≤ lateness +ᵥ point k f)
    {Φ : K × Instant → Multiset (Row H σ) → Multiset (Row H σ)}
    {fill : K → Instant → Row H σ} {grid grid' : K → Finset Instant}
    {w : Instant} (k : K) (hd : w ∈ grid k) (hd' : w ∈ grid' k)
    (hclosed : (size + lateness) +ᵥ w ≤ watermark (grain k)) :
    (dense grid' fill
        (fiberMap Φ (window (windowStarts size stride) point (union T A)))).rows (k, w) =
      (dense grid fill
        (fiberMap Φ (window (windowStarts size stride) point T))).rows (k, w) :=
  dense_fiberMap_rows_congr hd hd'
    (closedWindow_stable hstride point hlate k hclosed)

end Mensura.Units.Instant

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {D : Type _} [DecidableEq D]

/-! ## Item 4: the fact survives the demote

The motivating query reduces the rectangle at the entity key, one step past
the fill, and a reducing fold demands completeness there (ADR 0023).
`demote w` is a genuine coarsening, which ADR 0035 clears the fact at, and
the clearing is right in general: an absent fine key becomes a gap inside
the coarse bag.  After `dense` there is no absent fine key inside the grid,
so the coarse bag is the whole rectangle and the fact is re-derived from a
checked mechanism rather than claimed. -/

/-- The ideal rectangle the grid describes: exactly one row per slot. -/
def rectangle (grid : K → Finset D) (row : K → D → Row H σ) :
    Table (K × D) H σ :=
  ⟨fun p => if p.2 ∈ grid p.1 then {row p.1 p.2} else 0⟩

/-- **After the fill, the table is the rectangle.**  Three hypotheses, each
a side condition the checker enforces at the stage:

* `hin`: no present slot lies outside the grid.  `closed` ran, so nothing
  is present above the bound, and the stage's lower-bound argument fixes
  the other end (decision 3);
* `hone`: a present slot holds exactly one row, which is what it means for
  `dense` to run after the reduction (decision 1);
* `hfill`: the fill is the slot's true empty answer, which is
  `dense_fiberMap_foldFiber` at an identity-carrying combiner.

Their conjunction is the fact the demote rule needs, and stating it this
way keeps the rule honest: drop any one and the conclusion is false, not
merely unproved. -/
theorem dense_eq_rectangle {grid : K → Finset D} {fill row : K → D → Row H σ}
    {T : Table (K × D) H σ}
    (hin : ∀ k d, T.Present (k, d) → d ∈ grid k)
    (hone : ∀ k d, T.Present (k, d) → T.rows (k, d) = {row k d})
    (hfill : ∀ k d, ¬ T.Present (k, d) → fill k d = row k d) :
    dense grid fill T = rectangle grid row := by
  apply Table.ext_rows
  rintro ⟨k, d⟩
  rw [dense_rows, rectangle]
  by_cases hp : T.Present (k, d)
  · have hcard : (T.rows (k, d)).card ≠ 0 := fun h => hp (Multiset.card_eq_zero.mp h)
    simp [hin k d hp, hone k d hp]
  · have hzero : T.rows (k, d) = 0 := by
      simpa [Table.Present, not_not] using hp
    by_cases hd : d ∈ grid k
    · simp [hzero, hd, hfill k d hp]
    · simp [hzero, hd]

/-- **The coarse bag is complete with respect to the ideal rectangle**
(decision 7 item 4).  This is the one place a genuinely coarsening `demote`
re-derives completeness from a checked fact instead of clearing it
(ADR 0035, ADR 0038 decision 4).  The proof is the previous theorem: the
demoted rectangle and the demoted fill are the same table, so no coarse
fiber can be partial.

Contrast the fiber-gap witness recorded in
`Mensura.Completeness.CompleteOver`: there the coarsening turns an absent
fine key into a hole, and the reason it cannot happen here is that `dense`
left no absent fine key inside the grid
(`dense_present_of_mem_grid`). -/
theorem demote_fiberCompleteWrt_dense [Fintype D] {grid : K → Finset D}
    {fill row : K → D → Row H σ} {T : Table (K × D) H σ}
    (hin : ∀ k d, T.Present (k, d) → d ∈ grid k)
    (hone : ∀ k d, T.Present (k, d) → T.rows (k, d) = {row k d})
    (hfill : ∀ k d, ¬ T.Present (k, d) → fill k d = row k d) :
    FiberCompleteWrt (demote (rectangle grid row)) (demote (dense grid fill T)) := by
  rw [dense_eq_rectangle hin hone hfill]
  exact fun _ _ => rfl

end Mensura
