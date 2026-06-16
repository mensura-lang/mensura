# Grammar (unit, store, and shape subset)

This document specifies the concrete surface grammar Mensura's parser
accepts *today*.  It grows one feature at a time; the current subset covers
`unit` declarations, the basic form of `store` declarations, and `shape`
declarations (with an optional unit clause and `Unit`/`string` parameters,
the latter interpolated into attribute names) claimed through the `:`
conformance clause on stores.

The grammar is **LL(1)**: a hand-written recursive-descent parser decides
every alternative from one token of lookahead, with no backtracking, as
required by `ROADMAP.md`.  Constructs that cannot be expressed in LL(1) are
reworked at the syntax level rather than handled by parser tricks.

## Surface form

The parser implements the surface form specified in
`docs/language/02-stores.md` and `docs/language/03-shapes.md`: a store names
its unit with a `unit { U }` clause and resolves foreign keys in a separate
`domain { ... }` block; a store may claim conformance to one or more shapes
with a `:` clause after its name.  Per `CLAUDE.md`, the design docs are
authoritative; alternative spellings (such as an inline field-level domain
annotation) are deferred sugar and are not accepted yet.

Shapes may take a parameter list and may omit the unit clause, per
`03-shapes.md`.  Parameters are of kind `Unit` or `string`; a `string`
parameter may be interpolated into attribute names with backticks.  Numeric
and predicate parameters, and the parameter list on function signatures, are
deferred to a follow-up.

## Lexical basis

Tokens come from the lexer (`crates/mensura-syntax/src/lexer.rs`).  The lexer
emits every word as an `Ident`; it knows no keywords.  **Keywords are
contextual**: the parser recognizes words such as `unit`, `store`, `shape`,
`const`, `var`, `domain`, and `enum` by their text *in the position where
they are expected*, not by reserving them globally.

A backtick-delimited **template** (`` `{col}_z` ``) lexes to a single token
carrying its raw inner text; the parser splits it into literal and `{param}`
segments.  Template tokens appear only as shape attribute names.

`ident` below is a lexer `Ident` token (UAX#31 identifier); `string` is a
string literal and `template` a backtick template token.  Punctuation tokens
(`{`, `}`, `(`, `)`, `:`, `,`) are as the lexer produces them.

## Grammar

```ebnf
program       = { item } EOF ;

item          = unit_decl | store_decl | shape_decl ;

unit_decl     = "unit" ident "{" { field } "}" ;
field         = ident ":" type ;

store_decl    = "store" ident [ conforms ] "{" unit_clause { store_block } "}" ;
conforms      = ":" shape_ref { "," shape_ref } ;
shape_ref     = ident [ args ] ;
args          = "(" arg { "," arg } ")" ;
arg           = ident | string ;
unit_clause   = "unit" "{" ident "}" ;
store_block   = const_block | var_block | domain_block ;
const_block   = "const" "{" { attr } "}" ;
var_block     = "var" "{" { attr } "}" ;
attr          = ident ":" type ;
domain_block  = "domain" "{" { domain_entry } "}" ;
domain_entry  = ident ":" ident ;

shape_decl    = "shape" ident [ params ] "{" [ unit_clause ] { shape_block } "}" ;
params        = "(" param { "," param } ")" ;
param         = ident ":" ident ;
shape_block   = shape_const | shape_var ;
shape_const   = "const" "{" { shape_attr } "}" ;
shape_var     = "var" "{" { shape_attr } "}" ;
shape_attr    = attr_name ":" type ;
attr_name     = ident | template ;

type          = enum_type | named_type ;
enum_type     = "enum" "(" string { "," string } ")" ;
named_type    = ident ;
```

## Why this is LL(1)

- **`item`**: the parser peeks one token.  `unit` selects `unit_decl`,
  `store` selects `store_decl`, `shape` selects `shape_decl`; the three
  FIRST sets are disjoint.
- **`conforms`**: after a store name the next token is either `:` (the
  clause is present) or `{` (it is absent).  One token decides.
- **`shape_ref`**: after the shape name, `(` opens an argument list and any
  other token (`,` or `{`) ends the reference.  One token decides.
- **`arg`**: an `ident` (a unit name) and a `string` literal are distinct
  tokens, so the argument's form is fixed by the current token.
- **`attr_name`**: a shape attribute name is an `ident` or a `template`
  token, again distinct, so one token decides.
- **`params`**: after a shape name, `(` opens the parameter list and `{`
  skips it.  One token decides.
- **`shape_decl` body**: the optional `unit_clause` is taken when the body
  opens with the `unit` keyword, and skipped otherwise.  One token decides.
- **`store_block` loop**: at each turn the next token is either `}` (end the
  store body) or one of the introducers `const` / `var` / `domain`, all
  distinct words.  One token decides.
- **`shape_block` loop**: as `store_block`, minus `domain`; a `domain` word
  in a shape body is a parse error (shapes carry no foreign-key resolution).
- **`field` / `attr` loops**: a loop continues on `ident` and ends on `}`.
- **`type`**: the word `enum` selects `enum_type`; any other `ident` selects
  `named_type`.  The decision is made on the current token alone (the `(`
  that must follow `enum` is checked after committing, not used to decide).
  Enum variants are **string literals**, so their values are explicit and may
  contain characters that are not valid identifiers (`"in-progress"`, spaces,
  accents); this also matches how categorical values are stored and matched.

No production is left-recursive, and no nullable production creates a
FIRST/FOLLOW clash, so the freeze condition in `ROADMAP.md` M0 holds for this
subset.

## Notes and constraints

- **`unit` appears in two roles.**  At top level `unit Name { ... }` declares
  a unit; inside a store or shape `unit { Name }` names the tabulated unit.
  The two are never reachable from the same parser state, so there is no
  ambiguity.
- **A shape body cannot contain `domain`.**  A shape is a structural
  contract, not a store; foreign-key resolution is per-store.  The parser
  rejects a `domain` block inside a shape.
- **Clause order.**  A `store` body must begin with its `unit { U }` clause,
  followed by zero or more `const`, `var`, and `domain` blocks in any order.
  Repeated `const`/`var` blocks are allowed and merged by the resolver.
- **A shape's unit clause is optional.**  When present it comes first, as in
  a store; when absent the shape is unit-agnostic.  A shape claimed with
  arguments (`Tabular(Person)`, `Ageable("birthdate")`) binds its parameters
  positionally: a unit name fills a `Unit` parameter, a string literal a
  `string` parameter.  Numeric and predicate parameter kinds are rejected by
  the resolver as "not yet supported".
- **Backtick names interpolate `string` parameters.**  A shape attribute name
  may be a template such as `` `{col}_z` ``; its `{param}` holes must name
  `string` parameters, and the rendered name must be a valid identifier.
- **`enum` is positional.**  `enum` is a keyword only in `type` position; it
  cannot be a unit or store name there.
- **`domain` is parsed, not yet resolved.**  The grammar accepts `domain`
  blocks and unit-reference field types so the surface stays stable, but the
  current resolver rejects compound units and `domain` blocks as "not yet
  supported".

## Types in this subset

`named_type` is one of the recognized primitive types, otherwise it is read
as a reference to a unit (a compound field, deferred):

| Type        | Meaning                                   |
|-------------|-------------------------------------------|
| `string`    | text                                      |
| `number`    | numeric (integer or real)                 |
| `bool`      | boolean                                   |
| `date`      | calendar date (ISO 8601)                  |
| `enum(...)` | one of a fixed set of string-literal values |

Physical-unit types (dimensional quantities, precision) are a separate,
larger feature with their own design doc and are not in this subset.

## Worked example

The basic stores from `docs/language/02-stores.md` parse under this grammar:

```mensura
unit Person {
  id: string
}

unit Department {
  code: string
}

store Departments {
  unit { Department }
  const { name: string }
}

store Persons : Ageable("birthdate") {
  unit { Person }
  const { birthdate: date }
  var   { last_name: string }
}

store Students : PersonRecord, Tabular(Person) {
  unit { Person }
  const { admission: date }
}

shape PersonRecord {
  unit { Person }
  const { admission: date }
}

shape Tabular(U: Unit) {
  unit { U }
}

shape Named {
  const { name: string }
}

shape Ageable(date_field: string) {
  const { `{date_field}`: date }
}
```

`Students` claims the concrete-unit shape `PersonRecord` and the
unit-parameter shape `Tabular(Person)`; the resolver checks the store's unit
and `admission` attribute against the former and binds `U := Person` for the
latter.  `Persons` claims `Ageable("birthdate")`: the `string` argument
renders the templated attribute name to `birthdate`, which the store carries.
`Named` is unit-agnostic (no unit clause): any store carrying a
`const name: string` conforms.  `Courses` and `StudentGrades` from
`02-stores.md` are compound (their units reference other units and they carry
`domain` blocks); they parse but are rejected by the resolver until compound
support lands.

## Forward references

- Numeric and predicate parameter kinds, and the parameter list on function
  signatures.
- Compound units, `domain` resolution, and foreign keys.
- Annotations (`@audited`, `@versioned`, `@auto`, `@domain`, ...).
- Physical-unit and precision types.
- `device`, `view`, transforms, and pipeline operations, each of
  which extends this grammar and gets its own section here.
