/-
Mensura: a formalization of the data-handling algebra.

This is the root module.  It re-exports the formalization so that downstream
files (and `lake build`) can `import Mensura`.

The mathematical source is Chapter 5 ("Data handling") of the book "Data Science Project:
An Inductive Learning Approach".  Each definition box in that
chapter maps to a definition here, with the `\label` recorded in a doc comment.
-/

import Mensura.Table
