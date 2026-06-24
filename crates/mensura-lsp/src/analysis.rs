//! Turning a source string into semantic tokens and diagnostics.
//!
//! Both are derived from the existing pipeline (`lex -> parse -> resolve`), so
//! this module owns only span translation and classification, not analysis.
//! Highlighting runs over the parse when it succeeds (the AST tier) and falls
//! back to the raw token and trivia streams when it does not (the lex tier);
//! see `docs/toolkit/02-lsp.md`.

use lsp_types::{
    Diagnostic, DiagnosticSeverity, Position, Range, SemanticToken, SemanticTokenType,
};

use mensura_syntax::{
    Item, NameSeg, NameTemplate, Program, ShapeArg, Span, TokenKind, TypeExpr, lex, parse_with_meta,
};
use mensura_types::resolve;

use crate::line_index::{LineIndex, PositionEncoding, encoded_len};

/// The semantic-token legend, advertised at `initialize`.  Indices into this
/// list are the `token_type` field of each emitted [`SemanticToken`], so the
/// `*_TY` constants below must match this order.
pub fn token_legend() -> Vec<SemanticTokenType> {
    vec![
        SemanticTokenType::KEYWORD,
        SemanticTokenType::TYPE,
        SemanticTokenType::PROPERTY,
        SemanticTokenType::PARAMETER,
        SemanticTokenType::STRING,
        SemanticTokenType::NUMBER,
        SemanticTokenType::OPERATOR,
        SemanticTokenType::ENUM_MEMBER,
        SemanticTokenType::COMMENT,
    ]
}

const KEYWORD_TY: u32 = 0;
const TYPE_TY: u32 = 1;
const PROPERTY_TY: u32 = 2;
const PARAMETER_TY: u32 = 3;
const STRING_TY: u32 = 4;
const NUMBER_TY: u32 = 5;
const OPERATOR_TY: u32 = 6;
const ENUM_MEMBER_TY: u32 = 7;
const COMMENT_TY: u32 = 8;

/// What the server reports for one document version.
pub struct Analysis {
    pub tokens: Vec<SemanticToken>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Analyze `src`, producing semantic tokens and diagnostics with positions in
/// the negotiated `encoding`.
pub fn analyze(src: &str, encoding: PositionEncoding) -> Analysis {
    let line_index = LineIndex::new(src);
    let mut builder = TokenBuilder::new(&line_index, src, encoding);
    let mut diagnostics = Vec::new();

    let lexed = match lex(src) {
        Ok(lexed) => lexed,
        Err(err) => {
            // The lexer stops at the first malformed token, so there is no
            // usable token stream to highlight.
            diagnostics.push(diagnostic(
                &line_index,
                src,
                err.span,
                err.message,
                encoding,
            ));
            return Analysis {
                tokens: builder.finish(),
                diagnostics,
            };
        }
    };

    // Comments always highlight: they come from the trivia channel and do not
    // depend on the parse succeeding.
    for trivia in &lexed.trivia {
        builder.push(trivia.span, COMMENT_TY);
    }

    match parse_with_meta(&lexed.tokens) {
        Ok(parsed) => {
            // AST tier.  The parser is the only place that knows an `Ident`
            // was a keyword, so keyword spans come from it; names, properties,
            // and enum members come from the AST.
            for &span in &parsed.keyword_spans {
                builder.push(span, KEYWORD_TY);
            }
            highlight_program(&mut builder, &parsed.program);
            highlight_literals(&mut builder, &lexed.tokens);
            if let Err(errors) = resolve(&parsed.program) {
                for err in errors {
                    diagnostics.push(diagnostic(
                        &line_index,
                        src,
                        err.span,
                        err.message,
                        encoding,
                    ));
                }
            }
        }
        Err(err) => {
            // Lex tier.  Without an AST, keywords, types, and properties can
            // not be classified; literals and operators still color so a file
            // mid-edit does not go blank.
            highlight_literals(&mut builder, &lexed.tokens);
            diagnostics.push(diagnostic(
                &line_index,
                src,
                err.span,
                err.message,
                encoding,
            ));
        }
    }

    Analysis {
        tokens: builder.finish(),
        diagnostics,
    }
}

/// Emit type, property, parameter, and enum-member tokens from the AST.
fn highlight_program(builder: &mut TokenBuilder, program: &Program) {
    for item in &program.items {
        match item {
            Item::Unit(unit) => {
                builder.push(unit.name.span, TYPE_TY);
                for field in &unit.fields {
                    highlight_field(builder, field);
                }
            }
            Item::Store(store) => {
                builder.push(store.name.span, TYPE_TY);
                builder.push(store.unit.span, TYPE_TY);
                for shape_ref in &store.conforms {
                    builder.push(shape_ref.name.span, TYPE_TY);
                    for arg in &shape_ref.args {
                        // A `Str` argument is a string literal already colored
                        // by the token stream; only unit references need the
                        // type classification.
                        if let ShapeArg::Unit(id) = arg {
                            builder.push(id.span, TYPE_TY);
                        }
                    }
                }
                for field in store.consts.iter().chain(&store.vars) {
                    highlight_field(builder, field);
                }
                for entry in &store.domain {
                    builder.push(entry.field.span, PROPERTY_TY);
                    builder.push(entry.store.span, TYPE_TY);
                }
            }
            Item::Shape(shape) => {
                builder.push(shape.name.span, TYPE_TY);
                for param in &shape.params {
                    builder.push(param.name.span, PARAMETER_TY);
                    builder.push(param.kind.span, TYPE_TY);
                }
                if let Some(unit) = &shape.unit {
                    builder.push(unit.span, TYPE_TY);
                }
                for field in shape.consts.iter().chain(&shape.vars) {
                    highlight_field(builder, field);
                }
            }
            Item::Enum(decl) => {
                builder.push(decl.name.span, TYPE_TY);
                for variant in &decl.variants {
                    builder.push(variant.span, ENUM_MEMBER_TY);
                }
            }
        }
    }
}

fn highlight_field(builder: &mut TokenBuilder, field: &mensura_syntax::Field) {
    highlight_name(builder, &field.name);
    // A field type is a single identifier (primitive, unit, or named enum);
    // named enum variants are highlighted at the `enum` declaration.
    let TypeExpr::Named(id) = &field.ty;
    builder.push(id.span, TYPE_TY);
}

/// Highlight an attribute name.  A plain identifier is one `property` span; a
/// backtick template colors its `{param}` holes as `parameter` and the literal
/// remainder (text, braces, backticks) as `property`, split into
/// non-overlapping spans since LSP tokens may not nest.
fn highlight_name(builder: &mut TokenBuilder, name: &NameTemplate) {
    if name.as_literal().is_some() {
        builder.push(name.span, PROPERTY_TY);
        return;
    }
    let mut cursor = name.span.start;
    for seg in &name.segments {
        if let NameSeg::Param(id) = seg {
            if id.span.start > cursor {
                builder.push(Span::new(cursor, id.span.start), PROPERTY_TY);
            }
            builder.push(id.span, PARAMETER_TY);
            cursor = id.span.end;
        }
    }
    if cursor < name.span.end {
        builder.push(Span::new(cursor, name.span.end), PROPERTY_TY);
    }
}

/// Emit string, number, and operator tokens straight from the token stream.
/// Identifiers are left to the AST tier (or unclassified in the lex tier), so
/// they are skipped here.
fn highlight_literals(builder: &mut TokenBuilder, tokens: &[mensura_syntax::Token]) {
    for token in tokens {
        let ty = match token.kind {
            TokenKind::Str(_) => STRING_TY,
            TokenKind::Int(_) | TokenKind::Float(_) => NUMBER_TY,
            // Identifiers and template names are classified by the AST tier.
            TokenKind::Ident(_) | TokenKind::Template(_) | TokenKind::Eof => continue,
            // Everything else is an operator or punctuation.
            _ => OPERATOR_TY,
        };
        builder.push(token.span, ty);
    }
}

/// Accumulates raw spans, then resolves overlaps and delta-encodes them into
/// the LSP wire format.
struct TokenBuilder<'a> {
    line_index: &'a LineIndex,
    src: &'a str,
    encoding: PositionEncoding,
    raw: Vec<Raw>,
}

/// One single-line highlighted span, pre-translated to a position.
struct Raw {
    byte_start: usize,
    byte_end: usize,
    line: u32,
    character: u32,
    length: u32,
    token_type: u32,
}

impl<'a> TokenBuilder<'a> {
    fn new(line_index: &'a LineIndex, src: &'a str, encoding: PositionEncoding) -> Self {
        TokenBuilder {
            line_index,
            src,
            encoding,
            raw: Vec::new(),
        }
    }

    /// Record a span under a token type, splitting it at newlines (LSP tokens
    /// may not cross a line) and dropping empty segments.
    fn push(&mut self, span: Span, token_type: u32) {
        let mut seg_start = span.start;
        while seg_start < span.end {
            let rest = &self.src[seg_start..span.end];
            let seg_end = match rest.find('\n') {
                Some(nl) => seg_start + nl,
                None => span.end,
            };
            if seg_end > seg_start {
                let (line, character) =
                    self.line_index.position(self.src, seg_start, self.encoding);
                let length = encoded_len(&self.src[seg_start..seg_end], self.encoding);
                self.raw.push(Raw {
                    byte_start: seg_start,
                    byte_end: seg_end,
                    line,
                    character,
                    length,
                    token_type,
                });
            }
            // Advance past the segment and its newline, if any.
            seg_start = self.src[seg_start..span.end]
                .find('\n')
                .map(|nl| seg_start + nl + 1)
                .unwrap_or(span.end);
        }
    }

    /// Sort, drop overlaps (a more specific classification wins ties), and
    /// delta-encode into the protocol's `SemanticToken` stream.
    fn finish(mut self) -> Vec<SemanticToken> {
        self.raw
            .sort_by_key(|r| (r.byte_start, priority(r.token_type)));

        let mut tokens = Vec::new();
        let mut prev_line = 0;
        let mut prev_char = 0;
        let mut last_end = 0;
        for r in &self.raw {
            // Skip a span that overlaps one already emitted (e.g. an enum
            // variant string also seen as a bare string literal).
            if r.byte_start < last_end {
                continue;
            }
            last_end = r.byte_end;
            let delta_line = r.line - prev_line;
            let delta_start = if delta_line == 0 {
                r.character - prev_char
            } else {
                r.character
            };
            tokens.push(SemanticToken {
                delta_line,
                delta_start,
                length: r.length,
                token_type: r.token_type,
                token_modifiers_bitset: 0,
            });
            prev_line = r.line;
            prev_char = r.character;
        }
        tokens
    }
}

/// Lower value wins when two spans start at the same byte.  AST- and
/// keyword-derived types are more specific than token-derived ones.
fn priority(token_type: u32) -> u8 {
    match token_type {
        COMMENT_TY => 0,
        ENUM_MEMBER_TY => 1,
        KEYWORD_TY => 2,
        TYPE_TY => 3,
        PARAMETER_TY => 4,
        PROPERTY_TY => 5,
        STRING_TY => 6,
        NUMBER_TY => 7,
        _ => 8,
    }
}

fn diagnostic(
    line_index: &LineIndex,
    src: &str,
    span: Span,
    message: String,
    encoding: PositionEncoding,
) -> Diagnostic {
    let (sl, sc) = line_index.position(src, span.start, encoding);
    let (el, ec) = line_index.position(src, span.end, encoding);
    Diagnostic {
        range: Range {
            start: Position::new(sl, sc),
            end: Position::new(el, ec),
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("mensura".to_string()),
        message,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types_at(src: &str) -> Vec<(u32, u32, u32, u32)> {
        // (delta_line, delta_start, length, token_type) for each token.
        analyze(src, PositionEncoding::Utf8)
            .tokens
            .iter()
            .map(|t| (t.delta_line, t.delta_start, t.length, t.token_type))
            .collect()
    }

    #[test]
    fn highlights_a_unit_declaration() {
        // `unit` keyword, `U` type, `id` property, `string` type.
        let toks = types_at("unit U { id: string }");
        let kinds: Vec<u32> = toks.iter().map(|t| t.3).collect();
        assert!(kinds.contains(&KEYWORD_TY));
        assert!(kinds.contains(&TYPE_TY));
        assert!(kinds.contains(&PROPERTY_TY));
    }

    #[test]
    fn enum_variants_are_enum_members_not_strings() {
        let toks = types_at(r#"enum Status { "active", "inactive" }"#);
        let kinds: Vec<u32> = toks.iter().map(|t| t.3).collect();
        // The two variants are enum members; no plain string token survives.
        assert_eq!(kinds.iter().filter(|&&k| k == ENUM_MEMBER_TY).count(), 2);
        assert_eq!(kinds.iter().filter(|&&k| k == STRING_TY).count(), 0);
    }

    #[test]
    fn comments_highlight_even_when_the_parse_fails() {
        // `unit` with no name is a parse error, but the comment still colors.
        let analysis = analyze("// note\nunit", PositionEncoding::Utf8);
        let kinds: Vec<u32> = analysis.tokens.iter().map(|t| t.token_type).collect();
        assert!(kinds.contains(&COMMENT_TY));
        assert_eq!(analysis.diagnostics.len(), 1);
    }

    #[test]
    fn resolve_errors_become_diagnostics() {
        // A store naming an unknown unit resolves with an error.
        let analysis = analyze("store S { unit { Missing } }", PositionEncoding::Utf8);
        assert!(!analysis.diagnostics.is_empty());
    }

    #[test]
    fn shapes_highlight_keyword_type_and_parameter() {
        // `shape`/`var` keywords; `Sized`, `string`, `number` types; the
        // declared param `c` and the `{c}` template hole are both parameters;
        // the literal `_z` template segment is a property.
        let toks = types_at("shape Sized[c: string] { var { `{c}_z`: number } }");
        let kinds: Vec<u32> = toks.iter().map(|t| t.3).collect();
        assert!(kinds.contains(&KEYWORD_TY));
        assert!(kinds.contains(&TYPE_TY));
        assert!(kinds.contains(&PROPERTY_TY));
        assert_eq!(kinds.iter().filter(|&&k| k == PARAMETER_TY).count(), 2);
    }
}
