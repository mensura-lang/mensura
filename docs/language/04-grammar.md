# Grammar

This document specifies the surface grammar of Mensura.  Most of it is the
grammar the parser accepts *today*, and it grows one feature at a time: the
implemented subset covers `unit` declarations, the basic form of `store`
declarations, `shape` declarations (with an optional unit clause and
`Unit`/`string` parameters, the latter interpolated into attribute names)
claimed through the `:` conformance clause on stores, `enum` declarations,
`view` declarations, top-level `let` bindings and `import` items
(`12-modules-and-imports.md`, ADR 0027), and dimensioned types in type
position (`11-physical-units.md`, ADR 0026).

The final section, the expression sublanguage, is the grammar for the one
expression language of `06-expressions.md` (and
`docs/decisions/0007-single-expression-sublanguage.md`), kept here so the
declaration grammar and the expression grammar live in one place.  The parser
implements it (`parse_expr` in `crates/mensura-syntax/src/parser.rs`), and
`view` declarations host it (`10-views.md`); the remaining hosting sites
(`when:`, `where:`, `@auto`) land with their own features.

The store and shape attribute surface shown here (the `attr` block, with no
mutability markers) is the design decided in
`docs/decisions/0019-attr-blocks-and-dropped-const-var.md`.  It supersedes
the earlier `const`/`var` attribute blocks.

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
`attr`, `domain`, `enum`, and `view` by their text *in the position where
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

item          = unit_decl | store_decl | shape_decl | enum_decl | view_decl
              | let_decl | import_decl ;

unit_decl     = "unit" ident "{" { field } "}" ;
field         = ident ":" type ;

enum_decl     = "enum" ident "{" string { string } "}" ;

store_decl    = "store" ident [ conforms ] "{" unit_clause { store_block } "}" ;
conforms      = ":" shape_ref { "," shape_ref } ;
shape_ref     = ident [ args ] ;
args          = "[" arg { "," arg } "]" ;
arg           = ident | string ;
unit_clause   = "unit" "{" ident "}" ;
store_block   = attr_block | domain_block ;
attr_block    = "attr" [ "*" ] "{" { store_attr } "}" ;
store_attr    = ident ":" type ;
domain_block  = "domain" "{" { domain_entry } "}" ;
domain_entry  = ident ":" ident ;

shape_decl    = "shape" ident [ params ] "{" [ unit_clause ] { shape_block } "}" ;
params        = "[" param { "," param } "]" ;
param         = ident ":" ident ;
shape_block   = shape_attr_block ;
shape_attr_block = "attr" [ "*" ] "{" { shape_attr } "}" ;
shape_attr    = attr_name ":" type ;
attr_name     = ident | template ;

view_decl     = "view" ident [ conforms ] block ;

let_decl      = "let" ident ( value_let | alias_let ) ;
value_let     = [ ":" type ] block ;
alias_let     = "[" ident { "," ident } "]" "{" tl_expr "}" ;
import_decl   = "import" ident ;

type          = tl_expr [ "?" ] ;
tl_expr       = tl_term { ( "*" | "/" ) tl_term } ;
tl_term       = tl_factor [ "^" [ "-" ] int ] ;
tl_factor     = ident [ "[" ident "]" ]
              | "(" tl_expr ")" "[" ident "]" ;
```

## Why this is LL(1)

- **`item`**: the parser peeks one token.  `unit` selects `unit_decl`,
  `store` selects `store_decl`, `shape` selects `shape_decl`, `enum` selects
  `enum_decl`, `view` selects `view_decl`, `let` selects `let_decl`, and
  `import` selects `import_decl`; the seven FIRST sets are disjoint words.
- **`let_decl`**: after the bound name the next token decides the kind: `[`
  opens the alias parameter list (a type-level dimension alias, ADR 0026
  Decision 8) and a braced body parsed with the type grammar follows; `:`
  (an ascription) or `{` continues the value form, whose body is the
  ordinary expression-grammar `block`.  One token decides.
- **Item bodies are brace-closed.**  Every item with a body ends at a `}`
  (`import` has no body), which is what makes item boundaries independent
  of the expression grammar.  A top-level item has no terminator, so an
  unbraced expression body would let the application spine swallow the
  next item's leading keyword (`let x = 2.0 * meter let y = ...` would
  read `meter let` as an application), and every future extension of the
  expression grammar would have to be re-audited against the item-level
  FOLLOW set.  The brace closes that hazard class structurally; a `let`
  body is the same statement block a `view` hosts, with the const
  evaluator (not the grammar) enforcing what a constant may compute
  (`12-modules-and-imports.md`).
- **`import_decl`**: `import` selects it and a single module name follows.
- **`view_decl`**: after the view name the next token is either `:` (a
  `conforms` clause is present) or `{` (it is absent and the `block` body
  opens).  One token decides.  The body is the expression-grammar `block`
  (below), whose own `}` terminates it, so no new declaration grammar is
  needed; a view hosts an ordinary pipeline expression.
- **`enum_decl`**: `enum` selects it; the name, `{`, and the string-literal
  variants follow unambiguously.  Variants are juxtaposed with no separator,
  like the entries of an `attr` block: after a variant the next token is
  either a string (another variant) or `}` (the enum closes), so one token
  decides.  An empty `{ }` is rejected (an enum needs at least one variant).
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
  store body) or one of the introducers `attr` / `domain`, distinct words.
  One token decides.
- **`attr_block` cardinality marker**: after the `attr` word the next token
  is either `*` (an `attr*` block, whose attributes are bag-valued,
  ADR 0022) or `{` (a plain block).  One token decides.  The `*` is the
  ordinary `Star` operator token, so the keyword-free lexer is untouched;
  the block is conventionally written glued, `attr* { ... }`.
- **`shape_block` loop**: as `store_block`, minus `domain`; a shape body has
  only `attr` blocks, and a `domain` word in a shape body is a parse error
  (shapes carry no foreign-key resolution).
- **`field` / attribute loops**: a loop continues on `ident` (or a `template`
  name in a shape) and ends on `}`.
- **`type`**: a type is a type-level expression (`tl_expr`).  The common
  case is still a single `ident`: a primitive (`string`, `int`, `real`,
  ...), a unit reference, or a named `enum`.  Which it is, is the
  resolver's decision, not the parser's.  A dimensioned type continues
  from the identifier with `[`, `*`, `/`, or `^`
  (`temperature[real]`, `(length / time^2)[real]`; see
  `11-physical-units.md` and the subsection below).  A trailing `?` makes
  the value **optional** (it may be missing in an observed row; see ADR
  0010 and `02-stores.md`).  After the type-level expression the parser
  peeks one token and takes a single `?` if present, so the optional
  marker preserves LL(1).  The `?` is a punctuation token the lexer
  emits, and `parse_type` carries it on the `TypeExpr`; the resolver
  rejects `?` on a key field (whether a row exists is cardinality, a
  separate axis) and threads totality onto each resolved column.

No production is left-recursive, and no nullable production creates a
FIRST/FOLLOW clash, so the freeze condition in `ROADMAP.md` M0 holds for this
subset.

### Why the type grammar is LL(1)

The type-level expression grammar (`tl_expr`) is the expression grammar's
precedence-cascade idea in miniature: `*`/`/` loop over `tl_term`, `^`
is a single optional tail, and `tl_factor` splits on distinct tokens
(`ident` versus `(`).  After an `ident`, `[` opens the backing bracket
and any other token ends the factor; after a parenthesized group the `[`
is mandatory.  The exponent's optional `-` sits between `^` and `int`,
both fixed by position.  No production is left-recursive.

The continuation tokens of a `type` are `*`, `/`, `^`, `[`, and the
optional `?`.  The FOLLOW sets at every `parse_type` call site are
disjoint from them:

| type position | FOLLOW (what ends the type) | disjoint? |
| --- | --- | --- |
| a `unit` field / `attr` entry | next field `ident`/`template`, `}` | yes |
| a `let` / record-field ascription | `=` | yes |
| a lambda return ascription `\|x\| : T body` | FIRST(or_expr): idents, literals, `-`, `(`, `\|`, `{` | yes: `-` only follows `^`, and `(` only starts a factor (after `*`, `/`, or at the head), never follows a completed factor |

The lambda-return row is the one genuinely hazardous context: the body
begins immediately after the type, so the type must not be able to
swallow a body-starting token.  It cannot: a completed `tl_expr` can only
be continued by `*`, `/`, `^`, or `[`, none of which starts an
expression (`\|>` is a distinct token from `\|`, and unary minus appears
in FIRST(or_expr) but a bare `-` is not a type continuation).  So
`|x| : speed[real] x / s` parses the type as `speed[real]` and the body
as `x / s`.

## Notes and constraints

- **`unit` appears in two roles.**  At top level `unit Name { ... }` declares
  a unit; inside a store or shape `unit { Name }` names the tabulated unit.
  The two are never reachable from the same parser state, so there is no
  ambiguity.
- **A shape body cannot contain `domain`.**  A shape is a structural
  contract, not a store; foreign-key resolution is per-store.  The parser
  rejects a `domain` block inside a shape.
- **Clause order.**  A `store` body must begin with its `unit { U }` clause,
  followed by zero or more `attr` and `domain` blocks in any order.  Repeated
  `attr` blocks are allowed and merged by the resolver.
- **`attr` versus `attr*` declares cardinality (ADR 0022).**  A store (or
  shape) whose attributes are all in plain `attr` blocks is a `singletons`
  tabulation (the ADR 0001 discipline, `card <= 1` over the key); one whose
  attributes are all in `attr*` blocks is a `bag` (many observations per
  key, the entity-keyed form for recurring observations).  The parser
  accepts any mix; the resolver rejects a declaration that mixes the two as
  "not yet supported" (the ADR's deferred refinement).
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
  (`Tabular[Person]`) use `[ ]`, leaving `( )` free for grouping, collections,
  and records in the expression sublanguage.  No declaration form uses `( )`.
- **`enum` is a top-level declaration.**  An enumerated type is declared once,
  `enum Name { "v1" "v2" }`, and referenced by name in a field's type.  Its
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

A lone identifier in type position is one of the recognized primitive
types, the name of a declared `enum`, otherwise it is read as a reference
to a unit (a compound field, deferred):

| Type       | Meaning                                          |
|------------|--------------------------------------------------|
| `string`   | text                                             |
| `int`      | integer number                                   |
| `real`     | real number (dimensionless)                      |
| `bool`     | boolean                                          |
| `date`     | calendar date (ISO 8601)                         |
| `Name`     | a declared `enum`: one of its string variants    |
| `D[real]`  | a dimensioned quantity (`temperature[real]`)     |

`int` and `real` are distinct domains with no implicit widening between
them; only the key-eligible types (`string`, `int`, `bool`, `date`, `enum`)
may be key fields (ADR 0014).

A dimensioned type applies a dimension (a base dimension, an alias, or a
parenthesized `tl_expr` such as `(length / time^2)`) to a backing, `real`
today; see `11-physical-units.md` (ADR 0026).  A dimensioned column is
numeric and orderable but, like `real`, not key-eligible.

A trailing `?` (e.g. `date?`) makes any of these **optional**: the value may
be missing in an observed row (ADR 0010).  Without it the value is total
(known).  `?` is not allowed on a key field.

Precision types are a separate, later feature (a library extension of
`real`, ADR 0026 Decision 9) and are not in this subset.

## Worked example

The basic stores from `docs/language/02-stores.md` parse under this grammar:

```mensura
unit Person {
  id: string
}

unit Department {
  code: string
}

store departments {
  unit { Department }
  attr { name: string }
}

enum Status {
  "active"
  "inactive"
}

store persons : Ageable["birthdate"] {
  unit { Person }
  attr {
    birthdate: date
    last_name: string
    status:    Status
  }
}

store students : PersonRecord, Tabular[Person] {
  unit { Person }
  attr { admission: date }
}

shape PersonRecord {
  unit { Person }
  attr { admission: date }
}

shape Tabular[U: Unit] {
  unit { U }
}

shape Named {
  attr { name: string }
}

shape Ageable[date_field: string] {
  attr { `{date_field}`: date }
}
```

`students` claims the concrete-unit shape `PersonRecord` and the
unit-parameter shape `Tabular[Person]`; the resolver checks the store's unit
and `admission` attribute against the former and binds `U := Person` for the
latter.  `persons` claims `Ageable["birthdate"]`: the `string` argument
renders the templated attribute name to `birthdate`, which the store carries,
and its `status` is the named `enum Status`.
`Named` is unit-agnostic (no unit clause): any store carrying an
attribute `name: string` conforms.  `courses` and `student_grades` from
`02-stores.md` are compound (their units reference other units and they carry
`domain` blocks); they parse but are rejected by the resolver until compound
support lands.

## Expression grammar

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
cmp_expr    = tack_expr [ cmp_op tack_expr | "is" presence ] ;
cmp_op      = "==" | "!=" | "<" | "<=" | ">" | ">=" | "in" ;
presence    = "known" | "missing" ;
tack_expr   = add_expr { tack_op add_expr } ;
tack_op     = "<<" | ">>" | "<:" | ":>" ;
add_expr    = mul_expr { ( "+" | "-" ) mul_expr } ;
mul_expr    = unary_expr { ( "*" | "/" ) unary_expr } ;
unary_expr  = "-" unary_expr | pow_expr ;
pow_expr    = app_expr [ "^" unary_expr ] ;
app_expr    = card_expr { card_expr } ;
card_expr   = "#" card_expr | postfix ;
postfix     = primary { "." ident } ;
primary     = number | string | ident | combiner | lambda | conditional
            | paren | block ;
combiner    = template ;
lambda      = "|" [ ident { "," ident } ] "|" [ ":" type ] or_expr ;
conditional = "if" or_expr "then" or_expr "else" or_expr ;

paren       = "(" ( record_body | collection_body ) ")" ;
record_body = field { "," field } ;
field       = "." ident [ ":" type ] "=" expr ;
collection_body = [ expr { "," expr } ] ;

block       = "{" [ stmt { ";" stmt } [ ";" ] ] "}" ;
stmt        = let_stmt | assert_stmt | expr ;
let_stmt    = "let" ident [ ":" type ] "=" expr ;
assert_stmt = "assert" expr ;
```

The terminals `number`, `string`, `ident`, and `template` are lexer tokens.
Boolean literals (`true`, `false`) and the word operators and statement
keywords (`or`, `and`, `not`, `in`, `is`, `known`, `missing`, `let`,
`assert`) are `ident` tokens recognized by their text in the positions
shown; see the reserved-words note below.  `"|>"` is a single token
(`PipeArrow`): the lexer munches it maximally, so a lone `|` stays a `Pipe`
(a lambda bar), with the closing-bar caveat in `06-expressions.md`.

A `combiner` is a `template` token, the same backtick-quoted form the shape
attribute names use (`## Lexical basis`), read here as the *operator* it
quotes: `` `+` ``, `` `<<` ``, `` `:>` ``.  It needs no lexer work, and
because `` `or` `` and `` `and` `` are templates rather than words, quoting
them raises no reserved-word conflict.  The set of admissible operators is
closed and resolved by the checker, not the grammar, so an unknown combiner
is a typing diagnostic naming the table rather than a parse error
(ADR 0031, Decision 6).

The ordered primitives added no grammar either.  `scan`, `prescan`, and
`desc` are ordinary `ident`s applied by juxtaposition, exactly like `fold`,
`map`, and `to_real`, so none of them is a reserved word and the M0 freeze
surface (the reserved list below, and the FIRST/FOLLOW table) is untouched.
ADR 0031 Decision 7's choice to make a scan's order key an *argument* rather
than a `by` clause is what buys that: a clause would have needed a
production, a reserved word, and a lookahead argument.

Punctuation (`== != < <= > >= + - * / ^ . | ( ) { } [ ] : ; ,`) the lexer
already emits, so the records, blocks, and ascriptions here need no new
tokens.  ADR 0031 adds five: `#` (free, since comments are `//`) plus
`<<`, `>>`, `<:`, and `:>`.  Those four collide with nothing, and maximal
munch settles them: the lexer has no shift tokens, no production puts a `>`
after a `:`, and none puts a `:` after a `<` (there is no generic-argument
syntax, so `<` is only ever a comparison).  (An `NxE` measured literal
(`10x3`) was once reserved here for physical units; ADR 0026 supersedes
it.  Units need no literal form, and a measured-precision literal is
deferred with the precision library.)

`paren` covers grouping, the homogeneous collection, and records: `(e)` is
grouping, `()` the empty collection, `(a, b, ...)` a collection of like values
(the form a `flat_map` body uses to drop or expand rows, ADR 0015), and
`(.a = x, .b = y)` a labeled record.  A `( )` is *either* a positional
collection *or* all-labeled, never mixed.  A heterogeneous sequence
`([ ... ])` is reserved.  `conditional` is the prefix
`if c then a else b` (`if`/`then`/`else` reserved in expressions).  `block` is
a statement block
(`let`/`assert` statements and an optional trailing result expression),
separated by `;`.  Records moved into `( )` means a `{ }` in expression
position is *always* a block, so `completeness_check { assert ... }` is just
`completeness_check` applied to a block, with no special grammar.  Field, let,
and lambda-return ascriptions reuse the declaration grammar's `type`.

### Why the expression grammar is LL(1)

- **Precedence is layered, not recovered by backtracking.**  Each level is a
  left-recursion-free loop (`{ op operand }`) or a single optional
  (`[ op operand ]`) over the next-tighter level, so the operator token at
  hand decides whether to continue.  From loosest to tightest: `|>`, `or`,
  `and`, `not`, the comparisons, `<< >> <: :>`, `+ -`, `* /`, unary `-`, `^`,
  application, `#`, member access.
- **`not_expr`**: the ident `not` selects the prefix branch; any other token
  starts `cmp_expr`.  One token decides.
- **`cmp_expr`**: after the left operand, a comparison operator (or the ident
  `in`) opens the comparison branch and the ident `is` opens the presence
  branch; any other token ends the production, so comparisons do not chain.
  `in` and `is` are distinct idents, so one token picks the branch.  Both
  operand slots are `tack_expr`, so the level below is reachable from either
  side of a comparison.
- **`tack_expr`**: a loop over four distinct tokens (`<<`, `>>`, `<:`, `:>`),
  each one token of lookahead, none of which begins any other production at
  this point; any other token ends the loop.  All four share one level
  because they are one shape of operation, so no mixed expression needs a
  reading the layering does not give it.  Sitting between the comparisons and
  `+ -` is what makes `a + b << c` read `(a + b) << c` and `a << b < c` read
  `(a << b) < c` (ADR 0031, Decision 6).
- **`card_expr`**: `#` selects the prefix branch and recurses; any other
  token starts a `postfix`.  Placing it *inside* the application spine, with
  a `postfix` operand, is what gives `#` its two documented bindings at once:
  looser than member access (`#b.x` is `#(b.x)`, because `postfix` consumes
  the `.x`) and tighter than the comparisons (`#b > 3` is `(#b) > 3`, because
  the comparison level sits above).  It also keeps `#b` usable as an
  argument, so `f #b` is `f (#b)` with one token of lookahead
  (ADR 0031, Decision 9).
- **`pow_expr`**: `^` is right-associative because its right operand is a
  `unary_expr`.  That is also why `2^-3` is `2^(-3)`, while `-2^2` is
  `-(2^2)` (the leading `-` is a `unary_expr` wrapping the whole `pow_expr`).
- **`app_expr` (the application spine)**: the loop consumes another
  `card_expr` while the current token can start one, namely a `number`,
  `string`, `template` (a combiner), `(`, `{`, `#`, `|` (a lambda), or an
  `ident` that is *not* a reserved word (below).  It stops on any other
  operator, on `|>` (a different token from `|`), and on `)` and `,`.  A `|`
  starts a lambda argument; a `|>` never does, so a pipe always ends the
  spine and is handled by `pipe_expr`.  Note `#` is in the start set while
  `<<`/`>>`/`<:`/`:>` are not: a prefix can open an argument, an infix cannot.
- **`primary`**: `number`, `string`, `template`, and `ident` are distinct
  tokens; `(` opens a `paren`; `{` opens a `block`; `|` opens a `lambda`; the
  reserved ident `if` opens a `conditional`.  One token decides.  A
  `template` in this position is a `combiner`; in a declaration's attribute
  position the same token is a name, but that is a different parser state, as
  with `unit`'s two roles.
- **`conditional`**: the reserved ident `if` selects it; `then` and `else`
  are reserved idents that fix the two branch boundaries, so each sub-expression
  (an `or_expr`) is delimited by one token of lookahead.
- **`paren`**: after `(`, the next token chooses the body - `.` opens a
  `record_body` (labeled fields), anything else begins a `collection_body` whose
  first element is an expression; then `,` continues the collection and `)` ends
  a grouping.  Since an expression never starts with `.`, the record/collection
  choice is one token; `()` is the empty collection.  A record field is
  `.name [: Type] = value`; within a field the optional `:` ascription, then
  `=`, are fixed by position.
- **`block`**: `{` opens it; each statement is dispatched on its first token
  (`let` -> `let_stmt`, `assert` -> `assert_stmt`, otherwise a result
  `expr`); `;` separates statements and `}` ends.  This is the only `{ }` in
  expression position, so there is no record-versus-block ambiguity (records
  are `( )`).  As with `unit`'s two roles, a declaration body `{ ... }` is
  reached from a different parser state and never confused with a block.
- **`lambda`**: `|` opens it, an optional comma-separated ident list gives the
  parameters, a closing `|` ends them, then an optional `: Type` return
  ascription, then the body, an `or_expr`.  The `:` after the closing `|`
  decides whether a return type is present; the `type` grammar never starts
  with `(` or `{`, so it cannot swallow the body.  The body deliberately
  excludes a top-level `|>`, so `data |> flat_map |k, r| r.x |> next g` composes as
  `(data |> flat_map (|k, r| r.x)) |> next g`; a pipe *inside* a lambda body must be
  parenthesized.  A lambda that is not the last argument of an application
  must also be parenthesized, since its body extends maximally.

### FIRST/FOLLOW verification (the M0 freeze condition)

The per-production prose above argues decidability informally; this subsection
discharges the `ROADMAP.md` M0 condition explicitly: no left recursion, disjoint
FIRST sets at every choice, and FIRST/FOLLOW disjoint at every nullable
production.

**No left recursion.**  The expression grammar is a precedence cascade: each
non-terminal references only strictly tighter levels (`pipe_expr` -> `or_expr`
-> ... -> `primary`), and repetition is written as a right-iterative loop
`{ op operand }`, never as `A = A op B`.  `unary_expr = "-" unary_expr | ...`
and `not_expr = "not" not_expr | ...` recurse only after consuming a terminal
(`-`, `not`), so they are not left-recursive.  No production can derive itself
without first consuming input.

**Disjoint FIRST at each choice.**  The only productions with alternatives are
`not_expr` (`not` vs `FIRST(cmp_expr)`), `unary_expr` (`-` vs
`FIRST(pow_expr)`), `card_expr` (`#` vs `FIRST(postfix)`), `primary`,
`paren`'s body, and `stmt`.  `not`, `-`, `#`, and `if` are tokens distinct
from any value-starting token; `primary`'s six arms start with the disjoint
tokens `number` / `string` / `template` / `ident` / `|` / `{` (and `(` for
`paren`); `paren`'s body splits on `.` (record) versus everything else
(collection), and an expression never starts with `.`; `stmt` splits on the
reserved idents `let` / `assert` versus any other expression-starting token.

**FIRST/FOLLOW disjoint at each nullable or optional production.**  The nullable
points and their checks:

| nullable / optional | FIRST(optional part) | FOLLOW (what ends it) | disjoint? |
| --- | --- | --- | --- |
| `cmp_expr` tail `[ cmp_op tack_expr \| "is" presence ]` | `== != < <= > >=`, `in`, `is` | `and or \|> ) , ; } then else` | yes |
| `pow_expr` tail `[ "^" unary_expr ]` | `^` | everything looser than `^` | yes (`^` is not in FOLLOW) |
| each loop `{ op operand }` (`\|>`, `or`, `and`, `<< >> <: :>`, `+ -`, `* /`) | that level's operator token(s) | the next looser operator or a terminator | yes (operators are partitioned by level) |
| `app_expr = card_expr { card_expr }` | `number string template ident( non-reserved ) ( \| { #` | any infix operator, `\|>`, `) , then else ; }` | yes (no infix operator or terminator starts a `card_expr`) |
| `lambda` params `[ ident { "," ident } ]` | `ident` | `\|` (closing bar) | yes |
| return / field / let ascription `[ ":" type ]` | `:` | lambda body start, `=`, `}` | yes (`:` is distinct) |
| `collection_body = [ expr { "," expr } ]` | `FIRST(expr)` | `)` | yes (`expr` never starts with `)`) |
| `block` body `[ stmt { ";" stmt } ]` | `FIRST(stmt)` | `}` | yes (`stmt` never starts with `}`) |

Every choice is settled by one token of lookahead and no nullable production can
be confused with what follows it, so the expression sublanguage is LL(1).  With
the declaration grammar (proven above), the whole core grammar meets the M0
freeze condition.

### Reserved words in expressions

Combining juxtaposition application with word operators forces a small,
local exception to the lexer's keyword-freedom: inside an expression the
words `or`, `and`, `not`, `in`, `is`, `known`, `missing`, `if`, `then`,
`else`, and the statement keywords `let` and `assert` are **reserved** and
cannot name a value.  This is unavoidable with one token of lookahead,
since after an operand an ident could otherwise be read either as the next
argument (juxtaposition) or as an infix operator, and only reservation
resolves the choice.  Reserving `let` and `assert` throughout expressions
(not merely in statement position) closes the statement-boundary leak: in
`{ let t = a let u = b }` the missing `;` would otherwise let the
application spine read `a let` as an application and surface a mislocated
error (or, once general application lands, a mis-typed one); with the
reservation the parser stops at `let` and reports the missing separator.
For the same reason a reserved word cannot *name* a `let` binding: a value
named `let` or `known` could never be referenced.  The reservation is
local to the expression sublanguage; elsewhere these words remain ordinary
identifiers, as the keyword-free lexer intends.

## Forward references

- Numeric and predicate parameter kinds, and the parameter list on function
  signatures.
- Compound units, `domain` resolution, and foreign keys.
- Annotations (`@audited`, `@versioned`, `@auto`, `@domain`, ...).
- Precision types and any measured-precision literal (deferred with the
  precision library, ADR 0026 Decision 9; the dimensioned-type grammar is
  implemented above).
- Module-qualified names in type position (`geo.speed[real]`), and the
  `exposing` import refinement (`12-modules-and-imports.md`).
- The pipeline operations (`flat_map`, `map_bags`, `promote`/`demote`,
  joins, `split`/`union`, `unpivot`/`pivot`, `completeness_check`) are
  specified in `07-pipelines.md`; they are builtins applied through the
  expression grammar above (record literals, blocks, juxtaposition) and add no
  new grammar.
- `view` declarations host a pipeline and are specified in
  `10-views.md` (the `view_decl` production above is their grammar).  `device`
  and transforms, which also host or feed pipelines, and the streaming
  operations (`sliding_window`, `latest`), each get their own section here.
