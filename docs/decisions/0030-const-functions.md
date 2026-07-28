# 0030: Const functions: lambdas, explicit currying, and value-layer application

## Status

Accepted.  Settles open question 2 of
`docs/decisions/0018-application-piping-equivalence.md` ("general application
in the value layer: user-defined functions, partial application outside table
context") and lifts the corresponding exclusion in
`docs/toolkit/01-application-checking.md` ("What this does not add").  Extends
ADR 0027 Decision 1: a closure is a **const binding** in that decision's
sense (immutable, pure, order-independent, non-recursive), so the const
surface grows without a new declaration form.  ADR 0027 Decision 2 ("modules
export const bindings and type-level names only") is unchanged in letter and
now covers function exports; see Decision 8.

No formal backing is required, and none ships.  The table algebra, the
qualifier calculus, and every pipeline propagation rule are untouched: this
ADR is confined to the value layer and to compile-time evaluation, so the
ADR 0021 gate (a checker propagation rule ships only when a theorem under
`formal/` backs it) is not triggered.  The one piece of metatheory involved,
capture-avoiding substitution and its agreement with evaluation, is classic
and is deliberately not mechanized.

Reconciliation this ADR forces, landing in the same pull request:
`docs/language/12-modules-and-imports.md` (the const evaluator's bounds, the
constant-folding paragraph, the Deferred list),
`docs/toolkit/01-application-checking.md` (the negative-scope paragraph),
`docs/decisions/0018-application-piping-equivalence.md` (open question 2
marked settled), and `docs/language/09-typing-reference.md` section 5 (the
function value and its application rule).

It deliberately does **not** include:

- an arrow in the **type grammar**: a function binding cannot carry a
  `: type` ascription, and no type expression denotes a function;
- lambdas as values **inside view bodies**: a view uses a const function but
  cannot create one, and lambdas in view position remain pipeline-operation
  arguments;
- **runtime closures**: the runtime's value model is unchanged (Decision 5);
- **recursive** const functions, now rejected dynamically (Decision 6);
- a bundled module that actually exports a lambda (permitted by the model,
  shipped by nothing; Decision 8);
- higher-order **pipeline operations** (an op taking a user function), which
  would need lambdas as view-body values first.

## Context

Top-level `let` bindings evaluate at compile time
(`docs/language/12-modules-and-imports.md`; ADR 0027 Decision 1).  The const
evaluator accepts literals, names, module members, and arithmetic; a lambda
in a `let` body is rejected as "not a const expression".  So this program
parses cleanly today and fails only in the evaluator, once per binding:

```mensura
let add  { |a| |b| a + b }
let add1 { add 1 }
let three { add1 2 }
```

The gap it falls into is already catalogued.  ADR 0018 unified application
and piping onto one checker path and left its open question 2 for exactly
this surface: user-defined functions and partial application held as a
value.  `docs/toolkit/01-application-checking.md` repeats the exclusion.
And the M0 freeze already commits to the application model this needs:
`09-typing-reference.md` section 5.2 states that application is
juxtaposition, left-associative, and that "functions are curried, so partial
application is an ordinary value".  What is missing is not a model but the
two decisions the model leaves open: what a multi-parameter lambda *is*, and
what becomes of an application that is not saturated.

The forcing question is partial binding.  `add 1` must be a value.  If
`|a, b| a + b` were implicitly curried, `add 1` on it would also be a value,
and the language would need an arity-tracking partial-application mechanism:
closures that remember how many arguments remain, under-application as a
first-class state, and a rule for over-application.  The alternative is to
make the two shapes distinct and let ordinary application do all the work.

## Decision

### 1.  A lambda is a const expression, and a closure is a const value

The const evaluator accepts `ExprKind::Lambda`, producing a **closure**: the
parameter list, the body, and the environment captured **by value** at the
point the lambda is evaluated.  A closure is a const binding in ADR 0027
Decision 1's sense: immutable, pure, order-independent, non-recursive.  No
new declaration form exists; `let add { |a| |b| a + b }` is an ordinary
value `let`.

Capture is by value because const blocks are lexically scoped and a closure
may escape the block whose local `let`s it references.  The capture is a
snapshot of the visible locals; top-level bindings are not captured, they
are resolved on demand exactly as elsewhere.  Shadowing follows the
established rule (`12-modules-and-imports.md`): a parameter shadows a
captured local, and both shadow a top-level binding.

### 2.  A multi-parameter lambda is tupled; currying is written explicitly

`|a, b| e` binds **one** parameter that is a 2-tuple.  `|a| |b| e` is a
one-parameter function returning a one-parameter function.  These are
different functions with different application shapes, and the difference is
user-visible:

```mensura
let add    { |a| |b| a + b }     // curried
let add1   { add 1 }             // ordinary saturated application
let three  { add1 2 }

let addt   { |a, b| a + b }      // tupled: one 2-tuple parameter
let add1t  { |a| addt (1, a) }   // currying written by hand
let threet { add1t 2 }
```

The surface already distinguishes the two: `(1, a)` is a tuple expression,
and a single parenthesized expression `(e)` reduces to `e` rather than
becoming a 1-tuple (`06-expressions.md`), so `addt (1, a)` and `add1 2` are
different shapes in the tree with no ambiguity.

This preserves section 5.2's model exactly.  Application stays curried
juxtaposition; what this decision fixes is the *lambda literal*: currying is
a thing the author writes (nested lambdas), never a thing the language
infers from a parameter list.

### 3.  Every application is saturated or an error

Applying a function to an argument either matches its parameter shape or is
a type error.  For a one-parameter function, any value argument binds.  For
a tupled function of n parameters, the argument must be an n-tuple; `addt 1`
is rejected ("`addt` expects a tuple of 2 values, found `int`"), not held as
a partial binding.  Applying a non-function is rejected at the head's span.

There is **no partial-application mechanism**: no arity-tracking closures,
no under-application state, no currying-through on over-application.
`add 1 2` needs no special rule: `add 1` is a saturated application whose
result is a function, immediately applied to `2`.  Partial binding is
obtained by writing a curried function, and it is then just application.

### 4.  Functions are first-class in the checker, typed by substitution

The expression type system gains a **function value**.  Because parameters
carry no annotations, the type is not inferred from the lambda; it carries
the closure itself, and a saturated application is typed by substituting the
argument expressions into the body and typing the result in the caller's
context.  That is exact per-call-site checking, not an approximation:
`add1 1` types `int`, `add1 r.temp` types `temperature[real]`, each
correctly, with no inference engine.

Two inference designs are rejected (see Alternatives): committing the lambda
to one monomorphic signature, and Hindley-Milner.  The body-substitution
rule gets the polymorphism the examples need (one `add` serving `int`,
`real`, and every dimensioned instantiation) for free, because each call
site re-types the body at its own arguments.

Boundaries of the function value:

- it never enters a **column**: a record field that is a function is
  rejected ("a function is not a value or a bag"), and `ColumnType` gains
  no function case, since it crosses into storage;
- it cannot be **ascribed**: the type grammar has no arrow, so
  `let f: T { |x| ... }` is an error at the ascription;
- a **view body cannot create one**: lambdas in view position remain
  pipeline-operation arguments (`map_bags |k, b| ...`), handled where they
  are today.  A view body can *use* a const function by name.

A top-level `let` whose name collides with an ambient builtin (`sum`,
`to_real`, ...) is a compile error, per the existing collision-not-shadow
policy (ADR 0027 Decision 3).  This matters more now that a binding can be
a function: `let sum { |x| x }` silently shadowing the aggregate would
change the meaning of `sum b.x`.

### 5.  First-class in the checker, inlined in the backend

The runtime gains no closure representation.  Lowering **beta-reduces**
every saturated application of a const function: `add1 r.x` reaches the
runtime as `r.x + 1`, which it already evaluates.  Substitution is
capture-avoiding, reusing the shadow discipline lowering already applies to
lambda parameters and block `let`s.

This is sound and total because only a const binding can create a function
(Decision 4), so every closure at every application site is statically
known; there is no dynamic dispatch to preserve.  Const scalars fold to
literals; const functions fold to their bodies.  The runtime never sees
either, which extends the existing constant-folding contract
(`12-modules-and-imports.md`) rather than changing it.

What "first-class" therefore delivers, stated honestly: a named function
with a real type, explicit currying, functions passed to and returned from
other const functions, and use inside view bodies.  What it does not
deliver: lambdas created in view bodies, functions stored in columns, or
dynamic dispatch.  Each would require runtime closures and is deferred until
something needs it.

### 6.  Non-recursion is enforced dynamically as well as definitionally

The existing cycle detector catches definitional cycles (`let a { b }`,
`let b { a }`).  A lambda defers the reference: `let f { |x| f x }`
evaluates to a closure without touching `f`, so the definitional check
passes, and the recursion only manifests when `f` is applied.  The evaluator
therefore carries a **step budget** decremented per application; exhausting
it is a diagnostic ("const evaluation exceeded its budget: a binding may be
recursive"), not a crash.  The checker's substitution path carries the
analogous depth guard.

### 7.  Pipeline lambdas read uniformly as tupled

`map_bags |k, b| ...` means the operation passes the tuple `(key, bag)`;
`flat_map |k, r| ...` likewise.  This is a reinterpretation, not a change:
the checker already destructures these positionally and demands the exact
parameter count.  One rule now covers every lambda in the language, and the
operation arity errors ("`map_bags`'s lambda takes 2 parameter(s)") become
instances of Decision 3's shape matching.

### 8.  Modules may export functions; nothing ships one yet

A closure is a const binding, so ADR 0027 Decision 2 already permits a
bundled module to export one; no amendment is needed.  The model allows it
and `si.mensura` remains function-free.  Before a module lambda ships, the
diagnostic-provenance question must be settled (Consequences): a type error
inside a substituted module body would today report against source the user
cannot see, at the import site.

## Consequences

Positive.  ADR 0018's last open question closes, and the exclusion paragraph
in `01-application-checking.md` is lifted rather than grown.  The motivating
programs evaluate, with `three` reaching `3` at compile time.  Currying is
explicit, so the language never guesses whether `|a, b|` means one tuple or
two stages; the parameter list means one thing everywhere, pipeline lambdas
included.  The checker gains no inference machinery, no unification, and no
interaction with the dimension algebra beyond what re-typing the body
already provides.

Negative.  **Diagnostic provenance is the substantive cost.**  The checker
and lowering both work on substituted bodies whose spans point at the lambda
definition, so a type error in `add1 r.name` reports inside
`let add { ... }` rather than at the call.  A secondary "while applying
`add` here" diagnostic at the call span mitigates; a real fix wants file
identities and provenance on spans, which is already the third-party-module
prerequisite (`12-modules-and-imports.md`, "Diagnostics and spans") and
stays deferred with it.  **Inlining can grow bodies geometrically**
(`let f4 { |x| f2 (f2 x) }` doubles per layer); the step budget bounds the
evaluator, and lowering carries a node-count cap so the blowup is a
diagnostic rather than a hang.  This is the one place a runtime closure
would be strictly better, recorded here for the day dynamic dispatch is
actually needed.

Neutral.  The runtime, storage mapping, and every pipeline rule are
untouched.  Alpha-sensitive structural equality on closures is unobservable
(no language construct compares consts) and exists for tests only.

## Alternatives considered

### 1.  Implicitly curried multi-parameter lambdas

`|a, b| e` as sugar for `|a| |b| e`, making `addt 1` a partial binding.
Rejected: it needs the full partial-application mechanism (arity-tracking
closures, under-application as a value, an over-application rule), erases
the tuple/curried distinction the surface can already express, and leaves
`map_bags |k, b|` meaning something different from the same list in a const
lambda.  Decision 2 gets the same expressive power from nested lambdas with
none of the mechanism.

### 2.  A monomorphic arrow inferred from the body

Typing `|a| |b| a + b` once, at the definition.  Rejected: the body is
genuinely ambiguous across `int`, `real`, and every dimensioned
instantiation, so definition-time inference either rejects legitimate uses
or picks a default the author never wrote.

### 3.  Hindley-Milner inference

Rejected as out of proportion: unification variables, generalization, and a
story for the dimension algebra, whose exponent-vector arithmetic is not
first-order unification, added to a checker with no inference machinery, to
serve a feature the substitution rule already checks exactly.

### 4.  Runtime closures

A function value in the runtime, applied dynamically.  Rejected: every
closure is statically known (only consts create them), so the machinery
would preserve no behaviour that inlining does not, while adding a function
case to the runtime value model and the storage boundary.  Revisit only if
lambdas become creatable in view bodies.

### 5.  Const-only functions, not exported to view bodies

The evaluator half alone: the motivating programs work, but a view cannot
name `add`.  Rejected as the end state because first-class use in view
bodies is the goal; it survives as the natural first implementation stage.

## Open questions

- **An arrow in the type grammar**, so a function binding can be ascribed
  and a module's exported functions documented in types.  Deferred until
  something needs to *state* a function type rather than carry it.
- **Lambdas as values in view bodies**, the prerequisite for higher-order
  pipeline operations; needs runtime closures or a further inlining
  discipline.
- **Span provenance across substitution** (Consequences), shared with the
  third-party module layer.
- **When a bundled module first exports a function** (Decision 8), and
  whether `stats` (ADR 0028 Decision 4) is that module.

## Forward references

- `docs/decisions/0018-application-piping-equivalence.md` (the settled open
  question) and `docs/toolkit/01-application-checking.md` (the shared
  application path this extends).
- `docs/decisions/0027-modules-and-imports.md` Decisions 1-3 (const
  bindings, module exports, collisions) and
  `docs/language/12-modules-and-imports.md` (the implemented surface).
- `docs/language/09-typing-reference.md` section 5 (the expression typing
  rules the function value joins).
- `docs/decisions/0029-fold-and-scan.md`: the fold family is a separate
  track; its combiner table is closed by ADR and is not affected by
  user-defined functions, which remain outside fold's combiner slot.
