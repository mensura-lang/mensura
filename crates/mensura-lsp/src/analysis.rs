//! Translating classified spans and check errors into LSP wire types.
//!
//! Classification lives in `mensura-highlight`; this module only adapts its
//! byte-offset output to the protocol's position-encoded, delta-encoded
//! semantic tokens and `Diagnostic`s.  See `docs/toolkit/02-lsp.md` and
//! `docs/toolkit/03-book-highlighting.md`.

use lsp_types::{
    Diagnostic, DiagnosticSeverity, Position, Range, SemanticToken, SemanticTokenType,
};

use mensura_highlight::{CheckError, Highlight, HighlightKind, highlight};

use crate::line_index::{LineIndex, PositionEncoding, encoded_len};

/// The semantic-token legend, advertised at `initialize`.  The position of
/// each type is what [`legend_index`] returns for the matching
/// [`HighlightKind`], so the two must stay in the same order.
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

/// Index of a kind into [`token_legend`].
fn legend_index(kind: HighlightKind) -> u32 {
    match kind {
        HighlightKind::Keyword => 0,
        HighlightKind::Type => 1,
        HighlightKind::Property => 2,
        HighlightKind::Parameter => 3,
        HighlightKind::String => 4,
        HighlightKind::Number => 5,
        HighlightKind::Operator => 6,
        HighlightKind::EnumMember => 7,
        HighlightKind::Comment => 8,
    }
}

/// What the server reports for one document version.
pub struct Analysis {
    pub tokens: Vec<SemanticToken>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Analyze `src`, producing semantic tokens and diagnostics with positions in
/// the negotiated `encoding`.
pub fn analyze(src: &str, encoding: PositionEncoding) -> Analysis {
    let line_index = LineIndex::new(src);
    let result = highlight(src);
    Analysis {
        tokens: encode_tokens(&line_index, src, encoding, &result.spans),
        diagnostics: result
            .errors
            .iter()
            .map(|err| diagnostic(&line_index, src, err, encoding))
            .collect(),
    }
}

/// Delta-encode classified spans into the protocol's `SemanticToken` stream.
/// The spans arrive in source order and never cross a line, so each is one
/// token.
fn encode_tokens(
    line_index: &LineIndex,
    src: &str,
    encoding: PositionEncoding,
    spans: &[Highlight],
) -> Vec<SemanticToken> {
    let mut tokens = Vec::with_capacity(spans.len());
    let mut prev_line = 0;
    let mut prev_char = 0;
    for span in spans {
        let (line, character) = line_index.position(src, span.start, encoding);
        let length = encoded_len(&src[span.start..span.end], encoding);
        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 {
            character - prev_char
        } else {
            character
        };
        tokens.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: legend_index(span.kind),
            token_modifiers_bitset: 0,
        });
        prev_line = line;
        prev_char = character;
    }
    tokens
}

fn diagnostic(
    line_index: &LineIndex,
    src: &str,
    err: &CheckError,
    encoding: PositionEncoding,
) -> Diagnostic {
    let (sl, sc) = line_index.position(src, err.start, encoding);
    let (el, ec) = line_index.position(src, err.end, encoding);
    Diagnostic {
        range: Range {
            start: Position::new(sl, sc),
            end: Position::new(el, ec),
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("mensura".to_string()),
        message: err.message.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_types(src: &str) -> Vec<u32> {
        analyze(src, PositionEncoding::Utf8)
            .tokens
            .iter()
            .map(|t| t.token_type)
            .collect()
    }

    #[test]
    fn unit_declaration_has_keyword_type_and_property_tokens() {
        let types = token_types("unit U { id: string }");
        assert!(types.contains(&legend_index(HighlightKind::Keyword)));
        assert!(types.contains(&legend_index(HighlightKind::Type)));
        assert!(types.contains(&legend_index(HighlightKind::Property)));
    }

    #[test]
    fn diagnostics_report_an_unknown_unit() {
        let analysis = analyze("store S { unit { Missing } }", PositionEncoding::Utf8);
        assert!(!analysis.diagnostics.is_empty());
    }

    #[test]
    fn tokens_delta_encode_across_lines() {
        // The comment is on line 0; the `unit` keyword opens line 1, so its
        // token carries a positive `delta_line` and resets `delta_start`.
        let tokens = analyze("// note\nunit U { id: string }", PositionEncoding::Utf8).tokens;
        // First token is the comment at (0, 0).
        assert_eq!(tokens[0].delta_line, 0);
        assert_eq!(tokens[0].delta_start, 0);
        // The next token jumps to the following line.
        assert_eq!(tokens[1].delta_line, 1);
    }

    #[test]
    fn length_counts_code_units_in_the_negotiated_encoding() {
        // The enum variant `"é"` is one UTF-16 unit but two UTF-8 bytes,
        // counting the surrounding quotes: 3 UTF-16 units vs 4 UTF-8 bytes.
        let src = r#"enum E { "é" }"#;
        let utf16 = analyze(src, PositionEncoding::Utf16).tokens;
        let utf8 = analyze(src, PositionEncoding::Utf8).tokens;
        let variant16 = utf16
            .iter()
            .find(|t| t.token_type == legend_index(HighlightKind::EnumMember))
            .unwrap();
        let variant8 = utf8
            .iter()
            .find(|t| t.token_type == legend_index(HighlightKind::EnumMember))
            .unwrap();
        assert_eq!(variant16.length, 3);
        assert_eq!(variant8.length, 4);
    }
}
