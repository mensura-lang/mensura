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

Declared names split into two classes by what they denote.

- **Types** use **PascalCase**.  These are the names that classify values and
  appear in type position: `unit` and `shape`.  Examples: `Machine`,
  `TemperatureSensor`, `FeatureWindow`.
- **Terms** use **snake_case**.  These are named resources, instances, and
  fields: `store` and `collect` names, attribute (column) names, and
  `string`-valued shape parameters.  Examples: `temperature_readings`,
  `foundation_day`, `date_field`.

A shape parameter follows its kind, since the type/term split applies to it
too.  A `Unit` parameter is a type parameter (like `U` in `Tabular(U: Unit)`
or `FeatureWindow<U>`), so it is PascalCase.  A `string` parameter names a
value (like `date_field` in `Ageable(date_field: string)`), so it is
snake_case.

`view` is deferred: views are underdefined (they are both type-like and
served), so no rule is enforced on `view` names yet, and they may stay
PascalCase for now.

Enum variants are string literals (`enum("active", "inactive")`), not
identifiers, so they are unconstrained.

### Why two classes

A `store` or `collect` is a resource you query, mutate, and expose over a
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
PascalCase, or a `store`, `collect`, attribute, or parameter whose name is not
snake_case, is a resolution error.

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

Worked example, for a `collect temperature_readings` with a `machine` field:

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
`collect` by the inverse (`-` to `_`); the mapping is bijective because a
snake_case name uses `_` only as a separator and identifiers never contain
`-`.  See `docs/decisions/0005-identity-and-authorization.md` for how scopes
are auto-derived from resources.
