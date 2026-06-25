//! `mdbook-mensura`: an mdBook preprocessor that highlights Mensura code
//! blocks from the compiler's own classification and fails the build when a
//! block that claims to compile does not.  See
//! `docs/toolkit/03-book-highlighting.md`.

mod render;

use std::io::{Read, Write};
use std::process::ExitCode;

use serde_json::Value;

use crate::render::rewrite_markdown;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    // `mdbook-mensura supports <renderer>`: exit 0 for renderers we handle.
    if args.get(1).map(String::as_str) == Some("supports") {
        let renderer = args.get(2).map(String::as_str).unwrap_or("");
        return if renderer == "html" {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }

    match run() {
        Ok(book_json) => {
            let mut stdout = std::io::stdout();
            if stdout.write_all(book_json.as_bytes()).is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("mdbook-mensura: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Read `[context, book]` from stdin, rewrite the book, and return it as JSON.
fn run() -> Result<String, String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("reading stdin: {e}"))?;
    let mut payload: Value =
        serde_json::from_str(&input).map_err(|e| format!("parsing preprocessor input: {e}"))?;

    let book = payload
        .get_mut(1)
        .ok_or("preprocessor input is not [context, book]")?;
    let items = book
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .ok_or("book has no items array")?;

    let mut errors: Vec<String> = Vec::new();
    for item in items.iter_mut() {
        process_item(item, &mut errors);
    }
    if !errors.is_empty() {
        return Err(format!(
            "{} Mensura example(s) failed to check:\n\n{}",
            errors.len(),
            errors.join("\n\n")
        ));
    }

    serde_json::to_string(book).map_err(|e| format!("serializing book: {e}"))
}

/// Rewrite a chapter's content and recurse into its sub-chapters.  Non-chapter
/// items (separators, part titles) carry no content and are left alone.
fn process_item(item: &mut Value, errors: &mut Vec<String>) {
    let Some(chapter) = item.get_mut("Chapter") else {
        return;
    };

    let name = chapter
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("<unnamed>")
        .to_string();

    if let Some(content) = chapter.get("content").and_then(Value::as_str) {
        match rewrite_markdown(content) {
            Ok(rewritten) => {
                chapter["content"] = Value::String(rewritten);
            }
            Err(block_errors) => {
                for err in block_errors {
                    errors.push(format!("in chapter \"{name}\":\n{err}"));
                }
            }
        }
    }

    if let Some(sub_items) = chapter.get_mut("sub_items").and_then(Value::as_array_mut) {
        for sub in sub_items.iter_mut() {
            process_item(sub, errors);
        }
    }
}
