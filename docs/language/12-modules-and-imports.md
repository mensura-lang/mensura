# Modules, imports, and top-level bindings

This document specifies the module system's implemented surface: top-level
`let` bindings, the `import` item, and bundled resolution.  The model is
decided in `docs/decisions/0027-modules-and-imports.md`; the first client
is the `si` standard library
(`docs/decisions/0028-standard-library-si.md`), and the motivating feature
is dimensional units (`11-physical-units.md`, ADR 0026).

## Top-level `let`

`let` is lifted from the statement block (`06-expressions.md`) to item
position, where, like every other item with a body, it is **brace-closed**
(a top-level item has no terminator, so an unbraced expression body would
swallow the next item's leading keyword; `04-grammar.md`).  It binds one
of two kinds, disambiguated by the token after the name:

- **A const value binding**: `let name { ... }` (an optional `: type`
  ascription sits before the brace).  The body is the ordinary statement
  block a `view` hosts: local `let`s and a trailing result expression,
  evaluated at compile time.  The const evaluator, not the grammar,
  bounds what a constant may compute: literals and arithmetic
  (`+ - * / ^`, unary minus, grouping, member access) over the intrinsic
  base units, imported module members, other top-level bindings, and the
  block's local `let`s; and **lambdas and their application**
  (ADR 0030), below.  Anything effectful or table-shaped (pipelines,
  aggregates) is rejected as "not a const expression"; an `assert` in a
  const block is not yet supported.

  ```mensura
  let km { 1000.0 * meter }
  let newton { kilogram * meter / second^2 }
  let limit: temperature[real] { 350.0 * kelvin }
  ```

- **A const function binding** is the same kind: a value `let` whose
  result is a lambda (`docs/decisions/0030-const-functions.md`).  The
  closure captures the block's locals by value; a parameter shadows a
  captured local, and both shadow a top-level binding.  Two rules give
  the surface its shape.  A multi-parameter lambda is **tupled**:
  `|a, b| e` binds one parameter that is a 2-tuple, so currying is
  written explicitly as nested lambdas.  And every application is
  **saturated or an error**: partial binding is ordinary application of
  a curried function, never a mechanism of its own.

  ```mensura
  let add   { |a| |b| a + b }     // curried
  let add1  { add 1 }             // a function, by ordinary application
  let three { add1 2 }            // 3, at compile time

  let addt  { |a, b| a + b }      // tupled: addt (1, 2); `addt 1` errors
  ```

  A function binding cannot carry a `: type` ascription (the type
  grammar has no arrow), a function never enters a column, and a view
  body can *use* a const function but not *create* one.  Recursion is a
  compile error, caught definitionally where possible and by the
  evaluator's step budget otherwise.

- **A dimension alias**: `let name[T] { <type-level expression> }`
  (`11-physical-units.md`; ADR 0026, Decision 8).  The bracketed
  parameters mark the binding as type-level, and the braced body is
  parsed with the type grammar.

  ```mensura
  let speed[T] { (length / time)[T] }
  ```

Top-level bindings are **immutable, pure, and order-independent**: they
name values, not effects, so a binding may reference another declared
later in the file.  They are **non-recursive**: a reference cycle among
bindings (or aliases) is a compile error.  Casing follows the term/type
split (`05-naming-and-casing.md`): a value binding is a snake_case term
at every scope, and an alias name is a lowercase type name like the
built-in dimensions.

A const binding is a value, not a table: using one at pipeline position
is an error ("`km` is a constant, not a table").  Inside view bodies,
const names are resolved from the ambient environment and are
constant-folded before evaluation; a const *function* application is
**beta-reduced** at lowering (`add1 r.x` reaches the runtime as
`r.x + 1`), which is sound and total because only a const binding can
create a function, so every closure at every call site is statically
known.  Either way the runtime never sees a const name or a function
(ADR 0030, "first-class in the checker, inlined in the backend").

## Imports

```mensura
import si
```

- **A module is a file** that exports its const bindings and type-level
  names (dimension aliases).  It does not export stores, views, or
  pipelines: those are materialized, site-specific resources
  (ADR 0027, Decision 2).  A `registry` is never importable: its
  completeness guarantee comes from being the sole intake for its
  observations, and a second consumer would break it silently
  (`13-registries.md`).  This constrains the *module* boundary only; a
  `domain` edge inside one program consumes no observations and is
  unaffected.
- **Qualified by default.**  `import si` brings the module in under its
  name; members are referenced `si.km`, `si.newton`.  There is no glob
  import.  A selective-unqualified `exposing` form is a contemplated
  refinement, not implemented (ADR 0027, Open questions).
- **Collisions are compile errors, not shadows.**  An import is a
  disjoint union of environments: an import whose name collides with
  another import, a top-level binding, or an intrinsic is an error,
  matching the resolver's existing duplicate policy.
- **Acyclic.**  Module imports form a DAG; a cycle is a compile error.
- **A bare import resolves `bundled`, and only `bundled`** (ADR 0027,
  Decision 6): it names a module that ships with the toolchain (`si`, `bag`,
  and `series` today), needs no manifest and no network, and cannot be
  remapped.  An
  unknown name is a compile error at the import site.  The
  manifest-resolved marked form for third-party modules is provisional
  and not implemented (ADR 0027, Decision 7).

## The intrinsic / library split

The language provides a deliberately small **initial environment**: the seven
base units (`second`, `meter`, `kilogram`, `ampere`, `kelvin`, `mole`,
`candela`; ADR 0026, Decision 6), the reduction primitives `fold`, `scan`,
`prescan`, and `map`, the order marker `desc`, `to_real`, and the pipeline
operations.

ADR 0031 Decision 8 **amends ADR 0027 Decision 4** here: the aggregate
combinators that decision named as intrinsics have left.  With `fold` a
builtin there is no reason to keep so many *names* in the language, so
`sum`, `min`, `max`, `any`, `all`, and `prod` are const bindings in the
bundled `bag` module, `count` became the `#` operator, and the freed names
returned to users.  The ordered siblings live in the bundled `series` module,
in the same way and for the same reason: `cumsum`, `running_min`,
`running_max`, `first_value`, `lag`, `lead`, and `rank`, each a partial
application of `scan` or `prescan` (ADR 0029's Stage 2 gates them,
`formal/Mensura/Arranged.lean`).

The consequence is that **"no implicit prelude" now holds without
exception**.  `si` was already an ordinary import (`9.8 * meter / second^2`
type-checks with no import, while `9.8 * si.m / si.s^2` requires
`import si`), and the aggregate vocabulary is too: `bag.max b.temperature`
requires `import bag`, and without it `bag` is simply an unknown name.  A
qualified name reads one token longer than a bare one; that is the price of
the rule holding uniformly.

**Bundled modules may export functions.**  `bag`'s members are partial
applications of `fold`, so a module member can be applied
(`bag.max b.temperature`) and piped into (`b.temperature |> bag.max`) exactly
as a top-level `let` function can (ADR 0030).  `si`, which exports only
scalars, is the simpler case rather than the general one.  `series` goes one
step further: `lead` is a genuine lambda whose body returns a partially applied
primitive, so applying it walks a closure *and then* a builtin's slot rules.

Inside a lambda, a parameter may reuse an ambient name (`|meter, r| ...`);
the parameter wins locally.  This is ordinary lexical scoping, not the
top-level collision rule, and it is what keeps existing programs typing
when new intrinsics arrive.

## Diagnostics and spans

Spans carry byte offsets into one file, so a diagnostic that originates
*inside* a bundled module is reported at the importing `import` item's
span, prefixed with the module name (``in module `si`: ...``).  Bundled
modules are compiled in this repository's CI, so such a diagnostic is
effectively an internal error.  Attributing spans across files (a file
identity on `Span`) is the prerequisite for the third-party layer, where
module-internal diagnostics become user-facing; it lands with that layer.

## Grammar

```ebnf
item        = ... | let_decl | import_decl ;
let_decl    = "let" ident ( value_let | alias_let ) ;
value_let   = [ ":" type ] block ;
alias_let   = "[" ident { "," ident } "]" "{" tl_expr "}" ;
import_decl = "import" ident ;
```

`let` and `import` are contextual keywords in item position, like the
five existing declarations; the LL(1) argument is in `04-grammar.md`.

## Deferred

- The third-party distribution layer: the manifest (`mensura.toml`),
  hashes, `mensura pin`, the marked import form, and the check artifact
  (ADR 0027, Decision 7; provisional, no consumer yet).
- `exposing` lists (selective unqualified import).
- Module-qualified names in *type* position (`geo.speed[real]`): the type
  grammar has no `.` yet, so the form does not parse; it lands when a
  bundled module first exports an alias.  For the same reason a module
  that *declares* a dimension alias is rejected (its type-level exports
  would be unreachable).
- Imports *inside* a bundled module (with the DAG check they imply):
  `si` imports nothing, so module-internal imports are rejected with
  "not yet supported" until a second bundled module needs them.
- File identities on spans (above).
- Value-level `let` statements inside view blocks remain table-valued
  only; scalar block `let`s are a separate follow-up.
- `assert` statements inside a const block (compile-time checked
  assertions); rejected with "not yet supported".
- An arrow in the type grammar, so a function binding could be ascribed
  (ADR 0030, Open questions).
- Lambdas as values inside view bodies (the prerequisite for
  higher-order pipeline operations), and runtime closures generally: a
  view uses a const function; it does not create one.
- A bundled module that exports a function: permitted by the model
  (ADR 0030, Decision 8), shipped by nothing until span provenance
  lands (above).

## Forward references

- The decision record: `docs/decisions/0027-modules-and-imports.md`.
- Const functions: `docs/decisions/0030-const-functions.md`.
- The `si` library and its discipline:
  `docs/decisions/0028-standard-library-si.md`.
- The units feature this ships: `11-physical-units.md` (ADR 0026).
- The grammar: `04-grammar.md`; casing: `05-naming-and-casing.md`.
