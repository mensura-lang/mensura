# 0040: Partial operator precedence

## Status

Accepted.  Provoked by ADR 0039 decision 2, whose `??` slot was
settled by fiat (Swift's side of a Swift-versus-C# coin toss) because
the total precedence ladder offered no principled place for it.  This
ADR replaces the total order with a partial one and closes the
ladder, so no future operator faces the same toss.

Touches `mensura-syntax` (the parser's precedence cascade) and
`docs/language/04-grammar.md`, `06-expressions.md`,
`09-typing-reference.md`.  Amends the M0 freeze argument (decision 5).

## Context

The ladder has grown to 14 tiers.  The school-arithmetic core and the
SQL logic words carry universal conventions worth encoding, but every
operator the language has added since (`#`, the tacks, `??`) had to
be assigned a rank relative to operators it has nothing to do with,
and each assignment is a fact the reader must memorize or look up.
Precedent for refusing instead: Carbon orders precedence partially
and rejects unordered pairs without parentheses; Pony and Go flatten
or refuse mixing; in-language, the comparisons already refuse to
chain.  The goal is a shallow, learnable rule: school math plus SQL
logic, and parenthesize the rest.

## Decision

### 1.  An ordered spine, unchanged where convention is universal

The following relative orders remain exactly as today, loosest to
tightest:

```
|>
or / and        (one homogeneous level, decision 2)
not
== != < <= > >=, in, is    (non-associative, no chaining)
+ -
* /
- (unary)
^
application
#
.
```

Inside the spine the school conventions hold: `* /` and `+ -` chain
left (`9.8 * meter / second^2` is the dimensioned-value idiom), `^`
binds tighter than unary minus (`-2^2` is `-(2^2)`), `not` sits below
the comparisons (`not a == b` is `not (a == b)`, uniform with
`a < b and c < d` needing no parens), and `|>` stays loosest so
pipelines are noise-free.

### 2.  `and` and `or` are homogeneous

`and` and `or` share one level, and a chain must be one word:
`a and b and c` and `a or b or c` parse; `a and b or c` is a parse
error asking for parentheses.  The old ranking (`and` over `or`)
encoded a convention that is universal among language designers and
perpetually shaky among readers; a filter predicate is exactly where
that shakiness costs data.

### 3.  Unranked operators: the glue level

`??` and the four tacks `<< >> <: :>` have no rank against the
comparisons or against each other.  They share one parser level
between the comparisons and `+ -`, with these rules:

- An operand is arithmetic-or-tighter (an `add_expr`), so
  `a + b << c` is `(a + b) << c` and `r.previous ?? 0.0 * kelvin`
  stays paren-free.
- A self-chain keeps its associativity: `a ?? b ?? c` discharges
  right (ADR 0039), `a << b << c` folds left, and likewise for each
  tack on its own.
- Any other meeting is a parse error naming both operators and the
  fix.  Three meetings exist: a different unranked operator
  (`a << b >> c`, `a ?? b << c`), a comparison or `is` on either side
  (`a ?? b < c`, `a ?? b is known`), and a logic word over an
  unparenthesized unranked result (`a ?? b and c`).  Diagnostic
  shape: "mixing `??` with `<` needs parentheses (ADR 0040)".
- `|>` accepts anything: it is structural and loosest, and a
  pipeline stage ending in a discharge should not need parens.

The clamp idiom becomes `(a << hi) >> lo`, one pair of parens for an
expression whose reading order genuinely matters.

### 4.  The ladder is closed: new operators are born unranked

A future operator lands at the glue level with a self-chain and no
mixing.  A rank on the spine must be earned the way a combiner earns
its table row (ADR 0031): by decision record, citing a convention
universal enough that refusing it would surprise more than it
protects.  This is the combiner-table discipline applied to
precedence.

### 5.  The M0 freeze amendment

The grammar stays LL(1): every level still decides on one token, and
the glue level commits to its operator at the first glue token.  Two
side conditions are parser-enforced rather than grammatical, and the
M0 argument in `04-grammar.md` states them beside the FIRST/FOLLOW
table:

- **Homogeneous-chain commit**: the logic level and the glue level
  each commit to the first operator they see; a different same-level
  operator mid-chain is an error, not a parse branch.
- **Glue-meets-comparison/logic check**: an unparenthesized glue
  result may not be an operand of a comparison, an `is` test, or a
  logic word.  Parentheses clear the condition.

Both are single-token checks on already-parsed structure, so the
decidability argument is untouched.

## Consequences

Positive:

- The learnable rule fits in a breath: school math chains, one logic
  word chains, everything else takes parentheses.
- `??` needs no slot and never did: `(a < b) ?? false` and
  `r.peak ?? limit < t` both now say what they do, in the source.
- Future operators cost no ranking debate (decision 4).

Negative:

- `a and b or c`, `a << b >> c`, and `r.peak ?? limit < t` were legal
  and no longer are; the fix is always one pair of parens, and the
  diagnostic names it.  No shipped example or corpus case outside the
  clamp idiom is affected.
- Two parser-enforced side conditions live beside the grammar rather
  than in it (decision 5 accepts this).

## Alternatives considered

1. **Full flattening** (Pony: every mixed pair of binary operators
   takes parens).  Rejected: `9.8 * meter / second^2` is the
   dimensioned-value idiom, and taxing school arithmetic to simplify
   a rule nobody misreads protects nothing.
2. **Keep growing the total order.**  Rejected as the status quo that
   produced the `??` fiat: every new operator forces an arbitrary
   ranking against unrelated operators, and the ladder is already 14
   tiers deep.
3. **Strict same-level errors inside the school core** (make `* /`
   mixing an error too).  Rejected with alternative 1: those chains
   carry a universal convention, and the unit idiom depends on them.

## Forward references

- `docs/decisions/0031-fold-and-scan-primitives.md` (decision 6, the
  tacks' original total-order placement, superseded here).
- `docs/decisions/0039-missing-aware-expressions.md` (decision 2, the
  `??` slot this ADR replaces).
- `docs/language/04-grammar.md` (the glue and logic productions, the
  amended M0 argument).
- `docs/language/06-expressions.md` and `09-typing-reference.md` (the
  precedence tables, now the spine plus one unranked row).
