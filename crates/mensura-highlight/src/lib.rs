//! Turning a Mensura source string into classified, non-overlapping spans.
//!
//! Both the language server (`mensura-lsp`) and the book preprocessor
//! (`mensura-mdbook`) need to color source the same way the compiler sees it.
//! In a keyword-free grammar only the parser knows that an `Ident` was acting
//! as `unit`/`store`/`attr`, so highlighting runs over the real pipeline
//! (`lex -> parse -> resolve`) rather than a regex approximation.  This crate
//! owns classification and overlap resolution in byte offsets; consumers add
//! the protocol- or HTML-specific rendering.  See
//! `docs/toolkit/03-book-highlighting.md`.

use mensura_syntax::{
    Field, Item, LetKind, NameSeg, NameTemplate, Program, ShapeArg, Span, Token, TokenKind,
    TypeExpr, TypeKind, lex, parse_with_meta,
};
use mensura_types::resolve;

/// One of the nine token classes.  The order is the LSP semantic-token legend
/// (`mensura-lsp`), but this crate treats it only as a tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HighlightKind {
    Keyword,
    Function,
    Type,
    Property,
    Parameter,
    String,
    Number,
    Operator,
    EnumMember,
    Comment,
}

impl HighlightKind {
    /// Lower value wins when two spans start at the same byte.  AST- and
    /// keyword-derived classes are more specific than token-derived ones.
    fn priority(self) -> u8 {
        match self {
            HighlightKind::Comment => 0,
            HighlightKind::EnumMember => 1,
            HighlightKind::Keyword => 2,
            HighlightKind::Function => 3,
            HighlightKind::Type => 4,
            HighlightKind::Parameter => 5,
            HighlightKind::Property => 6,
            HighlightKind::String => 7,
            HighlightKind::Number => 8,
            HighlightKind::Operator => 9,
        }
    }
}

/// A classified, non-overlapping source span in byte offsets.  Spans never
/// cross a newline, so a consumer can position or wrap each one line by line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub start: usize,
    pub end: usize,
    pub kind: HighlightKind,
}

/// A lex, parse, or resolve error, located by byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckError {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

/// Classified spans plus the errors found, for a single source string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlighted {
    pub spans: Vec<Highlight>,
    pub errors: Vec<CheckError>,
}

/// Classify `src`, returning non-overlapping spans (in source order) and every
/// lex, parse, and resolve error.  Highlighting runs over the parse when it
/// succeeds (the AST tier) and falls back to the raw token and trivia streams
/// when it does not (the lex tier), so a file mid-edit never goes blank.
pub fn highlight(src: &str) -> Highlighted {
    let mut builder = Builder::new(src);
    let mut errors = Vec::new();

    let lexed = match lex(src) {
        Ok(lexed) => lexed,
        Err(err) => {
            // The lexer stops at the first malformed token, so there is no
            // usable token stream to highlight.
            errors.push(check_error(err.span, err.message));
            return Highlighted {
                spans: builder.finish(),
                errors,
            };
        }
    };

    // Comments always highlight: they come from the trivia channel and do not
    // depend on the parse succeeding.
    for trivia in &lexed.trivia {
        builder.push(trivia.span, HighlightKind::Comment);
    }

    match parse_with_meta(&lexed.tokens) {
        Ok(parsed) => {
            // AST tier.  The parser is the only place that knows an `Ident`
            // was a keyword, so keyword spans come from it; names, properties,
            // and enum members come from the AST.
            for &span in &parsed.keyword_spans {
                builder.push(span, HighlightKind::Keyword);
            }
            // Pipeline operation heads (the leftmost identifier of each `|>`
            // right-hand side) color as builtin functions.
            for &span in &parsed.op_spans {
                builder.push(span, HighlightKind::Function);
            }
            highlight_program(&mut builder, &parsed.program);
            highlight_literals(&mut builder, &lexed.tokens);
            if let Err(resolve_errors) = resolve(&parsed.program) {
                for err in resolve_errors {
                    errors.push(check_error(err.span, err.message));
                }
            }
        }
        Err(err) => {
            // Lex tier.  Without an AST, keywords, types, and properties can
            // not be classified; literals and operators still color.
            highlight_literals(&mut builder, &lexed.tokens);
            errors.push(check_error(err.span, err.message));
        }
    }

    Highlighted {
        spans: builder.finish(),
        errors,
    }
}

fn check_error(span: Span, message: String) -> CheckError {
    CheckError {
        start: span.start,
        end: span.end,
        message,
    }
}

/// Emit type, property, parameter, and enum-member spans from the AST.
fn highlight_program(builder: &mut Builder, program: &Program) {
    for item in &program.items {
        match item {
            Item::Unit(unit) => {
                builder.push(unit.name.span, HighlightKind::Type);
                for field in &unit.fields {
                    highlight_field(builder, field);
                }
            }
            Item::Store(store) => {
                builder.push(store.name.span, HighlightKind::Type);
                builder.push(store.unit.span, HighlightKind::Type);
                for shape_ref in &store.conforms {
                    builder.push(shape_ref.name.span, HighlightKind::Type);
                    for arg in &shape_ref.args {
                        // A `Str` argument is a string literal already colored
                        // by the token stream; only unit references need the
                        // type classification.
                        if let ShapeArg::Unit(id) = arg {
                            builder.push(id.span, HighlightKind::Type);
                        }
                    }
                }
                for attr in store.attrs.iter() {
                    highlight_field(builder, &attr.field);
                }
                for entry in &store.domain {
                    builder.push(entry.field.span, HighlightKind::Property);
                    builder.push(entry.store.span, HighlightKind::Type);
                }
            }
            Item::Shape(shape) => {
                builder.push(shape.name.span, HighlightKind::Type);
                for param in &shape.params {
                    builder.push(param.name.span, HighlightKind::Parameter);
                    builder.push(param.kind.span, HighlightKind::Type);
                }
                if let Some(unit) = &shape.unit {
                    builder.push(unit.span, HighlightKind::Type);
                }
                for attr in shape.attrs.iter() {
                    highlight_field(builder, &attr.field);
                }
            }
            Item::Enum(decl) => {
                builder.push(decl.name.span, HighlightKind::Type);
                for variant in &decl.variants {
                    builder.push(variant.span, HighlightKind::EnumMember);
                }
            }
            Item::View(view) => {
                builder.push(view.name.span, HighlightKind::Type);
                for shape_ref in &view.conforms {
                    builder.push(shape_ref.name.span, HighlightKind::Type);
                    for arg in &shape_ref.args {
                        if let ShapeArg::Unit(id) = arg {
                            builder.push(id.span, HighlightKind::Type);
                        }
                    }
                }
                // The body is a pipeline expression; its tokens are colored by
                // the token stream, so no declaration-level spans are added here.
            }
            Item::Import(import) => {
                // The `import` keyword is in the parser's keyword spans; the
                // module name is a term-level namespace, left unclassified
                // like other expression identifiers.
                let _ = import;
            }
            Item::Let(decl) => match &decl.kind {
                // A value binding's body is an expression; its tokens are
                // colored by the token stream, like a view body.
                LetKind::Value { ty, .. } => {
                    if let Some(ty) = ty {
                        highlight_type(builder, ty);
                    }
                }
                // A dimension alias is a type-level binding: its name, its
                // parameters, and every identifier in its body are types.
                LetKind::DimAlias { params, body } => {
                    builder.push(decl.name.span, HighlightKind::Type);
                    for p in params {
                        builder.push(p.span, HighlightKind::Parameter);
                    }
                    highlight_type(builder, body);
                }
            },
        }
    }
}

fn highlight_field(builder: &mut Builder, field: &Field) {
    highlight_name(builder, &field.name);
    highlight_type(builder, &field.ty);
}

/// Color every identifier in a type-level expression as a type: the base
/// names of a dimension expression and the backing of an application
/// (`temperature[real]`).  The `*`/`/`/`^`/`[`/`]` tokens and a trailing `?`
/// optional marker are colored as operators by the token stream; named enum
/// variants are highlighted at the `enum` declaration.
fn highlight_type(builder: &mut Builder, ty: &TypeExpr) {
    match &ty.kind {
        TypeKind::Named(id) => builder.push(id.span, HighlightKind::Type),
        TypeKind::Apply { base, backing } => {
            highlight_type(builder, base);
            builder.push(backing.span, HighlightKind::Type);
        }
        TypeKind::Mul(a, b) | TypeKind::Div(a, b) => {
            highlight_type(builder, a);
            highlight_type(builder, b);
        }
        TypeKind::Pow(base, _) => highlight_type(builder, base),
    }
}

/// Highlight an attribute name.  A plain identifier is one property span; a
/// backtick template colors its `{param}` holes as parameters and the literal
/// remainder (text, braces, backticks) as property, split into non-overlapping
/// spans.
fn highlight_name(builder: &mut Builder, name: &NameTemplate) {
    if name.as_literal().is_some() {
        builder.push(name.span, HighlightKind::Property);
        return;
    }
    let mut cursor = name.span.start;
    for seg in &name.segments {
        if let NameSeg::Param(id) = seg {
            if id.span.start > cursor {
                builder.push(Span::new(cursor, id.span.start), HighlightKind::Property);
            }
            builder.push(id.span, HighlightKind::Parameter);
            cursor = id.span.end;
        }
    }
    if cursor < name.span.end {
        builder.push(Span::new(cursor, name.span.end), HighlightKind::Property);
    }
}

/// Emit string, number, template, and operator spans straight from the token
/// stream.  Identifiers are left to the AST tier (or unclassified in the lex
/// tier), so they are skipped here.
fn highlight_literals(builder: &mut Builder, tokens: &[Token]) {
    for token in tokens {
        let kind = match token.kind {
            TokenKind::Str(_) => HighlightKind::String,
            TokenKind::Int(_) | TokenKind::Float(_) => HighlightKind::Number,
            // Identifiers are classified by the AST tier.
            TokenKind::Ident(_) | TokenKind::Eof => continue,
            // A template is an attribute *name* in a declaration and a
            // backticked *combiner* in an expression (ADR 0031, Decision 6).
            // Coloring it as an operator here is the combiner case: the AST
            // tier pushes the name case first, and `Operator` has the lowest
            // priority, so a claimed name span always wins the overlap.
            TokenKind::Template(_) => HighlightKind::Operator,
            // Everything else is an operator or punctuation.
            _ => HighlightKind::Operator,
        };
        builder.push(token.span, kind);
    }
}

/// Accumulates raw spans, then resolves overlaps into a non-overlapping
/// sequence in source order.  Spans are split at newlines on the way in so no
/// span ever crosses a line.
struct Builder<'a> {
    src: &'a str,
    raw: Vec<Highlight>,
}

impl<'a> Builder<'a> {
    fn new(src: &'a str) -> Self {
        Builder {
            src,
            raw: Vec::new(),
        }
    }

    /// Record a span under a kind, splitting it at newlines and dropping empty
    /// segments.
    fn push(&mut self, span: Span, kind: HighlightKind) {
        let mut seg_start = span.start;
        while seg_start < span.end {
            let rest = &self.src[seg_start..span.end];
            let seg_end = match rest.find('\n') {
                Some(nl) => seg_start + nl,
                None => span.end,
            };
            if seg_end > seg_start {
                self.raw.push(Highlight {
                    start: seg_start,
                    end: seg_end,
                    kind,
                });
            }
            // Advance past the segment and its newline, if any.
            seg_start = self.src[seg_start..span.end]
                .find('\n')
                .map(|nl| seg_start + nl + 1)
                .unwrap_or(span.end);
        }
    }

    /// Sort, then drop any span that overlaps one already kept (a more
    /// specific classification wins ties, e.g. an enum variant over the bare
    /// string literal underneath it).
    fn finish(mut self) -> Vec<Highlight> {
        self.raw.sort_by_key(|h| (h.start, h.kind.priority()));
        let mut spans: Vec<Highlight> = Vec::new();
        let mut last_end = 0;
        for h in self.raw {
            if h.start < last_end {
                continue;
            }
            last_end = h.end;
            spans.push(h);
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<HighlightKind> {
        highlight(src).spans.iter().map(|h| h.kind).collect()
    }

    #[test]
    fn highlights_a_unit_declaration() {
        let ks = kinds("unit U { id: string }");
        assert!(ks.contains(&HighlightKind::Keyword));
        assert!(ks.contains(&HighlightKind::Type));
        assert!(ks.contains(&HighlightKind::Property));
    }

    #[test]
    fn a_backticked_combiner_colors_as_an_operator() {
        // ADR 0031, Decision 6.  The combiner is a `Template` token in
        // expression position, which the lex tier now colors.
        let ks = kinds("view v { readings |> map_bags |k, b| (.n = #b.x) }");
        assert!(ks.contains(&HighlightKind::Operator));
    }

    #[test]
    fn a_templated_attribute_name_still_colors_as_a_property() {
        // The other `Template` case: an attribute name in a declaration.  The
        // AST tier claims the span first and `Operator` has the lowest
        // priority, so the name wins the overlap and no combiner coloring
        // leaks into a shape.
        let ks = kinds("shape S[p: string] { attr { `{p}_z`: real } }");
        assert!(ks.contains(&HighlightKind::Property));
        assert!(ks.contains(&HighlightKind::Parameter));
    }

    #[test]
    fn pipeline_operations_are_functions() {
        // Operation heads on the right of `|>` color as builtin functions.
        let ks = kinds("view v { readings |> promote machine }");
        assert!(ks.contains(&HighlightKind::Function));
    }

    #[test]
    fn predicate_operators_color_as_keywords() {
        // `is` and `known` are now recorded by the parser, so they color even
        // though they are not declaration keywords.  `view` is a keyword too,
        // so there are at least three keyword spans.
        let ks = kinds("view v { completeness_check { assert a is known } }");
        assert!(ks.iter().filter(|&&k| k == HighlightKind::Keyword).count() >= 3);
    }

    #[test]
    fn enum_variants_are_enum_members_not_strings() {
        let ks = kinds(r#"enum Status { "active" "inactive" }"#);
        assert_eq!(
            ks.iter()
                .filter(|&&k| k == HighlightKind::EnumMember)
                .count(),
            2
        );
        assert_eq!(
            ks.iter().filter(|&&k| k == HighlightKind::String).count(),
            0
        );
    }

    #[test]
    fn spans_are_ordered_and_non_overlapping() {
        let spans = highlight("unit U { id: string }").spans;
        let mut prev_end = 0;
        for h in &spans {
            assert!(h.start >= prev_end, "spans overlap or are out of order");
            assert!(h.end > h.start);
            prev_end = h.end;
        }
    }

    #[test]
    fn comments_highlight_even_when_the_parse_fails() {
        // `unit` with no name is a parse error, but the comment still colors.
        let result = highlight("// note\nunit");
        let ks: Vec<_> = result.spans.iter().map(|h| h.kind).collect();
        assert!(ks.contains(&HighlightKind::Comment));
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn resolve_errors_are_reported() {
        // A store naming an unknown unit resolves with an error.
        let result = highlight("store S { unit { Missing } }");
        assert!(!result.errors.is_empty());
    }

    #[test]
    fn clean_program_has_no_errors() {
        let result = highlight("unit U { id: string }");
        assert!(result.errors.is_empty());
    }
}
