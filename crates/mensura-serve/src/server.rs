//! The HTTP projection (ADR 0045 decision 7) and the materializer
//! (decision 5).
//!
//! | Request | Operation |
//! |---|---|
//! | `POST /<name>` with a JSON Lines body | `append` |
//! | `GET /<name>`, optionally `?min_position=N` | `read` |
//! | `POST /_mensura/floors/<name>/<column>` with `{"point": ...}` | `advance_floor` |
//! | `GET /_mensura/health` | `health` |
//!
//! Every error body is a JSON object carrying the same message the CLI
//! prints for the same failure, and the position travels in the
//! `Mensura-Position` header on `append` and `read`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use mensura_runtime::{
    Delta, IngestError, Materialized, RunError, StorageBackend, StorageError, decode_records,
    encode_rows, materialize_views, read_jsonl,
};
use mensura_types::ColumnType;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::watch;

use crate::auth::{AuthError, scope};
use crate::config::Role;
use crate::state::{AppState, FailedView, StartError, Target};

/// The response header carrying a position (decision 6).
pub const POSITION_HEADER: &str = "Mensura-Position";

/// How often the materializer checks for a new position when nothing has
/// woken it: polling is the portable mechanism, and the in-process wake
/// after each write is the optimization of it (decision 5).
const MATERIALIZER_POLL: Duration = Duration::from_secs(2);

/// How often a read waiting for `min_position` re-checks (decision 6).
const WAIT_POLL: Duration = Duration::from_millis(25);

/// The router over `state`: the four operations, with the body limit
/// applied inside the handlers so that a 413 carries the same error
/// shape as every other refusal.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/_mensura/health", get(health))
        .route("/_mensura/floors/{name}/{column}", post(advance_floor))
        .route("/{name}", get(read).post(append))
        .with_state(state)
}

/// Serve `state` until `SIGTERM` or `SIGINT`, running the materializer
/// beside the listener when this instance has that role.
pub async fn serve(state: AppState) -> Result<(), StartError> {
    let addr = state.config.listen_addr();
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(StartError::Io)?;
    let roles: Vec<&str> = state.config.roles.iter().map(|r| r.name()).collect();
    let position = {
        let backend = state.backend();
        backend.position()?
    };
    println!(
        "serving {} on {addr}: roles {}, position {position}, fingerprint {}",
        state.config.program.display(),
        roles.join(", "),
        state.fingerprint
    );

    let (stop_tx, stop_rx) = watch::channel(false);
    let materializer = state
        .runs(Role::Materializer)
        .then(|| tokio::spawn(materializer_loop(state.clone(), stop_rx)));

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(StartError::Io)?;

    // Stop the materializer after the listener, so a write accepted during
    // shutdown is still materialized if the loop gets to it; a
    // materialization in flight runs to completion either way.
    let _ = stop_tx.send(true);
    if let Some(task) = materializer {
        let _ = task.await;
    }
    Ok(())
}

async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}

// ---------------------------------------------------------------------------
// the materializer

/// Materialize every view once, at the latest position (decision 5).  A
/// view that fails keeps its table and its last position; the failure is
/// recorded for `health`, and the write that caused it has already
/// committed, so producers are never blocked by a view.
pub async fn materialize_once(state: &AppState) -> Result<Materialized, RunError> {
    let st = state.clone();
    // The position is read before the run, so a failure can be reported at
    // the position that caused it.  The error rides boxed: it is large,
    // and this path is the exceptional one.
    let result = tokio::task::spawn_blocking(move || {
        let mut backend = st.backend();
        let position = backend
            .position()
            .map_err(|e| Box::new((RunError::Storage(e), 0)))?;
        materialize_views(&mut *backend, &st.program).map_err(|e| Box::new((e, position)))
    })
    .await
    .expect("materialization does not panic");
    match result {
        Ok(materialized) => {
            state.health().failed_views.clear();
            Ok(materialized)
        }
        Err(failure) => {
            let (error, position) = *failure;
            if let RunError::View { name, .. } = &error {
                state.health().failed_views.insert(
                    name.clone(),
                    FailedView {
                        error: error.to_string(),
                        position,
                    },
                );
            }
            Err(error)
        }
    }
}

async fn materializer_loop(state: AppState, mut stop: watch::Receiver<bool>) {
    loop {
        if let Err(e) = materialize_once(&state).await {
            eprintln!("materializer: {e}");
        }
        tokio::select! {
            () = state.wake.notified() => {}
            () = tokio::time::sleep(MATERIALIZER_POLL) => {}
            _ = stop.changed() => {
                if *stop.borrow() {
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// errors

/// A refusal: the status code by cause (decision 7) and the message the
/// CLI would print.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<AuthError> for ApiError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::Unauthenticated => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "no credential, or an unknown one; present `Authorization: Bearer <token>`",
            ),
            AuthError::Forbidden { principal, scope } => ApiError::new(
                StatusCode::FORBIDDEN,
                format!("`{principal}` does not hold the scope `{scope}`"),
            ),
        }
    }
}

impl From<StorageError> for ApiError {
    fn from(e: StorageError) -> Self {
        let status = match &e {
            StorageError::DuplicateKey { .. }
            | StorageError::KeyConflict { .. }
            | StorageError::ForeignKey { .. }
            | StorageError::Lateness { .. }
            | StorageError::Refused(_) => StatusCode::CONFLICT,
            StorageError::Sqlite(_) | StorageError::Decode(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError::new(status, e.to_string())
    }
}

fn blocking_failed() -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "the storage task did not complete",
    )
}

fn position_header(position: u64) -> [(&'static str, HeaderValue); 1] {
    [(
        POSITION_HEADER,
        HeaderValue::from_str(&position.to_string()).expect("digits"),
    )]
}

// ---------------------------------------------------------------------------
// handlers

fn require_role(state: &AppState, role: Role) -> Result<(), ApiError> {
    if state.runs(role) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::MISDIRECTED_REQUEST,
            format!(
                "this instance does not run the `{}` role; it runs {}",
                role.name(),
                state
                    .config
                    .roles
                    .iter()
                    .map(|r| r.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ))
    }
}

fn resolve_target<'a>(state: &'a AppState, segment: &str) -> Result<Target<'a>, ApiError> {
    state.target(segment).ok_or_else(|| {
        let mut known: Vec<String> = state
            .program
            .schemas
            .iter()
            .map(|s| crate::auth::wire_segment(&s.store))
            .chain(
                state
                    .program
                    .views
                    .iter()
                    .map(|v| crate::auth::wire_segment(&v.name)),
            )
            .collect();
        known.sort_unstable();
        ApiError::new(
            StatusCode::NOT_FOUND,
            format!(
                "no store, registry, or view named `{segment}`; the program serves {}",
                known.join(", ")
            ),
        )
    })
}

async fn body_text(state: &AppState, request: Request) -> Result<String, ApiError> {
    let limit = state.config.limits.max_body_bytes;
    let bytes = to_bytes(request.into_body(), limit).await.map_err(|_| {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("the request body exceeds the configured limit of {limit} bytes"),
        )
    })?;
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "the request body is not UTF-8"))
}

/// `POST /<name>`: `append` (decision 3).  The batch is decoded and
/// authorized outside any transaction, then applied as one positioned
/// write.
async fn append(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, ApiError> {
    require_role(&state, Role::Intake)?;
    let principal = state.principals.authenticate(&headers)?.clone();
    let schema = match resolve_target(&state, &name)? {
        Target::Table(s) => s.clone(),
        Target::View(v) => {
            return Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                format!(
                    "`{}` is a view; a view is computed from its sources, not written to",
                    v.name
                ),
            ));
        }
    };
    principal.require(&scope("write", &schema.store))?;
    let text = body_text(&state, request).await?;
    let records = read_jsonl(&text).map_err(|e| malformed(&e))?;
    let rows = decode_records(&schema, &records).map_err(|e| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("record {}: {}", e.record, e.message),
        )
    })?;

    let st = state.clone();
    let shape = schema.shape();
    let applied = tokio::task::spawn_blocking(move || {
        st.backend()
            .apply_as(&shape, &Delta::appending(rows), &principal.id)
    })
    .await
    .map_err(|_| blocking_failed())??;
    state.wake.notify_one();
    Ok((
        StatusCode::OK,
        position_header(applied.position),
        Json(json!({
            "position": applied.position,
            "appended": applied.inserted,
            "skipped": applied.skipped,
        })),
    )
        .into_response())
}

fn malformed(e: &IngestError) -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        format!("line {}: {}", e.record, e.message),
    )
}

#[derive(Deserialize)]
struct ReadQuery {
    min_position: Option<u64>,
}

/// `GET /<name>`: `read` (decision 3), the whole table as a JSON array of
/// name-keyed records in key order, with the position it reflects.  With
/// `min_position`, waits up to the configured bound for the table to
/// reach it and answers 503 if it does not (decision 6).
async fn read(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<ReadQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    require_role(&state, Role::Read)?;
    let principal = state.principals.authenticate(&headers)?;
    let target = resolve_target(&state, &name)?;
    principal.require(&scope("read", target.name()))?;
    let is_view = matches!(target, Target::View(_));
    let table_name = target.name().to_string();
    let shape = target.shape();

    if let Some(min) = query.min_position {
        let bound = state
            .config
            .limits
            .min_position_wait()
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let started = Instant::now();
        loop {
            let reached = current_position(&state, &table_name, is_view).await?;
            if reached >= min {
                break;
            }
            if started.elapsed() >= bound {
                return Err(ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!(
                        "`{table_name}` is at position {reached}, below the requested \
                         {min}, and did not reach it within {}",
                        state.config.limits.min_position_wait
                    ),
                ));
            }
            tokio::time::sleep(WAIT_POLL).await;
        }
    }

    let st = state.clone();
    let name_for_position = table_name.clone();
    let (rows, position) = tokio::task::spawn_blocking(move || {
        let backend = st.backend();
        let position = if is_view {
            backend.view_position(&name_for_position)?
        } else {
            Some(backend.position()?)
        };
        // A view never materialized is the view at position zero, which is
        // empty: no write has happened, so no row exists.
        let rows = match position {
            Some(_) => backend.scan(&shape)?,
            None => Vec::new(),
        };
        Ok::<_, StorageError>((encode_rows(&shape, &rows), position.unwrap_or(0)))
    })
    .await
    .map_err(|_| blocking_failed())??;
    Ok((StatusCode::OK, position_header(position), Json(rows)).into_response())
}

async fn current_position(state: &AppState, name: &str, is_view: bool) -> Result<u64, ApiError> {
    let st = state.clone();
    let name = name.to_string();
    tokio::task::spawn_blocking(move || {
        let backend = st.backend();
        if is_view {
            Ok(backend.view_position(&name)?.unwrap_or(0))
        } else {
            backend.position()
        }
    })
    .await
    .map_err(|_| blocking_failed())?
    .map_err(ApiError::from)
}

#[derive(Deserialize)]
struct FloorBody {
    point: serde_json::Value,
}

/// `POST /_mensura/floors/<name>/<column>`: `advance_floor` (decision 3),
/// the operator's assertion that the world is closed through a point,
/// under the `floor:` scope, which a writer does not get by writing.
async fn advance_floor(
    State(state): State<AppState>,
    Path((name, column)): Path<(String, String)>,
    headers: HeaderMap,
    request: Request,
) -> Result<Response, ApiError> {
    require_role(&state, Role::Intake)?;
    let principal = state.principals.authenticate(&headers)?.clone();
    let schema = match resolve_target(&state, &name)? {
        Target::Table(s) => s.clone(),
        Target::View(v) => {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("`{}` is a view and declares no `lateness` contract", v.name),
            ));
        }
    };
    principal.require(&scope("floor", &schema.store))?;
    let Some(contract) = schema.lateness.iter().find(|l| l.column == column) else {
        let declared: Vec<&str> = schema.lateness.iter().map(|l| l.column.as_str()).collect();
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "`{}` declares no `lateness` on `{column}`{}; a floor is the closure \
                 half of a contract, so there is nothing for it to raise",
                schema.store,
                if declared.is_empty() {
                    String::new()
                } else {
                    format!(" (it contracts {})", declared.join(", "))
                }
            ),
        ));
    };
    let text = body_text(&state, request).await?;
    let body: FloorBody = serde_json::from_str(&text).map_err(|e| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("the body is not `{{\"point\": ...}}`: {e}"),
        )
    })?;
    let ty = schema
        .columns
        .iter()
        .find(|c| c.name == contract.column)
        .map(|c| &c.ty);
    let stored = match (ty, &body.point) {
        (Some(ColumnType::Instant), serde_json::Value::String(text)) => {
            mensura_runtime::temporal::normalize_instant(text)
                .map_err(|m| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, m))?
        }
        (Some(ColumnType::Instant), other) => {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "`{column}` is an instant; `point` must be an RFC 3339 string, not {other}"
                ),
            ));
        }
        (_, serde_json::Value::Number(n)) if n.is_i64() => n.to_string(),
        (_, other) => {
            return Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("`{column}` is a count; `point` must be an integer, not {other}"),
            ));
        }
    };

    let st = state.clone();
    let store = schema.store.clone();
    let col = column.clone();
    let floored = tokio::task::spawn_blocking(move || {
        st.backend()
            .advance_floor_as(&store, &col, &stored, &principal.id)
    })
    .await
    .map_err(|_| blocking_failed())??;
    state.wake.notify_one();
    Ok((
        StatusCode::OK,
        position_header(floored.position),
        Json(json!({ "position": floored.position })),
    )
        .into_response())
}

/// `GET /_mensura/health`: readiness, roles, positions, and the
/// fingerprint (decision 3), open to every caller.
async fn health(State(state): State<AppState>) -> Result<Response, ApiError> {
    let st = state.clone();
    let (position, views) = tokio::task::spawn_blocking(move || {
        let backend = st.backend();
        let position = backend.position()?;
        let mut views = BTreeMap::new();
        for v in &st.program.views {
            views.insert(v.name.clone(), backend.view_position(&v.name)?);
        }
        Ok::<_, StorageError>((position, views))
    })
    .await
    .map_err(|_| blocking_failed())??;
    let failed: BTreeMap<String, serde_json::Value> = state
        .health()
        .failed_views
        .iter()
        .map(|(name, f)| {
            (
                name.clone(),
                json!({ "error": f.error, "position": f.position }),
            )
        })
        .collect();
    let roles: Vec<&str> = state.config.roles.iter().map(|r| r.name()).collect();
    Ok(Json(json!({
        "ready": true,
        "roles": roles,
        "namespace": state.config.namespace,
        "fingerprint": state.fingerprint,
        "position": position,
        "views": views,
        "failed_views": failed,
    }))
    .into_response())
}

// Keep the `Body` import meaningful for readers of the handler
// signatures: `Request` is `Request<Body>`.
#[allow(dead_code)]
type RequestBody = Body;
