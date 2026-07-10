/-
Mensura: a formalization of the data-handling algebra (ADR 0008).

This is the root module.  It re-exports the formalization so that downstream
files (and `lake build`) can `import Mensura`.

The mathematical source is Chapter 5 ("Data handling") of the book "Data
Science Project: An Inductive Learning Approach".  Each definition box in
that chapter maps to a definition here, with the `\label` recorded in a doc
comment.

Module map (each file imports only what it builds on):

  Core/Defs          tables, rows, split/bind, the safety properties,
                     minimality
  Core/Ops           map, joins, aggregate, ungroup, project, tagged pair
  SplitSafety        per-operation safety and the composition payoff
  Laws               equational laws: the rewrite-rule seeds (ADR 0008)
  Reshape            unpivot / pivot / unpivotDrop and the inverse pair
                     (ADR 0020)
  Rectangle          Exhaustive / Total and the rectangle propagation
                     (ADR 0020)
  Completeness/
    FiberMap         the key-preserving safe-completeness characterization
    Reindex          its key-changing generalization; gatherMap and project
    Verbs            the derived verb catalogue (expressive completeness)
    PivotAttr        the split-safe attribute pivot and its reversibility
    CompleteOver     population-relative completeness and its propagation
                     through project/shrink_key (ADR 0023)

The statement inventory and its dependency graph live in the blueprint
(`formal/blueprint/`).
-/

import Mensura.Core.Defs
import Mensura.Core.Ops
import Mensura.SplitSafety
import Mensura.Laws
import Mensura.Reshape
import Mensura.Rectangle
import Mensura.Completeness.FiberMap
import Mensura.Completeness.Reindex
import Mensura.Completeness.Verbs
import Mensura.Completeness.PivotAttr
import Mensura.Completeness.CompleteOver
