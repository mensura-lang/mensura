/-
Mensura: a formalization of the data-handling algebra (ADR 0008).

This is the root module.  It re-exports the formalization so that downstream
files (and `lake build`) can `import Mensura`.

The mathematical source is Chapter 5 ("Data handling") of the book "Data
Science Project: An Inductive Learning Approach".  Each definition box in
that chapter maps to a definition here, with the `\label` recorded in a doc
comment.

Module flatMap (each file imports only what it builds on):

  Core/Defs          tables, rows, split/union, the safety properties,
                     minimality
  Core/Ops           flatMap, joins, aggregate, promote, demote, tagged pair
  SplitSafety        per-operation safety and the composition payoff
  Laws               equational laws: the rewrite-rule seeds (ADR 0008)
  Reshape            unpivot / pivot / unpivotDrop and the inverse pair
                     (ADR 0020)
  Rectangle          Exhaustive / Total and the rectangle propagation
                     (ADR 0020)
  Fold               the monoid-parameterized bag fold: the well-behaved
                     subclass of `aggregate` a closed combiner table can
                     implement, with the shard and presence lemmas
                     (ADR 0029 Stage 1, ADR 0031)
  Arranged           the arranged structure: a fiber's content lifted to a
                     list, the inclusive and exclusive scans, their
                     coherence with the fold, and the Tier 1 determinism
                     hypothesis (ADR 0029 Stage 2, ADR 0031)
  Completeness/
    FiberMap         the key-preserving safe-completeness characterization
    Reindex          its key-changing generalization; gatherMap and demote
    Verbs            the derived verb catalogue (expressive completeness)
    PivotAttr        the split-safe attribute pivot and its reversibility
    CompleteOver     population-relative completeness, its propagation
                     through demote/demote, and the fiber-level
                     trivial discharge at card <= 1 (ADR 0023)
    BinaryJoins      joins with both arguments split: the aligned
                     diagonal law and binary safe completeness
  Leak               joining back a whole-table statistic (and mean
                     imputation) is not split-invariant
  SelfLookup         a lookup against a reference built from its own
                     input is leak-free exactly on splits aligned with the
                     lookup key
  Units/
    Dimension        physical dimensions as the free abelian group over the
                     seven SI base dimensions, with decidable equality
                     (ADR 0026); standalone, independent of the table algebra
    Torsor           temporal point domains as additive torsors over their
                     difference groups: the abstract laws, the Instant
                     instantiation on the exact millisecond grid, and the
                     order-compatibility lemma ADR 0037's window gates rest
                     on (ADR 0036)
  Expr/
    Missing          scalar operators lifted over the missing axis and the
                     `??` discharge: absorption, agreement with the unlifted
                     operator, distribution over composition, and the chain
                     laws (ADR 0039); abstract in the scalar operations
  Window/
    Defs             streaming windows as a derived form (flatMap then
                     promote): split-safety by composition, the grading
                     extension, `closedWindow_stable`, and the concrete
                     stride-grid placement at `Instant` (ADR 0037)
    Dense            rectangularization over the window grid: the fill
                     agrees with the ideal completed reduction, is
                     idempotent, keeps a closed slot's row final, and makes
                     the coarse fiber a `demote w` produces complete
                     (ADR 0038)

The statement inventory and its dependency graph live in the blueprint
(`formal/blueprint/`).
-/

module

public import Mensura.Core.Defs
public import Mensura.Core.Ops
public import Mensura.SplitSafety
public import Mensura.Laws
public import Mensura.Reshape
public import Mensura.Rectangle
public import Mensura.Fold
public import Mensura.Arranged
public import Mensura.Completeness.FiberMap
public import Mensura.Completeness.Reindex
public import Mensura.Completeness.Verbs
public import Mensura.Completeness.PivotAttr
public import Mensura.Completeness.CompleteOver
public import Mensura.Completeness.BinaryJoins
public import Mensura.Leak
public import Mensura.SelfLookup
public import Mensura.Units.Dimension
public import Mensura.Units.Torsor
public import Mensura.Expr.Missing
public import Mensura.Window.Defs
public import Mensura.Window.Dense
