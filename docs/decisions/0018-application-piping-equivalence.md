# 0018: Application and piping are one rule

## Status

Accepted.  Sharpens `docs/decisions/0007-single-expression-sublanguage.md`
and `docs/decisions/0009-pipeline-surface.md`; the prose home is the
"Application and grouping" section of `docs/language/06-expressions.md`,
cross-linked from `docs/language/07-pipelines.md`.

## Context

A standing design goal is that the language not become a mess of rules:
function-application syntax and pipeline (`|>`) syntax must share one
rigorous rule so the surface stays learnable.  This was prompted by a
natural question, whether these four forms are all synonyms:

```
f a b
f (a, b)
b |> f a
(a, b) |> f
```

They are not all the same, and the reason is worth recording once so it
is never re-litigated per operation.  ADR 0007 already fixes one
expression sublanguage, and ADR 0009 already states that pipelines are
ordinary curried builtins applied by juxtaposition with "no separate
pipeline grammar".  What neither states outright is the equivalence that
ties application and the pipe together, and the obligation that the
implementation realize it through a single mechanism rather than two.

## Decision

- **`|>` is reversed application.**  `x |> g` is defined to mean `g x`,
  with no semantics of its own.  Application by juxtaposition is the one
  primitive; `|>`, `let`, and tuples only compose it.  This is the rule a
  learner carries, and everything below follows from it.
- **The four-form question has a two-class answer.**  Because the language
  is curried (`06-expressions.md`), applying the single rule collapses the
  four forms into two equivalence classes, not one:

  | juxtaposition | means              | pipe mirror     |
  | ------------- | ------------------ | --------------- |
  | `f a b`       | `(f a) b`          | `b \|> f a`     |
  | `f (a, b)`    | `f` applied to the pair `(a, b)` | `(a, b) \|> f` |

  So `f a b` is the same as `b |> f a`, and `f (a, b)` is the same as
  `(a, b) |> f`, but `f a b` is **not** the same as `f (a, b)`.  The first
  passes two curried arguments; the second passes one argument that is a
  pair.  This is not an extra rule to memorize: it falls out of currying,
  and each pipe form is the exact mirror of a juxtaposition form under
  `x |> g = g x`.
- **One implementation rule, not two.**  The type checker must check the
  right side of a `|>` as the application of that operation to the piped
  input, through the same path that checks a bare application.  A pipe
  stage is an application whose last argument arrives from the left;
  nothing about being in pipe position may change how the operation is
  resolved, how its arguments are counted, or how partial application is
  typed.  Tuples piped or applied (`(a, b) |> f`, `f (a, b)`) are one
  argument, a homogeneous collection, in both positions.
- **Present state is staged, not final.**  Today the checker does not yet
  meet that invariant.  The syntax layer is already consistent: the parser
  builds `ExprKind::App` for juxtaposition and a single `BinOp::Pipe`
  binary operator for `|>`, with no separate pipeline grammar.  The
  type-checking layer is not: `apply_op` in
  `crates/mensura-types/src/pipe_check.rs` flattens the right side of every
  `|>` itself and dispatches on a hardcoded set of builtin operation names,
  while the value layer in `crates/mensura-types/src/expr_check.rs` does
  not type a general application at all and rejects `BinOp::Pipe` outright.
  That split is acceptable as the "primitives only" staging of ADR 0009,
  but it is a known gap against this ADR, and the unifying refactor (one
  application path shared by `f a b` and `b |> f a`) is a deferred
  follow-up, not done in this round.  That follow-up has since landed; how
  the checker realizes the invariant is `docs/toolkit/01-application-checking.md`.

## Consequences

Positive:

- The learner's model is one sentence: juxtaposition applies, `|>`
  reverses it.  Every operation, present and future, obeys it; no
  operation gets its own calling convention.
- The equivalence is a refactoring license: a stage may always be rewritten
  between `data |> op args` and `op args data` without changing meaning,
  which keeps sugar (future named forms over the kernel) honest.
- It names a concrete target for the checker: collapse the pipe path and
  the application path onto one mechanism, so a single set of arity,
  currying, and tuple rules is maintained.

Negative:

- The ADR records an invariant the code does not yet satisfy.  Until the
  follow-up lands, the equivalence is guaranteed by the grammar and by
  review, not by a shared checker path.

Neutral:

- This ADR adds no grammar and no operations; it states a rule that the
  existing grammar already expresses and that the checker is expected to
  grow into.

## Alternatives considered

1. **A distinct pipeline grammar and semantics.**  Give `|>` its own
   parsing and its own typing, independent of application.  Rejected: this
   is exactly the "mess of rules" the goal forbids, and it duplicates arity
   and currying rules at every operation.
2. **Desugar `|>` in the parser, rewriting `x |> g` to `g x` before
   type-checking.**  A viable future that would satisfy the invariant
   mechanically by erasing the pipe before the checker sees it.  Not chosen
   now only because the AST keeps `BinOp::Pipe` for diagnostics and span
   fidelity; the checker can instead route both forms through one
   application routine without an AST rewrite.  Left open as an
   implementation choice for the follow-up.
3. **Leave the two checker paths permanently separate.**  Rejected as the
   long-term shape: it is the present staging, not the destination, and
   institutionalizing it would let the rules drift apart.

## Open questions

- **Which unification the follow-up takes.**  Parser-level desugar
  (alternative 2) versus a shared application routine the pipe path calls;
  this is settled when the checker refactor is specified.  *Settled:* the
  shared application routine, routing both forms onto one path without an AST
  rewrite (`docs/toolkit/01-application-checking.md`).
- **General application in the value layer.**  `expr_check` types only a
  fixed set of builtins today; the shared application path presumes a
  general application rule, whose surface (user-defined functions, partial
  application outside table context) is not yet specified.  *Settled:*
  `docs/decisions/0030-const-functions.md`: const bindings may be lambdas,
  multi-parameter lambdas are tupled (currying is written explicitly), and
  every application is saturated or an error, so partial binding is
  ordinary application of a curried function.
