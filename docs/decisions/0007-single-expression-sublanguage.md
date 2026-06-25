# 0007: A single expression sublanguage

## Status

Accepted.  Realized in `docs/language/06-expressions.md` and the expression
grammar in `docs/language/04-grammar.md`.

## Context

Several places in the language need to evaluate an expression over named
values, and they have so far been sketched independently:

- role predicates, `when: principal.kind == "device" and ...` (ADR 0005);
- instance-level authorization, `where: row.machine == lookup(principal).machine`
  (ADR 0005);
- auto-filled fields, `@auto(auth.id)`, `@auto(now)`, `@auto(prev.number + 1)`
  (proposal.md);
- pipeline operations, the `filter`, `mutate`, `aggregate`, and `case`
  expressions in `transform`/`view` blocks (proposal.md, iiot.md), which are
  the M3 algebra surface.

These are the same kind of thing: comparisons, boolean connectives, membership
tests, member access, literals, and a small set of builtins, evaluated against
a context of named values.  The grammar currently defines no expression syntax
at all (see `docs/language/04-grammar.md`).  If each site grows its own
expression syntax, the language ends up with several near-identical grammars
that later have to be reconciled, and the LL(1) freeze has to absorb each one
separately.

## Decision

There is **one** expression sublanguage, shared by every site that evaluates
an expression: `when:`, `where:`, `@auto(...)`, and the pipeline
`filter`/`mutate`/`aggregate`/`case` forms.  It is defined once, as an LL(1)
grammar, and each site differs only in:

- the **context** it exposes (the names in scope: `principal`, `row`, `jwt`,
  `now`, `prev`, pipeline columns, ...); and
- the **result type** it requires (a boolean for `when:`/`where:`/`filter`, a
  value for `@auto`/`mutate`, an aggregate for `aggregate`).

The grammar and the type rules are written once; a site is a (context,
expected-type) pair over that single language.  Whatever subset is specified
first (the auth predicates are the immediate need) is a subset of this one
language, not a separate dialect, and later work extends the same grammar
rather than introducing a parallel one.

## Consequences

Positive:

- One grammar to make LL(1), one set of type rules, one evaluator, one set of
  diagnostics.  Soundness and tooling are argued once.
- The auth predicates and the M3 pipeline expressions cannot drift into
  incompatible syntaxes, because they are the same syntax.
- A reader learns one expression language for the whole surface.

Negative:

- The expression language is logically part of the M3 algebra surface, but the
  M4 auth work needs it first.  Defining it now pulls an M3 concern forward,
  and the initial spec must be designed not to paint M3 into a corner.
- A single grammar must satisfy the union of needs (authorization context,
  pipeline context, auto-field context) while staying LL(1).  That is a
  stronger constraint than any one site would impose alone.

Neutral:

- Context and result type are per-site concerns layered on top of the shared
  grammar; they are not part of this decision beyond requiring that the grammar
  be parameterizable by them.

## Alternatives considered

1. **A separate predicate grammar per site.**  Simplest to start, since each
   site specs only what it needs.  Rejected: it produces several grammars that
   must later merge, multiplies the LL(1) and diagnostics work, and risks
   syntactic drift between auth predicates and pipeline expressions.

2. **A minimal auth-only predicate grammar now, generalized in M3.**
   Considered.  Rejected as the framing (though not as a scoping choice): the
   *initial specified subset* may well be auth-only, but it must be declared as
   a subset of the single language from the start, so M3 extends it instead of
   replacing it.  The decision here is that there is one language; how much of
   it is specified first is a scoping question, not a second grammar.

## Open questions

- **Operator and builtin set.**  Which operators (`==`, `!=`, `<`, `in`, `and`,
  `or`, `not`, arithmetic) and which builtins (`lookup`, `env`, `now`, `next`,
  aggregates like `sum`/`mean`/`min`/`max`) are in the core, and how is the set
  extended?
- **LL(1) shape.**  Precedence and associativity have to be expressed in an
  LL(1)-compatible way (precedence-climbing in the recursive-descent parser, or
  a layered grammar).  This must be designed against the M0 freeze condition.
- **Context model.**  How a site declares the names it exposes (`principal`,
  `row`, `jwt`, `now`, `prev`, columns) and how member access (`a.b.c`) is
  typed against it.
- **Result-type discipline.**  How the expected type per site (boolean, value,
  aggregate) is checked, and how aggregates are distinguished from scalar
  expressions in `aggregate` versus `mutate`.
- **First specified subset.**  Which milestone fixes which part: the auth
  predicates (M4) versus the full pipeline expression language (M3).
