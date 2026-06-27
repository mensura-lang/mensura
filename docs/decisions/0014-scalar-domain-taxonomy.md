# 0014: Scalar domain taxonomy

## Status

Accepted.  Splits `number` into `int` and `real`, introduces a key-eligibility
and a finite-enumerable tier over the scalar domains, and fixes strict numeric
typing.  Implementation touches `mensura-types` (`model`, `resolve`,
`expr_check`, `pipe_check`), the language docs (`09`, `01-units`, `06`), the
book, and the examples/corpus.  Not yet implemented; the rollout is a follow-up.

## Context

The basic scalar domains are `string`, `number`, `bool`, `date`, and `enum`
(`crates/mensura-types/src/model.rs`, resolved in `resolve.rs`).  A single
`number` conflates two roles: a **discrete identity or count** (a year, an
integer id, a cardinality) and a **continuous measurement** (a temperature, a
mass).  These behave differently where it matters:

- **Keys.**  `extend_key` (`09-typing-reference.md` section 6.3) and a unit's
  index fields need a rule for which domains may form a key.  Standard database
  practice and the project's identity discipline (ADR 0001) want a key to be a
  stable, comparable identity, not a measurement; a floating-point key is a
  known anti-pattern (unreliable equality, `NaN`).  The formal model's `ungroup`
  requires only `DecidableEq`, but the language wants the stronger "not a
  continuous measurement."
- **Reshaping.**  Index `pivot` and `unpivot` (section 6.6) spread a key's
  distinct values into column *names*, which needs a finite, listable domain.

`01-units.md` currently uses `year: number` as an index field; this ADR revises
that.

## Decision

### Domains

Retire `number`.  Split it into `int` (discrete, exact, equality-stable: counts,
years, integer identifiers) and `real` (a continuous measurement).  Every column
states which it is; there is no ambiguous default.  The scalar domains are
`string`, `int`, `real`, `bool`, `date`, `enum`.

### Domain properties

Each scalar domain carries a set of properties that gate operations.  They are
predicates over the flat domain set, not a new "kind" layer.

| domain   | equatable | orderable | numeric | finite-enumerable |
| ---      | ---       | ---       | ---     | ---               |
| `string` | yes       | no        | no      | no                |
| `int`    | yes       | yes       | yes     | no                |
| `real`   | **no**    | yes       | yes     | no                |
| `bool`   | yes       | no        | no      | no                |
| `date`   | yes       | yes       | no      | no                |
| `enum`   | yes       | no        | no      | yes               |

- **equatable** gates `== !=`.  `real` is the lone exception: exact equality on
  a continuous measurement is unsound (the float-equality problem).
- **orderable** gates `< <= > >=` and `min`/`max`.  A `date` is ordered, so
  these work on dates; `string` is treated as an opaque identifier (no
  collation-dependent ordering), and `bool`/`enum` are unordered for now (an
  ordinal `enum` is a separate future choice).
- **numeric** gates `+ - * ^`, `/`, and `sum`.
- **finite-enumerable** gates the operations that spread a domain across column
  *names*: index `pivot` and `unpivot` (section 6.6).  Only `enum` qualifies;
  `bool` is excluded because `true`/`false` as column names is awkward and breaks
  the `pivot`/`unpivot` round-trip.  These ops are not yet implemented; the rule
  is frozen for when they land.

**Key-eligibility = equatable.**  An index/key column must be equatable **and**
total: a key is identified by equality, and `real` (the one non-equatable
domain) is exactly the one barred from keys.  Enforced on unit index fields
(`resolve`) and `extend_key` (`pipe_check`).

### Operator typing (strict, no coercion)

- **Literals.**  An integer literal is `int`; a decimal literal is `real` (the
  lexer already emits distinct tokens).
- **`+ - * ^`** (numeric).  Both operands the same numeric type; the result is
  that type.  An `int` mixed with a `real` is a type error.
- **`/`** (numeric).  Defined on `real` only (true division yields a fraction).
  Integer division is not provided this round (a future `div`/`mod` may add it).
- **Ordering `< <= > >=`** (orderable).  Both operands the same orderable domain
  (`int`/`int`, `real`/`real`, `date`/`date`).
- **Equality `== !=`** (equatable).  Both operands the same equatable domain,
  plus the enum/string-literal exception of section 5.6.  Not defined on `real`,
  so `temperature > 30` is fine and `temperature == 30` is an error.
- No implicit `int`/`real` widening anywhere; conversion is explicit.

### Aggregates

Every aggregate requires a **total** bag (an optional column cannot be
aggregated until value narrowing exists, deferred).  Signatures:

- `count : bag<any> -> int`
- `sum` (numeric) : `bag<int> -> int`, `bag<real> -> real`
- `min`, `max` (orderable) : `bag<T> -> T` for `T` in `int`, `real`, `date`
- `any`, `all` : `bag<bool> -> bool`

`mean` is **not** a primitive.  It is `sum(x) / to_real(count(x))` for a `real`
bag, and `to_real(sum x) / to_real(count x)` for an `int` bag.  Fractional
statistics are derived this way; named sugar may reintroduce `mean` later.

### Conversion

`to_real` is a context builtin: `int -> real` on a value, lifted element-wise
over a bag (`bag<int> -> bag<real>`).  The `real -> int` direction (a
`round`/`floor`/`ceil`/`trunc` family) is deferred; turning a measurement into a
key is rare and wants an explicit, named rounding choice.

### Expression-typing model

`Ty::Bag` (in `expr_check`) gains a totality so an aggregate can demand a total
bag.

## Consequences

Positive:

- A key is exactly a stable, comparable identity; a continuous measurement can
  never be one, matching ADR 0001 and standard key discipline.
- The `int`/`real` split distinguishes counts/identities from measurements at
  the type level, and `real` is the precursor that M3's dimensional measures
  refine.
- The builtin set stays minimal (no `mean`); derived statistics are expressed
  from primitives, following the project's "general primitives, sugar later"
  discipline.
- The numeric rules are unambiguous and conservative: no silent coercion, no
  float equality, no integer-division surprise.

Migration:

- Every existing `number` column reclassifies: `01-units.md`'s `year: number`
  becomes `int`, measurement columns become `real`.  The examples, the book, and
  the CLI corpus migrate with it.
- `mean` usages in docs, examples, and merged tests (the `machine_temperature`
  view in `10-views.md`, the `pipe_check`/`expr_check` tests) rewrite to
  `sum`/`count`/`to_real`.
- `expr_check` (already built) changes: literal split, arithmetic/ordering/
  equality rules, aggregate signatures, `Ty::Bag` totality, the `to_real`
  builtin.  `pipe_check` adds the `extend_key` key-eligibility check, and
  `resolve` adds it to unit index fields.

Neutral:

- The continuous flavor is a placeholder; M3 attaches dimensional units and
  precision to `real` without reopening this taxonomy.
- `real` remains fully usable in expressions (ordering, arithmetic, `sum`/`min`/
  `max`); only `==`/`!=` and key positions exclude it.

## Alternatives considered

1. **Keep a single `number`.**  Rejected: it conflates identity and measurement
   and cannot gate keys.
2. **Keep `number` as continuous, add `int` (least churn).**  Rejected in favor
   of retiring `number` outright, so there is no ambiguous default and every
   column states discrete or continuous.
3. **Implicit `int -> real` widening / a numeric tower.**  Rejected for now:
   silent coercion hides the count-versus-measurement distinction; strict
   matching plus explicit `to_real` is unambiguous and revisable.
4. **Include `bool` in the finite-enumerable tier.**  Rejected: `true`/`false`
   as spread column names is awkward and breaks reversibility.
5. **`mean` as a primitive aggregate.**  Rejected: derivable; keeps the
   primitive set small.
6. **An `as` cast keyword for conversion.**  Rejected in favor of a `to_real`
   builtin: no grammar change, and consistent with builtins being a property of
   the context (`06-expressions.md`).

## Open questions

- Integer division (`div`/`mod`) and the `real -> int` rounding family: surface
  and semantics, when first needed.
- Ordinal `enum`s and `string` ordering: whether either joins the orderable set
  later.  `date` is orderable here but not numeric; `date` arithmetic (durations
  between dates) is also left open.
- Exactly how M3's dimensional units and precision refine `real`.
- Reintroducing `mean` and other statistics as named sugar.
- Missing-aware aggregates (working over an optional bag) once value narrowing
  lands.

## Forward references

- `docs/decisions/0001-unit-as-identity-discipline.md` (keys are identities),
  `docs/decisions/0013-qualifier-scope-and-the-content-boundary.md` (the
  `extend_key` totality constraint).
- `docs/language/09-typing-reference.md` (section 5 expression rules, section
  6.3 `extend_key`, section 6.6 `pivot`/`unpivot`), `docs/language/01-units.md`
  (index field types), `docs/language/06-expressions.md` (numeric and
  conversion).
- `ROADMAP.md` M3 (physical units, precision, measure semantics) refines `real`.
