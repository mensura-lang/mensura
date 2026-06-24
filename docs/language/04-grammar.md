# Grammar

This document specifies the surface grammar of Mensura.  Most of it is the
grammar the parser accepts *today*, and it grows one feature at a time: the
implemented subset covers `unit` declarations, the basic form of `store`
declarations, and `shape` declarations (with an optional unit clause and
`Unit`/`string` parameters, the latter interpolated into attribute names)
claimed through the `:` conformance clause on stores.

The final section, the expression sublanguage, is specified *ahead of the
parser*.  It is the grammar for the one expression language of
`06-expressions.md` (and
`docs/decisions/0007-single-expression-sublanguage.md`), written here so the
declaration grammar and the expression grammar live in one place, but it is
not yet implemented.  When the parser grows expressions, this is the grammar
it implements.

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

item          = unit_decl | store_decl | shape_decl | enum_decl ;

unit_decl     = "unit" ident "{" { field } "}" ;
field         = ident ":" type ;

enum_decl     = "enum" ident "{" string { "," string } "}" ;

store_decl    = "store" ident [ conforms ] "{" unit_clause { store_block } "}" ;
conforms      = ":" shape_ref { "," shape_ref } ;
shape_ref     = ident [ args ] ;
args          = "[" arg { "," arg } "]" ;
arg           = ident | string ;
unit_clause   = "unit" "{" ident "}" ;
store_block   = const_block | var_block | domain_block ;
const_block   = "const" "{" { attr } "}" ;
var_block     = "var" "{" { attr } "}" ;
attr          = ident ":" type ;
domain_block  = "domain" "{" { domain_entry } "}" ;
domain_entry  = ident ":" ident ;

shape_decl    = "shape" ident [ params ] "{" [ unit_clause ] { shape_block } "}" ;
params        = "[" param { "," param } "]" ;
param         = ident ":" ident ;
shape_block   = shape_const | shape_var ;
shape_const   = "const" "{" { shape_attr } "}" ;
shape_var     = "var" "{" { shape_attr } "}" ;
shape_attr    = attr_name ":" type ;
attr_name     = ident | template ;

type          = named_type ;
named_type    = ident ;
```

## Why this is LL(1)

- **`item`**: the parser peeks one token.  `unit` selects `unit_decl`,
  `store` selects `store_decl`, `shape` selects `shape_decl`, `enum` selects
  `enum_decl`; the four FIRST sets are disjoint.
- **`enum_decl`**: `enum` selects it; the name, `{`, and the string-literal
  variants follow unambiguously.  An empty `{ }` is rejected (an enum needs at
  least one variant).
- **`conforms`**: after a store name the next token is either `:` (the
  clause is present) or `{` (it is absent).  One token decides.
- **`shape_ref`**: after the shape name, `[` opens an argument list and any
  other token (`,` or `{`) ends the reference.  One token decides.
- **`arg`**: an `ident` (a unit name) and a `string` literal are distinct
  tokens, so the argument's form is fixed by the current token.
- **`attr_name`**: a shape attribute name is an `ident` or a `template`
  token, again distinct, so one token decides.
- **`params`**: after a shape name, `[` opens the parameter list and `{`
  skips it.  One token decides.
- **`shape_decl` body**: the optional `unit_clause` is taken when the body
  opens with the `unit` keyword, and skipped otherwise.  One token decides.
- **`store_block` loop**: at each turn the next token is either `}` (end the
  store body) or one of the introducers `const` / `var` / `domain`, all
  distinct words.  One token decides.
- **`shape_block` loop**: as `store_block`, minus `domain`; a `domain` word
  in a shape body is a parse error (shapes carry no foreign-key resolution).
- **`field` / `attr` loops**: a loop continues on `ident` and ends on `}`.
- **`type`**: a type is a single `ident`: a primitive (`string`, `number`,
  ...), a unit reference, or a named `enum`.  Which it is, is the resolver's
  decision, not the parser's; the parser commits on the lone identifier.

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
  arguments (`Tabular[Person]`, `Ageable["birthdate"]`) binds its parameters
  positionally: a unit name fills a `Unit` parameter, a string literal a
  `string` parameter.  Numeric and predicate parameter kinds are rejected by
  the resolver as "not yet supported".
- **Backtick names interpolate `string` parameters.**  A shape attribute name
  may be a template such as `` `{col}_z` ``; its `{param}` holes must name
  `string` parameters, and the rendered name must be a valid identifier.
- **Brackets are for parameters, parentheses are not used here.**  Shape
  parameter lists (`Tabular[U: Unit]`) and conformance arguments
  (`Tabular[Person]`) use `[ ]`, leaving `( )` free for grouping and tuples in
  the expression sublanguage.  No declaration form uses `( )`.
- **`enum` is a top-level declaration.**  An enumerated type is declared once,
  `enum Name { "v1", "v2" }`, and referenced by name in a field's type.  Its
  name is a type (PascalCase); its variants are **string literals**, so their
  values are explicit and may contain characters that are not valid
  identifiers (`"in-progress"`, spaces, accents), which also matches how
  categorical values are stored and matched.  `enum` is a keyword only in
  declaration position.
- **`domain` is parsed, not yet resolved.**  The grammar accepts `domain`
  blocks and unit-reference field types so the surface stays stable, but the
  current resolver rejects compound units and `domain` blocks as "not yet
  supported".

## Types in this subset

`named_type` is one of the recognized primitive types, the name of a declared
`enum`, otherwise it is read as a reference to a unit (a compound field,
deferred):

| Type     | Meaning                                          |
|----------|--------------------------------------------------|
| `string` | text                                             |
| `number` | numeric (integer or real)                        |
| `bool`   | boolean                                          |
| `date`   | calendar date (ISO 8601)                         |
| `Name`   | a declared `enum`: one of its string variants    |

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

enum Status {
  "active", "inactive"
}

store Persons : Ageable["birthdate"] {
  unit { Person }
  const { birthdate: date }
  var   { last_name: string }
  var   { status: Status }
}

store Students : PersonRecord, Tabular[Person] {
  unit { Person }
  const { admission: date }
}

shape PersonRecord {
  unit { Person }
  const { admission: date }
}

shape Tabular[U: Unit] {
  unit { U }
}

shape Named {
  const { name: string }
}

shape Ageable[date_field: string] {
  const { `{date_field}`: date }
}
```

`Students` claims the concrete-unit shape `PersonRecord` and the
unit-parameter shape `Tabular[Person]`; the resolver checks the store's unit
and `admission` attribute against the former and binds `U := Person` for the
latter.  `Persons` claims `Ageable["birthdate"]`: the `string` argument
renders the templated attribute name to `birthdate`, which the store carries,
and its `status` is the named `enum Status`.
`Named` is unit-agnostic (no unit clause): any store carrying a
`const name: string` conforms.  `Courses` and `StudentGrades` from
`02-stores.md` are compound (their units reference other units and they carry
`domain` blocks); they parse but are rejected by the resolver until compound
support lands.

## Expression grammar (specified ahead of the parser)

The expression sublanguage is defined in `06-expressions.md`; this section
gives its concrete LL(1) grammar.  It is one grammar, shared by every site
that evaluates an expression (`when:`, `where:`, `@auto(...)`, and the
pipeline operations); a site adds only a context of names and an expected
result type, neither of which is syntax.

```ebnf
expr        = pipe_expr ;

pipe_expr   = or_expr  { "|>" or_expr } ;
or_expr     = and_expr { "or" and_expr } ;
and_expr    = not_expr { "and" not_expr } ;
not_expr    = "not" not_expr | cmp_expr ;
cmp_expr    = add_expr [ cmp_op add_expr | "is" presence ] ;
cmp_op      = "==" | "!=" | "<" | "<=" | ">" | ">=" | "in" ;
presence    = "known" | "missing" ;
add_expr    = mul_expr { ( "+" | "-" ) mul_expr } ;
mul_expr    = unary_expr { ( "*" | "/" ) unary_expr } ;
unary_expr  = "-" unary_expr | pow_expr ;
pow_expr    = app_expr [ "^" unary_expr ] ;
app_expr    = postfix { postfix } ;
postfix     = primary { "." ident } ;
primary     = number | string | ident | lambda | group ;
lambda      = "|" [ ident { "," ident } ] "|" or_expr ;
group       = "(" [ expr { "," expr } ] ")" ;
```

The terminals `number`, `string`, and `ident` are lexer tokens.  Boolean
literals (`true`, `false`) and the word operators (`or`, `and`, `not`, `in`,
`is`, `known`, `missing`) are `ident` tokens recognized by their text in the
positions shown; see the reserved-words note below.  `"|>"` is a single
token, a new one: the lexer emits `|` as `Pipe` today and must munch `|>`
maximally, with the closing-bar caveat in `06-expressions.md`.  All other
operator tokens (`== != < <= > >= + - * / ^ . | ( ) ,`) the lexer already
emits.  The `NxE` measured literal (`10x3`) is a separate token reserved for
the physical-units feature and does not appear in this subset.

### Why the expression grammar is LL(1)

- **Precedence is layered, not recovered by backtracking.**  Each level is a
  left-recursion-free loop (`{ op operand }`) or a single optional
  (`[ op operand ]`) over the next-tighter level, so the operator token at
  hand decides whether to continue.  From loosest to tightest: `|>`, `or`,
  `and`, `not`, the comparisons, `+ -`, `* /`, unary `-`, `^`, application,
  member access.
- **`not_expr`**: the ident `not` selects the prefix branch; any other token
  starts `cmp_expr`.  One token decides.
- **`cmp_expr`**: after the left operand, a comparison operator (or the ident
  `in`) opens the comparison branch and the ident `is` opens the presence
  branch; any other token ends the production, so comparisons do not chain.
  `in` and `is` are distinct idents, so one token picks the branch.
- **`pow_expr`**: `^` is right-associative because its right operand is a
  `unary_expr`.  That is also why `2^-3` is `2^(-3)`, while `-2^2` is
  `-(2^2)` (the leading `-` is a `unary_expr` wrapping the whole `pow_expr`).
- **`app_expr` (the application spine)**: the loop consumes another
  `postfix` while the current token can start one, namely a `number`,
  `string`, `(`, `|` (a lambda), or an `ident` that is *not* a reserved word
  (below).  It stops on any operator, on `|>` (a different token from `|`),
  and on `)` and `,`.  A `|` starts a lambda argument; a `|>` never does, so
  a pipe always ends the spine and is handled by `pipe_expr`.
- **`primary`**: `number`, `string`, and `ident` are distinct tokens; `(`
  opens a `group`; `|` opens a `lambda`.  One token decides.
- **`group`**: after `(`, the first `expr` is parsed, then a `,` means a
  tuple (more elements follow) and a `)` means a grouping.  The decision is
  the one token after the first element, a normal repetition rather than
  backtracking.  `()` is the empty tuple.
- **`lambda`**: `|` opens it, an optional comma-separated ident list gives the
  parameters, a closing `|` ends them, and the body is an `or_expr`.  The
  body deliberately excludes a top-level `|>`, so
  `data |> filter |r| r.x > 0 |> map g` composes as
  `(data |> filter (|r| r.x > 0)) |> map g`; a pipe *inside* a lambda body
  must be parenthesized.  A lambda that is not the last argument of an
  application must also be parenthesized, since its body extends maximally.

### Reserved words in expressions

Combining juxtaposition application with word operators forces a small,
local exception to the lexer's keyword-freedom: inside an expression the
words `or`, `and`, `not`, `in`, `is`, `known`, and `missing` are
**reserved** and cannot name a value.  This is unavoidable with one token of
lookahead, since after an operand an ident could otherwise be read either as
the next argument (juxtaposition) or as an infix operator, and only
reservation resolves the choice.  The reservation is local to the expression
sublanguage; elsewhere these words remain ordinary identifiers, as the
keyword-free lexer intends.

## Forward references

- Numeric and predicate parameter kinds, and the parameter list on function
  signatures.
- Compound units, `domain` resolution, and foreign keys.
- Annotations (`@audited`, `@versioned`, `@auto`, `@domain`, ...).
- Physical-unit and precision types, including the `NxE` measured literal and
  the unit grammar.
- `device`, `view`, transforms, and the pipeline operations, which build on
  the expression grammar above and each get their own section here.
