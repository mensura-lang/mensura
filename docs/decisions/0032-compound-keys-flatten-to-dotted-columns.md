# 0032: Compound keys flatten to dotted columns

## Status

Accepted.  Extends `docs/decisions/0002-stores-tabulate-units.md`: the
`domain` block specified there is resolved by the mechanism decided here.
Settles the representation questions that `docs/language/01-units.md`
("hierarchical at the unit level, flat at the math level") and
`docs/language/02-stores.md` leave open, ahead of the M4 implementation
of compound units and `domain` resolution.

## Context

A compound unit's key is a tree: `Course`'s key is
`(department: (code: string), name: string, year: int)`.  The spec fixes
the access syntax (`course.department.code`) and the mathematical stance
(flattening a hierarchical key gives a flat tuple of scalars; the
algebra's typing rules apply unchanged).  It does not fix:

- how flattened columns are **named** in the resolved schema, the
  boundary IR, and storage;
- how the expression checker presents the hierarchy so that
  `course.department.code` typechecks as written;
- where foreign-key metadata lives and whether SQLite `FOREIGN KEY`
  clauses are emitted;
- whether the *unit* key-reference graph needs its own acyclicity check
  (02-stores.md requires acyclicity only of the *store* `domain` graph);
- whether a `domain` entry may target a `bag` store;
- how `?` (optionality) interacts with a unit-reference attribute.

The implementation constraints: the resolved `Schema` is a flat ordered
column list over a closed scalar `ColumnType`; the checker's content
model, gradings (ADR 0024), and shape conformance all resolve columns by
flat name; and the checker deliberately has no value-tuple type
(ADR 0031 records the tuple-key gap).  A compound key must therefore be
flattened before it reaches the algebra, exactly as 01-units.md
prescribes.

## Decision

1. **Nested in the checker, dotted at the boundary.**  The resolver
   flattens a unit-reference field depth-first, in declaration order,
   into columns named by the access path with `.` as the separator: an
   `Enrollment` field `course: Course` becomes the columns
   `course.department.code`, `course.name`, `course.year`.  The resolved
   schema, the boundary IR, gradings, shape conformance, and storage all
   use these flat dotted names; SQLite quotes them like any identifier.
   The expression layer presents the same columns as nested record
   groups, so `k.course.department.code` typechecks by ordinary member
   access.  The hierarchy is presentation; the dotted flat form is the
   single source of truth.

2. **Whole-row forwarding flattens back.**  When a view body forwards a
   row whose type contains a unit-reference group, the group is
   flattened back to its dotted columns and the output schema orders the
   flat names alphabetically, the same rule the evaluator already uses
   for row records.  Computing a *new* unit-reference group inside a
   view body (a record literal whose field is itself a record) stays
   rejected.

3. **Foreign-key metadata.**  The resolved `Schema` carries one
   `ForeignKey` entry per `domain` entry: the unit-reference field name,
   the referenced unit, the target store, the pairwise child/parent
   column lists (child columns are the field's flattened paths; parent
   columns are the target unit's flattened key paths), and whether the
   reference sits in the key or in the attributes.  The store's boundary
   `TableShape` carries the same entries; a view's carries none.

4. **Two acyclicity checks, independent of each other.**  (a) The unit
   key-reference graph must be acyclic: a unit whose key references
   itself, directly or through other units, has no finite key tree, so
   flattening would not terminate.  This check is per-unit and needs no
   stores.  (b) The store `domain` graph must be acyclic, as
   02-stores.md already requires.  The two do not subsume one another:
   two stores of basic units can form a store cycle through
   unit-reference *attributes* with no unit cycle in sight.

5. **`domain` targets must be `singletons` stores.**  "Constrained to
   values that appear as observations" is ill-defined against a `bag`,
   where an entity's presence is incidental to its readings;
   02-stores.md already directs per-entity facts to a companion
   `singletons` store "joined via `domain`"; and a `bag` store has no
   primary key for a foreign key to reference.  A `domain` entry naming
   a `bag` store is a compile error.

6. **`FOREIGN KEY` clauses are emitted now, unenforced.**  The storage
   backend emits one clause per `ForeignKey` entry in `CREATE TABLE`.
   SQLite accepts the clauses regardless of table-creation order and
   enforces them only under `PRAGMA foreign_keys = ON`, which the
   backend does not set: there is no write path until the ingestion
   slice, so enforcement has nothing to act on yet.  Whether ingestion
   turns the pragma on is that slice's decision.

7. **Collision rules.**  Identifiers cannot contain `.`, so a flattened
   dotted name can never collide with a user-declared column.  The
   *base* name of a unit-reference field reserves its whole prefix in
   the duplicate-column check: a store cannot declare both a
   unit-reference field `coordinator` and another column named
   `coordinator`, and two unit-reference fields cannot share a name.

8. **Deferred, recorded.**  Each of the following is rejected with a
   "not yet supported" diagnostic rather than designed here:

   - `?` on a unit-reference attribute.  The group's columns are
     missing all-or-nothing, a constraint per-column `NOT NULL` cannot
     express; the interaction gets designed when a consumer needs it.
   - Key moves (`promote`, `demote`) and reshape selectors naming a
     flattened component or a unit-reference group.
   - Unit-reference attributes in *shapes*.  Conformance semantics
     without a `domain` block are unsettled; the resolver keeps
     rejecting them, with a message pointing here.

## Consequences

Positive:

- Everything downstream of the resolver keeps operating on flat column
  names: gradings, join matching, shape conformance, the boundary IR,
  and the SQLite mapping need no structural change.
- `course.department.code` typechecks exactly as 01-units.md writes it,
  because nesting is reconstructed at the record boundary where member
  access already composes.
- Referential structure is visible in the stored schema (dotted columns
  plus `FOREIGN KEY` clauses) without any enforcement machinery landing
  before ingestion exists.

Negative:

- Dotted column names leak into anything that displays raw storage
  (SQL debugging, positional seeding in runtime tests must follow the
  flattened order).
- The checker's nested presentation and the evaluator's nested runtime
  records must mirror each other; the flatten-then-sort rule for
  whole-row forwarding is what keeps them aligned, and it is subtle.

Neutral:

- The singletons-only target rule forbids pointing a `domain` entry at
  a `bag` store even for "just documentation" purposes; a companion
  `singletons` store expresses that intent instead.

## Alternatives considered

1. **A nested column variant in the resolved model.**  Every consumer of
   the IR (DDL, encode/decode, the evaluator's source tables, shape
   conformance) pattern-matches scalar columns; a nested variant forces
   churn everywhere for a hierarchy the spec explicitly calls
   "presentation, not a new mathematical object".  Rejected.

2. **Underscore-joined flat names (`course_department_code`)
   everywhere, including expressions.**  Diverges from the access
   syntax 01-units.md fixes, and underscores collide with legal
   user-declared names, requiring a renaming scheme.  Rejected.

3. **Deferring foreign-key emission entirely to the ingestion slice.**
   Costs a second pass over the DDL later and leaves created schemas
   silent about referential structure in the meantime.  Emitting
   unenforced clauses now is strictly more informative.  Rejected.

4. **Allowing `bag` stores as `domain` targets** with "appears at least
   once" semantics.  Requires FK-less integrity checking at write time
   and contradicts the companion-store guidance already in
   02-stores.md.  Rejected.

## Open questions

- Whether ingestion (M4's later slice) turns `PRAGMA foreign_keys` on,
  and what diagnostic surface a violated reference gets at write time.
- The `?`-on-group interaction (deferred above) if a consumer appears.
- Surface syntax, if any, for projecting a whole unit-reference group
  as a value (today only leaf access and whole-row forwarding exist).
