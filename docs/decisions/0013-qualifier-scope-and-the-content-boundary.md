# 0013: Qualifier scope and the content boundary

## Status

Accepted.  Revises the `C`/`Qs` boundary set by
`docs/decisions/0004-qualifier-mechanism.md` and the "by arity" split frozen in
`docs/language/09-typing-reference.md`, section 1.  ADR 0004 keeps its number
and its extensibility decision; this ADR only redraws the boundary between its
two parts.  The rewrite of `09` sections 1, 3.1-3.5, and 4 and the
`docs/language/10-views.md` cross-references land with this acceptance.

## Context

ADR 0004 collapsed the original `Table<S, D, L, C>` quadruple into
`Table<Qs, C>`, where `Qs` is an open row of **qualifiers** (library-definable
properties with a state space, per-primitive propagation rules, and an optional
constraint hook) and `C` is the content schema.  The point was extensibility:
future invariants (privacy budget, freshness, PII taint, units) become library
qualifiers, not language changes.

The M0 freeze (`09-typing-reference.md`, section 1) then made `Qs` **concrete
and closed** for now (completeness and lineage) and assigned the four tracked
properties to `C` or `Qs` **by arity**: per-column and per-key facts live in
`C`, table-level facts live in `Qs`.  Under that rule totality (per-column)
sits in `C`, completeness (per-table) sits in `Qs`, and cardinality (described
as "per key" but uniform, so attached to the table) sits in `C`.

The arity boundary was decided before indexes, keys, totality, completeness,
and disjointness had matured as concepts, and it now reads as artificial.
Totality and completeness are the same *kind* of thing: propagated facts with
the establish / propagate / demand / assume surface that `08-lineage.md` and
section 8 of the reference describe.  Splitting them by arity puts one in the
schema and one in the qualifier row for a reason (where the value is indexed)
that has nothing to do with what they are.

The deeper cause is a limitation, not a principle: a *table-level* qualifier
row cannot hold a *per-column* value, so per-column facts were pushed into `C`.
This does not scale.  Units and precision (M3) and PII taint are all
per-column; under the arity rule they would keep accreting in `C` while
behaving exactly like qualifiers, and the extensibility ADR 0004 promised would
not actually reach them.

Extensibility remains the long-term target (the open qualifier row must
survive), so the reorganization has to keep a home for future library
qualifiers, including per-column ones.

## Decision

1. **The boundary is structure versus propagated fact, not arity.**
   - `C` is pure **structure**: index columns, non-index columns, and their
     domains.  What the data *is*.  Nothing propagated lives here.
   - `Qs` is every **propagated fact**: each is a qualifier with a declared
     scope.  Cardinality, totality, completeness, and lineage all move to (or
     stay in) `Qs`.

2. **Scope is a first-class field of a qualifier.**  A qualifier declares which
   structural node it rides:
   - **`table`**: one value for the whole table.  The value may be a *universal
     over keys* (a statement that holds for every key).  Completeness, lineage,
     and cardinality are table-scoped; future freshness and privacy budget fit
     here.
   - **`column`**: one value per column, spanning **both index and non-index
     columns**.  Totality is column-scoped; future units, precision, and PII
     taint fit here.

3. **There is no per-key scope.**  A fact "about keys" is one of:
   - a universal over all keys, which is **table**-scoped (cardinality,
     completeness, the disjointness invariant); or
   - a fact attached to an index column, which is **column** scope landing on a
     column that is part of the key.

   The type checker cannot track a value that varies per runtime key-tuple: the
   key set is data-dependent and unbounded, so the type level holds only a
   single value, a per-(static)-column value, or a universal over the dynamic
   key set (which collapses to one table-level value).  The enum-keyed table
   (a statically known, finite key set) is the one case where per-key-tuple
   facts are statically meaningful, and that case is already carried at table
   scope by the lineage tag tree (partition structure), not by a per-key map.

4. **Cardinality is table-scoped.**  Section 3.2's "per key" describes the
   *subject* of the bound (rows at a key), not its scope.  Type-level
   cardinality is necessarily a single uniform upper bound that holds for every
   key (a `left_join` against a non-functional right table, a mixed group, or
   uneven data all type as the worst case, `bag`), so it is one table-level
   value.  The "uniform across keys, so it attaches to the table" hedge in the
   current text disappears: there was no per-key scope to hedge against.

5. **"Index columns are always total" becomes a qualifier constraint, not a
   structural axiom.**  Because column scope spans index and non-index columns,
   `extend_key` moves a column (and its column-scoped values) into the index.
   The totality qualifier registers a constraint that fires on the non-index to
   index transition: a column must be total to enter the key, so an optional
   column must be narrowed first (`is known`, a default, or a
   missingness-aware aggregate, ADR 0010).  An existing hardcoded law thus
   becomes one instance of the qualifier mechanism.

6. **Built-in properties are written in the same form as future library
   qualifiers.**  Completeness, lineage, cardinality, and totality are simply
   the first entries in the open row.  A library author defining a per-column
   `pii` qualifier and the built-in per-column `totality` qualifier write the
   same kind of object, differing only in scope and state space.  That
   uniformity is the property that makes the eventual metaprogramming surface
   (ADR 0004's rule-combinator DSL) intuitive.

## Why per-key is not a scope (the stress test)

The taxonomy above was checked against the hardest case, a qualifier that
seems to want a value per key.

"Per-key" conflates two unrelated things.  The first is a fact *quantified over
all keys*: "for every key, card <= 1" (cardinality), "every key's bag is full"
(completeness), "at every key at least one side is empty" (disjointness).  The
value does not vary across keys; it is one universal statement, so its scope is
the whole table.  The second is a fact *attached to an index column*: a future
ordering or monotonicity fact for window functions (`rank`, `cumsum`, section
6.2), or a sampling stratum marker.  That is not a new scope; it is column
scope on a column that sits in the key.

What cannot exist at the type level is a qualifier whose value genuinely
differs per key-tuple, because the key set is runtime data.  Every static
candidate reduces to table scope (a universal) or column scope (a per-named-
column value).  So the scope set is exactly `{table, column}`, and the per-key
worry resolves into a clarifying distinction: the *subject* of a fact (often
keys) is independent of its *scope* (table or column).

The one wrinkle this surfaces is reindexing.  Column-scoped values must follow
their column when `extend_key` / `shrink_key` move it between the index and the
non-index part; the framework owns that threading, derived from the declared
scope, so a qualifier author writes a single per-primitive rule rather than a
re-keying step.  Totality's transition constraint (point 5) is the first
concrete instance.

## Consequences

Positive:

- The boundary is principled: `C` is what the data is, `Qs` is what is tracked
  about it.  Totality and completeness are recognized as the same kind of fact.
- The open row now hosts per-column qualifiers, so units and precision (M3) and
  PII taint join `Qs` with no language change, which is the extensibility ADR
  0004 actually promised.
- The metaprogramming surface is uniform: built-in and library qualifiers share
  one shape (state space, scope, per-primitive rules, optional hook).
- A hardcoded law ("index columns are total") is explained as a qualifier
  constraint rather than asserted.
- The scope taxonomy is two-valued, simpler than the three-bucket alternative.

Negative:

- Cardinality and totality move out of `C`.  `09-typing-reference.md` sections
  1, 3.2, and 3.3 must be rewritten, and `10-views.md`'s "Properties at the
  view boundary" cross-references updated (the four properties are still all
  present; only their grouping changes).
- The in-progress M1 `Table<Qs, C>` model changes: `C` becomes pure structure,
  and cardinality / totality become scoped qualifiers.  The scalar rule
  (section 5.3) now reads two qualifiers (cardinality at the read site,
  totality on the column) instead of one structural field.
- The framework must thread column-scoped values through reindexing and run
  transition constraints, machinery the closed-pair model did not need yet.

Neutral:

- `store` and `collect` stay pinned to `singletons` (ADR 0001): cardinality is
  now a table-scoped qualifier whose value is fixed at those boundaries, rather
  than a structural field.  The storage index to PRIMARY KEY mapping is
  unaffected.
- Shape conformance still checks structure (`C`) only (`10-views.md`,
  "Constraining a view with a shape"), so it is unaffected by the move: it
  never inspected qualifiers.
- `Qs` stays concrete and closed for the M0 freeze (completeness, lineage, and
  now cardinality, totality); reopening it to library qualifiers is still the
  ADR 0004 meta-calculus follow-up.

## Alternatives considered

1. **Keep the arity boundary.**  Rejected: it is the source of the artificial
   split and does not scale to the per-column M3 properties.

2. **Three explicit groups by scope** (structure, per-column facts, per-table
   facts).  Honest about arity by naming it, but it turns the open row into two
   rows (column-scoped and table-scoped), does not unify the four properties as
   one kind of thing, and leaves cardinality's "per key but uniform" framing
   awkward.  Rejected as a relabeling that does not dissolve the artificiality.

3. **Attributed schema, qualifiers attached inline to structural nodes** (no
   top-level `C`/`Qs` split).  Cleanest at the use-site (everything about a
   column in one place) and most faithful to "properties from mechanism", but
   it smears a qualifier's definition across node attachments, which fights the
   rule-combinator DSL's "one qualifier is one state space and one set of
   per-primitive rules".  It also discards the familiar two-part notation for
   more generality than M1 needs.  Recorded as the limit Approach 1 approaches
   if pushed; revisit if the use-site reading cost of the row model proves high.

## Open questions

- **Scope and the rule-combinator DSL (ADR 0004 open question).**  Does the
  author declare scope and the framework derive reindex-threading and
  transition constraints from it, or must some rules be written per transition?
  This ADR assumes the former.
- **Dependency and sampling scope.**  Ordering for windows looks like column
  scope on the order key; sampling looks like column scope on a stratum.
  Confirm when those qualifier documents are written (`09`, section 13).
- **Cardinality's standing.**  `exhaustive = singletons and completeness`
  (section 3.4) already derives one corner from two facts.  Whether cardinality
  stays a standalone table qualifier or is expressed in terms of others is left
  open.
- **Per-index-column versus per-non-index-column.**  If a future qualifier
  needs to distinguish them, this is expected to be a constraint on a
  column-scoped qualifier (as totality's index rule is), not a third scope.

## Forward references

- The qualifier mechanism and its extensibility goal are in
  `docs/decisions/0004-qualifier-mechanism.md`.
- The four tracked properties and the algebra that threads them are in
  `docs/language/09-typing-reference.md`; this ADR redraws how sections 1, 3.2,
  and 3.3 group them.
- The view boundary that surfaces all four properties is in
  `docs/language/10-views.md`.
- The unit-boundary `singletons` discipline for `store` / `collect` is
  `docs/decisions/0001-unit-as-identity-discipline.md`; the totality axis is
  `docs/decisions/0010-attribute-totality.md`.
