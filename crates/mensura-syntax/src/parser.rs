//! A hand-written, recursive-descent parser for the unit and store subset.
//!
//! It consumes the token slice produced by the lexer and builds the AST in
//! `ast.rs`, following the LL(1) grammar in `docs/language/04-grammar.md`:
//! one token of lookahead, no backtracking.  Keywords are contextual, matched
//! on the `Ident` text in the position where they are expected.

use crate::ast::{
    DomainEntry, Field, Ident, Item, NameSeg, NameTemplate, Program, ShapeArg, ShapeDecl,
    ShapeParam, ShapeRef, StoreDecl, StrLit, TypeExpr, UnitDecl,
};
use crate::token::{Span, Token, TokenKind};

/// A parse failure, located by a source span.
#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

/// Parse a token slice (lexer output, ending in [`TokenKind::Eof`]) into a
/// [`Program`].
pub fn parse(tokens: &[Token]) -> Result<Program, ParseError> {
    Parser { tokens, pos: 0 }.parse_program()
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
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
        } else if self.at_keyword("shape") {
            Ok(Item::Shape(self.parse_shape_decl()?))
        } else {
            Err(self.error("expected a `unit`, `store`, or `shape` declaration"))
        }
    }

    fn parse_unit_decl(&mut self) -> Result<UnitDecl, ParseError> {
        let start = self.cur_span().start;
        self.pos += 1; // `unit`
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
        self.pos += 1; // `store`
        let name = self.expect_ident("a store name")?;
        let conforms = self.parse_conforms_clause()?;
        self.expect(&TokenKind::LBrace, "`{` to open the store body")?;
        let unit = self.parse_unit_clause()?;

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
            conforms,
            consts,
            vars,
            domain,
            span: Span::new(start, end.end),
        })
    }

    fn parse_shape_decl(&mut self) -> Result<ShapeDecl, ParseError> {
        let start = self.cur_span().start;
        self.pos += 1; // `shape`
        let name = self.expect_ident("a shape name")?;
        let params = self.parse_param_list()?;
        self.expect(&TokenKind::LBrace, "`{` to open the shape body")?;
        // The unit clause is optional: a shape with none is unit-agnostic.
        let unit = if self.at_keyword("unit") {
            Some(self.parse_unit_clause()?)
        } else {
            None
        };

        let mut consts = Vec::new();
        let mut vars = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            if self.at_keyword("const") {
                self.parse_attr_block(&mut consts)?;
            } else if self.at_keyword("var") {
                self.parse_attr_block(&mut vars)?;
            } else if self.at_keyword("domain") {
                return Err(self.error("a shape cannot contain a `domain` block"));
            } else if self.at_keyword("unit") {
                return Err(
                    self.error("the `unit` clause may appear only once, at the start of the body")
                );
            } else {
                return Err(self.error("expected `const`, `var`, or `}`"));
            }
        }
        let end = self.expect(&TokenKind::RBrace, "`}` to close the shape body")?;
        Ok(ShapeDecl {
            name,
            params,
            unit,
            consts,
            vars,
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
    /// argument list, e.g. `Tabular(Person)` or `PersonRecord`.
    fn parse_shape_ref(&mut self, what: &str) -> Result<ShapeRef, ParseError> {
        let name = self.expect_ident(what)?;
        let mut args = Vec::new();
        let mut end = name.span.end;
        if self.check(&TokenKind::LParen) {
            self.pos += 1; // `(`
            args.push(self.parse_shape_arg()?);
            while self.eat(&TokenKind::Comma) {
                args.push(self.parse_shape_arg()?);
            }
            end = self
                .expect(&TokenKind::RParen, "`)` to close the argument list")?
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

    /// Parse an optional `(name: Kind, ...)` shape parameter list.  Returns an
    /// empty vector when absent.  An empty `()` is rejected.
    fn parse_param_list(&mut self) -> Result<Vec<ShapeParam>, ParseError> {
        let mut params = Vec::new();
        if self.check(&TokenKind::LParen) {
            self.pos += 1; // `(`
            if self.check(&TokenKind::RParen) {
                return Err(self.error("empty parameter list; omit the `()`"));
            }
            params.push(self.parse_param()?);
            while self.eat(&TokenKind::Comma) {
                params.push(self.parse_param()?);
            }
            self.expect(&TokenKind::RParen, "`)` to close the parameter list")?;
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
        self.pos += 1; // `unit`
        self.expect(&TokenKind::LBrace, "`{` after `unit`")?;
        let unit = self.expect_ident("the tabulated unit name")?;
        self.expect(&TokenKind::RBrace, "`}` to close the `unit` clause")?;
        Ok(unit)
    }

    /// Parse a `const { ... }` or `var { ... }` block, appending its fields.
    /// Shared by stores and shapes.
    fn parse_attr_block(&mut self, out: &mut Vec<Field>) -> Result<(), ParseError> {
        self.pos += 1; // `const` / `var`
        self.expect(&TokenKind::LBrace, "`{` to open the block")?;
        while !self.check(&TokenKind::RBrace) && !self.at_eof() {
            out.push(self.parse_field()?);
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
        self.pos += 1; // `domain`
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

    fn parse_type(&mut self) -> Result<TypeExpr, ParseError> {
        if self.at_keyword("enum") {
            let start = self.cur_span().start;
            self.pos += 1; // `enum`
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
        assert_eq!(person.fields[0].name.as_literal(), Some("id"));

        let Item::Store(persons) = &program.items[3] else {
            panic!("expected a store");
        };
        assert_eq!(persons.name.name, "Persons");
        assert_eq!(persons.unit.name, "Person");
        assert_eq!(persons.consts.len(), 1);
        assert_eq!(persons.consts[0].name.as_literal(), Some("birthdate"));
        assert_eq!(persons.vars.len(), 1);
        assert_eq!(persons.vars[0].name.as_literal(), Some("last_name"));
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
    fn parses_shape_and_conformance() {
        let src = r#"
            shape PersonRecord {
              unit { Person }
              const { admission: date }
            }

            store Students : PersonRecord {
              unit { Person }
              const { admission: date }
            }
        "#;
        let program = parse_str(src).unwrap();

        let Item::Shape(shape) = &program.items[0] else {
            panic!("expected a shape");
        };
        assert_eq!(shape.name.name, "PersonRecord");
        assert_eq!(shape.unit.as_ref().unwrap().name, "Person");
        assert_eq!(shape.consts[0].name.as_literal(), Some("admission"));

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

    #[test]
    fn parses_unit_parameter_shape() {
        let program = parse_str("shape Tabular(U: Unit) { unit { U } }").unwrap();
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
        let program = parse_str("shape Named { const { name: string } }").unwrap();
        let Item::Shape(shape) = &program.items[0] else {
            panic!("expected a shape");
        };
        assert!(shape.params.is_empty());
        assert!(shape.unit.is_none());
        assert_eq!(shape.consts[0].name.as_literal(), Some("name"));
    }

    #[test]
    fn parses_parametric_conformance() {
        let program = parse_str("store S : Tabular(Person) { unit { Person } }").unwrap();
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
            shape Ageable(date_field: string) {
              const { `{date_field}`: date }
            }
            store Persons : Ageable("birthdate") {
              unit { Person }
              const { birthdate: date }
            }
        "#;
        let program = parse_str(src).unwrap();

        let Item::Shape(shape) = &program.items[0] else {
            panic!("expected a shape");
        };
        assert_eq!(shape.params[0].kind.name, "string");
        // The attribute name is a single interpolated parameter.
        assert_eq!(shape.consts[0].name.segments.len(), 1);
        let NameSeg::Param(p) = &shape.consts[0].name.segments[0] else {
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
        let program = parse_str("shape S(col: string) { const { `{col}_z`: number } }").unwrap();
        let Item::Shape(shape) = &program.items[0] else {
            panic!("expected a shape");
        };
        let segs = &shape.consts[0].name.segments;
        assert_eq!(segs.len(), 2);
        assert!(matches!(&segs[0], NameSeg::Param(p) if p.name == "col"));
        assert!(matches!(&segs[1], NameSeg::Lit(s) if s == "_z"));
    }

    #[test]
    fn empty_interpolation_is_an_error() {
        let err = parse_str("shape S { const { `{}`: number } }").unwrap_err();
        assert!(err.message.contains("empty `{}`"));
    }

    #[test]
    fn empty_parameter_list_is_an_error() {
        let err = parse_str("shape Bad() { unit { U } }").unwrap_err();
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
        assert!(err.message.contains("`unit`, `store`, or `shape`"));
    }
}
