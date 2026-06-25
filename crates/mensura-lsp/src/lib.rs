//! The `mensura lsp` language server.
//!
//! A synchronous LSP server over stdio (`lsp-server` + `lsp-types`) that
//! exposes the basic feature set: full-document sync, semantic-token
//! highlighting, and push diagnostics.  It adds no analysis of its own; it
//! drives the compiler pipeline and translates spans onto the wire.  See
//! `docs/toolkit/02-lsp.md`.

// `lsp_types::Uri` has interior mutability (a cache of the parsed URI) that
// does not affect its `Hash`/`Eq`, so it is sound as a `HashMap` key.
#![allow(clippy::mutable_key_type)]

mod analysis;
mod line_index;

use std::collections::HashMap;
use std::error::Error;

use lsp_server::{
    Connection, ExtractError, Message, Notification, Request, Response, ResponseError,
};
use lsp_types::{
    InitializeParams, InitializeResult, PositionEncodingKind, PublishDiagnosticsParams,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams,
    SemanticTokensResult, SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
        PublishDiagnostics,
    },
    request::{Request as _, SemanticTokensFullRequest},
};

use crate::analysis::{analyze, token_legend};
use crate::line_index::PositionEncoding;

type LspResult = Result<(), Box<dyn Error + Sync + Send>>;

/// Run the language server to completion over stdio.  Returns when the client
/// asks the server to exit.
pub fn run() -> LspResult {
    let (connection, io_threads) = Connection::stdio();

    let (id, params) = connection.initialize_start()?;
    let params: InitializeParams = serde_json::from_value(params)?;
    let encoding = negotiate_encoding(&params);

    let result = InitializeResult {
        capabilities: server_capabilities(encoding),
        server_info: Some(ServerInfo {
            name: "mensura-lsp".to_string(),
            version: option_env!("CARGO_PKG_VERSION").map(str::to_string),
        }),
    };
    connection.initialize_finish(id, serde_json::to_value(result)?)?;

    // `main_loop` takes the connection by value so it (and its sender) is
    // dropped before the join; otherwise the writer thread never sees the
    // channel disconnect and `join` blocks forever.
    main_loop(connection, encoding)?;
    io_threads.join()?;
    Ok(())
}

/// Prefer UTF-8 when the client offers it, else fall back to the UTF-16
/// default.
fn negotiate_encoding(params: &InitializeParams) -> PositionEncoding {
    let offered = params
        .capabilities
        .general
        .as_ref()
        .and_then(|g| g.position_encodings.as_ref());
    match offered {
        Some(kinds) if kinds.contains(&PositionEncodingKind::UTF8) => PositionEncoding::Utf8,
        _ => PositionEncoding::Utf16,
    }
}

fn server_capabilities(encoding: PositionEncoding) -> ServerCapabilities {
    let position_encoding = Some(match encoding {
        PositionEncoding::Utf8 => PositionEncodingKind::UTF8,
        PositionEncoding::Utf16 => PositionEncodingKind::UTF16,
    });
    ServerCapabilities {
        position_encoding,
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: token_legend(),
                    token_modifiers: vec![],
                },
                full: Some(SemanticTokensFullOptions::Bool(true)),
                range: Some(false),
                work_done_progress_options: Default::default(),
            },
        )),
        ..Default::default()
    }
}

fn main_loop(connection: Connection, encoding: PositionEncoding) -> LspResult {
    let mut documents: HashMap<Uri, String> = HashMap::new();

    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    return Ok(());
                }
                handle_request(&connection, &documents, encoding, request)?;
            }
            Message::Notification(notification) => {
                handle_notification(&connection, &mut documents, encoding, notification)?;
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_request(
    connection: &Connection,
    documents: &HashMap<Uri, String>,
    encoding: PositionEncoding,
    request: Request,
) -> LspResult {
    match request.method.as_str() {
        SemanticTokensFullRequest::METHOD => {
            let (id, params) = cast_request::<SemanticTokensFullRequest>(request)?;
            let params: SemanticTokensParams = params;
            let result = documents.get(&params.text_document.uri).map(|src| {
                SemanticTokensResult::Tokens(lsp_types::SemanticTokens {
                    result_id: None,
                    data: analyze(src, encoding).tokens,
                })
            });
            connection.sender.send(Message::Response(Response {
                id,
                result: Some(serde_json::to_value(result)?),
                error: None,
            }))?;
        }
        _ => {
            // Nothing else is advertised; answer with a protocol error rather
            // than leaving the request unanswered.
            connection.sender.send(Message::Response(Response {
                id: request.id,
                result: None,
                error: Some(ResponseError {
                    code: -32601, // MethodNotFound
                    message: format!("unhandled request: {}", request.method),
                    data: None,
                }),
            }))?;
        }
    }
    Ok(())
}

fn handle_notification(
    connection: &Connection,
    documents: &mut HashMap<Uri, String>,
    encoding: PositionEncoding,
    notification: Notification,
) -> LspResult {
    match notification.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params = cast_notification::<DidOpenTextDocument>(notification)?;
            let uri = params.text_document.uri;
            documents.insert(uri.clone(), params.text_document.text);
            publish(
                connection,
                documents,
                &uri,
                encoding,
                Some(params.text_document.version),
            )?;
        }
        DidChangeTextDocument::METHOD => {
            let params = cast_notification::<DidChangeTextDocument>(notification)?;
            let uri = params.text_document.uri;
            // Full sync: the last change carries the whole document.
            if let Some(change) = params.content_changes.into_iter().next_back() {
                documents.insert(uri.clone(), change.text);
            }
            publish(
                connection,
                documents,
                &uri,
                encoding,
                Some(params.text_document.version),
            )?;
        }
        DidCloseTextDocument::METHOD => {
            let params = cast_notification::<DidCloseTextDocument>(notification)?;
            documents.remove(&params.text_document.uri);
            // Clear diagnostics for the closed document.
            publish_diagnostics(connection, params.text_document.uri, vec![], None)?;
        }
        _ => {}
    }
    Ok(())
}

/// Re-analyze a document and push its diagnostics.
fn publish(
    connection: &Connection,
    documents: &HashMap<Uri, String>,
    uri: &Uri,
    encoding: PositionEncoding,
    version: Option<i32>,
) -> LspResult {
    if let Some(src) = documents.get(uri) {
        let diagnostics = analyze(src, encoding).diagnostics;
        publish_diagnostics(connection, uri.clone(), diagnostics, version)?;
    }
    Ok(())
}

fn publish_diagnostics(
    connection: &Connection,
    uri: Uri,
    diagnostics: Vec<lsp_types::Diagnostic>,
    version: Option<i32>,
) -> LspResult {
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics,
        version,
    };
    connection.sender.send(Message::Notification(Notification {
        method: PublishDiagnostics::METHOD.to_string(),
        params: serde_json::to_value(params)?,
    }))?;
    Ok(())
}

fn cast_request<R>(
    request: Request,
) -> Result<(lsp_server::RequestId, R::Params), ExtractError<Request>>
where
    R: lsp_types::request::Request,
{
    request.extract(R::METHOD)
}

fn cast_notification<N>(notification: Notification) -> Result<N::Params, ExtractError<Notification>>
where
    N: lsp_types::notification::Notification,
{
    notification.extract(N::METHOD)
}
