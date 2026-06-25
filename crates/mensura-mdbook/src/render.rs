//! Rewriting Mensura code fences into pre-highlighted HTML.
//!
//! A fenced block whose info string starts with `mensura` is replaced by a
//! `<pre>` of class-tagged `<span>`s, colored from `mensura-highlight`.  The
//! mapping from token class to CSS class lives here, not in the shared crate:
//! HTML class names are this renderer's vocabulary.  See
//! `docs/toolkit/03-book-highlighting.md`.

use mensura_highlight::{HighlightKind, highlight};

/// How strictly a `mensura` block is checked, taken from its info string.
struct Modifiers {
    /// `mensura,ignore`: highlight but do not gate on check errors (snippets
    /// from a later milestone, or deliberately rejected programs).
    ignore: bool,
}

/// Rewrite every Mensura code fence in `content`, leaving all other Markdown
/// untouched.  Returns the rewritten Markdown, or the list of check failures
/// found in non-ignored blocks (which must fail the build).
pub fn rewrite_markdown(content: &str) -> Result<String, Vec<String>> {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(modifiers) = mensura_fence(lines[i]) else {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        };
        // Collect the block body up to the next closing fence.
        let mut j = i + 1;
        while j < lines.len() && !is_closing_fence(lines[j]) {
            j += 1;
        }
        if j >= lines.len() {
            // Unterminated fence: not our concern, pass the opener through and
            // let the Markdown renderer deal with it.
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }
        let code = lines[i + 1..j].join("\n");
        match render_block(&code, &modifiers) {
            Ok(html) => out.push(html),
            Err(messages) => errors.push(messages),
        }
        i = j + 1; // skip the closing fence
    }
    if errors.is_empty() {
        Ok(out.join("\n"))
    } else {
        Err(errors)
    }
}

/// If `line` opens a Mensura fence (` ```mensura ` with optional modifiers),
/// return its modifiers.
fn mensura_fence(line: &str) -> Option<Modifiers> {
    let info = line.trim_start().strip_prefix("```")?.trim();
    let mut parts = info.split(',').map(str::trim);
    if parts.next()? != "mensura" {
        return None;
    }
    Some(Modifiers {
        ignore: parts.any(|p| p == "ignore"),
    })
}

/// A closing fence is a line of three or more backticks and nothing else.
fn is_closing_fence(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && t.bytes().all(|b| b == b'`')
}

/// Render one block's source to highlighted HTML, or report its check errors.
fn render_block(code: &str, modifiers: &Modifiers) -> Result<String, String> {
    let result = highlight(code);
    if !modifiers.ignore && !result.errors.is_empty() {
        let mut report = String::from("mensura example failed to check:\n");
        for err in &result.errors {
            report.push_str(&format!("  - {}\n", err.message));
        }
        report.push_str("--- source ---\n");
        report.push_str(code);
        return Err(report);
    }

    // `nohighlight` and `data-highlighted` both tell mdBook's bundled
    // highlight.js to leave this block alone; the spans are already colored.
    let mut html = String::from(
        "<pre class=\"mensura\"><code class=\"mn-code nohighlight\" data-highlighted=\"yes\">",
    );
    let mut cursor = 0;
    for span in &result.spans {
        if span.start > cursor {
            push_escaped(&mut html, &code[cursor..span.start]);
        }
        html.push_str("<span class=\"");
        html.push_str(css_class(span.kind));
        html.push_str("\">");
        push_escaped(&mut html, &code[span.start..span.end]);
        html.push_str("</span>");
        cursor = span.end;
    }
    if cursor < code.len() {
        push_escaped(&mut html, &code[cursor..]);
    }
    html.push_str("</code></pre>");
    Ok(html)
}

/// The CSS class the book stylesheet keys on for each token class.
fn css_class(kind: HighlightKind) -> &'static str {
    match kind {
        HighlightKind::Keyword => "mn-keyword",
        HighlightKind::Type => "mn-type",
        HighlightKind::Property => "mn-property",
        HighlightKind::Parameter => "mn-parameter",
        HighlightKind::String => "mn-string",
        HighlightKind::Number => "mn-number",
        HighlightKind::Operator => "mn-operator",
        HighlightKind::EnumMember => "mn-enum-member",
        HighlightKind::Comment => "mn-comment",
    }
}

/// Append `text` to `html`, escaping the three characters that matter inside
/// element content.
fn push_escaped(html: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            _ => html.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_mensura_content_is_untouched() {
        let md = "# Title\n\n```rust\nfn main() {}\n```\n\nText.";
        assert_eq!(rewrite_markdown(md).unwrap(), md);
    }

    #[test]
    fn a_clean_block_becomes_highlighted_html() {
        let md = "Intro\n\n```mensura\nunit U { id: string }\n```\n\nOutro";
        let out = rewrite_markdown(md).unwrap();
        // Surrounding prose is preserved.
        assert!(out.starts_with("Intro\n\n<pre class=\"mensura\">"));
        assert!(out.ends_with("</pre>\n\nOutro"));
        // The keyword and a type are wrapped in their classes.
        assert!(out.contains("<span class=\"mn-keyword\">unit</span>"));
        assert!(out.contains("<span class=\"mn-type\">U</span>"));
        // highlight.js is told to skip the block.
        assert!(out.contains("nohighlight"));
        // No fences survive.
        assert!(!out.contains("```"));
    }

    #[test]
    fn angle_brackets_in_source_are_escaped() {
        // A resolve error is fine here; we only check escaping, so ignore it.
        let md = "```mensura,ignore\nunit U { id: string } // a < b & c > d\n```";
        let out = rewrite_markdown(md).unwrap();
        assert!(out.contains("a &lt; b &amp; c &gt; d"));
        assert!(!out.contains("a < b & c > d"));
    }

    #[test]
    fn a_broken_block_fails_the_build() {
        let md = "```mensura\nstore S { unit { Missing } }\n```";
        let err = rewrite_markdown(md).unwrap_err();
        assert_eq!(err.len(), 1);
        assert!(err[0].contains("failed to check"));
    }

    #[test]
    fn ignore_modifier_suppresses_the_check_gate() {
        let md = "```mensura,ignore\nstore S { unit { Missing } }\n```";
        // Renders without error despite the unresolved unit.
        let out = rewrite_markdown(md).unwrap();
        assert!(out.contains("<pre class=\"mensura\">"));
    }
}
