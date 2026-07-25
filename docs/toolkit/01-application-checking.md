# Application and pipe checking

`docs/decisions/0018-application-piping-equivalence.md` fixes the rule that
`|>` is *reversed application*: `x |> g` means `g x`, with application by
juxtaposition the one primitive.  The ADR requires the type checker to check
the right side of a `|>` as the application of an operation to the piped
input, **through the same path that checks a bare application**, and records
that the checker did not yet do so.  This document specifies how the checker
realizes that invariant.  The language rule itself lives in
`docs/language/06-expressions.md` ("Application and grouping"),
`docs/language/07-pipelines.md`, and `docs/language/09-typing-reference.md`
(section 5.2); this is the implementation home.

## The invariant

For any operation, `data |> op args` and `op args data` must be checked
identically: same resolution of `op`, same argument count, same partial-
application typing, same result, same diagnostics.  A pipe stage is an
application whose last argument arrives from the left.  Nothing about being in
pipe position may change how the operation is checked.

## Two layers, one shape

The checker has two type domains and they stay separate:

- The **pipe layer** (`crates/mensura-types/src/pipe_check.rs`) types table-
  valued expressions against named `Sources`, producing a `PipeTy`
  (`Table` or `Pair`).  Its operations are the Tier A primitives.
- The **value layer** (`crates/mensura-types/src/expr_check.rs`) types scalar
  expressions against a row/bag/key `Context`, producing a `Ty` (`Value`,
  `Bag`, `Bool`, `Record`).  Its applicable builtins are `to_real` and the
  aggregates.

"One rule" is **structural symmetry**, not one shared function: each layer
flattens the application, treats the piped input as the last positional
argument, and dispatches.  The return types and dispatch tables differ, so
merging the two into a single routine would be wrong.  Only `op_join` needs
`Sources`; the value layer has no `Sources` at all.  Keeping them separate is
what lets each stay honest to its domain.

## The shared routine, per layer

Each layer has one application routine that both the bare-application AST node
and the pipe operator call.  In the pipe layer:

```rust
fn apply_application(sources, op_expr, piped: Option<PipeTy>) -> Result<PipeTy, _> {
    let (head, mut args) = flatten_app(op_expr);   // f a b -> (f, [a, b])
    let ExprKind::Name(op) = &head.kind else {
        return Err(error("expected a pipeline operation", op_expr.span));
    };
    let input = match piped {
        Some(p) => p,                              // data |> op args
        None => {                                  // op args data
            let Some(last) = args.pop() else {
                return Err(error("a pipeline operation needs an input", head.span));
            };
            type_pipeline(sources, last)?          // input typed as a sub-pipeline
        }
    };
    dispatch_op(sources, op, &args, input, head.span)
}
```

`type_pipeline` routes a bare application `ExprKind::App(..)` through this with
`piped = None`, and routes `lhs |> rhs` through it with `piped =
Some(type_pipeline(lhs)?)`.  `dispatch_op` is the operation match (the Tier A
names plus the unknown-operation suggestion hint); it is unchanged from the
old `apply_op` body and the per-operation handlers keep their signatures.  The
value layer mirrors this with `apply_value`, dispatching to `type_to_real` and
the aggregates.

Because both entry points converge on the same flatten, the same input-as-
last-argument rule, and the same dispatch, the two spellings are
indistinguishable by construction.

## Routing, not desugaring

The checker routes the two forms onto one path; it does **not** rewrite
`x |> g` to `g x` in the parser (ADR 0018, alternative 2).  The AST keeps
`ExprKind::App` and `BinOp::Pipe` distinct.  Two reasons:

- **Span fidelity.**  Every diagnostic anchors on a real source span (the
  operation name, each argument, a lambda body, the piped input).  A desugar
  would have to synthesize spans for the rewritten application; routing keeps
  the original spans verbatim, so `op args BADDATA` reports on `BADDATA` and
  `data |> op args` reports on `data`, each in its own place.
- **Blast radius.**  The change stays inside `mensura-types`.  The AST that
  `mensura lsp` (`docs/toolkit/02-lsp.md`) and other consumers see is
  unchanged.

## What this does not add

This realizes the equivalence for the operations the checker already knows.
It does **not** introduce general application: user-defined functions,
partial application held as a value, or application of anything outside the
built-in operation sets.  That surface is ADR 0018's open question 2 and
stays open.  The practical consequence is the saturation guard above: peeling
the trailing argument as the input is sound only for a saturated stage, so an
unsaturated bare form such as `promote cols` with no table reports "a
pipeline operation needs an input" rather than being typed as a partial
application.
