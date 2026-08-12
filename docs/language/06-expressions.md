# Expressions

Mensura has one expression sublanguage.  The same grammar and the same
typing rules are used everywhere an expression is evaluated: an
authorization predicate (`when:`, `where:`), an auto-filled field
(`@auto(...)`), and, later, every operation in a data pipeline.  A site
differs only in the **context** it exposes (the names in scope) and the
**result type** it requires; the language itself is defined once.  This
is the decision recorded in `docs/decisions/0007-single-expression-sublanguage.md`.

This document defines the expression sublanguage: how values are written
and combined, what the operators are and how they union, and how the
multiset of rows at a key, and the possibly missing values inside them,
surface in expressions.  The
concrete LL(1) grammar lives in `04-grammar.md`; this document is about
meaning and shape, and quotes grammar only where it clarifies a design
choice.  Casing of names follows `05-naming-and-casing.md`.  The
table-level operations (`flat_map`, `map_bags`, the joins, `split`, the `|>`
pipe) are part of this same sublanguage but are catalogued in the
pipeline document; this document stops at the value level.

The syntax shown is preliminary, like the rest of the language docs at
this stage; the design content is not.

## Purity and contextual execution

Every expression is **pure** and **lazy**.  It is a description of a
value computed from the names in scope; it reads no external state,
performs no side effect, and does not decide when it runs.  Evaluation
is **contextual**: the site that hosts the expression supplies the free
names, fixes the expected result type, and owns the decision of when and
how the description is executed.  An authorization predicate runs when a
request is checked; an `@auto` expression runs when a row is written; a
pipeline expression runs when its view is materialized.  The expression
text is the same kind of thing in every case.

A consequence worth stating up front: because expressions are pure
descriptions and not statements, there are no special evaluation
contexts.  An operation that needs to compute something per row receives
an explicit function (a lambda), not an implicitly scoped block.  This
keeps every construct an ordinary value or an ordinary application.

## Application and grouping

Function application is **juxtaposition**, written left to right and
left-associative: `f x` applies `f` to `x`, and `f x y` is `(f x) y`.
Multi-argument functions are **curried**: a two-argument function is
applied as `f x y`, and partial application (`f x`) is an ordinary
value, which is what lets pipeline operations compose with the `|>`
pipe.  There is no `f(x)` call form; `f (x)` is simply `f` applied to a
parenthesized group and means the same as `f x`.

Each bracket has exactly one role:

- **`( )`** is for grouping, the homogeneous collection, and records.  `(e)`
  is `e`; `()` is the empty collection and `(a, b, ...)` a collection of like
  values (the form a `flat_map` body uses to drop or expand rows, and the form a
  merge consumes, for example `(train, test)`); and `(.a = x, .b = y)` is a
  labeled **record**, where the leading `.` marks a field.  A `( )` is
  *either* a positional collection or all-labeled, never mixed.  A
  heterogeneous sequence `([ ... ])` is reserved for the future (ADR 0015).
- **`{ }`** is for blocks and declaration bodies, never a value.  In
  expression position it is a statement block (`let` / `assert` statements and
  an optional result), which is why `completeness_check { ... }` is just
  `completeness_check` applied to a block.
- **`[ ]`** is a parameter list at a declaration site, such as
  `Tabular[Person]` or `FeatureWindow[U]`.  It does not appear in
  expressions.
- Application is juxtaposition and uses no bracket at all.

Because application binds tighter than every infix operator, `f x + g y`
is `(f x) + (g y)`, and `data |> flat_map f` is `data |> (flat_map f)`.

The `|>` pipe is **reversed application**: `x |> g` means `g x`, nothing
more.  Application is the one primitive; the pipe only reverses it.  This
single rule, with currying, is what relates the application and pipe
spellings of an operation.  It collapses the four ways one might write a
two-argument call into **two** equivalence classes, not one:

| juxtaposition | means                            | pipe mirror     |
| ------------- | -------------------------------- | --------------- |
| `f a b`       | `(f a) b`                        | `b \|> f a`     |
| `f (a, b)`    | `f` applied to the pair `(a, b)` | `(a, b) \|> f`  |

So `f a b` is the same as `b |> f a`, and `f (a, b)` is the same as
`(a, b) |> f`, but `f a b` is **not** the same as `f (a, b)`: the first
passes two curried arguments, the second passes one argument that is a
pair.  That distinction is not an extra rule to learn; it falls out of
currying, and each pipe form is the exact mirror of a juxtaposition form.
This is the discipline recorded in
`docs/decisions/0018-application-piping-equivalence.md`; how the checker
realizes it is `docs/toolkit/01-application-checking.md`.

## Values

The atomic values are:

- **Numbers**: integer and real literals such as `42` and `3.14`.  These
  are dimensionless and are stored in whatever numeric representation the
  runtime configuration selects.  Ordinary arithmetic applies to them.
- **Strings**: `"text"`.
- **Booleans**: `true`, `false`.
- **Collections**: `(a, b, ...)`, a homogeneous sequence of like values; `()`
  is the empty collection.  A `flat_map` body uses this to drop (`()`) or expand
  (`(a, b)`) rows (ADR 0015).
- **Records**: `(.a = x, .b = y)`, labeled products; a field may carry an
  explicit type, `(.a : T = x)`.  `:` is typing, `=` is the value, matching
  the other expression-level binder, the statement `let`
  (`name [: Type] = value`); item-level bindings are brace-closed instead
  (`12-modules-and-imports.md`).  A record field carries no
  `const`/`var` role marker; the marker ADR 0015 reserved is dropped
  (`docs/decisions/0019-attr-blocks-and-dropped-const-var.md`).
- **Lambdas**: `|x| e` and `|x, y| e` (see below); an optional return type is
  written `|x| : T e`.
- **Conditionals**: `if c then a else b` (see below).
- **Names**: an identifier resolved against the site's context.

Member access is written `a.b.c` and binds tighter than application, so
`f a.b` is `f (a.b)`.

Dimensioned values need no literal form: a physical quantity is written
with ordinary multiplication against a unit constant
(`9.8 * meter / second^2`; see `11-physical-units.md`, ADR 0026).  The
`NxE` measured literal (`10x3`) once reserved here coupled a dimension
with a measurement precision; ADR 0026 separates the two, so the literal
is deferred with the future precision library and is no longer reserved
for units.

### Lambdas

A lambda is an anonymous function written `|x| e`, with parameters
between bars and the body after, following Rust.  Multiple parameters
are comma-separated: `|a, b| a + b`.  Lambdas are the explicit way to
give an operation a per-element computation, for example a reduction's
mapper `|v| v * v`, or a predicate folded with `` `or` `` to quantify
(`fold `or` (|v| v > 30) b.readings`).  Pipeline lambdas are **key-first**,
binding the key before the
value: `flat_map`/join `|k, r|`, `map_bags |k, b|`, `split |k|` (ADR 0015, and
`07-pipelines.md`).  `|_, r|` ignores the key.

The closing bar of a lambda and the `|>` pipe both use `|`.  The two
never collide in practice: `|>` is the pipe and is always infix, while a
lambda's bars are `|` immediately followed by a parameter list, never by
`>`.  The single lexing wrinkle is a closing bar pressed against a `>`
with no space (`|x|>0`), which a maximal-munch lexer would read as
`|x` then `|>`; writing the comparison with a space (`|x| > 0`)
resolves it, and the formatter enforces that spacing.

### Conditionals

A conditional is written `if c then a else b` (ADR 0015): the condition `c` is
a known boolean, and the two branches must have the same type, which is the
type of the whole expression.  Both branches are always present; there is no
`else`-less form.  It is an ordinary value, so it nests anywhere an expression
is expected, including a field value, `(.flag = if r.hot then 1 else 0)`, and
a `flat_map` body, where `if c then r else ()` keeps or drops the row.  The
conditional is the introduction site for the deferred `is known` narrowing.

## Operators and precedence

The operators, from loosest-binding to tightest:

| Operators | Associativity | Notes |
|---|---|---|
| `\|>` | left | the pipe; its consumers are pipelines |
| `or` | left | |
| `and` | left | |
| `not` | prefix | |
| `== != < <= > >=`, `in`, `is known`, `is missing` | non-associative | |
| `??` | right | the coalescing discharge (ADR 0039) |
| `<< >>`, `<: :>` | left | binary minimum and maximum; keep-left and keep-right |
| `+ -` | left | |
| `* /` | left | |
| `-` | prefix (unary) | |
| `^` | right | |
| application | left | juxtaposition |
| `#` | prefix | cardinality; its operand is a member access |
| `.` | postfix | member access, tightest |

Most operators use tokens the lexer already emits; ADR 0031 adds `#`, `<<`,
`>>`, `<:`, and `:>`, and ADR 0039 adds `??`.  A few rules the layering
implies:

- **Comparisons do not chain.**  `a < b < c` is rejected; a conjunction
  (`a < b and b < c`) says it instead.  This keeps the comparison level
  non-associative and unambiguous.
- **`not` sits below the comparisons**, so `not a == b` is
  `not (a == b)`, matching the common reading.
- **Unary minus and `^`.**  `^` binds tighter than unary minus, so
  `-2^2` is `-(2^2)`; the right operand of `^` may itself be a unary
  expression, so `2^-3` is `2^(-3)`.
- **`-` between two atoms is subtraction**, never application of a
  negated argument.  `f - x` is subtraction; a negated argument must be
  parenthesized, `f (-x)`.  This is the one ambiguity juxtaposition
  introduces, and it is resolved in favour of the binary reading.
- **`<<` and `>>` sit between arithmetic and the comparisons**, so
  `a + b << c` is `(a + b) << c` and `a << b < c` is `(a << b) < c`.
- **`??` sits between the tacks and the comparisons**, so a value
  discharge sits inside a comparison unparenthesized
  (`r.peak ?? limit < t` is `(r.peak ?? limit) < t`) while a boolean
  policy discharge is written with parens, `(a < b) ?? false`, which
  reads as the deliberate statement it is.
- **`#` binds looser than `.` and tighter than the comparisons**, so
  `#b.x` is `#(b.x)` and `#b > 3` reads as written.  It also sits inside
  the application spine, so `f #b` is `f (#b)`.

### The four operators of ADR 0031

`a << b` and `a >> b` are the **binary minimum and maximum**.  Both
operands are of one orderable domain, dimension included, and the result is
of that domain, so the earlier of two dates and the smaller of two
temperatures both work.  They are independently useful (clamping,
earlier-of-two-dates), and they are also the rows the aggregate minimum and
maximum derive from.

`a <: b` and `a :> b` are **keep-left** and **keep-right** (APL's tacks):
`a <: b` is `a`, `a :> b` is `b`, both operands of one domain.  Their
algebra is two lines deep, associative but *not* commutative, with no
identity.  Their scalar reading is trivial by design; their habitat is the
backticked combiner slot (`07-pipelines.md`), where they are what make the
ordered window operations derivable.

`#e` is the **cardinality** of a bag: `#b` is the number of rows in a
group, `#b.x` the size of a projected bag.  Unlike the value reductions it
does not require a total bag, because it never reads the values: a row
whose column is missing still counts, and the empty bag counts zero.

A backtick-quoted operator (`` `+` ``, `` `<<` ``, `` `:>` ``) is a
**combiner**, the argument the reductions take.  The set of admissible
operators is closed, so an unknown combiner is an error naming the table;
it extends by decision record, never by assertion at a call site.

### Temporal arithmetic (ADR 0036)

The temporal point domains carry torsor arithmetic, not numeric
arithmetic: a point and a duration are different things, and the
operator rules keep them apart.  Three rows, gated by
`formal/Mensura/Units/Torsor.lean` (ADR 0021):

- `instant - instant : time[real]`.  The difference of two absolute
  points is an ordinary duration, usable wherever a quantity is:
  `r.ended - r.started > 10.0 * si.minute` is a plain dimensioned
  comparison.
- `instant + time[real] : instant` and `instant - time[real] : instant`.
  A duration translates a point, and the point is written first; nothing
  else moves a point.

Nothing else types.  Two instants do not add (a point is not a
quantity, ADR 0036 decision 3), a point never scales, `date` has no
arithmetic at all until `diff(date)` is settled (decision 4), and
`instant` never mixes with `date`: the two are different temporal
families, and no conversion relates them without a zone.

**Translation is exact-or-error.**  The duration operand must be a
whole number of milliseconds, `instant`'s resolution: translating by
`0.0001 * second` (a tenth of a millisecond) is an evaluation error,
never a rounding.  Silent rounding could send equal points to unequal
ones (breaking key identity) and would drift a window grid slowly and
invisibly; an error at the first evaluation is the honest failure.  A
result outside the representable range (years 0001-9999) is likewise an
error.  The extents that matter most are const expressions, so the M5
windows slice moves their check to compile time (ADR 0030).

## Cardinality and missing values

A table keys a **multiset of nested rows**: at a key there may be no row
(`card 0`, "not sampled"), one row, or several.  That row count is the
**cardinality**, and it is the only multiset in the model.  A single
value inside a row is **not** a multiset: each value is either **known**
or **missing**, always 0 or 1 (`Cell = Option` in
`formal/Mensura/Core/Defs.lean`).  Cardinality (how many rows) and
missingness (whether a value is there) are orthogonal axes.

A value-scoped expression runs at one row, so a bare column read there is
a single value.  A bag-scoped expression (the `b` of a `|k, b|` lambda, see
`07-pipelines.md`) sees the whole bag of rows at a key, so a column read
there is the **bag** of that column's values across the rows.  Operators
state what they accept on each axis, and the language never silently
bridges either gap:

- **Scalar operators** (`+ - * / ^`, the comparisons, `and`/`or`/`not`)
  require **a single value**: cardinality 1.  Applying one to a bag is a
  **hard type error**, not an implicit fold.  Numeric splits into `int`
  and `real`, and operators are gated by the scalar domain (ADR 0014):
  operands must match with no coercion, `/` is `real`-only, `==`/`!=` is
  not defined on `real`, and ordering (`< <= > >=`) and `min`/`max`
  apply to the orderable domains (`int`, `real`, `date`, `instant`).
- **The missing axis lifts** (ADR 0039).  An optional operand is
  accepted, and the result is then optional: absent when any operand is
  absent, the ordinary application otherwise, with the domain and
  dimension rules unchanged under the `?`
  (`formal/Mensura/Expr/Missing.lean`).  So `r.previous + 1.0` over a
  `real?` is a `real?`, and `r.temp > r.previous` is an **optional
  boolean**.  There is no three-valued logic: `false and missing` is
  missing, not false, because absence absorbs uniformly.  (For the same
  reason `and`/`or` evaluate both operands rather than short-circuiting,
  like the tacks: a diagnostic in the discarded side still surfaces.)
- **Decision boundaries stay total** (ADR 0039 decision 3).  Absence
  flows through values; it never flows past a decision unconsulted.  An
  `if` condition (and therefore a `flat_map` filter) demands a total
  boolean, so an optional comparison must state its absent-row policy:
  `(r.temp > r.previous) ?? false`.  A fold or scan accumulates total
  values, so a mapper over an optional column is rejected until it is
  discharged; no aggregate skips absent values silently.  Keys are total
  (ADR 0010).  `#` counts rows, never values, and is unaffected.
- **`??` is the only exit from optionality**: `e ?? d` is the present
  value or the default, whose domain must match, dimension included
  (`?? 0.0 * kelvin`, not `?? 0`).  Right-associative: a chain
  discharges at its first present value and is total exactly when its
  final default is.  Every `??` is a grep-able policy statement; there
  is no implicit collapse anywhere else in the language.
- **Reduction** is the explicit way to consume a bag.  `fold` is the
  primitive (`fold `+` (|v| v) b.credits`), `#` counts (`#b.credits`), and
  membership `v in b.tags` tests.  The named reductions (`bag.sum`,
  `bag.min`, `bag.max`, `bag.any`, `bag.all`, and `bag.prod`) are const
  bindings in the `bag` module rather than builtins, so they are imported
  (ADR 0031, `12-modules-and-imports.md`); `mean` is derived,
  `bag.sum b.x / to_real (#b.x)`.  Each returns a single value.  A literal
  is a single value.

So a bag is always collapsed deliberately, by reduction
(`bag.max b.readings > 30.0`, or `fold `>>` (|v| v) b.readings` written
out) and never by accident.  A possibly missing value *propagates
honestly and is discharged deliberately*: carry the `?` as far as it
goes and write `??` only where the default is a true statement about
the domain (**discharge late**, ADR 0039 decision 4).  The fleet
example's `reading_rate` is the worked case: the rate is honestly
absent on each machine's first reading, and the serving layer renders
the absence.  Values are **total** (always known) by default; an
**optional** value, one that may be missing, is written with a `?` on
its type (ADR 0010).

### Known and missing values, and the row

`is missing` tests whether a value is **missing**, `is known` whether it
is present.  They apply to values only, lifted results included
(`(r.a + r.b) is known`), and always return a *total* boolean.  An
**optional** value is the one place either may hold; on a **total**
value `is known` is always true.

`is known` does **not** narrow: the guarded branch still sees the
optional type, because flow-sensitive narrowing is deferred (ADR 0039
alternative 4; `??` subsumes the common cases and is forward-compatible
with it).  The discharge is `??`, above.

A *row* (an entity) being absent is a different thing: it is the key
having no rows at all (`card 0`), "not sampled."  A value-scoped
expression never observes this, because it only ever runs where a row
exists, so **testing a row for absence is not allowed** at the expression
level for now.  The intended future form is a row-cardinality operator,
`#row == 0` for "not sampled," reserved but not specified here.

## The context model

A site is a pair: the **context** of names it puts in scope, and the
**result type** it requires.  The expression grammar is the same across
sites; only this pair changes.

- The **context** is a set of named values.  An authorization predicate
  exposes `principal` and `row` (see
  `docs/decisions/0005-identity-and-authorization.md`); an `@auto`
  expression exposes the ambient values it is allowed to read; a
  pipeline operation exposes the columns of the table it runs over,
  through the lambda it is given.  A bare name resolves against this
  context, and member access (`principal.kind`, `r.machine`) is typed
  against the named value's type.  Names that classify values (units,
  shapes, enums) are PascalCase; the value-level names in a context
  (columns, principals, parameters) are snake_case, per
  `05-naming-and-casing.md`.
- The **result type** is what the site checks the expression against: a
  boolean for a predicate (`when:`, `where:`, a `split` predicate), a
  value for `@auto` or a derived column, and so on.  The aggregates form
  a distinct family: they are well-typed only where an aggregate result
  is expected, and a later document fixes which builtins each context
  admits.

Which builtins (`now`, `env`, `lookup`, `prev`, the aggregates) are in
scope is therefore a property of the context, not of the grammar.  The
grammar knows only names, application, member access, and the operators
above.

## Enumerated values

An enumerated type is declared once, by name, and referenced by that
name, rather than written inline at each use:

```
enum Status {
  "active"
  "inactive"
  "in-progress"
}
```

`Status` is a type, so it is PascalCase, and its variants are string
literals, so they may hold values that are not valid identifiers
(`"in-progress"`, spaces, accents) and map directly onto the categorical
representation the storage layer uses.  In an expression an enumerated
value is compared as a string, `r.status == "active"`, and the checker
validates the literal against the type's variant set, so `== "activ"` is
a compile error.  The declaration form is grammar-level and is specified
in `04-grammar.md`; it replaces the earlier inline `enum(...)` type.

## Worked examples

An authorization predicate (boolean result, context exposes `principal`):

```
principal.kind == "device" and "temperature-sensor" in principal.roles
```

A derived value over a single row (the key-first lambda ignores the key and
binds the value row; `mass` and `height` are single-valued columns):

```
|_, r| r.mass / r.height ^ 2.0
```

A predicate that reduces a bag before comparing (a bag lambda; `b.readings`
is the bag of readings across the key, so a scalar comparison on it would be
a type error and `max` collapses it first):

```
|_, b| bag.max b.readings > 30.0
```

A membership test over such a bag:

```
|_, b| "staff" in b.roles
```

A `flat_map` body that filters, keeping only the rows that need attention (the
empty collection `()` drops a row, the value row `r` keeps it; ADR 0015):

```
|_, r| if r.status == "degraded" then r else ()
```

## Forward references and open questions

- **Consolidated rules.**  The expression rules above are collected, with the
  pipeline and completeness rules, in `09-typing-reference.md` (the M0 freeze).
- **Measured SI values.**  Settled by `11-physical-units.md` (ADR 0026):
  units are ordinary dimensioned constants combined with the explicit
  operators (`9.8 * meter / second^2`), so there is no juxtaposition
  attachment, no `SI(...)` constructor, and no unit grammar beyond the
  operators above.  Dimensional checking and automatic conversion are
  specified there.  The once-reserved `NxE` measured literal is deferred
  with the precision library (ADR 0026, Decision 9).
- **The pipeline level.**  The `|>` pipe, the operation catalogue
  (`promote`, `flat_map`, `map_bags`, the joins, `split`, `union`) and their
  split-safety obligations are the same sublanguage applied at table type,
  catalogued in the pipeline document.  Filtering is not a primitive: it is
  `flat_map |k, r| if c then r else ()` (ADR 0015).  `|>` appears in the precedence table here because it is one
  language, but its consumers live there.
- **The builtin catalogue per context.**  Exactly which ambient names each
  site admits (`now`, `env`, `lookup`, `prev`, `next`, ...) is fixed per site
  as those sites are specified, not by this document.  What is settled is the
  **initial environment**, which ADR 0031 Decision 8 made small: the seven
  intrinsic base units (`second`, `meter`, `kilogram`, `ampere`, `kelvin`,
  `mole`, `candela`), the reduction primitives `fold` and `map`, `to_real`,
  and the pipeline operations.  Nothing else, and in particular no aggregate
  vocabulary: those are const bindings in the `bag` module and must be
  imported, so ADR 0027 Decision 4's "nothing else is in scope that you did
  not import" holds without exception.  Top-level `let` bindings and imported
  module members join the context the same way
  (`11-physical-units.md`, `12-modules-and-imports.md`).  `to_real` is the one
  remaining word-builtin that is neither a primitive nor a pipeline
  operation; by the same logic it belongs in a future `math` module.
- **ADR follow-up.**  The authorization examples in
  `docs/decisions/0005-identity-and-authorization.md`, written today as
  `lookup(principal)` and `@auto(auth.id)`, are still to be re-spelled to
  juxtaposition (`lookup principal`, `@auto (auth.id)`).  (The expression
  productions and the named `enum` declaration now live in `04-grammar.md`.)
