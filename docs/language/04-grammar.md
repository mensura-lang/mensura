# Grammar (unit and store subset)

This document specifies the concrete surface grammar Mensura's parser
accepts *today*.  It grows one feature at a time; the current subset covers
`unit` declarations and the basic form of `store` declarations, enough to
implement the first feature: creating a store.

The grammar is **LL(1)**: a hand-written recursive-descent parser decides
every alternative from one token of lookahead, with no backtracking, as
required by `ROADMAP.md`.  Constructs that cannot be expressed in LL(1) are
reworked at the syntax level rather than handled by parser tricks.

## Surface form

The parser implements the surface form specified in
`docs/language/02-stores.md`: a store names its unit with a `unit { U }`
clause and resolves foreign keys in a separate `domain { ... }` block.  Per
`CLAUDE.md`, the design docs are authoritative; alternative spellings (such
as an inline field-level domain annotation) are deferred sugar and are not
accepted yet.

## Lexical basis

Tokens come from the lexer (`crates/mensura-syntax/src/lexer.rs`).  The lexer
emits every word as an `Ident`; it knows no keywords.  **Keywords are
contextual**: the parser recognizes words such as `unit`, `store`, `const`,
`var`, `domain`, and `enum` by their text *in the position where they are
expected*, not by reserving them globally.

`ident` below is a lexer `Ident` token (UAX#31 identifier).  Punctuation
tokens (`{`, `}`, `(`, `)`, `:`, `,`) are as the lexer produces them.

## Grammar

```ebnf
program       = { item } EOF ;

item          = unit_decl | store_decl ;

unit_decl     = "unit" ident "{" { field } "}" ;
field         = ident ":" type ;

store_decl    = "store" ident "{" unit_clause { store_block } "}" ;
unit_clause   = "unit" "{" ident "}" ;
store_block   = const_block | var_block | domain_block ;
const_block   = "const" "{" { attr } "}" ;
var_block     = "var" "{" { attr } "}" ;
attr          = ident ":" type ;
domain_block  = "domain" "{" { domain_entry } "}" ;
domain_entry  = ident ":" ident ;

type          = enum_type | named_type ;
enum_type     = "enum" "(" ident { "," ident } ")" ;
named_type    = ident ;
```

## Why this is LL(1)

- **`item`**: the parser peeks one token.  `unit` selects `unit_decl`,
  `store` selects `store_decl`; the FIRST sets are disjoint.
- **`store_block` loop**: at each turn the next token is either `}` (end the
  store body) or one of the introducers `const` / `var` / `domain`, all
  distinct words.  One token decides.
- **`field` / `attr` loops**: a loop continues on `ident` and ends on `}`.
- **`type`**: the word `enum` selects `enum_type`; any other `ident` selects
  `named_type`.  The decision is made on the current token alone (the `(`
  that must follow `enum` is checked after committing, not used to decide).

No production is left-recursive, and no nullable production creates a
FIRST/FOLLOW clash, so the freeze condition in `ROADMAP.md` M0 holds for this
subset.

## Notes and constraints

- **`unit` appears in two roles.**  At top level `unit Name { ... }` declares
  a unit; inside a store `unit { Name }` names the tabulated unit.  The two
  are never reachable from the same parser state, so there is no ambiguity.
- **Clause order.**  A `store` body must begin with its `unit { U }` clause,
  followed by zero or more `const`, `var`, and `domain` blocks in any order.
  Repeated `const`/`var` blocks are allowed and merged by the resolver.
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
| `enum(...)` | one of a fixed set of bare-word variants  |

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

store Persons {
  unit { Person }
  const { birthdate: date }
  var   { last_name: string }
}

store Students {
  unit { Person }
  const { admission: date }
}
```

`Courses` and `StudentGrades` from that document are compound (their units
reference other units and they carry `domain` blocks); they parse but are
rejected by the resolver until compound support lands.

## Forward references

- Compound units, `domain` resolution, and foreign keys.
- Annotations (`@audited`, `@versioned`, `@auto`, `@domain`, ...).
- Physical-unit and precision types.
- `device`, `shape`, `view`, transforms, and pipeline operations, each of
  which extends this grammar and gets its own section here.
