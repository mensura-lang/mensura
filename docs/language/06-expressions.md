# Expressions

Mensura has one expression sublanguage.  The same grammar and the same
typing rules are used everywhere an expression is evaluated: an
authorization predicate (`when:`, `where:`), an auto-filled field
(`@auto(...)`), and, later, every operation in a data pipeline.  A site
differs only in the **context** it exposes (the names in scope) and the
**result type** it requires; the language itself is defined once.  This
is the decision recorded in `docs/decisions/0007-single-expression-sublanguage.md`.

This document defines the expression sublanguage: how values are written
and combined, what the operators are and how they bind, and how the
multiset of rows at a key, and the possibly missing values inside them,
surface in expressions.  The
concrete LL(1) grammar lives in `04-grammar.md`; this document is about
meaning and shape, and quotes grammar only where it clarifies a design
choice.  Casing of names follows `05-naming-and-casing.md`.  The
table-level operations (`filter`, `map`, `aggregate`, joins, the `|>`
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

- **`( )`** is for grouping and product values.  `(e)` is `e`; `(a, b)` is a
  positional **tuple**, a genuine product value (the form a merge consumes,
  for example `(train, test)`); and `(.a = x, .b = y)` is a labeled
  **record**, where the leading `.` marks a field.  A `( )` is *either*
  all-positional or all-labeled, never mixed.
- **`{ }`** is for blocks and declaration bodies, never a value.  In
  expression position it is a statement block (`let` / `assert` statements and
  an optional result), which is why `completeness_check { ... }` is just
  `completeness_check` applied to a block.
- **`[ ]`** is a parameter list at a declaration site, such as
  `Tabular[Person]` or `FeatureWindow[U]`.  It does not appear in
  expressions.
- Application is juxtaposition and uses no bracket at all.

Because application binds tighter than every infix operator, `f x + g y`
is `(f x) + (g y)`, and `data |> filter p` is `data |> (filter p)`.

## Values

The atomic values are:

- **Numbers**: integer and real literals such as `42` and `3.14`.  These
  are dimensionless and are stored in whatever numeric representation the
  runtime configuration selects.  Ordinary arithmetic applies to them.
- **Strings**: `"text"`.
- **Booleans**: `true`, `false`.
- **Tuples**: `(a, b, ...)`, positional products of values.
- **Records**: `(.a = x, .b = y)`, labeled products; a field may carry an
  explicit type, `(.a : T = x)`.  `:` is typing, `=` is the value, matching
  every other binder (`name [: Type] = value`).
- **Lambdas**: `|x| e` and `|x, y| e` (see below); an optional return type is
  written `|x| : T e`.
- **Names**: an identifier resolved against the site's context.

Member access is written `a.b.c` and binds tighter than application, so
`f a.b` is `f (a.b)`.

A second family of numeric literal, the `NxE` measured literal
(`10x3`, meaning ten times ten-to-the-three with the precision its
integer significand implies), is reserved for measured SI values and is
specified with the physical-units feature, not here.  An `NxE` literal
is never an ordinary number: it carries a dimension and a precision, so
mixing it with plain arithmetic (`10x3 + 1`) is a type error by
construction.  See the forward references.

### Lambdas

A lambda is an anonymous function written `|x| e`, with parameters
between bars and the body after, following Rust.  Multiple parameters
are comma-separated: `|a, b| a + b`.  Lambdas are the explicit way to
give an operation a per-element computation, for example a row predicate
`|r| r.age >= 18` or a quantifier body `|x| x > 30`.

The closing bar of a lambda and the `|>` pipe both use `|`.  The two
never collide in practice: `|>` is the pipe and is always infix, while a
lambda's bars are `|` immediately followed by a parameter list, never by
`>`.  The single lexing wrinkle is a closing bar pressed against a `>`
with no space (`|x|>0`), which a maximal-munch lexer would read as
`|x` then `|>`; writing the comparison with a space (`|x| > 0`)
resolves it, and the formatter enforces that spacing.

## Operators and precedence

The operators, from loosest-binding to tightest:

| Operators | Associativity | Notes |
|---|---|---|
| `\|>` | left | the pipe; its consumers are pipelines |
| `or` | left | |
| `and` | left | |
| `not` | prefix | |
| `== != < <= > >=`, `in`, `is known`, `is missing` | non-associative | |
| `+ -` | left | |
| `* /` | left | |
| `-` | prefix (unary) | |
| `^` | right | |
| application | left | juxtaposition |
| `.` | postfix | member access, tightest |

All operators use tokens the lexer already emits.  A few rules the
layering implies:

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

## Cardinality and missing values

A table keys a **multiset of nested rows**: at a key there may be no row
(`card 0`, "not sampled"), one row, or several.  That row count is the
**cardinality**, and it is the only multiset in the model.  A single
value inside a row is **not** a multiset: each value is either **known**
or **missing**, always 0 or 1 (`Cell = Option` in
`formal/Mensura/Table.lean`).  Cardinality (how many rows) and
missingness (whether a value is there) are orthogonal axes.

A value-scoped expression runs at one row, so a bare column read there is
a single value.  A group-scoped expression (a `|g|` lambda, see
`07-pipelines.md`) sees the whole bag of rows at a key, so a column read
there is the **bag** of that column's values across the rows.  Operators
state what they accept on each axis, and the language never silently
bridges either gap:

- **Scalar operators** (`+ - * / ^`, the comparisons, `and`/`or`/`not`)
  require **a single known value**: cardinality 1 and not missing.
  Applying one to a bag, or to a value that may be missing, is a **hard
  type error**, not an implicit fold or default.  `r.temperature > 30`
  is well-typed only when `temperature` is read at one row and is total
  (known).
- **Bag combinators** are the explicit way to consume a bag.  Membership
  `v in g.tags` tests it; `count`, `any`, and `all` summarize it; the
  aggregates `sum`, `mean`, `min`, `max` reduce it to one value.  These
  return a single value.  A literal is a single value.

So a bag is always collapsed deliberately, by reduction
(`mean g.readings > 30`) or by quantification (`any (|x| x > 30)
g.readings`), and never by accident.  A possibly missing value is
eliminated just as deliberately, by a default or coalesce, by an
aggregate defined over missingness, or by narrowing (below); it never
silently propagates.  Values are **total** (always known) by default; an
**optional** value, one that may be missing, is written with a `?` on
its type (ADR 0010).

### Known and missing values, and the row

`is missing` tests whether a value is **missing**, `is known` whether it
is present.  They apply to values only.  An **optional** value is the one
place either may hold; on a **total** value `is known` is always true.

`is known` **narrows**: inside the branch guarded by `r.x is known`, and
on every row after a table-level `filter (|r| r.x is known)`, the optional
`x` is treated as total, so a scalar operator may then use it.  This is
the third way to establish that a value is known, alongside a default or
coalesce and an aggregate defined over missingness (ADR 0010).

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
  boolean for a predicate (`when:`, `where:`, a `filter` lambda), a
  value for `@auto` or a derived column, and so on.  The aggregates form
  a distinct group: they are well-typed only where an aggregate result
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
enum Status { "active", "inactive", "in-progress" }
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

A derived value over a single row (the lambda binds the row; `mass` and
`height` are single-valued columns):

```
|r| r.mass / r.height ^ 2
```

A predicate that reduces a bag before comparing (a group-scoped lambda;
`g.readings` is the bag of readings across the group, so a scalar
comparison on it would be a type error and `mean` collapses it first):

```
|g| mean g.readings > 30
```

A membership test over such a bag:

```
|g| "staff" in g.roles
```

## Forward references and open questions

- **Measured SI values.**  The `NxE` literal (`10x3`), the attachment of
  a unit by juxtaposition (`10x3 m`, with no `SI(...)` constructor), the
  unit grammar (`m/s^2` under ordinary operator precedence, with no
  whitespace-significance), dimensional checking, and conversion between
  units all belong to the physical-units and precision feature.  This
  document fixes only that such literals are a distinct kind and do not
  participate in plain arithmetic.
- **The pipeline level.**  The `|>` pipe, the operation catalogue
  (`filter`, `map`, `aggregate`, `ungroup`, the joins, `pivot`,
  `unpivot`, `bind`, `split`) and their split-safety obligations are the
  same sublanguage applied at table type, catalogued in the pipeline
  document.  `|>` appears in the precedence table here because it is one
  language, but its consumers live there.
- **Row cardinality.**  `#row` (and possibly a general cardinality
  operator `#x`) is reserved for a later round; for now only value-level
  `is known` / `is missing` exist.
- **The builtin catalogue per context.**  Exactly which ambient names
  and aggregates each site admits (`now`, `env`, `lookup`, `prev`,
  `next`, `sum`, `mean`, `min`, `max`, `count`, `any`, `all`, ...) is
  fixed per site as those sites are specified, not by this document.
- **ADR follow-up.**  The authorization examples in
  `docs/decisions/0005-identity-and-authorization.md`, written today as
  `lookup(principal)` and `@auto(auth.id)`, are still to be re-spelled to
  juxtaposition (`lookup principal`, `@auto (auth.id)`).  (The expression
  productions and the named `enum` declaration now live in `04-grammar.md`.)
```
