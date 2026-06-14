//! A hand-written, recursive-descent parser for the unit and store subset.
//!
//! It consumes the token slice produced by the lexer and builds the AST in
//! `ast.rs`, following the LL(1) grammar in `docs/language/04-grammar.md`:
//! one token of lookahead, no backtracking.  Keywords are contextual, matched
//! on the `Ident` text in the position where they are expected.

use crate::ast::{DomainEntry, Field, Ident, Item, Program, StoreDecl, StrLit, TypeExpr, UnitDecl};
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
    /// Spans of the contextual keywords (`unit`, `store`, `const`, `var`,
    /// `domain`, `enum`), in source order.
    pub keyword_spans: Vec<Span>,
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
    };
    let program = parser.parse_program()?;
    Ok(Parsed {
        program,
        keyword_spans: parser.keyword_spans,
    })
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    keyword_spans: Vec<Span>,
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
            Ok(Item::Store(self.parse_store_decl()?))
        } else {
            Err(self.error("expected a `unit` or `store` declaration"))
        }
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

    fn parse_store_decl(&mut self) -> Result<StoreDecl, ParseError> {
        let start = self.cur_span().start;
        self.bump_keyword(); // `store`
        let name = self.expect_ident("a store name")?;
        self.expect(&TokenKind::LBrace, "`{` to open the store body")?;

        // The body must begin with the `unit { U }` clause.
        if !self.at_keyword("unit") {
            return Err(self.error("a store body must begin with a `unit { ... }` clause"));
        }
        self.bump_keyword(); // `unit`
        self.expect(&TokenKind::LBrace, "`{` after `unit`")?;
        let unit = self.expect_ident("the tabulated unit name")?;
        self.expect(&TokenKind::RBrace, "`}` to close the `unit` clause")?;

        let mut consts = Vec::new();
        let mut vars = Vec::new();
        let mut domain = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            if self.at_keyword("const") {
                self.parse_attr_block(&mut consts)?;
            } else if self.at_keyword("var") {
                self.parse_attr_block(&mut vars)?;
            } else if self.at_keyword("domain") {
                self.parse_domain_block(&mut domain)?;
            } else if self.at_keyword("unit") {
                return Err(
                    self.error("the `unit` clause may appear only once, at the start of the body")
                );
            } else {
                return Err(self.error("expected `const`, `var`, `domain`, or `}`"));
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`}` to close the store body")?;
        Ok(StoreDecl {
            name,
            unit,
            consts,
            vars,
            domain,
            span: Span::new(start, end.end),
        })
    }

    /// Parse a `const { ... }` or `var { ... }` block, appending its fields.
    fn parse_attr_block(&mut self, out: &mut Vec<Field>) -> Result<(), ParseError> {
        self.bump_keyword(); // `const` / `var`
        self.expect(&TokenKind::LBrace, "`{` to open the block")?;
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            out.push(self.parse_field()?);
        }
        self.expect(&TokenKind::RBrace, "`}` to close the block")?;
        Ok(())
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

    /// `name : type`, shared by unit fields and store attributes.
    fn parse_field(&mut self) -> Result<Field, ParseError> {
        let name = self.expect_ident("a field name")?;
        self.expect(&TokenKind::Colon, "`:` after the field name")?;
        let ty = self.parse_type()?;
        let span = Span::new(name.span.start, ty.span().end);
        Ok(Field { name, ty, span })
    }

    fn parse_type(&mut self) -> Result<TypeExpr, ParseError> {
        if self.at_keyword("enum") {
            let start = self.cur_span().start;
            self.bump_keyword(); // `enum`
            self.expect(&TokenKind::LParen, "`(` after `enum`")?;
            let mut variants = vec![self.expect_str("an enum variant string")?];
            while self.eat(&TokenKind::Comma) {
                variants.push(self.expect_str("an enum variant string")?);
            }
            let end = self.expect(&TokenKind::RParen, "`)` to close the enum")?;
            Ok(TypeExpr::Enum {
                variants,
                span: Span::new(start, end.end),
            })
        } else {
            Ok(TypeExpr::Named(self.expect_ident("a type")?))
        }
    }
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
              const { name: string }
            }

            store Persons {
              unit { Person }
              const { birthdate: date }
              var   { last_name: string }
            }
        "#;
        let program = parse_str(src).expect("should parse");
        assert_eq!(program.items.len(), 4);

        let Item::Unit(person) = &program.items[0] else {
            panic!("expected a unit");
        };
        assert_eq!(person.name.name, "Person");
        assert_eq!(person.fields.len(), 1);
        assert_eq!(person.fields[0].name.name, "id");

        let Item::Store(persons) = &program.items[3] else {
            panic!("expected a store");
        };
        assert_eq!(persons.name.name, "Persons");
        assert_eq!(persons.unit.name, "Person");
        assert_eq!(persons.consts.len(), 1);
        assert_eq!(persons.consts[0].name.name, "birthdate");
        assert_eq!(persons.vars.len(), 1);
        assert_eq!(persons.vars[0].name.name, "last_name");
    }

    #[test]
    fn parses_enum_type() {
        let src = r#"store S { unit { U } var { status: enum("active", "inactive") } }"#;
        let program = parse_str(src).unwrap();
        let Item::Store(s) = &program.items[0] else {
            panic!("expected a store");
        };
        match &s.vars[0].ty {
            TypeExpr::Enum { variants, .. } => {
                let values: Vec<&str> = variants.iter().map(|v| v.value.as_str()).collect();
                assert_eq!(values, ["active", "inactive"]);
            }
            other => panic!("expected an enum type, got {other:?}"),
        }
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
              const { class_id: string }
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
    fn empty_program_is_ok() {
        assert_eq!(parse_str("").unwrap().items.len(), 0);
    }

    #[test]
    fn store_without_unit_clause_is_an_error() {
        let err = parse_str("store S { const { a: string } }").unwrap_err();
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
        assert!(err.message.contains("`const`, `var`, `domain`"));
    }

    #[test]
    fn empty_enum_is_an_error() {
        let err = parse_str("unit U { x: enum() }").unwrap_err();
        assert!(err.message.contains("enum variant"));
    }

    #[test]
    fn junk_at_top_level_is_an_error() {
        let err = parse_str("wat X { }").unwrap_err();
        assert!(err.message.contains("`unit` or `store`"));
    }

    #[test]
    fn keyword_spans_cover_every_contextual_keyword() {
        let src = r#"unit U { x: enum("a") } store S { unit { U } const { a: string } var { b: number } }"#;
        let tokens = tokenize(src).expect("should lex");
        let parsed = parse_with_meta(&tokens).expect("should parse");
        let words: Vec<&str> = parsed.keyword_spans.iter().map(|s| s.slice(src)).collect();
        // In source order: the unit decl, its enum, the store decl, the
        // `unit` clause, then `const` and `var`.
        assert_eq!(words, ["unit", "enum", "store", "unit", "const", "var"]);
    }
}
