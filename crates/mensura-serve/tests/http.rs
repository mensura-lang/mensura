//! The HTTP projection end to end (ADR 0045 decisions 5 to 9): positions
//! travel with every append and read, a served view equals the batch
//! materialization over the same writes, every refusal carries its status
//! by cause, scopes gate the operations, the floor is a positioned write
//! under its own scope, and the schema guard survives a restart.

use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use mensura_runtime::{
    Delta, SqliteBackend, StorageBackend, decode_jsonl, encode_rows, materialize_views,
};
use mensura_serve::{AppState, Config, StartError, auth, frontend, materialize_once, router};
use tower::ServiceExt;

const PROGRAM: &str = r#"
import bag
import si

unit Machine { machine_id: string }

store machines {
  unit { Machine }
  attr { label: string }
}

unit Reading { machine_id: string  taken_at: instant }

registry readings {
  unit { Reading }
  attr { temperature: temperature[real] }
  lateness { taken_at: 10.0 * si.minute }
}

view peaks {
  readings |> demote taken_at
           |> assume { complete }
           |> map_bags |k, b| (.peak = bag.max b.temperature)
}
"#;

const WRITER: &str = "writer-token";
const READER: &str = "reader-token";
const OPERATOR: &str = "operator-token";

fn config(max_body_bytes: usize) -> Config {
    let text = format!(
        r#"
program   = "x.mensura"
database  = {{ env = "MENSURA_SERVE_TEST_UNUSED" }}
namespace = "test"
listen    = "127.0.0.1:0"

[limits]
max_body_bytes    = {max_body_bytes}
min_position_wait = "150ms"

[[principal]]
id           = "spiffe://local/device/sensor/1"
token_sha256 = "{}"
scopes       = ["write:readings", "write:machines"]

[[principal]]
id           = "spiffe://local/service/dashboard"
token_sha256 = "{}"
scopes       = ["read:all"]

[[principal]]
id           = "spiffe://local/service/operator"
token_sha256 = "{}"
scopes       = ["floor:readings"]
"#,
        auth::digest(WRITER),
        auth::digest(READER),
        auth::digest(OPERATOR),
    );
    Config::parse(&text, Path::new("/tmp")).expect("valid configuration")
}

fn program(src: &str) -> mensura_types::ResolvedProgram {
    frontend::resolve_source(Path::new("x.mensura"), src).expect("resolves")
}

fn state_over(backend: SqliteBackend, max_body_bytes: usize) -> AppState {
    AppState::open(config(max_body_bytes), program(PROGRAM), backend).expect("starts")
}

fn state() -> AppState {
    state_over(SqliteBackend::open_in_memory().unwrap(), 8_388_608)
}

fn request(method: &str, uri: &str, token: Option<&str>, body: &str) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn call(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, serde_json::Value) {
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{e}: {:?}", bytes))
    };
    (status, headers, body)
}

fn position_of(headers: &HeaderMap) -> u64 {
    headers["Mensura-Position"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap()
}

const BATCH: &str = r#"{"machine_id":"m-01","taken_at":"2026-01-01T10:00:00Z","temperature":300.0}
{"machine_id":"m-01","taken_at":"2026-01-01T10:05:00Z","temperature":312.5}
{"machine_id":"m-02","taken_at":"2026-01-01T10:00:00+02:00","temperature":355.0}
"#;

#[tokio::test]
async fn health_is_open_and_reports_positions() {
    let app = router(state());
    let (status, _, body) = call(&app, request("GET", "/_mensura/health", None, "")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ready"], true);
    assert_eq!(body["position"], 0);
    assert_eq!(body["roles"].as_array().unwrap().len(), 3);
    assert!(body["views"]["peaks"].is_null(), "{body}");
    assert_eq!(body["fingerprint"].as_str().unwrap().len(), 64);
}

#[tokio::test]
async fn appends_take_positions_and_a_served_view_equals_the_batch_materialization() {
    let st = state();
    let app = router(st.clone());

    let (status, headers, body) =
        call(&app, request("POST", "/readings", Some(WRITER), BATCH)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(position_of(&headers), 1);
    assert_eq!(
        body,
        serde_json::json!({"position": 1, "appended": 3, "skipped": 0})
    );

    // Before the materializer runs, the view is at position zero and empty,
    // and a read that insists on position 1 waits, then answers 503.
    let (status, headers, body) = call(&app, request("GET", "/peaks", Some(READER), "")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(position_of(&headers), 0);
    assert_eq!(body, serde_json::json!([]));
    let (status, _, body) = call(
        &app,
        request("GET", "/peaks?min_position=1", Some(READER), ""),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");

    let materialized = materialize_once(&st).await.unwrap();
    assert_eq!(materialized.position, 1);

    let (status, headers, served) = call(
        &app,
        request("GET", "/peaks?min_position=1", Some(READER), ""),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{served}");
    assert_eq!(position_of(&headers), 1);
    assert_eq!(
        served,
        serde_json::json!([
            {"machine_id": "m-01", "peak": 312.5},
            {"machine_id": "m-02", "peak": 355.0},
        ])
    );

    // Prefix consistency: the same writes through the batch path, on an
    // independent database, materialize to byte-identical records.
    let prog = program(PROGRAM);
    let mut oracle = SqliteBackend::open_in_memory().unwrap();
    for s in &prog.schemas {
        oracle.ensure_store(s).unwrap();
    }
    let readings = prog.schemas.iter().find(|s| s.store == "readings").unwrap();
    let rows = decode_jsonl(readings, BATCH).unwrap();
    oracle
        .apply(&readings.shape(), &Delta::appending(rows))
        .unwrap();
    assert_eq!(materialize_views(&mut oracle, &prog).unwrap().position, 1);
    let peaks = prog
        .views
        .iter()
        .find(|v| v.name == "peaks")
        .unwrap()
        .shape();
    let expected = encode_rows(&peaks, &oracle.scan(&peaks).unwrap());
    assert_eq!(served.to_string(), expected.to_string());

    // A registry reads back as the decoder's inverse, at the log position.
    let (status, headers, body) = call(&app, request("GET", "/readings", Some(READER), "")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(position_of(&headers), 1);
    assert_eq!(body[2]["taken_at"], "2026-01-01T08:00:00.000Z");
    assert_eq!(body[1]["temperature"], 312.5);

    // Health now reports the view's position.
    let (_, _, body) = call(&app, request("GET", "/_mensura/health", None, "")).await;
    assert_eq!(body["views"]["peaks"], 1);
    assert_eq!(body["position"], 1);
}

#[tokio::test]
async fn credentials_and_scopes_gate_every_operation_but_health() {
    let app = router(state());
    let (status, _, _) = call(&app, request("POST", "/readings", None, BATCH)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = call(&app, request("POST", "/readings", Some("nope"), BATCH)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, body) = call(&app, request("POST", "/readings", Some(READER), BATCH)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body["error"].as_str().unwrap().contains("write:readings"),
        "{body}"
    );
    let (status, _, _) = call(&app, request("GET", "/readings", Some(WRITER), "")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, body) = call(&app, request("GET", "/nothing", Some(READER), "")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("peaks"), "{body}");
    let (status, _, _) = call(&app, request("POST", "/peaks", Some(WRITER), BATCH)).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn refusals_carry_their_status_by_cause() {
    let app = router(state());
    let (status, _, body) = call(
        &app,
        request("POST", "/readings", Some(WRITER), "{not json"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, _, body) = call(
        &app,
        request(
            "POST",
            "/readings",
            Some(WRITER),
            r#"{"machine_id":"m-01","taken_at":"2026-01-01T10:00:00Z","temperature":"hot"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"].as_str().unwrap().starts_with("record 1"),
        "{body}"
    );

    let (status, _, _) = call(&app, request("POST", "/readings", Some(WRITER), BATCH)).await;
    assert_eq!(status, StatusCode::OK);
    // The same batch again is a no-op that still takes a position
    // (decision 11).
    let (status, headers, body) =
        call(&app, request("POST", "/readings", Some(WRITER), BATCH)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(position_of(&headers), 2);
    assert_eq!(body["appended"], 0);
    assert_eq!(body["skipped"], 3);
    // Different content at a stored key is a conflict.
    let (status, _, body) = call(
        &app,
        request(
            "POST",
            "/readings",
            Some(WRITER),
            r#"{"machine_id":"m-01","taken_at":"2026-01-01T10:00:00Z","temperature":1.0}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("already holds a different row"),
        "{body}"
    );
    // A late row names its grain.
    let (status, _, body) = call(
        &app,
        request(
            "POST",
            "/readings",
            Some(WRITER),
            r#"{"machine_id":"m-01","taken_at":"2026-01-01T09:00:00Z","temperature":1.0}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let message = body["error"].as_str().unwrap();
    assert!(
        message.contains("too late") && message.contains("`machine_id` = m-01"),
        "{body}"
    );
    // The rejected batches took no position.
    let (_, _, body) = call(&app, request("GET", "/_mensura/health", None, "")).await;
    assert_eq!(body["position"], 2);
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_with_the_same_error_shape() {
    let app = router(state_over(SqliteBackend::open_in_memory().unwrap(), 64));
    let (status, _, body) = call(&app, request("POST", "/readings", Some(WRITER), BATCH)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("64 bytes"),
        "{body}"
    );
}

#[tokio::test]
async fn the_floor_is_a_positioned_write_under_its_own_scope() {
    let app = router(state());
    let point = r#"{"point": "2026-01-01T12:00:00Z"}"#;
    let (status, _, _) = call(
        &app,
        request(
            "POST",
            "/_mensura/floors/readings/taken_at",
            Some(WRITER),
            point,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, headers, body) = call(
        &app,
        request(
            "POST",
            "/_mensura/floors/readings/taken_at",
            Some(OPERATOR),
            point,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(position_of(&headers), 1);
    // Lowering is refused.
    let lower = r#"{"point": "2026-01-01T11:00:00Z"}"#;
    let (status, _, body) = call(
        &app,
        request(
            "POST",
            "/_mensura/floors/readings/taken_at",
            Some(OPERATOR),
            lower,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    // A column without a contract, and a point of the wrong kind.
    let (status, _, _) = call(
        &app,
        request(
            "POST",
            "/_mensura/floors/readings/temperature",
            Some(OPERATOR),
            point,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _, _) = call(
        &app,
        request(
            "POST",
            "/_mensura/floors/readings/taken_at",
            Some(OPERATOR),
            r#"{"point": 5}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    // The floor now closes the intake: a reading older than it is late.
    let (status, _, body) = call(
        &app,
        request(
            "POST",
            "/readings",
            Some(WRITER),
            r#"{"machine_id":"m-09","taken_at":"2026-01-01T11:00:00Z","temperature":1.0}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

#[test]
fn the_guard_refuses_a_changed_store_across_restarts() {
    let path = std::env::temp_dir().join(format!(
        "mensura-serve-guard-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let open = || SqliteBackend::open(&path).unwrap();
    let first = AppState::open(config(1), program(PROGRAM), open()).expect("creates");
    let fingerprint = first.fingerprint.clone();
    drop(first);

    let same = AppState::open(config(1), program(PROGRAM), open()).expect("verifies");
    assert_eq!(same.fingerprint, fingerprint);
    drop(same);

    let view_edited = PROGRAM.replace("bag.max", "bag.min");
    AppState::open(config(1), program(&view_edited), open()).expect("a view change is accepted");

    let store_edited = PROGRAM.replace("label: string", "label: string?");
    let Err(err) = AppState::open(config(1), program(&store_edited), open()) else {
        panic!("a changed store must be refused");
    };
    let StartError::SchemaMismatch { tables, stored, .. } = err else {
        panic!("expected a schema mismatch, got {err}");
    };
    assert_eq!(tables, vec!["machines".to_string()]);
    assert_eq!(stored, fingerprint);
    let _ = std::fs::remove_file(&path);
}

/// The readiness probe of `mensura health` (ADR 0046 decision 6) against
/// a listening instance: a real socket, not the router in process.
#[tokio::test(flavor = "multi_thread")]
async fn the_health_probe_reads_a_listening_instance() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(axum::serve(listener, router(state())).into_future());
    let ready = tokio::task::spawn_blocking(move || {
        mensura_serve::probe::health(&addr.to_string(), std::time::Duration::from_secs(5))
    })
    .await
    .unwrap()
    .expect("the instance answers ready");
    assert_eq!(ready.namespace, "test");
    assert_eq!(ready.position, 0);
    server.abort();
}

#[test]
fn a_new_token_matches_its_digest_format() {
    let token = auth::new_token().unwrap();
    assert_eq!(token.len(), 64);
    assert_ne!(token, auth::new_token().unwrap());
    assert_eq!(auth::digest(&token).len(), 64);
}

/// A redeploy that edits a view, with no write in between, serves the new
/// definition rather than the old rows (ADR 0045 decision 9: a changed
/// view is rebuilt; ADR 0046 decision 7), even when the edit changes the
/// view's columns.
#[tokio::test]
async fn a_redefined_view_is_rebuilt_without_a_new_write() {
    let path = std::env::temp_dir().join(format!(
        "mensura-serve-redefine-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let open = || SqliteBackend::open(&path).unwrap();

    let first = AppState::open(config(8_388_608), program(PROGRAM), open()).unwrap();
    let app = router(first.clone());
    let (status, _, body) = call(&app, request("POST", "/readings", Some(WRITER), BATCH)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    materialize_once(&first).await.unwrap();
    let (_, _, before) = call(&app, request("GET", "/peaks", Some(READER), "")).await;
    assert!(before[0]["peak"].is_number(), "{before}");
    drop(app);
    drop(first);

    let edited = PROGRAM.replace(
        "(.peak = bag.max b.temperature)",
        "(.low = bag.min b.temperature)",
    );
    let second = AppState::open(config(8_388_608), program(&edited), open()).unwrap();
    assert_eq!(materialize_once(&second).await.unwrap().position, 1);
    let app = router(second);
    let (status, headers, after) = call(&app, request("GET", "/peaks", Some(READER), "")).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(position_of(&headers), 1);
    assert!(after[0]["low"].is_number(), "{after}");
    assert!(after[0].get("peak").is_none(), "{after}");
    let _ = std::fs::remove_file(&path);
}
