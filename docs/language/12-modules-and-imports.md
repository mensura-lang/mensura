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
  block's local `let`s.  Anything effectful or table-shaped (pipelines,
  lambdas, aggregates) is rejected as "not a const expression"; an
  `assert` in a const block is not yet supported.

  ```mensura
  let km { 1000.0 * meter }
  let newton { kilogram * meter / second^2 }
  let limit: temperature[real] { 350.0 * kelvin }
  ```

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
constant-folded before evaluation, so the runtime never sees them.

## Imports

```mensura
import si
```

- **A module is a file** that exports its const bindings and type-level
  names (dimension aliases).  It does not export stores, views, or
  pipelines: those are materialized, site-specific resources
  (ADR 0027, Decision 2).  A `registry` (M4) is never importable.
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
  Decision 6): it names a module that ships with the toolchain (`si`
  today), needs no manifest and no network, and cannot be remapped.  An
  unknown name is a compile error at the import site.  The
  manifest-resolved marked form for third-party modules is provisional
  and not implemented (ADR 0027, Decision 7).

## The intrinsic / library split

The language provides an **initial environment** of intrinsics: the seven
base units (`second`, `meter`, `kilogram`, `ampere`, `kelvin`, `mole`,
`candela`; ADR 0026, Decision 6) and the ambient builtins that already
exist (the aggregate combinators, `to_real`, the pipeline operations).

There is **no implicit prelude** beyond the intrinsics.  In particular
`si` (the unit symbols, prefixes, and named derived units) is an ordinary
import: `9.8 * meter / second^2` type-checks with no import, while
`9.8 * si.m / si.s^2` requires `import si`.

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

## Forward references

- The decision record: `docs/decisions/0027-modules-and-imports.md`.
- The `si` library and its discipline:
  `docs/decisions/0028-standard-library-si.md`.
- The units feature this ships: `11-physical-units.md` (ADR 0026).
- The grammar: `04-grammar.md`; casing: `05-naming-and-casing.md`.
