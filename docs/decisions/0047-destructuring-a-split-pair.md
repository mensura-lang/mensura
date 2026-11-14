# 0047: Destructuring a split pair

## Status

Proposed.  The first of two ADRs that prepare the language for leak
detection across a `split` (ROADMAP M6, taken ahead of order); the
second is `docs/decisions/0048-reach-and-self-derived-lookups.md`, which
needs the sides this one makes reachable.  Changes the view-body `let`
production of `docs/language/04-grammar.md`.  No `formal/` work: binding
a name is transport by identity, as ADR 0042 decision 1 argues for view
names.

## Context

`split |k| pred` yields a pair, and a pair has exactly one consumer
today: `union`.  The documents promise more.  `07-pipelines.md` and
`08-lineage.md` write `let (train, test) = data |> split |k| ...`, the
book does the same in "What the types track", and `08-lineage.md` also
writes `(folds.0, folds.1)`.  None of it parses.  The grammar's
`let_stmt` binds one `ident`, so `let (train, test) = ...` fails with
"expected a name after `let`", and there is no positional projection.
A view must end in a single table, so a pair cannot leave a view either.

The consequence is that a split can only be undone.  No program can fit
on one side and apply to the other, and so no program can leak across a
split, nor be shown not to.  Checking the leak pattern of
`formal/Mensura/Leak.lean` against the checker confirmed this: a
whole-table statistic looked up back into the table and then split is
accepted, harmlessly, because its two sides can only be recombined.
Every M6 question (what `fit` and `evaluate` demand, what ADR 0048
tracks) starts from a program that can name one side.

## Decision

### 1.  `let` destructures a pair

The view-body `let` statement gains a pair pattern:

```
let_stmt  = "let" ( ident [ ":" type ] | pair_pat ) "=" expr ;
pair_pat  = "(" ident "," ident ")" ;
```

After `let`, one token decides: an `ident` is the existing form and `(`
the new one, so the grammar stays LL(1).  The right-hand side must type
as a pair (today only `split` produces one, directly or through a `let`
that bound it); a single table there is a type error naming the
expected pair.  The pattern has exactly two names, since every pair
has exactly two sides, and the names must differ.  `_` discards a side,
as it does in a lambda's parameter list:

```mensura
view training_rows {
  let (train, _) = enrollments |> split |k| k.year < 2024;
  train
}
```

Each bound name is an ordinary local table.  It may be used any number
of times, in pipeline position, as a tuple item, and on the right of
`lookup` and `lookup_total` (ADR 0042 decision 2's table positions).
Its type is the side's type, with its sibling lineage tag, unchanged by
the binding.

### 2.  No positional projection

`folds.0` and `folds.1` are not added.  Destructuring covers every use
the documents show, it names the sides where they are introduced, and
a second spelling for the same thing is the kind of surface this
language avoids until a need appears.  `08-lineage.md`'s
`(folds.0, folds.1) |> union` becomes `folds |> union` when that
document is next revised; the pair is already a valid `union` input.

### 3.  A view still ends in one table

A pair-valued view is not added.  A view materializes one table, and
both its runtime storage and the serving projection
(`docs/decisions/0045-serving-a-program.md`) assume that.  A view may
end in either side of a pair, which is enough to materialize a training
set or an evaluation set as a named view.

### 4.  Lineage is unchanged

`split` already gives each side a sibling tag identified by the split
stage's source position (`08-lineage.md`, `09-typing-reference.md`
section 9).  Destructuring adds nothing to it.  Two sides bound in one
view body are disjoint, as they were inside the pair.

## Consequences

- A program can name one side of a split, so it can materialize a
  training view, and later (M6) fit on one side and evaluate on the
  other.  The leak of `Leak.lean` becomes expressible, which is what
  makes ADR 0048's tracking observable.
- The examples in `07-pipelines.md`, `08-lineage.md`, and the book
  become valid syntax.  They are checked against the implementation when
  it lands; `08-lineage.md`'s positional projection is rewritten then.
- The runtime needs a pair-valued local in the evaluator's environment,
  which it already has for `let parts = ... |> split ...`.

## Alternatives considered

- **Positional projection only (`p.0`, `p.1`).**  Rejected as the
  primary form: anonymous sides make every later use say `.0` where a
  name says what the side is for.  It stays available if a need appears.
- **Both forms.**  Rejected for now, per the discipline of one spelling
  per concept.
- **Named outputs on `split` itself** (`split |k| pred as (train,
  test)`).  Rejected: it would make `split` a binding form, while every
  other stage is an expression that `let` binds.

## Open questions

- **Pair-valued views.**  A split identifies its tag by source position,
  so two views that each compute the same split carry unrelated tags
  (ADR 0042 decision 1), and a training view and an evaluation view
  written separately cannot be shown disjoint.  Today they must come
  from one view body.  A view whose value is a pair, read by
  destructuring, would let two views share one split.  It needs a
  storage answer for the pair and a serving answer for its two
  halves, and it waits for M6 to show whether fit and evaluate must
  live in different views.
- **Arity beyond two.**  k-fold validation wants k sides.  Whether that
  is a nested `split`, a `folds` operation yielding a k-tuple, or
  something else is M6's question; the pattern generalizes to
  `"(" ident { "," ident } ")"` if a k-tuple appears.
