# Naming and casing: types, terms, and the wire

Mensura fixes a casing convention for declared names, enforces it at compile
time, and translates the canonical name deterministically onto each
transport.  One name in the source becomes an idiomatic name on every wire,
and the convention is a hard rule rather than a style suggestion so that the
translation is always unambiguous.

The web-service surface itself (which transports exist, how a surface is
exposed) is settled in `docs/decisions/0006-transport-agnostic-surface.md`;
this document is only about names.

## The convention

Declared names split into two classes by what they denote, plus a fixed
built-in vocabulary.

- **Types** use **PascalCase**.  These are the names that classify values and
  appear in type position: `unit` and `shape`.  Examples: `Machine`,
  `TemperatureSensor`, `FeatureWindow`.
- **Terms** use **snake_case**.  These are named resources, instances, and
  fields: `store` and `registry` names, attribute (column) names,
  `string`-valued shape parameters, and `let` value bindings at every
  scope (a block statement or a top-level item,
  `12-modules-and-imports.md`).  Examples: `temperature_readings`,
  `foundation_day`, `date_field`, `km`.
- **Built-in type names** are a fixed **lowercase** vocabulary the
  resolver matches rather than case-checks: the primitives (`int`,
  `real`, `string`, `bool`, `date`), the seven base dimensions
  (`time`, `length`, `mass`, `current`, `temperature`, `amount`,
  `luminosity`), and dimension aliases declared with a generic `let`
  (`let speed[T] { ... }`), which extend the vocabulary and take the same
  lowercase form (ADR 0026, Decision 5).  The PascalCase rule governs
  user-declared type *names* (`unit`, `shape`, `enum`), not this
  vocabulary.

A shape parameter follows its kind, since the type/term split applies to it
too.  A `Unit` parameter is a type parameter (like `U` in `Tabular[U: Unit]`
or `FeatureWindow[U]`), so it is PascalCase.  A `string` parameter names a
value (like `date_field` in `Ageable[date_field: string]`), so it is
snake_case.

The `[...]` bracket is uniformly **type-level parameter application**
(ADR 0026, Decision 5): shape parameters (`Tabular[U: Unit]`), the
backing of a dimension (`temperature[real]`), and the parameters of a
dimension alias (`speed[T]`) are one construct.

Unit *names and symbols* are value-level terms and follow snake_case:
the intrinsics (`meter`, `second`, ...) and the `si` library's bindings
(`si.newton`, `si.km`).  Because a term binding must be snake_case,
`si` binds full lowercase names always and short SI symbols only where
they are already snake_case-valid (`s`, `m`, `g`, `kg`, `km`, `ms`,
`mol`, `cd`, ...); uppercase and mixed-case symbols (`A`, `K`, `N`,
`Pa`, `Hz`) are not bound.  This resolves ADR 0028's SI-symbol casing
question in favor of one rule with no exceptions; a future `exposing`
form with renaming can revisit the terse spellings without a casing
change.

A `view` is a materialized, queried resource, like a `store` or `registry`: it
is defined by a pipeline and exposed over a wire
(`docs/decisions/0012-view-hosting.md`, `10-views.md`), not a type that
classifies rows.  It therefore takes the term convention, **snake_case**
(`temperature_summary`, `feature_window`), and its wire names are derived by the
same transport translation below.  A view may *claim* a shape to constrain its
output, but the shape is the type, not the view; the view itself is the
resource.

An `enum` is a named type, so its name is PascalCase (`Status`), like a
`unit` or `shape`.  Its variants are string literals
(`enum Status { "active" "inactive" }`), not identifiers, so the variant
values themselves are unconstrained.

A softer style layer sits on top of the enforced case rule and is not
checked: a unit reads **singular** and a store **plural** (`01-units.md`),
and a shape reads **singular**, as a noun phrase when it names content
(`PersonRecord`) and preferably as an **adjective** when it asserts a
property (`Ageable`, `Independent`); see `03-shapes.md`, "Naming
convention".

### Why two classes

A `store` or `registry` is a resource you query, mutate, and expose over a
wire, not a type; it reads like a value, so it takes the value convention.  A
`unit` or `shape` classifies rows, so it takes the type convention.  This is
the familiar types-PascalCase, values-snake_case split, and it keeps a
collection (`temperature_readings`) visually distinct from the unit it is
built on (`Machine`).

It also resolves a divergence in the source material: `proposal.md` writes
stores lowercase (`people`, `registrations`), while `iiot.md` and the
committed example write them PascalCase (`Machines`, `TemperatureReadings`).
The convention here is snake_case, and the example
`docs/examples/college-stores.mensura` is aligned to it.

### Why not kebab-case

Identifiers follow UAX#31 (see `crates/mensura-syntax/src/lexer.rs`), where
`-` is not an identifier character: `temperature-readings` lexes as
`temperature` minus `readings`.  So the lowercase, multi-word form must be
snake_case.  Kebab-case appears only on the wire (REST paths, MQTT topics),
where names are strings, never identifiers.

## The exact rule

The check is defined to behave sensibly under the full UAX#31 identifier set,
including non-ASCII and caseless scripts.

- **snake_case**: the identifier contains no uppercase character (every
  character is lowercase or caseless), and `_` is allowed as a separator.
- **PascalCase**: the first cased character is uppercase, and there is no `_`
  separator.
- **Caseless exemption**: an identifier with no cased characters at all (for
  example a CJK name such as `温度`) satisfies neither "has an uppercase first
  character" nor "has no uppercase", so it is exempt from the case check and
  accepted in any position.  The rule constrains only identifiers that contain
  cased characters.

Leading underscores and other identifier details the lexer already accepts
are out of scope for the case check; it judges case and separators only.

## Enforcement

The convention is a **hard compile-time error**, not a warning.  The resolver
(`crates/mensura-types/src/resolve.rs`) rejects a name in the wrong class and
collects the diagnostic alongside the others rather than failing fast, so a
single run reports every violation.  A `unit` or `shape` whose name is not
PascalCase, or a `store`, `registry`, `view`, attribute, parameter, `let`
value binding, or `import` name that is not snake_case, is a resolution
error.  A dimension alias declared with a generic `let` must be lowercase
(it joins the built-in type vocabulary), which the snake_case check
already enforces.

Enforcing rather than warning is what lets wire-name translation be
total and deterministic: every declared name is in a known case, so its REST,
GraphQL, gRPC, and MQTT projections are computable without ambiguity.

## Transport name-translation

A surface or field has one canonical Mensura name.  Each transport projects
it with that transport's idiom; the projection is deterministic.

| Surface | Type names | Field / resource names |
|---|---|---|
| Canonical (Mensura) | PascalCase | snake_case |
| REST | (paths only) | kebab-case path segment |
| GraphQL | PascalCase type | camelCase field |
| gRPC / protobuf | PascalCase message, service, RPC | snake_case field |
| MQTT | (topics only) | kebab-case topic segment |
| Permission scope | (n/a) | kebab-case resource |

Worked example, for a `registry temperature_readings` with a `machine` field:

- REST: `POST /temperature-readings`, resource `machine`.
- GraphQL: query field `temperatureReadings`, field `machine`.
- gRPC: message `TemperatureReadings`, field `machine`.
- MQTT: topic segment `temperature-readings`.
- Permission scope: `read:temperature-readings`, `write:temperature-readings`.

Because the canonical name is always in the expected case (enforcement
guarantees it), each of these is a pure function of the canonical name: the
compiler can generate every wire name, and round-tripping is unambiguous.

Permission scopes are a wire form too, not a special case: a scope appears in
IdP-issued tokens and OAuth scope strings, so its resource half uses the same
kebab-case as a REST path.  Mensura maps a scope back to its `store` or
`registry` by the inverse (`-` to `_`); the mapping is bijective because a
snake_case name uses `_` only as a separator and identifiers never contain
`-`.  See `docs/decisions/0005-identity-and-authorization.md` for how scopes
are auto-derived from resources.
