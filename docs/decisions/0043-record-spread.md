# 0043: Record spread

## Status

Accepted.  Resolves issue #73.  Partly discharges the `mutate` and
`select` bullet of the named-sugar deferral in
`docs/language/07-pipelines.md`: the general construct underneath
both lands here, and the named forms stay deferred.

Touches `mensura-syntax` (one token, the `record_body` production),
`mensura-types` (the row and bag record checkers), `mensura-runtime`
(`eval`), and `docs/language/04-grammar.md`, `06-expressions.md`,
`07-pipelines.md`, `09-typing-reference.md`.  No `formal/` work
follows (decision 6).

## Context

A `flat_map` body is all-or-nothing today.  `flat_map |k, r| r`
keeps every column, `flat_map |k, r| (.celsius = r.kelvin - 273.15)`
keeps none, and "compute one column and keep the rest" means listing
every surviving column by hand.  `map_bags` has the same gap in its
window shape: `book/src/examples/window-order.mensura` writes
`.at = b.taken_at` only to carry one original column beside the
computed one, and every other column of the fiber is lost.

The obvious fix is a named `mutate` stage.  It would need its own
entry in `07-pipelines.md`, its own tier, and its own lemma, and it
would still leave `map_bags` untouched.  A record spread is the one
general construct underneath `mutate`, `select`, and `rename`, and it
is cheaper than any of them, because it lives in the record literal
rather than in the algebra.

## Decision

### 1.  `...e` in a record body expands to `e`'s fields

A record body may carry spread items beside its labeled fields:

```
field = "." ident [ ":" type ] "=" expr | "..." expr ;
```

The operand is a **path** to a record: a lambda parameter (the value
row `r` or key `k` of a `flat_map`, the fiber `b` or key `k` of a
`map_bags`) or a unit-reference group inside one (`...r.course`).  A
spread of a single value is a type error, and so is a spread of a
computed expression.  Paths cover every use in sight, and they are
what lets the evaluator know a spread's columns from the input's
columns alone, before any row exists (a stage over an empty input
still has a schema).  Admitting more operands later is additive.  The
spread
elaborates at check time to one labeled field per top-level field of
the operand, so

```mensura
readings |> flat_map |k, r| (.celsius = r.kelvin - 273.15, ...r)
```

is the ordinary record body

```mensura
(.celsius = r.kelvin - 273.15, .kelvin = r.kelvin, .machine = r.machine)
```

A record body still needs at least one item, and a `( )` is still
either all positional or all record items: `(...r)` is a record,
`(...r, x)` is a parse error.

### 2.  The fiber spreads as bags, so `map_bags` needs no rule

Because `b.x` is projection sugar (`map (|r| r.x) b`, ADR 0031
decision 1), the same elaboration applied to `...b` yields a record
of **bag-valued** fields, which the existing `map_bags` classifier
already reads as the window shape, one output row per input row:

```mensura
map_bags |k, b| (.running = series.cumsum (|r| r.energy) (|r| r.taken_at) b,
                 ...b)
```

is the window column plus every original column, with no new rule.
`(.total = bag.sum b.x, ...b)` is rejected by the existing "all
aggregates or all window values, not a mix" check, and that
rejection stays.  The aggregate shape's real wish, keeping the
columns that are constant within the bag, is not a spread: it needs a
functional-dependency fact the ADR 0024 gradings cannot supply (a
grading that fitted the coarse key would have made the table
`singletons` and left no bag to reduce).  The honest surface for it
is a reducer that demands agreement, a separate design.  The mix
diagnostic names the spread when one is present, because `...b` is
what people reach for first.

### 3.  Collisions: an explicit field overrides, anything else errs

- An explicit field wins over a spread field of the same name,
  **regardless of position**.  This is the mutate idiom,
  `(.value = r.value * 2.0, ...r)`, which collides on purpose.
- Two explicit fields of one name are an error.  (This was
  unchecked before the spread and is closed with it.)
- Two spreads that share a field name are an error, even when an
  explicit field overrides that name, so no reader has to ask which
  spread position the override takes.

Positional last-wins, the JavaScript rule, is rejected: it makes
reordering the items of a record silently change its meaning.

### 4.  Column order: a spread expands in place

Column order is observable (the branches of an `if` and the items of
an expanding collection must carry the same columns in the same
order), so it gets a rule.  A spread contributes its operand's fields
at its own position, in the operand's order, which for `r`, `k`, and
`b` is the name order the whole-row form `r` already uses.  An
overridden name keeps its **spread** position, not the explicit
field's.  An explicit field that overrides nothing sits where it is
written.  So `(...r)` is exactly `r`, and a mutate never reshuffles
columns.

Two rows that carry the same columns in different orders are still a
mismatch (`if c then (.a = x, ...r) else (...r, .a = y)` when `r` has
no `a`); canonicalizing silently would make column order depend on
which branch the checker read first.  The mismatch diagnostic says
that only the order differs.

### 5.  Nesting: a spread forwards top-level fields whole

`r` may carry a nested record for a compound unit reference
(ADR 0032).  `...r` spreads the top-level fields, so a unit-reference
group forwards whole and re-flattens to its dotted columns exactly as
a bare `r` does.  Override applies to a whole top-level field, never
to a dotted component; since a field name is an `ident`, a dotted
override is not writable anyway.  In `map_bags`, `...b` forwards a
group the same way, as bag-valued dotted columns.

### 6.  Nothing reaches the algebra

The elaborated body is an ordinary record body, so
`flatMap_splitSafe` and `fiberMap_splitSafe` cover it unchanged, and
`formal/` needs no new theorem.  This is the argument for the spread
over a named `mutate`.

### 7.  The token is `...`, and `..` is a lex error

The lexer emits `...` as one `Ellipsis` token.  After `(`, the parser
now predicts a `record_body` on either `.` or `...`, a two-element
predictor set, still one token, so the grammar stays LL(1).  No `..`
operator is planned, so maximal munch is trivial and a bare `..` is
rejected by the lexer rather than read as two dots.

### 8.  Keys stay out, and `map_bags` now checks it too

`r` and `b` are built from non-key columns only, so spreading them
can never set a key column.  `...k` can, and `flat_map` already
rejects a row that names a key column.  `map_bags` had no such check
at all, so an explicit `.machine = k.machine` over a table keyed by
`machine` produced a duplicated column; it gets the same check here,
which covers `...k` as well.

## Consequences

- The deferred `mutate` and `select` are now expressible without
  listing every column: a mutate is `(.x = f r, ...r)`, a select is
  still the explicit record.  `rename` and dropping a column from a
  spread (`...r without x`) remain deferred sugar.
- The book's window example drops its `.at = b.taken_at` workaround
  for `...b`.
- A record literal's items are no longer all labeled, so every
  consumer of `ExprKind::Record` (the checker, the evaluator, the
  const-function lowering, the source collector) walks spread
  operands too.  The override and ordering rules live in one function
  (`mensura_types::record::elaborate`) that the checker and the
  evaluator both call, so the two cannot disagree on column order.
