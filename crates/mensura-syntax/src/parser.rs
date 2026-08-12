//! A hand-written, recursive-descent parser for the unit and store subset.
//!
//! It consumes the token slice produced by the lexer and builds the AST in
//! `ast.rs`, following the LL(1) grammar in `docs/language/04-grammar.md`:
//! one token of lookahead, no backtracking.  Keywords are contextual, matched
//! on the `Ident` text in the position where they are expected.

use crate::ast::{
    Attr, DomainEntry, EnumDecl, Field, Ident, ImportDecl, Item, LetDecl, LetKind, NameSeg,
    NameTemplate, Program, ShapeArg, ShapeDecl, ShapeParam, ShapeRef, StoreDecl, StoreKind, StrLit,
    TypeExpr, TypeKind, UnitDecl, ViewDecl,
};
use crate::expr::{BinOp, Block, Expr, ExprKind, Presence, RecordField, Stmt, UnOp};
use crate::token::{Span, Token, TokenKind};

/// A parse failure, located by a source span.
#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

/// A parsed program together with the side information later passes want but
/// the AST does not carry.  Currently the spans of every contextual keyword
/// the parser recognized: since the lexer is keyword-free, the parser is the
/// only place that knows an `Ident` was acting as a keyword in context, so it
/// records that here for tooling (highlighting, formatting) to reuse.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub program: Program,
    /// Spans of the contextual keywords, in source order: the declaration
    /// headers (`unit`, `store`, `shape`, `attr`, `domain`, `enum`,
    /// `view`), the conditional (`if`, `then`, `else`), the predicate operators
    /// (`or`, `and`, `not`, `is`, `known`, `missing`), and the statement
    /// keywords (`let`, `assert`).
    pub keyword_spans: Vec<Span>,
    /// Spans of pipeline operation heads: the leftmost identifier of each `|>`
    /// right-hand side, in source order.  Recognized by position, so any
    /// identifier in operation position is recorded (a mistyped operation is
    /// still colored; `resolve` reports the unknown-operation error).
    pub op_spans: Vec<Span>,
}

/// Parse a token slice (lexer output, ending in [`TokenKind::Eof`]) into a
/// [`Program`].
pub fn parse(tokens: &[Token]) -> Result<Program, ParseError> {
    parse_with_meta(tokens).map(|parsed| parsed.program)
}

/// Parse a token slice into a [`Parsed`]: the [`Program`] plus the
/// classified-span table (keyword spans).  Used by the language server.
pub fn parse_with_meta(tokens: &[Token]) -> Result<Parsed, ParseError> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        keyword_spans: Vec::new(),
        op_spans: Vec::new(),
    };
    let program = parser.parse_program()?;
    Ok(Parsed {
        program,
        keyword_spans: parser.keyword_spans,
        op_spans: parser.op_spans,
    })
}

/// Parse a token slice (lexer output, ending in [`TokenKind::Eof`]) as a single
/// expression, per the expression grammar in `docs/language/04-grammar.md`.
///
/// The whole input must be one expression: trailing tokens are an error.  This
/// is the entry the future expression-hosting sites (`when:`, `where:`,
/// `@auto`, pipeline operations) and the test corpus exercise.
pub fn parse_expr(tokens: &[Token]) -> Result<Expr, ParseError> {
    let mut parser = Parser {
        tokens,
        pos: 0,
        keyword_spans: Vec::new(),
        op_spans: Vec::new(),
    };
    let expr = parser.parse_expr_inner()?;
    if !parser.at_eof() {
        return Err(parser.error("unexpected token after expression"));
    }
    Ok(expr)
}

/// The words reserved inside an expression: they can never name a value.
/// See the "Reserved words in expressions" note in
/// `docs/language/04-grammar.md`.  The statement keywords `let` and
/// `assert` are reserved throughout expressions, not merely in statement
/// position: otherwise a missing `;` lets the application spine swallow
/// the next statement's keyword (`{ let t = a let u = b }`) and the error
/// surfaces far from the mistake, or not at all.
fn is_expr_reserved(word: &str) -> bool {
    matches!(
        word,
        "or" | "and"
            | "not"
            | "in"
            | "is"
            | "known"
            | "missing"
            | "if"
            | "then"
            | "else"
            | "let"
            | "assert"
    )
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    keyword_spans: Vec<Span>,
    op_spans: Vec<Span>,
}

impl<'a> Parser<'a> {
    // --- cursor helpers -----------------------------------------------------

    fn cur_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn cur_span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn at_eof(&self) -> bool {
        matches!(self.cur_kind(), TokenKind::Eof)
    }

    fn check(&self, kind: &TokenKind) -> bool {
        self.cur_kind() == kind
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Consume the expected punctuation token or fail with `what`.
    fn expect(&mut self, kind: &TokenKind, what: &str) -> Result<Span, ParseError> {
        if self.check(kind) {
            let span = self.cur_span();
            self.pos += 1;
            Ok(span)
        } else {
            Err(self.error(format!("expected {what}")))
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<Ident, ParseError> {
        match self.cur_kind() {
            TokenKind::Ident(name) => {
                let id = Ident {
                    name: name.clone(),
                    span: self.cur_span(),
                };
                self.pos += 1;
                Ok(id)
            }
            _ => Err(self.error(format!("expected {what}"))),
        }
    }

    /// An identifier that names a `let` binding: like [`Parser::expect_ident`],
    /// but a reserved word is rejected (a value named `let` or `known` could
    /// never be referenced; `docs/language/04-grammar.md`, "Reserved words in
    /// expressions").
    fn expect_binder(&mut self, what: &str) -> Result<Ident, ParseError> {
        if let TokenKind::Ident(name) = self.cur_kind()
            && is_expr_reserved(name)
        {
            return Err(self.error(format!(
                "`{name}` is a reserved word and cannot name a binding"
            )));
        }
        self.expect_ident(what)
    }

    fn expect_str(&mut self, what: &str) -> Result<StrLit, ParseError> {
        match self.cur_kind() {
            TokenKind::Str(value) => {
                let lit = StrLit {
                    value: value.clone(),
                    span: self.cur_span(),
                };
                self.pos += 1;
                Ok(lit)
            }
            _ => Err(self.error(format!("expected {what}"))),
        }
    }

    fn at_keyword(&self, word: &str) -> bool {
        matches!(self.cur_kind(), TokenKind::Ident(s) if s == word)
    }

    /// Consume the current token, which the caller has confirmed is a
    /// contextual keyword, recording its span for the classified-span table.
    fn bump_keyword(&mut self) {
        self.keyword_spans.push(self.cur_span());
        self.pos += 1;
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            span: self.cur_span(),
        }
    }

    // --- grammar ------------------------------------------------------------

    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut items = Vec::new();
        while !self.at_eof() {
            items.push(self.parse_item()?);
        }
        Ok(Program { items })
    }

    fn parse_item(&mut self) -> Result<Item, ParseError> {
        if self.at_keyword("unit") {
            Ok(Item::Unit(self.parse_unit_decl()?))
        } else if self.at_keyword("store") {
            Ok(Item::Store(self.parse_store_decl(StoreKind::Store)?))
        } else if self.at_keyword("registry") {
            Ok(Item::Registry(self.parse_store_decl(StoreKind::Registry)?))
        } else if self.at_keyword("shape") {
            Ok(Item::Shape(self.parse_shape_decl()?))
        } else if self.at_keyword("enum") {
            Ok(Item::Enum(self.parse_enum_decl()?))
        } else if self.at_keyword("view") {
            Ok(Item::View(self.parse_view_decl()?))
        } else if self.at_keyword("let") {
            Ok(Item::Let(self.parse_let_decl()?))
        } else if self.at_keyword("import") {
            Ok(Item::Import(self.parse_import_decl()?))
        } else {
            Err(self.error(
                "expected a `unit`, `store`, `registry`, `shape`, `enum`, `view`, `let`, or \
                 `import` declaration",
            ))
        }
    }

    /// `import_decl = "import" ident`
    /// (`docs/language/12-modules-and-imports.md`, ADR 0027).
    fn parse_import_decl(&mut self) -> Result<ImportDecl, ParseError> {
        let start = self.cur_span().start;
        self.bump_keyword(); // `import`
        let name = self.expect_ident("a module name")?;
        let span = Span::new(start, name.span.end);
        Ok(ImportDecl { name, span })
    }

    /// `let_decl = "let" ident ( value_let | alias_let )`.  After the name,
    /// `[` opens a dimension alias's parameter list and a braced body parsed
    /// with the type grammar follows (ADR 0026, Decision 8); `:` or `{`
    /// continues the value form, whose body is the ordinary statement block
    /// (ADR 0027, Decision 1 as revised).  Like every other item body, both
    /// forms are brace-closed, so item boundaries stay independent of the
    /// expression grammar.  One token decides.
    fn parse_let_decl(&mut self) -> Result<LetDecl, ParseError> {
        let start = self.cur_span().start;
        self.bump_keyword(); // `let`
        let name = self.expect_binder("a name after `let`")?;
        if self.eat(&TokenKind::LBracket) {
            let mut params = vec![self.expect_ident("an alias parameter")?];
            while self.eat(&TokenKind::Comma) {
                params.push(self.expect_ident("an alias parameter")?);
            }
            self.expect(&TokenKind::RBracket, "`]` to close the alias parameters")?;
            self.expect(&TokenKind::LBrace, "`{` to open the alias body")?;
            let body = self.parse_tl_expr()?;
            let end = self
                .expect(&TokenKind::RBrace, "`}` to close the alias body")?
                .end;
            Ok(LetDecl {
                name,
                kind: LetKind::DimAlias { params, body },
                span: Span::new(start, end),
            })
        } else {
            let ty = if self.eat(&TokenKind::Colon) {
                Some(self.parse_type()?)
            } else {
                None
            };
            if !self.check(&TokenKind::LBrace) {
                return Err(self.error(
                    "`{` to open the `let` body (a top-level `let` is brace-closed: \
                     `let name { ... }`)",
                ));
            }
            let value = self.parse_block()?;
            let span = Span::new(start, value.span.end);
            Ok(LetDecl {
                name,
                kind: LetKind::Value { ty, value },
                span,
            })
        }
    }

    /// `view_decl = "view" ident [ conforms ] block` (`docs/language/10-views.md`).
    fn parse_view_decl(&mut self) -> Result<ViewDecl, ParseError> {
        let start = self.cur_span().start;
        self.bump_keyword(); // `view`
        let name = self.expect_ident("a view name")?;
        let conforms = self.parse_conforms_clause()?;
        let body = self.parse_block()?;
        let end = body.span.end;
        Ok(ViewDecl {
            name,
            conforms,
            body,
            span: Span::new(start, end),
        })
    }

    fn parse_enum_decl(&mut self) -> Result<EnumDecl, ParseError> {
        let start = self.cur_span().start;
        self.bump_keyword(); // `enum`
        let name = self.expect_ident("an enum name")?;
        self.expect(&TokenKind::LBrace, "`{` to open the enum body")?;
        let mut variants = vec![self.expect_str("an enum variant string")?];
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            variants.push(self.expect_str("an enum variant string")?);
        }
        let end = self.expect(&TokenKind::RBrace, "`}` to close the enum")?;
        Ok(EnumDecl {
            name,
            variants,
            span: Span::new(start, end.end),
        })
    }

    fn parse_unit_decl(&mut self) -> Result<UnitDecl, ParseError> {
        let start = self.cur_span().start;
        self.bump_keyword(); // `unit`
        let name = self.expect_ident("a unit name")?;
        self.expect(&TokenKind::LBrace, "`{` to open the unit body")?;
        let mut fields = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            fields.push(self.parse_field()?);
        }
        let end = self.expect(&TokenKind::RBrace, "`}` to close the unit body")?;
        Ok(UnitDecl {
            name,
            fields,
            span: Span::new(start, end.end),
        })
    }

    /// `store_decl` and the identical `registry_decl` (ADR 0033): the two
    /// share a production, so they share a parser, and `kind` supplies the
    /// noun for each diagnostic.
    fn parse_store_decl(&mut self, kind: StoreKind) -> Result<StoreDecl, ParseError> {
        let start = self.cur_span().start;
        self.bump_keyword(); // `store` or `registry`
        let name = self.expect_ident(&format!("a {} name", kind.keyword()))?;
        let conforms = self.parse_conforms_clause()?;
        self.expect(
            &TokenKind::LBrace,
            &format!("`{{` to open the {} body", kind.keyword()),
        )?;
        let unit = self.parse_unit_clause()?;

        let mut attrs = Vec::new();
        let mut domain = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            if self.at_keyword("attr") {
                self.parse_attr_block(&mut attrs)?;
            } else if self.at_keyword("domain") {
                self.parse_domain_block(&mut domain)?;
            } else if self.at_keyword("unit") {
                return Err(
                    self.error("the `unit` clause may appear only once, at the start of the body")
                );
            } else {
                return Err(self.error("expected `attr`, `domain`, or `}`"));
            }
        }
        let end = self.expect(
            &TokenKind::RBrace,
            &format!("`}}` to close the {} body", kind.keyword()),
        )?;
        Ok(StoreDecl {
            kind,
            name,
            unit,
            conforms,
            attrs,
            domain,
            span: Span::new(start, end.end),
        })
    }

    fn parse_shape_decl(&mut self) -> Result<ShapeDecl, ParseError> {
        let start = self.cur_span().start;
        self.bump_keyword(); // `shape`
        let name = self.expect_ident("a shape name")?;
        let params = self.parse_param_list()?;
        self.expect(&TokenKind::LBrace, "`{` to open the shape body")?;
        // The unit clause is optional: a shape with none is unit-agnostic.
        let unit = if self.at_keyword("unit") {
            Some(self.parse_unit_clause()?)
        } else {
            None
        };

        let mut attrs = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            if self.at_keyword("attr") {
                self.parse_attr_block(&mut attrs)?;
            } else if self.at_keyword("domain") {
                return Err(self.error("a shape cannot contain a `domain` block"));
            } else if self.at_keyword("unit") {
                return Err(
                    self.error("the `unit` clause may appear only once, at the start of the body")
                );
            } else {
                return Err(self.error("expected `attr` or `}`"));
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`}` to close the shape body")?;
        Ok(ShapeDecl {
            name,
            params,
            unit,
            attrs,
            span: Span::new(start, end.end),
        })
    }

    /// Parse the optional `: ShapeRef, ShapeRef, ...` conformance clause that
    /// may follow a store name.  Returns an empty vector when absent.
    fn parse_conforms_clause(&mut self) -> Result<Vec<ShapeRef>, ParseError> {
        let mut shapes = Vec::new();
        if self.eat(&TokenKind::Colon) {
            shapes.push(self.parse_shape_ref("a shape name after `:`")?);
            while self.eat(&TokenKind::Comma) {
                shapes.push(self.parse_shape_ref("a shape name after `,`")?);
            }
        }
        Ok(shapes)
    }

    /// Parse one conformance entry: a shape name with an optional positional
    /// argument list, e.g. `Tabular[Person]` or `PersonRecord`.
    fn parse_shape_ref(&mut self, what: &str) -> Result<ShapeRef, ParseError> {
        let name = self.expect_ident(what)?;
        let mut args = Vec::new();
        let mut end = name.span.end;
        if self.check(&TokenKind::LBracket) {
            self.pos += 1; // `[`
            args.push(self.parse_shape_arg()?);
            while self.eat(&TokenKind::Comma) {
                args.push(self.parse_shape_arg()?);
            }
            end = self
                .expect(&TokenKind::RBracket, "`]` to close the argument list")?
                .end;
        }
        Ok(ShapeRef {
            span: Span::new(name.span.start, end),
            name,
            args,
        })
    }

    /// Parse one conformance argument: a bare identifier (for a `Unit`
    /// parameter) or a string literal (for a `string` parameter).
    fn parse_shape_arg(&mut self) -> Result<ShapeArg, ParseError> {
        match self.cur_kind() {
            TokenKind::Ident(name) => {
                let id = Ident {
                    name: name.clone(),
                    span: self.cur_span(),
                };
                self.pos += 1;
                Ok(ShapeArg::Unit(id))
            }
            TokenKind::Str(value) => {
                let lit = StrLit {
                    value: value.clone(),
                    span: self.cur_span(),
                };
                self.pos += 1;
                Ok(ShapeArg::Str(lit))
            }
            _ => Err(self.error("expected a shape argument (a unit name or a string)")),
        }
    }

    /// Parse an optional `[name: Kind, ...]` shape parameter list.  Returns an
    /// empty vector when absent.  An empty `[]` is rejected.
    fn parse_param_list(&mut self) -> Result<Vec<ShapeParam>, ParseError> {
        let mut params = Vec::new();
        if self.check(&TokenKind::LBracket) {
            self.pos += 1; // `[`
            if self.check(&TokenKind::RBracket) {
                return Err(self.error("empty parameter list; omit the `[]`"));
            }
            params.push(self.parse_param()?);
            while self.eat(&TokenKind::Comma) {
                params.push(self.parse_param()?);
            }
            self.expect(&TokenKind::RBracket, "`]` to close the parameter list")?;
        }
        Ok(params)
    }

    /// Parse one shape parameter `name : Kind`.
    fn parse_param(&mut self) -> Result<ShapeParam, ParseError> {
        let name = self.expect_ident("a parameter name")?;
        self.expect(&TokenKind::Colon, "`:` after the parameter name")?;
        let kind = self.expect_ident("a parameter kind (e.g. `Unit`)")?;
        Ok(ShapeParam {
            span: Span::new(name.span.start, kind.span.end),
            name,
            kind,
        })
    }

    /// Parse the `unit { U }` clause that opens a store or shape body.
    fn parse_unit_clause(&mut self) -> Result<Ident, ParseError> {
        if !self.at_keyword("unit") {
            return Err(self.error("the body must begin with a `unit { ... }` clause"));
        }
        self.bump_keyword(); // `unit`
        self.expect(&TokenKind::LBrace, "`{` after `unit`")?;
        let unit = self.expect_ident("the tabulated unit name")?;
        self.expect(&TokenKind::RBrace, "`}` to close the `unit` clause")?;
        Ok(unit)
    }

    /// Parse an `attr { ... }` or `attr* { ... }` block, appending its
    /// attributes.  Shared by stores and shapes; repeated blocks merge into
    /// one attribute list.  The optional `*` (the `attr` identifier followed
    /// by the `Star` operator, ADR 0022) marks every attribute in the block
    /// bag-valued; one token after `attr` decides, so the form stays LL(1).
    fn parse_attr_block(&mut self, out: &mut Vec<Attr>) -> Result<(), ParseError> {
        self.bump_keyword(); // `attr`
        let many = if self.check(&TokenKind::Star) {
            let span = self.cur_span();
            self.pos += 1;
            Some(span)
        } else {
            None
        };
        self.expect(&TokenKind::LBrace, "`{` to open the block")?;
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            out.push(Attr {
                field: self.parse_field()?,
                many,
            });
        }
        self.expect(&TokenKind::RBrace, "`}` to close the block")?;
        Ok(())
    }

    /// Parse an attribute or field name: a plain identifier (one literal
    /// segment) or a backtick template split into literal and `{param}`
    /// segments.  Interpolating templates only resolve in a shape, but the
    /// surface syntax is uniform across units, stores, and shapes.
    fn parse_name_template(&mut self) -> Result<NameTemplate, ParseError> {
        match self.cur_kind() {
            TokenKind::Ident(name) => {
                let span = self.cur_span();
                let segments = vec![NameSeg::Lit(name.clone())];
                self.pos += 1;
                Ok(NameTemplate { segments, span })
            }
            TokenKind::Template(raw) => {
                let raw = raw.clone();
                let span = self.cur_span();
                self.pos += 1;
                split_template(&raw, span)
            }
            _ => Err(self.error("expected an attribute name")),
        }
    }

    fn parse_domain_block(&mut self, out: &mut Vec<DomainEntry>) -> Result<(), ParseError> {
        self.bump_keyword(); // `domain`
        self.expect(&TokenKind::LBrace, "`{` to open the domain block")?;
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            let field = self.expect_ident("a field name")?;
            self.expect(&TokenKind::Colon, "`:` after the field name")?;
            let store = self.expect_ident("a store name")?;
            let span = Span::new(field.span.start, store.span.end);
            out.push(DomainEntry { field, store, span });
        }
        self.expect(&TokenKind::RBrace, "`}` to close the domain block")?;
        Ok(())
    }

    /// `name : type`, shared by unit fields and store/shape attributes.  The
    /// name may be a backtick template.
    fn parse_field(&mut self) -> Result<Field, ParseError> {
        let name = self.parse_name_template()?;
        self.expect(&TokenKind::Colon, "`:` after the field name")?;
        let ty = self.parse_type()?;
        let span = Span::new(name.span.start, ty.span().end);
        Ok(Field { name, ty, span })
    }

    /// Parse a type: a type-level expression (`tl_expr`,
    /// `docs/language/04-grammar.md`), optionally followed by a `?` optional
    /// marker.  The common case is still a single identifier (a primitive, a
    /// unit reference, or a named `enum`); the dimension forms of
    /// `11-physical-units.md` continue from it with `[`, `*`, `/`, or `^`.
    /// Which the base type is, is the resolver's decision; the `?` makes the
    /// value optional (ADR 0010).  One token of lookahead takes a single `?`
    /// if present, so the marker preserves LL(1).
    fn parse_type(&mut self) -> Result<TypeExpr, ParseError> {
        let mut ty = self.parse_tl_expr()?;
        if self.check(&TokenKind::Question) {
            let q = self.cur_span();
            self.pos += 1;
            ty.optional = Some(q);
            ty.span = Span::new(ty.span.start, q.end);
        }
        Ok(ty)
    }

    /// `tl_expr = tl_term { ("*" | "/") tl_term }`, left-associative.
    fn parse_tl_expr(&mut self) -> Result<TypeExpr, ParseError> {
        let mut lhs = self.parse_tl_term()?;
        loop {
            let mul = self.check(&TokenKind::Star);
            if !mul && !self.check(&TokenKind::Slash) {
                break;
            }
            self.pos += 1;
            let rhs = self.parse_tl_term()?;
            let span = Span::new(lhs.span.start, rhs.span.end);
            let kind = if mul {
                TypeKind::Mul(Box::new(lhs), Box::new(rhs))
            } else {
                TypeKind::Div(Box::new(lhs), Box::new(rhs))
            };
            lhs = TypeExpr {
                kind,
                optional: None,
                span,
            };
        }
        Ok(lhs)
    }

    /// `tl_term = tl_factor [ "^" [ "-" ] int ]`.  The exponent is an
    /// integer literal (the result dimension is computed at compile time),
    /// optionally negated; the lexer has no signed-int token, so the `-` is
    /// the ordinary `Minus`.
    fn parse_tl_term(&mut self) -> Result<TypeExpr, ParseError> {
        let base = self.parse_tl_factor()?;
        if !self.eat(&TokenKind::Caret) {
            return Ok(base);
        }
        let negated = self.eat(&TokenKind::Minus);
        let TokenKind::Int(n) = *self.cur_kind() else {
            return Err(self.error("expected an integer exponent after `^` in a type"));
        };
        let end = self.cur_span().end;
        self.pos += 1;
        let signed = if negated { -n } else { n };
        let exp = i32::try_from(signed)
            .map_err(|_| self.error("the type-level exponent is out of range"))?;
        let span = Span::new(base.span.start, end);
        Ok(TypeExpr {
            kind: TypeKind::Pow(Box::new(base), exp),
            optional: None,
            span,
        })
    }

    /// `tl_factor = ident [ "[" ident "]" ] | "(" tl_expr ")" "[" ident "]"`.
    /// The bracket argument is a single identifier (a backing such as `real`,
    /// or an alias parameter); after a parenthesized group the bracket is
    /// mandatory, since a bare parenthesized dimension is not a type.
    fn parse_tl_factor(&mut self) -> Result<TypeExpr, ParseError> {
        if self.check(&TokenKind::LParen) {
            let start = self.cur_span().start;
            self.pos += 1;
            let inner = self.parse_tl_expr()?;
            self.expect(&TokenKind::RParen, "`)` to close the dimension group")?;
            self.expect(
                &TokenKind::LBracket,
                "`[` to apply the dimension to a backing (write `(...)[real]`)",
            )?;
            let backing = self.expect_ident("a backing type such as `real`")?;
            let end = self
                .expect(&TokenKind::RBracket, "`]` to close the type application")?
                .end;
            return Ok(TypeExpr {
                kind: TypeKind::Apply {
                    base: Box::new(inner),
                    backing,
                },
                optional: None,
                span: Span::new(start, end),
            });
        }
        let name = self.expect_ident("a type")?;
        let start = name.span.start;
        let named = TypeExpr {
            span: name.span,
            kind: TypeKind::Named(name),
            optional: None,
        };
        if !self.eat(&TokenKind::LBracket) {
            return Ok(named);
        }
        let backing = self.expect_ident("a backing type such as `real`")?;
        let end = self
            .expect(&TokenKind::RBracket, "`]` to close the type application")?
            .end;
        Ok(TypeExpr {
            kind: TypeKind::Apply {
                base: Box::new(named),
                backing,
            },
            optional: None,
            span: Span::new(start, end),
        })
    }

    // --- expression sublanguage ---------------------------------------------
    //
    // One method per grammar production in `docs/language/04-grammar.md`,
    // layered loosest-binding to tightest.  Each level is a left-recursion-free
    // loop or a single optional over the next-tighter level, so one token of
    // lookahead decides whether to continue.

    /// `expr = pipe_expr`.
    fn parse_expr_inner(&mut self) -> Result<Expr, ParseError> {
        self.parse_pipe()
    }

    /// `pipe_expr = logic_expr { "|>" logic_expr }`, left-associative.  The
    /// pipe is structural and loosest, so it accepts an unparenthesized glue
    /// result (ADR 0040): a stage ending in a discharge needs no parens.
    fn parse_pipe(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_logic()?;
        while self.check(&TokenKind::PipeArrow) {
            self.pos += 1;
            let rhs = self.parse_logic()?;
            self.record_pipe_op(&rhs);
            lhs = self.binary(BinOp::Pipe, lhs, rhs);
        }
        Ok(lhs)
    }

    /// Record the operation head of a `|>` right-hand side.  The side is a
    /// curried application; its leftmost atom is the operation.  When that atom
    /// is a name (it always is for a real operation), its span is recorded for
    /// highlighting.
    fn record_pipe_op(&mut self, rhs: &Expr) {
        let mut head = rhs;
        while let ExprKind::App(callee, _) = &head.kind {
            head = callee;
        }
        if matches!(head.kind, ExprKind::Name(_)) {
            self.op_spans.push(head.span);
        }
    }

    /// `logic_expr = not_expr { logic_word not_expr }`: `and` and `or` share
    /// one homogeneous level (ADR 0040, Decision 2).  The chain commits to
    /// its first word, so `a and b and c` folds left while `a and b or c` is
    /// a parse error asking for parentheses.  An operand carrying an
    /// unparenthesized glue operator is also an error (Decision 3):
    /// `a ?? b and c` must be written `(a ?? b) and c`.
    fn parse_logic(&mut self) -> Result<Expr, ParseError> {
        let (mut lhs, lhs_glue) = self.parse_not()?;
        let word = if self.at_keyword("and") {
            "and"
        } else if self.at_keyword("or") {
            "or"
        } else {
            return Ok(lhs);
        };
        if let Some(glue) = lhs_glue {
            return Err(self.mixing_error(glue, word));
        }
        let op = if word == "and" { BinOp::And } else { BinOp::Or };
        while self.at_keyword(word) {
            self.bump_keyword();
            let (rhs, rhs_glue) = self.parse_not()?;
            if let Some(glue) = rhs_glue {
                return Err(self.mixing_error(glue, word));
            }
            lhs = self.binary(op, lhs, rhs);
        }
        let other = if word == "and" { "or" } else { "and" };
        if self.at_keyword(other) {
            return Err(self.mixing_error(word, other));
        }
        Ok(lhs)
    }

    /// `not_expr = "not" not_expr | cmp_expr`.  Alongside the expression it
    /// reports an unparenthesized glue operator at its top (through any
    /// `not` prefixes), so `parse_logic` can enforce the ADR 0040 meeting
    /// rule on its operands.
    fn parse_not(&mut self) -> Result<(Expr, Option<&'static str>), ParseError> {
        if self.at_keyword("not") {
            let start = self.cur_span().start;
            self.bump_keyword();
            let (inner, glue) = self.parse_not()?;
            let span = Span::new(start, inner.span.end);
            Ok((
                Expr {
                    kind: ExprKind::Unary(UnOp::Not, Box::new(inner)),
                    span,
                },
                glue,
            ))
        } else {
            self.parse_cmp()
        }
    }

    /// `cmp_expr = glue_expr [ cmp_op glue_expr | "is" presence ]`.
    /// Non-associative: at most one comparison or presence test.  The glue
    /// operators have no rank against the comparisons (ADR 0040,
    /// Decision 3), so an unparenthesized glue chain on either side of a
    /// consumed comparison or `is` is a parse error; when no comparison is
    /// consumed the flag propagates for `parse_logic` to check.
    fn parse_cmp(&mut self) -> Result<(Expr, Option<&'static str>), ParseError> {
        let (lhs, lhs_glue) = self.parse_glue()?;
        if self.at_keyword("is") {
            if let Some(glue) = lhs_glue {
                return Err(self.mixing_error(glue, "is"));
            }
            self.bump_keyword();
            let pres = if self.at_keyword("known") {
                self.bump_keyword();
                Presence::Known
            } else if self.at_keyword("missing") {
                self.bump_keyword();
                Presence::Missing
            } else {
                return Err(self.error("expected `known` or `missing` after `is`"));
            };
            let end = self.tokens[self.pos - 1].span.end;
            let span = Span::new(lhs.span.start, end);
            return Ok((
                Expr {
                    kind: ExprKind::Presence(Box::new(lhs), pres),
                    span,
                },
                None,
            ));
        }
        let op = match self.cur_kind() {
            TokenKind::EqEq => Some((BinOp::Eq, "==")),
            TokenKind::BangEq => Some((BinOp::Ne, "!=")),
            TokenKind::Lt => Some((BinOp::Lt, "<")),
            TokenKind::LtEq => Some((BinOp::Le, "<=")),
            TokenKind::Gt => Some((BinOp::Gt, ">")),
            TokenKind::GtEq => Some((BinOp::Ge, ">=")),
            TokenKind::Ident(s) if s == "in" => Some((BinOp::In, "in")),
            _ => None,
        };
        match op {
            Some((op, spelling)) => {
                if let Some(glue) = lhs_glue {
                    return Err(self.mixing_error(glue, spelling));
                }
                self.pos += 1;
                let (rhs, rhs_glue) = self.parse_glue()?;
                if let Some(glue) = rhs_glue {
                    return Err(self.mixing_error(glue, spelling));
                }
                Ok((self.binary(op, lhs, rhs), None))
            }
            None => Ok((lhs, lhs_glue)),
        }
    }

    /// `glue_expr = add_expr [ glue_op add_expr { glue_op add_expr } ]`: the
    /// unranked level (ADR 0040, Decision 3).  `??` and the four tacks share
    /// it, and a chain is one operator, committed at the first glue token:
    /// `??` is right-associative (a chain discharges at its first present
    /// value, ADR 0039) and each tack folds left (ADR 0031).  A different
    /// glue operator mid-chain is a parse error asking for parentheses, and
    /// the returned flag lets the comparison and logic levels reject an
    /// unparenthesized meeting on their side.
    fn parse_glue(&mut self) -> Result<(Expr, Option<&'static str>), ParseError> {
        let lhs = self.parse_add()?;
        let Some((op, spelling)) = self.glue_op() else {
            return Ok((lhs, None));
        };
        let mut operands = vec![lhs];
        while let Some((next, _)) = self.glue_op() {
            if next != op {
                break;
            }
            self.pos += 1;
            operands.push(self.parse_add()?);
        }
        if let Some((_, other)) = self.glue_op() {
            return Err(self.mixing_error(spelling, other));
        }
        let expr = if op == BinOp::Coalesce {
            let mut rev = operands.into_iter().rev();
            let mut acc = rev.next().expect("the chain holds at least the lhs");
            for operand in rev {
                acc = self.binary(op, operand, acc);
            }
            acc
        } else {
            let mut fwd = operands.into_iter();
            let mut acc = fwd.next().expect("the chain holds at least the lhs");
            for operand in fwd {
                acc = self.binary(op, acc, operand);
            }
            acc
        };
        Ok((expr, Some(spelling)))
    }

    /// The glue tokens: `??` (ADR 0039) and the four tacks (ADR 0031).
    fn glue_op(&self) -> Option<(BinOp, &'static str)> {
        match self.cur_kind() {
            TokenKind::QuestionQuestion => Some((BinOp::Coalesce, "??")),
            TokenKind::LtLt => Some((BinOp::Min, "<<")),
            TokenKind::GtGt => Some((BinOp::Max, ">>")),
            TokenKind::LtColon => Some((BinOp::KeepLeft, "<:")),
            TokenKind::ColonGt => Some((BinOp::KeepRight, ":>")),
            _ => None,
        }
    }

    /// The ADR 0040 meeting diagnostic: two operators with no relative rank
    /// met without parentheses.
    fn mixing_error(&self, left: &str, right: &str) -> ParseError {
        self.error(format!(
            "mixing `{left}` with `{right}` needs parentheses (ADR 0040)"
        ))
    }

    /// `add_expr = mul_expr { ("+" | "-") mul_expr }`.
    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.cur_kind() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_mul()?;
            lhs = self.binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    /// `mul_expr = unary_expr { ("*" | "/") unary_expr }`.
    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.cur_kind() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_unary()?;
            lhs = self.binary(op, lhs, rhs);
        }
        Ok(lhs)
    }

    /// `unary_expr = "-" unary_expr | pow_expr`.
    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if self.check(&TokenKind::Minus) {
            let start = self.cur_span().start;
            self.pos += 1;
            let inner = self.parse_unary()?;
            let span = Span::new(start, inner.span.end);
            Ok(Expr {
                kind: ExprKind::Unary(UnOp::Neg, Box::new(inner)),
                span,
            })
        } else {
            self.parse_pow()
        }
    }

    /// `pow_expr = app_expr [ "^" unary_expr ]`, right-associative through the
    /// `unary_expr` right operand.
    fn parse_pow(&mut self) -> Result<Expr, ParseError> {
        let base = self.parse_app()?;
        if self.check(&TokenKind::Caret) {
            self.pos += 1;
            let rhs = self.parse_unary()?;
            Ok(self.binary(BinOp::Pow, base, rhs))
        } else {
            Ok(base)
        }
    }

    /// `app_expr = card_expr { card_expr }`, application by juxtaposition,
    /// left-associative.
    fn parse_app(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_card()?;
        while self.at_postfix_start() {
            let arg = self.parse_card()?;
            let span = Span::new(e.span.start, arg.span.end);
            e = Expr {
                kind: ExprKind::App(Box::new(e), Box::new(arg)),
                span,
            };
        }
        Ok(e)
    }

    /// `card_expr = "#" card_expr | postfix` (ADR 0031, Decision 9).
    ///
    /// The operand is a `card_expr`, not a whole `unary_expr`, which is what
    /// makes `#` bind *looser than member access* (`#b.x` is `#(b.x)`, since
    /// `postfix` consumes the `.x`) and *tighter than the comparisons*
    /// (`#b > 3` is `(#b) > 3`, since the comparison level sits above).
    /// Sitting inside the application spine also keeps `#b` usable as an
    /// argument: `f #b` is `f (#b)`, one token of lookahead, no ambiguity.
    fn parse_card(&mut self) -> Result<Expr, ParseError> {
        if self.check(&TokenKind::Hash) {
            let start = self.cur_span().start;
            self.pos += 1;
            let inner = self.parse_card()?;
            let span = Span::new(start, inner.span.end);
            Ok(Expr {
                kind: ExprKind::Unary(UnOp::Card, Box::new(inner)),
                span,
            })
        } else {
            self.parse_postfix()
        }
    }

    /// True when the current token can begin a `card_expr` (a `#` prefix or a
    /// primary), so the application spine should consume another argument.  A
    /// `|` opens a lambda argument; `|>` (a distinct token) never does, so a
    /// pipe ends the spine.  A `Template` is a backticked combiner
    /// (`` fold `+` (|v| v) b ``, ADR 0031 Decision 6) and a `Hash` a
    /// cardinality operand (`f #b`).
    fn at_postfix_start(&self) -> bool {
        match self.cur_kind() {
            TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::Str(_)
            | TokenKind::Template(_)
            | TokenKind::LParen
            | TokenKind::LBrace
            | TokenKind::Hash
            | TokenKind::Pipe => true,
            TokenKind::Ident(s) => !is_expr_reserved(s),
            _ => false,
        }
    }

    /// `postfix = primary { "." ident }`, member access, the tightest binding.
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_primary()?;
        while self.check(&TokenKind::Dot) {
            self.pos += 1;
            let field = self.expect_ident("a field name after `.`")?;
            let span = Span::new(e.span.start, field.span.end);
            e = Expr {
                kind: ExprKind::Member(Box::new(e), field),
                span,
            };
        }
        Ok(e)
    }

    /// `primary = number | string | ident | lambda | paren | block`.
    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let span = self.cur_span();
        let kind = self.cur_kind().clone();
        match kind {
            TokenKind::Int(n) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::Int(n),
                    span,
                })
            }
            TokenKind::Float(f) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::Float(f),
                    span,
                })
            }
            TokenKind::Str(s) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::Str(s),
                    span,
                })
            }
            TokenKind::Ident(s) => {
                if s == "if" {
                    return self.parse_if();
                }
                if is_expr_reserved(&s) {
                    return Err(self.error(format!("`{s}` is a reserved operator, not a value")));
                }
                let kind = match s.as_str() {
                    "true" => ExprKind::Bool(true),
                    "false" => ExprKind::Bool(false),
                    _ => ExprKind::Name(s),
                };
                self.pos += 1;
                Ok(Expr { kind, span })
            }
            // A backticked combiner (ADR 0031, Decision 6).  The token already
            // lexes as a `Template`; because `` `or` `` and `` `and` `` are
            // templates rather than words, the reserved-operator collision that
            // makes bare combiner names impossible never arises.
            TokenKind::Template(raw) => {
                self.pos += 1;
                Ok(Expr {
                    kind: ExprKind::Combiner(raw),
                    span,
                })
            }
            TokenKind::Pipe => self.parse_lambda(),
            TokenKind::LParen => self.parse_paren(),
            TokenKind::LBrace => self.parse_block_expr(),
            _ => Err(self.error("expected an expression")),
        }
    }

    /// `if_expr = "if" logic_expr "then" logic_expr "else" logic_expr`
    /// (ADR 0015).
    fn parse_if(&mut self) -> Result<Expr, ParseError> {
        let start = self.cur_span().start;
        self.bump_keyword(); // `if`
        let cond = self.parse_logic()?;
        if !self.at_keyword("then") {
            return Err(self.error("expected `then` after the `if` condition"));
        }
        self.bump_keyword(); // `then`
        let then = self.parse_logic()?;
        if !self.at_keyword("else") {
            return Err(self.error("expected `else` after the `then` branch"));
        }
        self.bump_keyword(); // `else`
        let els = self.parse_logic()?;
        let end = els.span.end;
        Ok(Expr {
            kind: ExprKind::If {
                cond: Box::new(cond),
                then: Box::new(then),
                els: Box::new(els),
            },
            span: Span::new(start, end),
        })
    }

    /// `lambda = "|" [ ident { "," ident } ] "|" [ ":" type ] logic_expr`.
    /// The body is a `logic_expr`, so a top-level `|>` inside a lambda must
    /// be parenthesized.
    fn parse_lambda(&mut self) -> Result<Expr, ParseError> {
        let start = self.cur_span().start;
        self.pos += 1; // opening `|`
        let mut params = Vec::new();
        if !self.check(&TokenKind::Pipe) {
            params.push(self.expect_ident("a lambda parameter")?);
            while self.eat(&TokenKind::Comma) {
                params.push(self.expect_ident("a lambda parameter")?);
            }
        }
        self.expect(&TokenKind::Pipe, "`|` to close the lambda parameters")?;
        let ret = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_logic()?;
        let span = Span::new(start, body.span.end);
        Ok(Expr {
            kind: ExprKind::Lambda {
                params,
                ret,
                body: Box::new(body),
            },
            span,
        })
    }

    /// `paren = "(" ( record_body | tuple_body ) ")"`.  A leading `.` selects a
    /// record; otherwise a tuple body, whose single-element form `(e)` is plain
    /// grouping and reduces to `e`.
    fn parse_paren(&mut self) -> Result<Expr, ParseError> {
        let start = self.cur_span().start;
        self.pos += 1; // `(`
        if self.check(&TokenKind::RParen) {
            let end = self.cur_span().end;
            self.pos += 1;
            return Ok(Expr {
                kind: ExprKind::Tuple(Vec::new()),
                span: Span::new(start, end),
            });
        }
        if self.check(&TokenKind::Dot) {
            let mut fields = vec![self.parse_record_field()?];
            while self.eat(&TokenKind::Comma) {
                fields.push(self.parse_record_field()?);
            }
            let end = self
                .expect(&TokenKind::RParen, "`)` to close the record")?
                .end;
            return Ok(Expr {
                kind: ExprKind::Record(fields),
                span: Span::new(start, end),
            });
        }
        let mut elems = vec![self.parse_expr_inner()?];
        while self.eat(&TokenKind::Comma) {
            elems.push(self.parse_expr_inner()?);
        }
        let end = self
            .expect(&TokenKind::RParen, "`)` to close the group")?
            .end;
        if elems.len() == 1 {
            // Grouping: `(e)` is `e`.
            Ok(elems.pop().unwrap())
        } else {
            Ok(Expr {
                kind: ExprKind::Tuple(elems),
                span: Span::new(start, end),
            })
        }
    }

    /// `field = "." ident [ ":" type ] "=" expr`, one labeled record field.
    fn parse_record_field(&mut self) -> Result<RecordField, ParseError> {
        let start = self.expect(&TokenKind::Dot, "`.` to start a record field")?;
        let name = self.expect_ident("a field name")?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&TokenKind::Eq, "`=` after the field name")?;
        let value = self.parse_expr_inner()?;
        let span = Span::new(start.start, value.span.end);
        Ok(RecordField {
            name,
            ty,
            value,
            span,
        })
    }

    /// A `block` in expression position.
    fn parse_block_expr(&mut self) -> Result<Expr, ParseError> {
        let block = self.parse_block()?;
        let span = block.span;
        Ok(Expr {
            kind: ExprKind::Block(block),
            span,
        })
    }

    /// `block = "{" [ stmt { ";" stmt } [ ";" ] ] "}"`.
    fn parse_block(&mut self) -> Result<Block, ParseError> {
        let start = self.expect(&TokenKind::LBrace, "`{` to open the block")?;
        let mut stmts = Vec::new();
        if !self.check(&TokenKind::RBrace) {
            stmts.push(self.parse_stmt()?);
            while self.eat(&TokenKind::Semi) {
                if self.check(&TokenKind::RBrace) {
                    break; // an optional trailing `;`
                }
                stmts.push(self.parse_stmt()?);
            }
        }
        // `let`/`assert` are reserved in expressions, so a statement keyword
        // here means the separator was forgotten; say so instead of the
        // generic close-brace error.
        if self.at_keyword("let") || self.at_keyword("assert") {
            return Err(self.error("expected `;` before the next statement"));
        }
        let end = self
            .expect(&TokenKind::RBrace, "`}` to close the block")?
            .end;
        Ok(Block {
            stmts,
            span: Span::new(start.start, end),
        })
    }

    /// `stmt = let_stmt | assert_stmt | expr`.  `let` and `assert` are reserved
    /// only here, in statement position.
    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        if self.at_keyword("let") {
            self.bump_keyword();
            let name = self.expect_binder("a name after `let`")?;
            let ty = if self.eat(&TokenKind::Colon) {
                Some(self.parse_type()?)
            } else {
                None
            };
            self.expect(&TokenKind::Eq, "`=` in a `let` binding")?;
            let value = self.parse_expr_inner()?;
            Ok(Stmt::Let { name, ty, value })
        } else if self.at_keyword("assert") {
            self.bump_keyword();
            let e = self.parse_expr_inner()?;
            Ok(Stmt::Assert(e))
        } else {
            Ok(Stmt::Expr(self.parse_expr_inner()?))
        }
    }

    /// Build a binary node spanning from its left to its right operand.
    fn binary(&self, op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        let span = Span::new(lhs.span.start, rhs.span.end);
        Expr {
            kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
            span,
        }
    }
}

/// Split the raw inner text of a backtick template into literal and `{param}`
/// segments.  `span` covers the whole token (including the backticks), so a
/// parameter's span is measured from `span.start + 1`, the first inner byte.
fn split_template(raw: &str, span: Span) -> Result<NameTemplate, ParseError> {
    let base = span.start + 1;
    let mut segments = Vec::new();
    let mut lit = String::new();
    let mut chars = raw.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '{' => {
                if !lit.is_empty() {
                    segments.push(NameSeg::Lit(std::mem::take(&mut lit)));
                }
                let name_start = base + i + 1; // first byte after `{`
                let mut name = String::new();
                let mut closed = false;
                while let Some(&(_, d)) = chars.peek() {
                    chars.next();
                    if d == '}' {
                        closed = true;
                        break;
                    }
                    name.push(d);
                }
                if !closed {
                    return Err(ParseError {
                        message: "unterminated `{` in template name".into(),
                        span,
                    });
                }
                if name.is_empty() {
                    return Err(ParseError {
                        message: "empty `{}` in template name".into(),
                        span,
                    });
                }
                let name_span = Span::new(name_start, name_start + name.len());
                if !crate::lexer::is_identifier(&name) {
                    return Err(ParseError {
                        message: format!("`{name}` is not a valid name parameter"),
                        span: name_span,
                    });
                }
                segments.push(NameSeg::Param(Ident {
                    name,
                    span: name_span,
                }));
            }
            '}' => {
                return Err(ParseError {
                    message: "unmatched `}` in template name".into(),
                    span,
                });
            }
            _ => lit.push(c),
        }
    }
    if !lit.is_empty() {
        segments.push(NameSeg::Lit(lit));
    }
    if segments.is_empty() {
        return Err(ParseError {
            message: "empty template name".into(),
            span,
        });
    }
    Ok(NameTemplate { segments, span })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize;

    fn parse_str(src: &str) -> Result<Program, ParseError> {
        let tokens = tokenize(src).expect("should lex");
        parse(&tokens)
    }

    #[test]
    fn parses_worked_example() {
        let src = r#"
            unit Person {
              id: string
            }

            unit Department {
              code: string
            }

            store Departments {
              unit { Department }
              attr { name: string }
            }

            store Persons {
              unit { Person }
              attr { birthdate: date }
              attr { last_name: string }
            }
        "#;
        let program = parse_str(src).expect("should parse");
        assert_eq!(program.items.len(), 4);

        let Item::Unit(person) = &program.items[0] else {
            panic!("expected a unit");
        };
        assert_eq!(person.name.name, "Person");
        assert_eq!(person.fields.len(), 1);
        assert_eq!(person.fields[0].name.as_literal(), Some("id"));

        let Item::Store(persons) = &program.items[3] else {
            panic!("expected a store");
        };
        assert_eq!(persons.name.name, "Persons");
        assert_eq!(persons.unit.name, "Person");
        // Repeated `attr` blocks merge into one attribute list.
        assert_eq!(persons.attrs.len(), 2);
        assert_eq!(persons.attrs[0].field.name.as_literal(), Some("birthdate"));
        assert_eq!(persons.attrs[1].field.name.as_literal(), Some("last_name"));
    }

    #[test]
    fn parses_view_declaration() {
        let program = parse_str("view machine_temperature { readings }").expect("should parse");
        let Item::View(v) = &program.items[0] else {
            panic!("expected a view");
        };
        assert_eq!(v.name.name, "machine_temperature");
        assert!(v.conforms.is_empty());
        assert_eq!(v.body.stmts.len(), 1);
    }

    #[test]
    fn parses_conditional() {
        let toks = tokenize("if a then b else c").expect("lex");
        let expr = crate::parse_expr(&toks).expect("parse");
        assert!(matches!(expr.kind, ExprKind::If { .. }));
    }

    #[test]
    fn conditional_requires_then_and_else() {
        let toks = tokenize("if a then b").expect("lex");
        assert!(crate::parse_expr(&toks).is_err());
    }

    #[test]
    fn parses_view_with_conformance() {
        let program =
            parse_str("view feature : Tabular[Machine] { readings }").expect("should parse");
        let Item::View(v) = &program.items[0] else {
            panic!("expected a view");
        };
        assert_eq!(v.conforms.len(), 1);
        assert_eq!(v.conforms[0].name.name, "Tabular");
    }

    #[test]
    fn parses_enum_declaration_and_reference() {
        let src = r#"
            enum Status {
              "active"
              "inactive"
            }
            store S { unit { U } attr { status: Status } }
        "#;
        let program = parse_str(src).unwrap();

        let Item::Enum(e) = &program.items[0] else {
            panic!("expected an enum declaration");
        };
        assert_eq!(e.name.name, "Status");
        let values: Vec<&str> = e.variants.iter().map(|v| v.value.as_str()).collect();
        assert_eq!(values, ["active", "inactive"]);

        let Item::Store(s) = &program.items[1] else {
            panic!("expected a store");
        };
        assert_eq!(s.attrs[0].field.ty.named().unwrap().name, "Status");
        assert!(!s.attrs[0].field.ty.is_optional());
    }

    #[test]
    fn parses_optional_type_marker() {
        let src = r#"
            store S {
              unit { U }
              attr { last_service: date? }
              attr { vibration: number }
            }
        "#;
        let program = parse_str(src).unwrap();
        let Item::Store(s) = &program.items[0] else {
            panic!("expected a store");
        };
        // `date?` is optional and its span covers the `?`.
        let opt = &s.attrs[0].field.ty;
        assert_eq!(opt.named().unwrap().name, "date");
        assert!(opt.is_optional());
        assert_eq!(opt.span().slice(src), "date?");
        // A bare type stays total.
        assert!(!s.attrs[1].field.ty.is_optional());
    }

    #[test]
    fn parses_attr_star_block() {
        // `attr*` is the `attr` identifier followed by the `*` operator
        // (ADR 0022): every attribute in the block is marked bag-valued.
        let src = r#"
            store readings {
              unit { Machine }
              attr* {
                kelvin: real
                rpm:    int
              }
            }
            shape SensorLog {
              attr* { kelvin: real }
            }
        "#;
        let program = parse_str(src).unwrap();

        let Item::Store(s) = &program.items[0] else {
            panic!("expected a store");
        };
        assert_eq!(s.attrs.len(), 2);
        assert!(s.attrs.iter().all(|a| a.many.is_some()));
        assert_eq!(s.attrs[0].field.name.as_literal(), Some("kelvin"));
        // The recorded `*` span points at the marker itself.
        assert_eq!(s.attrs[0].many.unwrap().slice(src), "*");

        let Item::Shape(sh) = &program.items[1] else {
            panic!("expected a shape");
        };
        assert!(sh.attrs[0].many.is_some());
    }

    #[test]
    fn attr_and_attr_star_blocks_carry_their_own_markers() {
        // Mixing blocks parses (the resolver rejects it, ADR 0022's deferred
        // refinement); each attribute keeps the marker of its own block.
        let src = "store s { unit { U } attr { a: int } attr* { b: int } }";
        let program = parse_str(src).unwrap();
        let Item::Store(s) = &program.items[0] else {
            panic!("expected a store");
        };
        assert!(s.attrs[0].many.is_none());
        assert!(s.attrs[1].many.is_some());
    }

    #[test]
    fn parses_domain_block() {
        let src = r#"
            store StudentGrades {
              unit { Enrollment }
              domain {
                student: Students
                course:  Courses
              }
              attr { class_id: string }
            }
        "#;
        let program = parse_str(src).unwrap();
        let Item::Store(s) = &program.items[0] else {
            panic!("expected a store");
        };
        assert_eq!(s.domain.len(), 2);
        assert_eq!(s.domain[0].field.name, "student");
        assert_eq!(s.domain[0].store.name, "Students");
    }

    #[test]
    fn parses_shape_and_conformance() {
        let src = r#"
            shape PersonRecord {
              unit { Person }
              attr { admission: date }
            }

            store Students : PersonRecord {
              unit { Person }
              attr { admission: date }
            }
        "#;
        let program = parse_str(src).unwrap();

        let Item::Shape(shape) = &program.items[0] else {
            panic!("expected a shape");
        };
        assert_eq!(shape.name.name, "PersonRecord");
        assert_eq!(shape.unit.as_ref().unwrap().name, "Person");
        assert_eq!(shape.attrs[0].field.name.as_literal(), Some("admission"));

        let Item::Store(store) = &program.items[1] else {
            panic!("expected a store");
        };
        let claimed: Vec<&str> = store
            .conforms
            .iter()
            .map(|s| s.name.name.as_str())
            .collect();
        assert_eq!(claimed, ["PersonRecord"]);
    }

    #[test]
    fn parses_multiple_conformance_entries() {
        let src = "store S : A, B, C { unit { U } }";
        let program = parse_str(src).unwrap();
        let Item::Store(store) = &program.items[0] else {
            panic!("expected a store");
        };
        let claimed: Vec<&str> = store
            .conforms
            .iter()
            .map(|s| s.name.name.as_str())
            .collect();
        assert_eq!(claimed, ["A", "B", "C"]);
    }

    #[test]
    fn store_without_conformance_has_empty_list() {
        let program = parse_str("store S { unit { U } }").unwrap();
        let Item::Store(store) = &program.items[0] else {
            panic!("expected a store");
        };
        assert!(store.conforms.is_empty());
    }

    // --- `registry` declarations (`13-registries.md`, ADR 0033) -----------

    #[test]
    fn parses_registry_declaration() {
        let program = parse_str("registry readings { unit { Reading } }").unwrap();
        let Item::Registry(registry) = &program.items[0] else {
            panic!("expected a registry");
        };
        assert_eq!(registry.kind, StoreKind::Registry);
        assert_eq!(registry.name.name, "readings");
        assert_eq!(registry.unit.name, "Reading");
    }

    #[test]
    fn store_declaration_carries_the_store_kind() {
        let program = parse_str("store machines { unit { Machine } }").unwrap();
        let Item::Store(store) = &program.items[0] else {
            panic!("expected a store");
        };
        assert_eq!(store.kind, StoreKind::Store);
    }

    #[test]
    fn registry_body_is_a_stores_verbatim() {
        // Conformance, `attr*`, and `domain` all parse in a registry exactly
        // as they do in a store: the two share one production.
        let src = r#"
            registry events : Logged {
              unit { Event }
              domain { machine: machines }
              attr* { kind: EventKind }
            }
        "#;
        let program = parse_str(src).unwrap();
        let Item::Registry(registry) = &program.items[0] else {
            panic!("expected a registry");
        };
        assert_eq!(registry.conforms.len(), 1);
        assert_eq!(registry.domain.len(), 1);
        assert_eq!(registry.attrs.len(), 1);
        assert!(registry.attrs[0].many.is_some());
    }

    #[test]
    fn registry_without_unit_clause_is_an_error() {
        let err = parse_str("registry r { attr { x: int } }").unwrap_err();
        assert!(err.message.contains("unit"));
    }

    #[test]
    fn unknown_registry_block_is_an_error() {
        let err = parse_str("registry r { unit { U } nope { } }").unwrap_err();
        assert!(err.message.contains("expected `attr`, `domain`, or `}`"));
    }

    #[test]
    fn registry_keyword_is_recorded_as_a_keyword_span() {
        // The highlighter colors every `bump_keyword` span, so recording the
        // introducer here is what makes `registry` highlight (ADR 0033).
        let src = "registry r { unit { U } }";
        let tokens = tokenize(src).expect("should lex");
        let parsed = parse_with_meta(&tokens).expect("should parse");
        let words: Vec<&str> = parsed
            .keyword_spans
            .iter()
            .map(|s| &src[s.start..s.end])
            .collect();
        assert_eq!(words, vec!["registry", "unit"]);
    }

    #[test]
    fn parses_unit_parameter_shape() {
        let program = parse_str("shape Tabular[U: Unit] { unit { U } }").unwrap();
        let Item::Shape(shape) = &program.items[0] else {
            panic!("expected a shape");
        };
        assert_eq!(shape.params.len(), 1);
        assert_eq!(shape.params[0].name.name, "U");
        assert_eq!(shape.params[0].kind.name, "Unit");
        assert_eq!(shape.unit.as_ref().unwrap().name, "U");
    }

    #[test]
    fn parses_unit_agnostic_shape() {
        let program = parse_str("shape Named { attr { name: string } }").unwrap();
        let Item::Shape(shape) = &program.items[0] else {
            panic!("expected a shape");
        };
        assert!(shape.params.is_empty());
        assert!(shape.unit.is_none());
        assert_eq!(shape.attrs[0].field.name.as_literal(), Some("name"));
    }

    #[test]
    fn parses_parametric_conformance() {
        let program = parse_str("store S : Tabular[Person] { unit { Person } }").unwrap();
        let Item::Store(store) = &program.items[0] else {
            panic!("expected a store");
        };
        assert_eq!(store.conforms[0].name.name, "Tabular");
        let ShapeArg::Unit(arg) = &store.conforms[0].args[0] else {
            panic!("expected a unit argument");
        };
        assert_eq!(arg.name, "Person");
    }

    #[test]
    fn parses_string_argument_and_template() {
        let src = r#"
            shape Ageable[date_field: string] {
              attr { `{date_field}`: date }
            }
            store Persons : Ageable["birthdate"] {
              unit { Person }
              attr { birthdate: date }
            }
        "#;
        let program = parse_str(src).unwrap();

        let Item::Shape(shape) = &program.items[0] else {
            panic!("expected a shape");
        };
        assert_eq!(shape.params[0].kind.name, "string");
        // The attribute name is a single interpolated parameter.
        assert_eq!(shape.attrs[0].field.name.segments.len(), 1);
        let NameSeg::Param(p) = &shape.attrs[0].field.name.segments[0] else {
            panic!("expected an interpolated segment");
        };
        assert_eq!(p.name, "date_field");

        let Item::Store(store) = &program.items[1] else {
            panic!("expected a store");
        };
        let ShapeArg::Str(arg) = &store.conforms[0].args[0] else {
            panic!("expected a string argument");
        };
        assert_eq!(arg.value, "birthdate");
    }

    #[test]
    fn parses_mixed_template_segments() {
        let program = parse_str("shape S[col: string] { attr { `{col}_z`: number } }").unwrap();
        let Item::Shape(shape) = &program.items[0] else {
            panic!("expected a shape");
        };
        let segs = &shape.attrs[0].field.name.segments;
        assert_eq!(segs.len(), 2);
        assert!(matches!(&segs[0], NameSeg::Param(p) if p.name == "col"));
        assert!(matches!(&segs[1], NameSeg::Lit(s) if s == "_z"));
    }

    #[test]
    fn empty_interpolation_is_an_error() {
        let err = parse_str("shape S { attr { `{}`: number } }").unwrap_err();
        assert!(err.message.contains("empty `{}`"));
    }

    #[test]
    fn empty_parameter_list_is_an_error() {
        let err = parse_str("shape Bad[] { unit { U } }").unwrap_err();
        assert!(err.message.contains("empty parameter list"));
    }

    #[test]
    fn shape_with_domain_block_is_an_error() {
        let err = parse_str("shape Sh { unit { U } domain { x: Y } }").unwrap_err();
        assert!(err.message.contains("`domain`"));
    }

    #[test]
    fn empty_program_is_ok() {
        assert_eq!(parse_str("").unwrap().items.len(), 0);
    }

    #[test]
    fn store_without_unit_clause_is_an_error() {
        let err = parse_str("store S { attr { a: string } }").unwrap_err();
        assert!(err.message.contains("unit"));
    }

    #[test]
    fn second_unit_clause_is_an_error() {
        let err = parse_str("store S { unit { U } unit { V } }").unwrap_err();
        assert!(err.message.contains("only once"));
    }

    #[test]
    fn missing_colon_is_an_error() {
        let err = parse_str("unit U { id string }").unwrap_err();
        assert!(err.message.contains("`:`"));
    }

    #[test]
    fn missing_closing_brace_is_an_error() {
        let err = parse_str("unit U { id: string").unwrap_err();
        assert!(err.message.contains("`}`"));
    }

    #[test]
    fn unknown_store_block_is_an_error() {
        let err = parse_str("store S { unit { U } bogus { } }").unwrap_err();
        assert!(err.message.contains("`attr`, `domain`"));
    }

    #[test]
    fn empty_enum_is_an_error() {
        let err = parse_str(r#"enum Status { }"#).unwrap_err();
        assert!(err.message.contains("enum variant"));
    }

    #[test]
    fn comma_between_enum_variants_is_an_error() {
        let err = parse_str(r#"enum Status { "active", "inactive" }"#).unwrap_err();
        assert!(err.message.contains("enum variant"));
    }

    #[test]
    fn junk_at_top_level_is_an_error() {
        let err = parse_str("wat X { }").unwrap_err();
        assert!(
            err.message.contains(
                "`unit`, `store`, `registry`, `shape`, `enum`, `view`, `let`, or `import`"
            )
        );
    }

    // --- top-level `let` and `import` (`12-modules-and-imports.md`) ---------

    #[test]
    fn parses_import_and_top_level_let() {
        let src = "import si\nlet km { 1000.0 * meter }\nlet g: real { 9.8 }";
        let program = parse_str(src).unwrap();
        let Item::Import(import) = &program.items[0] else {
            panic!("expected an import");
        };
        assert_eq!(import.name.name, "si");
        let Item::Let(km) = &program.items[1] else {
            panic!("expected a let");
        };
        assert_eq!(km.name.name, "km");
        assert!(matches!(km.kind, LetKind::Value { ty: None, .. }));
        assert_eq!(km.span.slice(src), "let km { 1000.0 * meter }");
        let Item::Let(g) = &program.items[2] else {
            panic!("expected a let");
        };
        let LetKind::Value { ty: Some(ty), .. } = &g.kind else {
            panic!("expected an ascribed value let");
        };
        assert_eq!(ty.named().unwrap().name, "real");
    }

    #[test]
    fn top_level_let_body_is_a_statement_block() {
        // The value body is the ordinary statement block (ADR 0027,
        // Decision 1 as revised): local `let`s and a trailing result.
        let src = "let overheat { let base = 300.0; base + 50.0 }";
        let program = parse_str(src).unwrap();
        let Item::Let(decl) = &program.items[0] else {
            panic!("expected a let");
        };
        let LetKind::Value { value, .. } = &decl.kind else {
            panic!("expected a value let");
        };
        assert_eq!(value.stmts.len(), 2);
        assert!(matches!(value.stmts[0], Stmt::Let { .. }));
        assert!(matches!(value.stmts[1], Stmt::Expr(_)));
    }

    #[test]
    fn unbraced_top_level_let_points_at_the_brace_form() {
        let err = parse_str("let km = 1000.0 * meter").unwrap_err();
        assert!(err.message.contains("brace-closed"));
    }

    #[test]
    fn reserved_words_cannot_name_bindings() {
        // `let` and `assert` (and the other expression-reserved words) can
        // never be referenced as values, so they cannot be bound either.
        let err = parse_str("let let { 1 }").unwrap_err();
        assert!(err.message.contains("reserved word"));
        let err = parse_str("view v { let known = 1; known }").unwrap_err();
        assert!(err.message.contains("reserved word"));
    }

    #[test]
    fn missing_statement_separator_is_pointed() {
        // With `let`/`assert` reserved in expressions, a forgotten `;` stops
        // at the next statement keyword instead of mis-parsing it as an
        // application argument.
        let err = parse_str("view v { let t = a let u = b; u }").unwrap_err();
        assert!(err.message.contains("expected `;`"), "{}", err.message);
        let err = parse_str("view v { let t = a assert b; t }").unwrap_err();
        assert!(err.message.contains("expected `;`"), "{}", err.message);
    }

    #[test]
    fn parses_dimension_alias_let() {
        // `let speed[T] { (length / time)[T] }`: the `[` after the name
        // selects the type-level alias form (ADR 0026, Decision 8), with a
        // braced body like every other item.
        let src = "let speed[T] { (length / time)[T] }\nlet accel[T] { speed[T] / time[T] }";
        let program = parse_str(src).unwrap();
        let Item::Let(speed) = &program.items[0] else {
            panic!("expected a let");
        };
        let LetKind::DimAlias { params, body } = &speed.kind else {
            panic!("expected a dimension alias");
        };
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "T");
        let TypeKind::Apply { base, backing } = &body.kind else {
            panic!("expected an application body");
        };
        assert!(matches!(base.kind, TypeKind::Div(..)));
        assert_eq!(backing.name, "T");
        // The second alias divides two applied types.
        let Item::Let(accel) = &program.items[1] else {
            panic!("expected a let");
        };
        let LetKind::DimAlias { body, .. } = &accel.kind else {
            panic!("expected a dimension alias");
        };
        let TypeKind::Div(lhs, rhs) = &body.kind else {
            panic!("expected a quotient body");
        };
        assert!(matches!(lhs.kind, TypeKind::Apply { .. }));
        assert!(matches!(rhs.kind, TypeKind::Apply { .. }));
    }

    #[test]
    fn top_level_let_and_import_record_keyword_spans() {
        let src = "import si\nlet km { 1000.0 * meter }";
        let tokens = tokenize(src).expect("should lex");
        let parsed = parse_with_meta(&tokens).expect("should parse");
        let words: Vec<&str> = parsed.keyword_spans.iter().map(|s| s.slice(src)).collect();
        assert_eq!(words, ["import", "let"]);
    }

    // --- dimensioned types (`11-physical-units.md`) --------------------------

    #[test]
    fn parses_dimensioned_column_types() {
        let src = r#"
            store readings {
              unit { Machine }
              attr* {
                temperature: temperature[real]
                vibration:   (length / time^2)[real]
                speed:       speed[real]?
              }
            }
        "#;
        let program = parse_str(src).unwrap();
        let Item::Store(s) = &program.items[0] else {
            panic!("expected a store");
        };
        let temp = &s.attrs[0].field.ty;
        let TypeKind::Apply { base, backing } = &temp.kind else {
            panic!("expected an applied type");
        };
        assert_eq!(base.named().unwrap().name, "temperature");
        assert_eq!(backing.name, "real");
        // The grouped dimension expression keeps its precedence shape:
        // `length / (time^2)`.
        let vib = &s.attrs[1].field.ty;
        let TypeKind::Apply { base, .. } = &vib.kind else {
            panic!("expected an applied type");
        };
        let TypeKind::Div(num, den) = &base.kind else {
            panic!("expected a quotient");
        };
        assert_eq!(num.named().unwrap().name, "length");
        let TypeKind::Pow(t, 2) = &den.kind else {
            panic!("expected time^2");
        };
        assert_eq!(t.named().unwrap().name, "time");
        // A trailing `?` rides on the whole applied type.
        let sp = &s.attrs[2].field.ty;
        assert!(sp.is_optional());
        assert_eq!(sp.span().slice(src), "speed[real]?");
    }

    #[test]
    fn parses_negative_type_level_exponents() {
        // The lexer has no signed ints, so `^-1` is `Caret Minus Int`.
        let src = "let hz[T] { time^-1[T] }";
        // A `^-1` binds to the factor, so the bracket applies to the whole
        // `time^-1` only when grouped; ungrouped, `[T]` after the exponent is
        // a parse error (the factor is complete).  Write it grouped.
        let err = parse_str(src).unwrap_err();
        assert!(err.message.contains("expected"));
        let src = "let hz[T] { (time^-1)[T] }";
        let program = parse_str(src).unwrap();
        let Item::Let(decl) = &program.items[0] else {
            panic!("expected a let");
        };
        let LetKind::DimAlias { body, .. } = &decl.kind else {
            panic!("expected an alias");
        };
        let TypeKind::Apply { base, .. } = &body.kind else {
            panic!("expected an application");
        };
        assert!(matches!(base.kind, TypeKind::Pow(_, -1)));
    }

    #[test]
    fn bare_parenthesized_dimension_is_an_error() {
        // A parenthesized dimension group must be applied to a backing.
        let err = parse_str("unit U { x: (length / time) }").unwrap_err();
        assert!(err.message.contains("backing"));
    }

    #[test]
    fn type_level_exponent_must_be_an_integer() {
        let err = parse_str("unit U { x: (length / time^2.0)[real] }").unwrap_err();
        assert!(err.message.contains("integer exponent"));
    }

    #[test]
    fn keyword_spans_cover_every_contextual_keyword() {
        let src = r#"enum E { "a" } shape Sh { attr { t: string } } unit U { x: string } store S { unit { U } attr { a: string } attr { b: number } }"#;
        let tokens = tokenize(src).expect("should lex");
        let parsed = parse_with_meta(&tokens).expect("should parse");
        let words: Vec<&str> = parsed.keyword_spans.iter().map(|s| s.slice(src)).collect();
        // In source order: the enum decl; the shape decl and its `attr`; the
        // unit decl; the store decl, its `unit` clause, and two `attr`s.
        assert_eq!(
            words,
            [
                "enum", "shape", "attr", "unit", "store", "unit", "attr", "attr"
            ]
        );
    }

    #[test]
    fn keyword_spans_cover_predicate_and_statement_keywords() {
        // The expression operators (`or`, `and`, `not`, `is`, `known`,
        // `missing`) and the statement keywords (`let`, `assert`) are recorded
        // like the declaration keywords, so a highlighter colors them.
        let src = "view v { let t = (a and b) or not c; assert t is known; assert c is missing }";
        let tokens = tokenize(src).expect("should lex");
        let parsed = parse_with_meta(&tokens).expect("should parse");
        let words: Vec<&str> = parsed.keyword_spans.iter().map(|s| s.slice(src)).collect();
        for kw in [
            "let", "and", "or", "not", "assert", "is", "known", "missing",
        ] {
            assert!(
                words.contains(&kw),
                "missing keyword span for `{kw}`: {words:?}"
            );
        }
    }

    #[test]
    fn op_spans_capture_pipeline_operation_heads() {
        // Each `|>` right-hand side is a curried application; its leftmost
        // identifier is the operation, recorded for highlighting.
        let src = "view v { readings |> promote machine |> map_bags |k, b| (.m = #b.x) }";
        let tokens = tokenize(src).expect("should lex");
        let parsed = parse_with_meta(&tokens).expect("should parse");
        let ops: Vec<&str> = parsed.op_spans.iter().map(|s| s.slice(src)).collect();
        assert_eq!(ops, ["promote", "map_bags"]);
    }

    // --- expression sublanguage ---------------------------------------------

    use crate::expr::{BinOp, Expr, ExprKind, Presence, Stmt, UnOp};

    fn expr(src: &str) -> Expr {
        let tokens = tokenize(src).expect("should lex");
        parse_expr(&tokens).unwrap_or_else(|e| panic!("should parse `{src}`: {}", e.message))
    }

    fn expr_err(src: &str) -> ParseError {
        let tokens = tokenize(src).expect("should lex");
        parse_expr(&tokens).expect_err("should not parse")
    }

    /// A compact s-expression rendering so precedence and associativity are
    /// asserted by shape rather than by deep pattern matches.
    fn sexpr(e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(n) => n.to_string(),
            ExprKind::Float(f) => f.to_string(),
            ExprKind::Str(s) => format!("{s:?}"),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Name(s) => s.clone(),
            ExprKind::Combiner(s) => format!("`{s}`"),
            ExprKind::Member(b, f) => format!("(. {} {})", sexpr(b), f.name),
            ExprKind::App(f, x) => format!("(app {} {})", sexpr(f), sexpr(x)),
            ExprKind::Unary(op, x) => {
                let op = match op {
                    UnOp::Not => "not",
                    UnOp::Neg => "neg",
                    UnOp::Card => "#",
                };
                format!("({op} {})", sexpr(x))
            }
            ExprKind::Binary(op, a, b) => {
                let op = match op {
                    BinOp::Pipe => "|>",
                    BinOp::Or => "or",
                    BinOp::And => "and",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::In => "in",
                    BinOp::Coalesce => "??",
                    BinOp::Min => "<<",
                    BinOp::Max => ">>",
                    BinOp::KeepLeft => "<:",
                    BinOp::KeepRight => ":>",
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Pow => "^",
                };
                format!("({op} {} {})", sexpr(a), sexpr(b))
            }
            ExprKind::Presence(x, p) => {
                let p = match p {
                    Presence::Known => "known",
                    Presence::Missing => "missing",
                };
                format!("(is-{p} {})", sexpr(x))
            }
            ExprKind::Lambda { params, body, .. } => {
                let ps: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
                format!("(lam [{}] {})", ps.join(" "), sexpr(body))
            }
            ExprKind::Tuple(es) => {
                let es: Vec<String> = es.iter().map(sexpr).collect();
                format!("(tuple {})", es.join(" "))
            }
            ExprKind::Record(fs) => {
                let fs: Vec<String> = fs
                    .iter()
                    .map(|f| format!("{}={}", f.name.name, sexpr(&f.value)))
                    .collect();
                format!("(record {})", fs.join(" "))
            }
            ExprKind::Block(b) => {
                let ss: Vec<String> = b
                    .stmts
                    .iter()
                    .map(|s| match s {
                        Stmt::Let { name, value, .. } => {
                            format!("let {}={}", name.name, sexpr(value))
                        }
                        Stmt::Assert(e) => format!("assert {}", sexpr(e)),
                        Stmt::Expr(e) => sexpr(e),
                    })
                    .collect();
                format!("(block {})", ss.join("; "))
            }
            ExprKind::If { cond, then, els } => {
                format!("(if {} {} {})", sexpr(cond), sexpr(then), sexpr(els))
            }
        }
    }

    #[test]
    fn atoms_and_names() {
        assert_eq!(sexpr(&expr("42")), "42");
        assert_eq!(sexpr(&expr("3.14")), "3.14");
        assert_eq!(sexpr(&expr(r#""text""#)), "\"text\"");
        assert_eq!(sexpr(&expr("true")), "true");
        assert_eq!(sexpr(&expr("false")), "false");
        assert_eq!(sexpr(&expr("machine")), "machine");
    }

    #[test]
    fn arithmetic_precedence_and_associativity() {
        // `*` binds tighter than `+`; both are left-associative.
        assert_eq!(sexpr(&expr("1 + 2 * 3")), "(+ 1 (* 2 3))");
        assert_eq!(sexpr(&expr("1 - 2 - 3")), "(- (- 1 2) 3)");
        assert_eq!(sexpr(&expr("a / b / c")), "(/ (/ a b) c)");
    }

    #[test]
    fn glue_operators_bind_looser_than_arithmetic() {
        // ADR 0040, Decision 3: a glue operand is arithmetic-or-tighter, so
        // a computed bound or default needs no parens on either side.
        assert_eq!(sexpr(&expr("a + b << c")), "(<< (+ a b) c)");
        assert_eq!(sexpr(&expr("a << b + c")), "(<< a (+ b c))");
        assert_eq!(sexpr(&expr("a ?? b + c")), "(?? a (+ b c))");
        assert_eq!(sexpr(&expr("a + b ?? c")), "(?? (+ a b) c)");
    }

    #[test]
    fn glue_self_chains_keep_their_associativity() {
        // ADR 0040, Decision 3: a chain is one operator.  `??` discharges
        // right (its first present value wins, ADR 0039); a tack folds left
        // (ADR 0031).
        assert_eq!(sexpr(&expr("a ?? b ?? c")), "(?? a (?? b c))");
        assert_eq!(sexpr(&expr("a << b << c")), "(<< (<< a b) c)");
        assert_eq!(sexpr(&expr("a :> b :> c")), "(:> (:> a b) c)");
    }

    #[test]
    fn mixed_glue_operators_need_parens() {
        // ADR 0040, Decision 3: the glue operators have no rank against
        // each other, so any mixed meeting is an error naming both.
        for src in ["a << b >> c", "a ?? b << c", "a <: b :> c"] {
            let err = expr_err(src);
            assert!(err.message.contains("mixing"), "{src}: {}", err.message);
            assert!(err.message.contains("ADR 0040"), "{src}: {}", err.message);
        }
        let err = expr_err("a << b >> c");
        assert!(err.message.contains("`<<`"), "{}", err.message);
        assert!(err.message.contains("`>>`"), "{}", err.message);
        // Parenthesized, the clamp idiom parses.
        assert_eq!(sexpr(&expr("(a << b) >> c")), "(>> (<< a b) c)");
    }

    #[test]
    fn glue_meeting_a_comparison_needs_parens() {
        // ADR 0040, Decision 3: no rank against the comparisons or `is`,
        // on either side.
        for src in ["a ?? b < c", "a < b ?? c", "a << b < c", "a ?? b is known"] {
            let err = expr_err(src);
            assert!(err.message.contains("mixing"), "{src}: {}", err.message);
        }
        let err = expr_err("a ?? b < c");
        assert!(err.message.contains("`??`"), "{}", err.message);
        assert!(err.message.contains("`<`"), "{}", err.message);
        // Parenthesized, both readings are available.
        assert_eq!(sexpr(&expr("(a ?? b) < c")), "(< (?? a b) c)");
        assert_eq!(sexpr(&expr("(a < b) ?? false")), "(?? (< a b) false)");
        assert_eq!(sexpr(&expr("(a ?? b) is known")), "(is-known (?? a b))");
    }

    #[test]
    fn glue_meeting_a_logic_word_needs_parens() {
        // ADR 0040, Decisions 2 and 3: a logic word takes no
        // unparenthesized glue operand, on either side, and the flag rides
        // through a `not` prefix.
        for src in ["a ?? b and c", "a and b ?? c", "not a ?? b and c"] {
            let err = expr_err(src);
            assert!(err.message.contains("mixing"), "{src}: {}", err.message);
        }
        let err = expr_err("a ?? b and c");
        assert!(err.message.contains("`??`"), "{}", err.message);
        assert!(err.message.contains("`and`"), "{}", err.message);
        assert_eq!(sexpr(&expr("(a ?? b) and c")), "(and (?? a b) c)");
        // The pipe is structural and accepts a glue result bare.
        assert_eq!(sexpr(&expr("a ?? b |> f")), "(|> (?? a b) f)");
    }

    #[test]
    fn cardinality_binds_looser_than_member_access() {
        // ADR 0031, Decision 9: `#b.x` is `#(b.x)`, and `#b > 3` reads as
        // expected because `#` is tighter than the comparisons.
        assert_eq!(sexpr(&expr("#b.x")), "(# (. b x))");
        assert_eq!(sexpr(&expr("#b > 3")), "(> (# b) 3)");
        assert_eq!(sexpr(&expr("#b + 1")), "(+ (# b) 1)");
        // Inside an application spine, so `#b` is usable as an argument.
        assert_eq!(sexpr(&expr("f #b")), "(app f (# b))");
        // Nested, and over a parenthesized operand.
        assert_eq!(sexpr(&expr("#(a)")), "(# a)");
    }

    #[test]
    fn a_backticked_combiner_parses_in_expression_position() {
        // The `fold` call shape of ADR 0031: a combiner, a mapper lambda, and
        // the trailing bag, all one application spine.
        assert_eq!(
            sexpr(&expr("fold `+` (|v| v) b")),
            "(app (app (app fold `+`) (lam [v] v)) b)"
        );
        // The reserved words `or` and `and` are fine inside backticks.
        assert_eq!(
            sexpr(&expr("fold `or` p b")),
            "(app (app (app fold `or`) p) b)"
        );
    }

    #[test]
    fn power_is_right_associative_and_below_unary_minus() {
        // `^` binds tighter than unary minus: `-2^2` is `-(2^2)`.
        assert_eq!(sexpr(&expr("-2^2")), "(neg (^ 2 2))");
        // `^` is right-associative; its right operand may be a unary `-`.
        assert_eq!(sexpr(&expr("2^3^2")), "(^ 2 (^ 3 2))");
        assert_eq!(sexpr(&expr("2^-3")), "(^ 2 (neg 3))");
    }

    #[test]
    fn application_binds_tighter_than_operators() {
        // `f x + g y` is `(f x) + (g y)`.
        assert_eq!(sexpr(&expr("f x + g y")), "(+ (app f x) (app g y))");
        // Application is left-associative: `f x y` is `(f x) y`.
        assert_eq!(sexpr(&expr("f x y")), "(app (app f x) y)");
    }

    #[test]
    fn member_access_binds_tightest() {
        // `f a.b` is `f (a.b)`.
        assert_eq!(sexpr(&expr("f a.b")), "(app f (. a b))");
        assert_eq!(sexpr(&expr("a.b.c")), "(. (. a b) c)");
    }

    #[test]
    fn subtraction_versus_negated_argument() {
        // `f - x` is subtraction; a negated argument must be parenthesized.
        assert_eq!(sexpr(&expr("f - x")), "(- f x)");
        assert_eq!(sexpr(&expr("f (-x)")), "(app f (neg x))");
    }

    #[test]
    fn boolean_and_comparison_layering() {
        // `not` sits below the comparisons: `not a == b` is `not (a == b)`.
        assert_eq!(sexpr(&expr("not a == b")), "(not (== a b))");
        // A conjunction of comparisons needs no parens.
        assert_eq!(sexpr(&expr("a < b and c < d")), "(and (< a b) (< c d))");
        // ADR 0040, Decision 2: the logic level is homogeneous.  One word
        // chains; mixing the two words is an error naming both.
        assert_eq!(sexpr(&expr("a and b and c")), "(and (and a b) c)");
        assert_eq!(sexpr(&expr("a or b or c")), "(or (or a b) c)");
        for src in ["a or b and c", "a and b or c"] {
            let err = expr_err(src);
            assert!(err.message.contains("mixing"), "{src}: {}", err.message);
            assert!(err.message.contains("ADR 0040"), "{src}: {}", err.message);
        }
        let err = expr_err("a and b or c");
        assert!(err.message.contains("`and`"), "{}", err.message);
        assert!(err.message.contains("`or`"), "{}", err.message);
        assert_eq!(sexpr(&expr("(a and b) or c")), "(or (and a b) c)");
    }

    #[test]
    fn comparisons_do_not_chain() {
        let err = expr_err("a < b < c");
        assert!(err.message.contains("after expression"), "{}", err.message);
    }

    #[test]
    fn presence_tests() {
        assert_eq!(sexpr(&expr("r.rul is known")), "(is-known (. r rul))");
        assert_eq!(sexpr(&expr("x is missing")), "(is-missing x)");
        let err = expr_err("x is whatever");
        assert!(err.message.contains("`known` or `missing`"));
    }

    #[test]
    fn membership_uses_in() {
        assert_eq!(
            sexpr(&expr(r#""staff" in r.roles"#)),
            "(in \"staff\" (. r roles))"
        );
    }

    #[test]
    fn pipe_is_loosest_and_left_associative() {
        assert_eq!(sexpr(&expr("a |> b |> c")), "(|> (|> a b) c)");
        // The pipe is looser than application: `data |> filter p`.
        assert_eq!(sexpr(&expr("data |> filter p")), "(|> data (app filter p))");
    }

    #[test]
    fn lambda_body_extends_maximally_and_excludes_pipe() {
        // `flat_map |r| r.x` applies `flat_map` to the lambda; the body grabs `r.x`.
        assert_eq!(
            sexpr(&expr("flat_map |r| r.x")),
            "(app flat_map (lam [r] (. r x)))"
        );
        // A top-level `|>` ends the lambda body rather than entering it.
        assert_eq!(
            sexpr(&expr("data |> flat_map |r| r.x |> g")),
            "(|> (|> data (app flat_map (lam [r] (. r x)))) g)"
        );
    }

    #[test]
    fn multi_parameter_lambda_and_return_type() {
        assert_eq!(sexpr(&expr("|a, b| a + b")), "(lam [a b] (+ a b))");
        // A return ascription parses and does not swallow the body.
        let e = expr("|x| : number x + 1");
        assert_eq!(sexpr(&e), "(lam [x] (+ x 1))");
    }

    #[test]
    fn glued_closing_bar_and_gt_is_a_parse_error() {
        // `|x|>0` lexes the second bar glued to `>`; the lambda has no closing
        // bar, so it is rejected (write `|x| > 0` in a comparison instead).
        let err = expr_err("|x|>0");
        assert!(err.message.contains("close the lambda"), "{}", err.message);
    }

    #[test]
    fn grouping_tuples_and_empty_tuple() {
        // `(e)` is grouping and reduces to `e`.
        assert_eq!(sexpr(&expr("(1 + 2)")), "(+ 1 2)");
        // Grouping overrides precedence.
        assert_eq!(sexpr(&expr("(1 + 2) * 3")), "(* (+ 1 2) 3)");
        assert_eq!(sexpr(&expr("(a, b)")), "(tuple a b)");
        assert_eq!(sexpr(&expr("()")), "(tuple )");
    }

    #[test]
    fn records_with_optional_ascription() {
        assert_eq!(sexpr(&expr("(.a = x, .b = y)")), "(record a=x b=y)");
        assert_eq!(sexpr(&expr("(.a : number = 1)")), "(record a=1)");
    }

    #[test]
    fn record_field_role_marker_is_rejected() {
        // The `const`/`var` role marker ADR 0015 reserved is dropped by ADR
        // 0019; a record field is `.name [: Type] = value`.
        assert!(expr_err("(.b var = 1)").message.contains("`=`"));
        assert!(expr_err("(.c const = 1)").message.contains("`=`"));
    }

    #[test]
    fn blocks_with_let_and_assert() {
        assert_eq!(
            sexpr(&expr("{ let x = 1; assert x > 0; x }")),
            "(block let x=1; assert (> x 0); x)"
        );
        // A `{ }` in expression position is always a block, and applying a name
        // to it is ordinary juxtaposition: `completeness_check { assert p }`.
        assert_eq!(
            sexpr(&expr("completeness_check { assert p }")),
            "(app completeness_check (block assert p))"
        );
        // Trailing `;` is allowed.
        assert_eq!(sexpr(&expr("{ let x = 1; }")), "(block let x=1)");
    }

    #[test]
    fn reserved_words_cannot_name_values() {
        assert!(expr_err("and").message.contains("reserved operator"));
    }

    #[test]
    fn worked_examples_from_the_spec() {
        // The authorization predicate from `06-expressions.md`.
        assert_eq!(
            sexpr(&expr(
                r#"principal.kind == "device" and "temperature-sensor" in principal.roles"#
            )),
            "(and (== (. principal kind) \"device\") (in \"temperature-sensor\" (. principal roles)))"
        );
        // A derived value over a single row.
        assert_eq!(
            sexpr(&expr("|r| r.mass / r.height ^ 2")),
            "(lam [r] (/ (. r mass) (^ (. r height) 2)))"
        );
        // A bag reduced before comparison.
        assert_eq!(
            sexpr(&expr("|r| mean r.readings > 30")),
            "(lam [r] (> (app mean (. r readings)) 30))"
        );
    }
}
