/-
Verbs: a representative slice of "all useful changes", in the primitives.

This is the tangible form of *expressive completeness* (the reference-model
layer).  "Expresses every transformation" is not a theorem one can state: the
function space `Table → Table` is uncountable and full of non-computable
junk, and a data-handling layer that was Turing complete would lose the
totality and optimizability that make it worth having.  Completeness is
therefore always stated *relative to a reference model*, exactly as Codd's
"relational completeness" is relative to the relational calculus.

The data model here is a `K`-indexed family of multisets of records, i.e. a
*nested (grouped) bag relation*.  The accepted theory of "what a
transformation language on such data can do" is the **Nested Relational
Calculus over the bag monad with an aggregation fold** (bag-NRC + Σ), whose
meta-theory is settled (Buneman-Naqvi-Tannen-Wong; Libkin-Wong for bags +
aggregates; Wong's conservativity theorem).  At flat input/output it is
provably equal to *bag relational algebra with group-by/aggregate*, the
expressive core of SQL, and it subsumes the documented dataframe verb sets
(Wickham's tidyverse; Petersohn's pandas algebra).  bag-NRC + Σ is generated
by a small fixed signature, and each generator is one of our operations or a
short pipeline:

| bag-NRC + Σ generator         | operation                          |
| ----------------------------- | ---------------------------------- |
| empty, singleton, union `⊎`   | empty table, one-row table, `union` |
| functorial flatMap, tuple, π      | `flatMap`                              |
| flatten `μ` (one level)       | `promote`                          |
| nest / group-by               | `demote`                          |
| equality, conditional, filter | `flatMap` with predicate body          |
| additive fold `Σ`             | `aggregate`                        |
| product / join                | `lookup` / `lookupTotal`           |

`pivot` is then derived (nest-by-name then reshape) and so is anti-join /
difference (`antiJoin` below = `lookup` then filter on the unmatched side),
so nothing in the practical verb set is missing.  Each verb below is defined
purely in terms of the core operations, with its split-safety machine-checked
when it holds: the practical dataframe/SQL verb set lives inside the algebra,
and the leakage-safe verbs are exactly the `SplitSafe` ones.
-/

import Mathlib.Data.Multiset.Dedup
import Mensura.SplitSafety
import Mensura.Completeness.FiberMap

namespace Mensura

variable {K H : Type _} {σ : H → Type}
variable {U G : Type _} {τ : G → Type}
variable {V : Type}

/-- `filter` (dplyr `filter`, relational selection): keep the rows satisfying a
predicate.  A `flatMap`, hence split-safe. -/
def filterRows (p : K → Row H σ → Bool) : Table K H σ → Table K H σ :=
  flatMap (fun k f => if p k f then {f} else 0)

theorem filterRows_splitSafe (p : K → Row H σ → Bool) :
    SplitSafe (filterRows (σ := σ) p) := flatMap_splitSafe _

/-- `mutate` (dplyr `mutate`, relational extension): add a derived column.  A
`flatMap`, hence split-safe. -/
def mutateCol (g : K → Row H σ → V) (T : Table K H σ) :
    Table K (H ⊕ Unit) (Sum.elim σ (fun _ => V)) :=
  flatMap (fun k f => {Row.elim f (fun _ => some (g k f))}) T

theorem mutateCol_splitSafe (g : K → Row H σ → V) :
    SplitSafe (mutateCol (σ := σ) g) := by
  unfold mutateCol; exact flatMap_splitSafe _

/-- `anti_join` / set difference against a fixed table: keep the left rows with
*no* match on the right.  This is the operation that recovers difference, the one
classical relational verb not among the positive generators -- and it is here, as
a `flatMap`, hence split-safe.  No separate primitive is needed. -/
def antiJoin (key : K → U) (right : Table U G τ) (T : Table K H σ) : Table K H σ :=
  flatMap (fun k f => if (right.rows (key k)).card = 0 then {f} else 0) T

theorem antiJoin_splitSafe (key : K → U) (right : Table U G τ) :
    SplitSafe (antiJoin (σ := σ) key right) := by
  unfold antiJoin; exact flatMap_splitSafe _

/-- `distinct` (dplyr `distinct`, duplicate elimination): dedup each key's bag.
It is neither a per-row `flatMap` nor a single-row `aggregate`, but it *is* a strict
`fiberMap`, so it is split-safe directly by `fiberMap_splitSafe` -- a concrete
verb showing `fiberMap` earns its keep as the universal safe primitive. -/
def distinct [DecidableEq (Row H σ)] (T : Table K H σ) : Table K H σ :=
  fiberMap (fun _ m => m.dedup) T

theorem distinct_splitSafe [DecidableEq (Row H σ)] :
    SplitSafe (distinct (K := K) (H := H) (σ := σ)) :=
  fiberMap_splitSafe (fun _ => Multiset.dedup_zero)

end Mensura
