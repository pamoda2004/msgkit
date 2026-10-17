//! HTTP sidecar around `msgkit-core`.
//!
//! Lets any language use msgkit without a native binding: POST an export, get
//! normalized events back; point WhatsApp/Messenger webhooks at it and poll
//! `/v1/events`. The wire contract is `openapi.yaml` at the repository root.
//!
//! Webhook events are kept in memory, or in a SQLite database when
//! `MSGKIT_DB_PATH` is set so they survive restarts. Consumers should read
//! with `GET /v1/events` and confirm with `POST /v1/events/ack`.
//!
//! Security model:
//! * Binds to loopback by default.
//! * `/v1/*` can be protected with a bearer token (`MSGKIT_API_TOKEN`).
//! * Webhooks are never accepted unless an app secret is configured and the
//!   `X-Hub-Signature-256` header verifies.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use msgkit_core::device::{self, DeviceBatch, DeviceOptions};
use msgkit_core::facebook::{self, FacebookExportOptions};
use msgkit_core::store::{EventStore, MemoryStore};
use msgkit_core::whatsapp_export::{self, WhatsAppExportOptions};
use msgkit_core::{webhook, Event, ImportReport, SCHEMA_VERSION};
use msgkit_store_sqlite::{SqliteStore, DEFAULT_RETENTION};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

/// Largest accepted import body (WhatsApp exports of busy chats are big).
const IMPORT_LIMIT_BYTES: usize = 64 * 1024 * 1024;
/// Webhook bodies from Meta are small; anything bigger is rejected.
const WEBHOOK_LIMIT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct WebhookConfig {
    pub app_secret: String,
    pub verify_token: String,
}

impl fmt::Debug for WebhookConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookConfig")
            .field("app_secret", &"<redacted>")
            .field("verify_token", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Default, Debug)]
pub struct Config {
    pub whatsapp: Option<WebhookConfig>,
    pub messenger: Option<WebhookConfig>,
    /// When set, `/v1/*` requires `Authorization: Bearer <token>`.
    pub api_token: Option<String>,
    /// SQLite file for webhook events. When unset, events are kept in memory
    /// and lost on restart.
    pub db_path: Option<PathBuf>,
}

impl Config {
    /// Reads configuration from `MSGKIT_*` environment variables. Empty values
    /// count as unset.
    pub fn from_env() -> Self {
        fn var(name: &str) -> Option<String> {
            std::env::var(name).ok().filter(|v| !v.is_empty())
        }
        fn webhook(prefix: &str) -> Option<WebhookConfig> {
            Some(WebhookConfig {
                app_secret: var(&format!("MSGKIT_{prefix}_APP_SECRET"))?,
                verify_token: var(&format!("MSGKIT_{prefix}_VERIFY_TOKEN")).unwrap_or_default(),
            })
        }
        Config {
            whatsapp: webhook("WHATSAPP"),
            messenger: webhook("MESSENGER"),
            api_token: var("MSGKIT_API_TOKEN"),
            db_path: var("MSGKIT_DB_PATH").map(PathBuf::from),
        }
    }
}

type SharedStore = Arc<Mutex<Box<dyn EventStore + Send>>>;

#[derive(Clone)]
pub struct AppState {
    config: Arc<Config>,
    store: SharedStore,
}

impl fmt::Debug for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppState").field("config", &self.config).finish_non_exhaustive()
    }
}

impl AppState {
    /// Builds the state, opening the SQLite database if `config.db_path` is
    /// set (and forgetting acknowledged ids older than the retention window),
    /// otherwise using an in-memory store.
    pub fn new(config: Config) -> msgkit_core::Result<Self> {
        let store: Box<dyn EventStore + Send> = match &config.db_path {
            Some(path) => {
                let mut db = SqliteStore::open(path)?;
                db.prune_acknowledged(DEFAULT_RETENTION)?;
                Box::new(db)
            }
            None => Box::new(MemoryStore::new()),
        };
        Ok(Self::with_store(config, store))
    }

    /// Uses a caller-provided store (any [`EventStore`] implementation).
    #[must_use]
    pub fn with_store(config: Config, store: Box<dyn EventStore + Send>) -> Self {
        AppState { config: Arc::new(config), store: Arc::new(Mutex::new(store)) }
    }

    /// Runs `f` on the store on a blocking thread, so disk I/O never stalls
    /// the async runtime.
    async fn with_store_blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut dyn EventStore) -> msgkit_core::Result<T> + Send + 'static,
    ) -> Result<T, ApiError> {
        let store = Arc::clone(&self.store);
        let result = blocking(move || {
            let mut guard = store.lock().unwrap_or_else(PoisonError::into_inner);
            f(guard.as_mut())
        })
        .await?;
        result.map_err(|e| {
            // Details stay in the log; clients get a generic message.
            tracing::error!("storage failure: {e}");
            ApiError(StatusCode::INTERNAL_SERVER_ERROR, "storage error".into())
        })
    }
}

/// Forgets acknowledged event ids older than the retention window. Safe to
/// call while the service is running (SQLite serializes writers); `main` calls
/// it periodically so a long-running service does not grow without bound.
pub fn prune_database(path: &Path) -> msgkit_core::Result<usize> {
    SqliteStore::open(path)?.prune_acknowledged(DEFAULT_RETENTION)
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

// Takes the message by value because it is also used directly as `map_err(bad_request)`.
#[allow(clippy::needless_pass_by_value)]
fn bad_request(msg: impl ToString) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.to_string())
}

/// A JSON request body whose rejection never repeats the body.
///
/// axum's own rejection text quotes the offending value (`invalid type: string "..."`), and request
/// bodies hold message content, which must never reach responses or logs.
struct AppJson<T>(T);

impl<S, T> FromRequest<S> for AppJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(AppJson(value)),
            Err(rejection) => Err(ApiError(rejection.status(), json_rejection_text(&rejection).to_owned())),
        }
    }
}

fn json_rejection_text(rejection: &JsonRejection) -> &'static str {
    match rejection {
        JsonRejection::MissingJsonContentType(_) => "expected a request with content-type application/json",
        JsonRejection::JsonSyntaxError(_) => "the request body is not valid JSON",
        JsonRejection::JsonDataError(_) => "the JSON body does not match the expected shape",
        JsonRejection::BytesRejection(_) => "the request body could not be read (too large?)",
        _ => "the request body was rejected",
    }
}

/// A query string whose rejection never repeats it (it can contain names and numbers).
struct AppQuery<T>(T);

impl<S, T> FromRequestParts<S> for AppQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, state: &S) -> Result<Self, ApiError> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(AppQuery(value)),
            Err(rejection) => {
                let QueryRejection::FailedToDeserializeQueryString(_) = &rejection else {
                    return Err(ApiError(rejection.status(), "the query string was rejected".to_owned()));
                };
                Err(ApiError(StatusCode::BAD_REQUEST, "the query string is invalid".to_owned()))
            }
        }
    }
}

/// Compares secrets without leaking their length-matching prefix via timing.
fn secrets_match(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

async fn require_token(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(expected) = &state.config.api_token {
        let supplied = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if !supplied.is_some_and(|s| secrets_match(s, expected)) {
            return ApiError(StatusCode::UNAUTHORIZED, "missing or invalid bearer token".into())
                .into_response();
        }
    }
    next.run(req).await
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "worker failed".into()))
}

async fn version() -> Json<Value> {
    Json(json!({
        "service_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION,
    }))
}

async fn import_whatsapp(
    AppQuery(opts): AppQuery<WhatsAppExportOptions>,
    body: String,
) -> Result<Json<ImportReport>, ApiError> {
    Ok(Json(blocking(move || whatsapp_export::parse(&body, &opts)).await?))
}

async fn import_facebook(
    AppQuery(opts): AppQuery<FacebookExportOptions>,
    AppJson(thread): AppJson<Value>,
) -> Result<Json<ImportReport>, ApiError> {
    let report = blocking(move || facebook::parse_value(thread, &opts)).await?;
    report.map(Json).map_err(bad_request)
}

async fn import_device(
    AppQuery(opts): AppQuery<DeviceOptions>,
    AppJson(batch): AppJson<DeviceBatch>,
) -> Result<Json<ImportReport>, ApiError> {
    Ok(Json(blocking(move || device::normalize(&batch, &opts)).await?))
}

#[derive(Deserialize)]
struct EventsQuery {
    #[serde(default)]
    drain: bool,
}

async fn events(
    State(state): State<AppState>,
    AppQuery(q): AppQuery<EventsQuery>,
) -> Result<Json<Vec<Event>>, ApiError> {
    let events = state.with_store_blocking(move |s| if q.drain { s.drain() } else { s.list() }).await?;
    Ok(Json(events))
}

#[derive(Deserialize)]
struct AckRequest {
    ids: Vec<String>,
}

async fn acknowledge(
    State(state): State<AppState>,
    AppJson(req): AppJson<AckRequest>,
) -> Result<Json<Value>, ApiError> {
    let n = state.with_store_blocking(move |s| s.acknowledge(&req.ids)).await?;
    Ok(Json(json!({ "acknowledged": n })))
}

#[derive(Deserialize)]
struct VerifyQuery {
    #[serde(rename = "hub.mode")]
    mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    token: Option<String>,
    #[serde(rename = "hub.challenge")]
    challenge: Option<String>,
}

/// Meta's webhook subscription handshake: echo `hub.challenge` if the token matches.
fn verify_subscription(cfg: Option<&WebhookConfig>, q: VerifyQuery) -> Response {
    let Some(cfg) = cfg else {
        return ApiError(StatusCode::SERVICE_UNAVAILABLE, "webhook not configured".into()).into_response();
    };
    let ok = !cfg.verify_token.is_empty()
        && q.mode.as_deref() == Some("subscribe")
        && q.token.as_deref().is_some_and(|t| secrets_match(t, &cfg.verify_token));
    match (ok, q.challenge) {
        (true, Some(challenge)) => challenge.into_response(),
        _ => ApiError(StatusCode::FORBIDDEN, "verification failed".into()).into_response(),
    }
}

async fn receive_webhook(
    state: &AppState,
    cfg: Option<&WebhookConfig>,
    headers: &HeaderMap,
    body: &Bytes,
    normalize: fn(&Value) -> msgkit_core::Result<ImportReport>,
) -> Response {
    // Refuse to process anything unauthenticated, even when unconfigured.
    let Some(cfg) = cfg else {
        return ApiError(StatusCode::SERVICE_UNAVAILABLE, "webhook not configured".into()).into_response();
    };
    let signature = headers.get("x-hub-signature-256").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !webhook::verify_signature(cfg.app_secret.as_bytes(), body, signature) {
        return ApiError(StatusCode::UNAUTHORIZED, "invalid signature".into()).into_response();
    }

    let payload: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return bad_request(format!(
                "invalid JSON: {}",
                msgkit_core::error::describe_json_error(&e, true)
            ))
            .into_response()
        }
    };
    let report = match normalize(&payload) {
        Ok(r) => r,
        Err(e) => return bad_request(e).into_response(),
    };

    // The batch is stored atomically. If storage fails we answer 500 *without*
    // having stored anything, so Meta's retry delivers the whole payload again.
    let total = report.events.len();
    let events = report.events;
    let accepted = match state.with_store_blocking(move |s| s.insert_all(events)).await {
        Ok(n) => n,
        Err(e) => return e.into_response(),
    };
    Json(json!({
        "accepted": accepted,
        "duplicates": total - accepted,
        "warnings": report.warnings,
    }))
    .into_response()
}

async fn whatsapp_verify(State(s): State<AppState>, AppQuery(q): AppQuery<VerifyQuery>) -> Response {
    verify_subscription(s.config.whatsapp.as_ref(), q)
}

async fn whatsapp_receive(State(s): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    receive_webhook(&s, s.config.whatsapp.as_ref(), &headers, &body, webhook::normalize_whatsapp).await
}

async fn messenger_verify(State(s): State<AppState>, AppQuery(q): AppQuery<VerifyQuery>) -> Response {
    verify_subscription(s.config.messenger.as_ref(), q)
}

async fn messenger_receive(State(s): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    receive_webhook(&s, s.config.messenger.as_ref(), &headers, &body, webhook::normalize_messenger).await
}

pub fn router(state: AppState) -> Router {
    let v1 = Router::new()
        .route(
            "/v1/import/whatsapp-export",
            post(import_whatsapp).layer(DefaultBodyLimit::max(IMPORT_LIMIT_BYTES)),
        )
        .route(
            "/v1/import/facebook-export",
            post(import_facebook).layer(DefaultBodyLimit::max(IMPORT_LIMIT_BYTES)),
        )
        .route("/v1/import/device", post(import_device).layer(DefaultBodyLimit::max(IMPORT_LIMIT_BYTES)))
        .route("/v1/events", get(events))
        .route("/v1/events/ack", post(acknowledge))
        .route("/v1/version", get(version))
        .layer(middleware::from_fn_with_state(state.clone(), require_token));

    let webhooks = Router::new()
        .route("/webhooks/whatsapp", get(whatsapp_verify).post(whatsapp_receive))
        .route("/webhooks/messenger", get(messenger_verify).post(messenger_receive))
        .layer(DefaultBodyLimit::max(WEBHOOK_LIMIT_BYTES));

    Router::new().route("/healthz", get(|| async { "ok" })).merge(v1).merge(webhooks).with_state(state)
}
