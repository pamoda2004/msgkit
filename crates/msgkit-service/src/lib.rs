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
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use msgkit_core::device::{self, DeviceBatch, DeviceOptions};
use msgkit_core::error::codes;
use msgkit_core::facebook::{self, FacebookExportOptions};
use msgkit_core::limits::Limits;
use msgkit_core::store::{EventStore, MemoryStore, RetentionStore};
use msgkit_core::whatsapp_export::{self, WhatsAppExportOptions};
use msgkit_core::{webhook, Event, ImportReport, SCHEMA_VERSION};
use msgkit_store_sqlite::{SqliteStore, DEFAULT_RETENTION};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

/// Webhook bodies from Meta are small; anything bigger is rejected (or less, when `max_input_bytes` is lower).
const WEBHOOK_LIMIT_BYTES: usize = 2 * 1024 * 1024;

/// A byte limit as an axum body limit: no limit is the largest value.
fn body_limit(bytes: Option<u64>) -> usize {
    bytes.map_or(usize::MAX, |b| usize::try_from(b).unwrap_or(usize::MAX))
}

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

/// The service's settings.
///
/// `#[non_exhaustive]`, so a setting can be added without breaking callers: start from [`Default`] (or
/// [`Config::from_env`]) and chain the `with_*` setters.
#[derive(Clone, Default, Debug)]
#[non_exhaustive]
pub struct Config {
    pub whatsapp: Option<WebhookConfig>,
    pub messenger: Option<WebhookConfig>,
    /// When set, `/v1/*` requires `Authorization: Bearer <token>`.
    pub api_token: Option<String>,
    /// SQLite file for webhook events. When unset, events are kept in memory
    /// and lost on restart.
    pub db_path: Option<PathBuf>,
    /// Input limits for every import and webhook. Set by the operator only: a request cannot raise them. The
    /// default is [`Limits::default`] (32 MiB bodies); the body of an import is capped at `max_input_bytes`.
    pub limits: Limits,
    /// Host names (without scheme or path; a port is ignored) accepted in the `Host` header of `/v1/*` requests
    /// when no `api_token` is set, in addition to `localhost`, `*.localhost` and loopback addresses. See
    /// [`Config::try_from_env`] (`MSGKIT_ALLOWED_HOSTS`). Ignored when an `api_token` is set.
    pub allowed_hosts: Vec<String>,
    /// How long the id of an acknowledged (or erased or expired) event is remembered, so a redelivered webhook is
    /// still recognised. `None` is [`DEFAULT_RETENTION`] (30 days). The id is pseudonymous personal data; meta
    /// retries failed deliveries for several days, so going below that risks duplicates. `MSGKIT_ID_RETENTION_DAYS`.
    pub id_retention: Option<Duration>,
    /// When set, events that arrived this long ago and were never acknowledged are erased (their ids stay for
    /// [`id_retention`](Self::id_retention)). `None`, the default, keeps them until a consumer acknowledges them.
    /// `MSGKIT_PENDING_MAX_AGE_DAYS`.
    pub pending_max_age: Option<Duration>,
}

/// The longest retention an environment variable may ask for, in days (a century): it keeps the arithmetic safe.
const MAX_RETENTION_DAYS: u64 = 36_500;

/// Reads one `MSGKIT_*_DAYS` value: a whole number of days from 1 to 36500. Unset or empty is `None`.
fn days_var(get: &dyn Fn(&str) -> Option<String>, name: &str) -> Result<Option<Duration>, String> {
    let Some(raw) = get(name).filter(|v| !v.is_empty()) else { return Ok(None) };
    match raw.trim().parse::<u64>() {
        Ok(n) if (1..=MAX_RETENTION_DAYS).contains(&n) => Ok(Some(Duration::from_secs(n * 86_400))),
        _ => Err(format!("{name} must be a whole number of days from 1 to {MAX_RETENTION_DAYS}")),
    }
}

/// Reads one `MSGKIT_MAX_*` value: a whole number of at least 1. Unset or empty keeps the default (`None`).
/// `0` is refused, so a limit is never switched off by accident; `MSGKIT_NO_LIMITS=true` does that explicitly.
fn limit_var(get: &dyn Fn(&str) -> Option<String>, name: &str) -> Result<Option<u64>, String> {
    let Some(raw) = get(name).filter(|v| !v.is_empty()) else { return Ok(None) };
    match raw.trim().parse::<u64>() {
        Ok(n) if n >= 1 => Ok(Some(n)),
        _ => Err(format!("{name} must be a whole number of at least 1")),
    }
}

/// `MSGKIT_ALLOWED_HOSTS`: comma-separated host names, each a plain name or address (no scheme, path, wildcard or
/// port), stored in lower case.
fn allowed_hosts_from(get: &dyn Fn(&str) -> Option<String>) -> Result<Vec<String>, String> {
    let Some(raw) = get("MSGKIT_ALLOWED_HOSTS") else { return Ok(Vec::new()) };
    let mut hosts = Vec::new();
    for entry in raw.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let name = entry.to_ascii_lowercase();
        let plain = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'));
        if !plain {
            return Err(
                "MSGKIT_ALLOWED_HOSTS entries must be host names or addresses: no scheme, path or wildcard"
                    .into(),
            );
        }
        hosts.push(host_without_port(&name).to_owned());
    }
    Ok(hosts)
}

/// `MSGKIT_NO_LIMITS`: `true` or `1` switches every limit off, `false`, `0` or empty leaves them on.
fn no_limits(get: &dyn Fn(&str) -> Option<String>) -> Result<bool, String> {
    match get("MSGKIT_NO_LIMITS").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        None | Some("" | "false" | "0") => Ok(false),
        Some("true" | "1") => Ok(true),
        Some(_) => Err("MSGKIT_NO_LIMITS must be `true` or `false`".to_owned()),
    }
}

/// The limits an operator configured through `MSGKIT_MAX_*` and `MSGKIT_NO_LIMITS`, read with `get` (the
/// environment, or a stub in tests). Unset values keep the library defaults.
fn limits_from(get: &dyn Fn(&str) -> Option<String>) -> Result<Limits, String> {
    let mut limits = Limits::default();
    limits.max_input_bytes = limit_var(get, "MSGKIT_MAX_INPUT_BYTES")?;
    limits.max_line_bytes = limit_var(get, "MSGKIT_MAX_LINE_BYTES")?;
    limits.max_messages = limit_var(get, "MSGKIT_MAX_MESSAGES")?;
    limits.max_json_depth = limit_var(get, "MSGKIT_MAX_JSON_DEPTH")?;
    if no_limits(get)? {
        limits = Limits::unlimited();
    }
    Ok(limits)
}

impl Config {
    /// Like [`Config::from_env`], but reports a bad `MSGKIT_MAX_*` value instead of ignoring it. The error
    /// names the variable, never its value.
    ///
    /// # Errors
    /// A message when `MSGKIT_MAX_INPUT_BYTES`, `MSGKIT_MAX_LINE_BYTES`, `MSGKIT_MAX_MESSAGES` or
    /// `MSGKIT_MAX_JSON_DEPTH` is not a whole number of at least 1, `MSGKIT_NO_LIMITS` is not `true` or `false`, an
    /// entry of `MSGKIT_ALLOWED_HOSTS` is not a plain host name or address, or `MSGKIT_ID_RETENTION_DAYS` or
    /// `MSGKIT_PENDING_MAX_AGE_DAYS` is not a whole number of days from 1 to 36500.
    pub fn try_from_env() -> Result<Self, String> {
        let limits = limits_from(&|name| std::env::var(name).ok())?;
        let allowed_hosts = allowed_hosts_from(&|name| std::env::var(name).ok())?;
        let get = |name: &str| std::env::var(name).ok();
        let mut config = Self::from_env().with_limits(limits).with_allowed_hosts(allowed_hosts);
        config.id_retention = days_var(&get, "MSGKIT_ID_RETENTION_DAYS")?;
        config.pending_max_age = days_var(&get, "MSGKIT_PENDING_MAX_AGE_DAYS")?;
        Ok(config)
    }

    /// Sets [`id_retention`](Self::id_retention).
    #[must_use]
    pub fn with_id_retention(mut self, retention: Duration) -> Self {
        self.id_retention = Some(retention);
        self
    }

    /// Sets [`pending_max_age`](Self::pending_max_age).
    #[must_use]
    pub fn with_pending_max_age(mut self, max_age: Duration) -> Self {
        self.pending_max_age = Some(max_age);
        self
    }

    /// Sets [`whatsapp`](Self::whatsapp).
    #[must_use]
    pub fn with_whatsapp(mut self, webhook: WebhookConfig) -> Self {
        self.whatsapp = Some(webhook);
        self
    }

    /// Sets [`messenger`](Self::messenger).
    #[must_use]
    pub fn with_messenger(mut self, webhook: WebhookConfig) -> Self {
        self.messenger = Some(webhook);
        self
    }

    /// Sets [`api_token`](Self::api_token).
    #[must_use]
    pub fn with_api_token(mut self, token: impl Into<String>) -> Self {
        self.api_token = Some(token.into());
        self
    }

    /// Sets [`db_path`](Self::db_path).
    #[must_use]
    pub fn with_db_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.db_path = Some(path.into());
        self
    }

    /// Sets [`allowed_hosts`](Self::allowed_hosts).
    #[must_use]
    pub fn with_allowed_hosts<I, S>(mut self, hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.allowed_hosts = hosts
            .into_iter()
            .map(|h| host_without_port(&h.as_ref().to_ascii_lowercase()).to_owned())
            .collect();
        self
    }

    /// Sets [`limits`](Self::limits).
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Reads configuration from `MSGKIT_*` environment variables. Empty values
    /// count as unset. The `MSGKIT_MAX_*` limits are read by [`Config::try_from_env`]; here they keep their
    /// defaults.
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
            limits: Limits::default(),
            allowed_hosts: Vec::new(),
            id_retention: None,
            pending_max_age: None,
        }
    }
}

/// Refuses a configuration that would expose an unauthenticated API: a bind address that is not a loopback address
/// while no `api_token` is set. Anyone who can reach such a port could read and drain the event queue and use the
/// import endpoints. The binary calls this before it listens; there is deliberately no override.
///
/// # Errors
/// A message that names `MSGKIT_API_TOKEN` and the bind address, and shows how to make a token.
pub fn check_bind(bind: SocketAddr, config: &Config) -> Result<(), String> {
    if bind.ip().is_loopback() || config.api_token.is_some() {
        return Ok(());
    }
    Err(format!(
        "refusing to listen on {bind}: it is not a loopback address and MSGKIT_API_TOKEN is not set, so anyone who \
         can reach the port could read and drain the event queue. Set MSGKIT_API_TOKEN to a long random value (for \
         example `openssl rand -hex 32`), or bind to 127.0.0.1"
    ))
}

/// The host part of a `Host` header value or an allowed-hosts entry: no port, no trailing dot, lower case.
fn host_without_port(value: &str) -> &str {
    let value = value.trim();
    let host = if let Some(rest) = value.strip_prefix('[') {
        // `[::1]:8080` or `[::1]`: keep the brackets, drop the port.
        rest.find(']').map_or(value, |end| &value[..end + 2])
    } else if value.matches(':').count() == 1 {
        value.split(':').next().unwrap_or(value)
    } else {
        value
    };
    host.trim_end_matches('.')
}

/// Whether `host` (already lower case, no port) is the machine itself: `localhost`, `*.localhost` or a loopback
/// address.
fn is_loopback_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// DNS rebinding defence for a service without an API token: a web page can make a visitor's browser call
/// `http://localhost:8080` under an attacker's host name, and every `/v1/*` route would answer it. Browsers always
/// send the host name they used, so only the machine's own names (and `Config::allowed_hosts`) are accepted. A
/// request without a `Host` header cannot come from a browser and is let through. With an API token set the check
/// is skipped: a page cannot know the token.
async fn require_allowed_host(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.config.api_token.is_some() {
        return next.run(req).await;
    }
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or_else(|| req.uri().authority().map(|a| a.as_str().to_owned()));
    if let Some(host) = host {
        let host = host_without_port(&host.to_ascii_lowercase()).to_owned();
        if !is_loopback_host(&host) && !state.config.allowed_hosts.contains(&host) {
            // The rejected host is not repeated: it is attacker-chosen text.
            return ApiError::new(
                StatusCode::FORBIDDEN,
                "the Host header is not allowed (set MSGKIT_ALLOWED_HOSTS, or MSGKIT_API_TOKEN)",
                codes::HOST_NOT_ALLOWED,
            )
            .into_response();
        }
    }
    next.run(req).await
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
        config.limits.validate()?;
        let store: Box<dyn EventStore + Send> = match &config.db_path {
            Some(path) => {
                let mut db = SqliteStore::open(path)?;
                maintain(&mut db, &config)?;
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
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "storage error", codes::STORAGE_ERROR)
        })
    }
}

/// Forgets acknowledged event ids older than the default retention window (30 days). Safe to
/// call while the service is running (SQLite serializes writers). Kept for callers that only want that; the
/// service itself uses [`maintain_database`], which also honours the configured retention.
pub fn prune_database(path: &Path) -> msgkit_core::Result<usize> {
    SqliteStore::open(path)?.prune_acknowledged(DEFAULT_RETENTION)
}

/// What one maintenance pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Maintenance {
    /// Held events erased because they were older than `pending_max_age`.
    pub expired: usize,
    /// Event ids forgotten because they were older than `id_retention`.
    pub forgotten: usize,
}

/// One pass of retention: erases events nobody collected within `pending_max_age` (when set), forgets ids
/// older than `id_retention` (30 days unless set), and truncates the write-ahead log. Nothing is logged here and
/// no content is returned.
fn maintain(db: &mut SqliteStore, config: &Config) -> msgkit_core::Result<Maintenance> {
    let expired = match config.pending_max_age {
        Some(max_age) => db.expire_pending(max_age)?,
        None => 0,
    };
    let forgotten = RetentionStore::prune_acknowledged(db, config.id_retention.unwrap_or(DEFAULT_RETENTION))?;
    // Acknowledging does not empty the write-ahead log (it would make every acknowledge expensive), so every pass
    // does, whether or not it deleted anything: acknowledged content leaves the log within one interval. A busy
    // reader can make this fail to truncate once; the next pass tries again.
    db.checkpoint()?;
    Ok(Maintenance { expired, forgotten })
}

/// Runs one retention pass on the database at `path` (see [`Config::id_retention`] and
/// [`Config::pending_max_age`]). `main` calls it every six hours, so a long-running service does not grow without
/// bound and does not keep personal data longer than configured.
pub fn maintain_database(path: &Path, config: &Config) -> msgkit_core::Result<Maintenance> {
    maintain(&mut SqliteStore::open(path)?, config)
}

/// An error response: `{"error": "<text>", "code": "<stable code>"}` (see docs/ERRORS.md). The text is for
/// people and may be reworded; the code is the contract.
struct ApiError(StatusCode, String, &'static str);

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>, code: &'static str) -> Self {
        ApiError(status, message.into(), code)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1, "code": self.2 }))).into_response()
    }
}

/// A request msgkit's core refused: the core's own error code (`invalid_input` or `parse_error`).
fn core_error(e: &msgkit_core::Error) -> ApiError {
    let status = if e.code() == codes::LIMIT_EXCEEDED {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        StatusCode::BAD_REQUEST
    };
    ApiError::new(status, e.to_string(), e.code())
}

/// A request whose body or query could not be used (`invalid_input`), or was too large (`limit_exceeded`).
fn rejected(status: StatusCode, message: impl Into<String>) -> ApiError {
    let code =
        if status == StatusCode::PAYLOAD_TOO_LARGE { codes::LIMIT_EXCEEDED } else { codes::INVALID_INPUT };
    ApiError::new(status, message, code)
}

/// axum answers a body over the limit with a plain-text 413 when the handler reads `String` or `Bytes`. Give those
/// the same JSON error (and code) as every other error response.
async fn json_payload_too_large(res: Response) -> Response {
    let is_json = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if res.status() == StatusCode::PAYLOAD_TOO_LARGE && !is_json {
        return rejected(StatusCode::PAYLOAD_TOO_LARGE, "the request body is too large").into_response();
    }
    res
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
            Err(rejection) => Err(rejected(rejection.status(), json_rejection_text(&rejection))),
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
                    return Err(rejected(rejection.status(), "the query string was rejected"));
                };
                Err(rejected(StatusCode::BAD_REQUEST, "the query string is invalid"))
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
            return ApiError::new(
                StatusCode::UNAUTHORIZED,
                "missing or invalid bearer token",
                codes::UNAUTHORIZED,
            )
            .into_response();
        }
    }
    next.run(req).await
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "worker failed", codes::INTERNAL_ERROR))
}

async fn version() -> Json<Value> {
    Json(json!({
        "service_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION,
    }))
}

async fn import_whatsapp(
    State(state): State<AppState>,
    AppQuery(mut opts): AppQuery<WhatsAppExportOptions>,
    body: String,
) -> Result<Json<ImportReport>, ApiError> {
    opts.limits = state.config.limits.clone();
    let report = blocking(move || whatsapp_export::try_parse(&body, &opts)).await?;
    report.map(Json).map_err(|e| core_error(&e))
}

async fn import_facebook(
    State(state): State<AppState>,
    AppQuery(mut opts): AppQuery<FacebookExportOptions>,
    AppJson(thread): AppJson<Value>,
) -> Result<Json<ImportReport>, ApiError> {
    opts.limits = state.config.limits.clone();
    let report = blocking(move || facebook::parse_value(thread, &opts)).await?;
    report.map(Json).map_err(|e| core_error(&e))
}

async fn import_device(
    State(state): State<AppState>,
    AppQuery(mut opts): AppQuery<DeviceOptions>,
    AppJson(batch): AppJson<DeviceBatch>,
) -> Result<Json<ImportReport>, ApiError> {
    opts.limits = state.config.limits.clone();
    let report = blocking(move || device::try_normalize(&batch, &opts)).await?;
    report.map(Json).map_err(|e| core_error(&e))
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
        return ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook not configured",
            codes::NOT_CONFIGURED,
        )
        .into_response();
    };
    let ok = !cfg.verify_token.is_empty()
        && q.mode.as_deref() == Some("subscribe")
        && q.token.as_deref().is_some_and(|t| secrets_match(t, &cfg.verify_token));
    match (ok, q.challenge) {
        (true, Some(challenge)) => challenge.into_response(),
        _ => ApiError::new(StatusCode::FORBIDDEN, "verification failed", codes::VERIFICATION_FAILED)
            .into_response(),
    }
}

async fn receive_webhook(
    state: &AppState,
    cfg: Option<&WebhookConfig>,
    headers: &HeaderMap,
    body: &Bytes,
    normalize: fn(&Value, &Limits) -> msgkit_core::Result<ImportReport>,
) -> Response {
    // Refuse to process anything unauthenticated, even when unconfigured.
    let Some(cfg) = cfg else {
        return ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "webhook not configured",
            codes::NOT_CONFIGURED,
        )
        .into_response();
    };
    let signature = headers.get("x-hub-signature-256").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !webhook::verify_signature(cfg.app_secret.as_bytes(), body, signature) {
        return ApiError::new(StatusCode::UNAUTHORIZED, "invalid signature", codes::SIGNATURE_INVALID)
            .into_response();
    }

    let payload: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            return rejected(
                StatusCode::BAD_REQUEST,
                format!("invalid JSON: {}", msgkit_core::error::describe_json_error(&e, true)),
            )
            .into_response()
        }
    };
    let report = match normalize(&payload, &state.config.limits) {
        Ok(r) => r,
        Err(e) => return core_error(&e).into_response(),
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
    receive_webhook(&s, s.config.whatsapp.as_ref(), &headers, &body, webhook::normalize_whatsapp_with_limits)
        .await
}

async fn messenger_verify(State(s): State<AppState>, AppQuery(q): AppQuery<VerifyQuery>) -> Response {
    verify_subscription(s.config.messenger.as_ref(), q)
}

async fn messenger_receive(State(s): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    receive_webhook(
        &s,
        s.config.messenger.as_ref(),
        &headers,
        &body,
        webhook::normalize_messenger_with_limits,
    )
    .await
}

pub fn router(state: AppState) -> Router {
    let import_limit = body_limit(state.config.limits.input_bytes_limit());
    let webhook_limit = import_limit.min(WEBHOOK_LIMIT_BYTES);
    let v1 = Router::new()
        .route("/v1/import/whatsapp-export", post(import_whatsapp).layer(DefaultBodyLimit::max(import_limit)))
        .route("/v1/import/facebook-export", post(import_facebook).layer(DefaultBodyLimit::max(import_limit)))
        .route("/v1/import/device", post(import_device).layer(DefaultBodyLimit::max(import_limit)))
        .route("/v1/events", get(events))
        .route("/v1/events/ack", post(acknowledge))
        .route("/v1/version", get(version))
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .layer(middleware::from_fn_with_state(state.clone(), require_allowed_host));

    let webhooks = Router::new()
        .route("/webhooks/whatsapp", get(whatsapp_verify).post(whatsapp_receive))
        .route("/webhooks/messenger", get(messenger_verify).post(messenger_receive))
        .layer(DefaultBodyLimit::max(webhook_limit));

    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(v1)
        .merge(webhooks)
        .layer(middleware::map_response(json_payload_too_large))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn unset_variables_keep_the_library_defaults() {
        assert_eq!(limits_from(&env(&[])).unwrap(), Limits::default());
        assert_eq!(limits_from(&env(&[("MSGKIT_MAX_MESSAGES", "")])).unwrap(), Limits::default());
        assert_eq!(limits_from(&env(&[("MSGKIT_NO_LIMITS", "")])).unwrap(), Limits::default());
    }

    #[test]
    fn numbers_are_read_per_limit() {
        let l = limits_from(&env(&[
            ("MSGKIT_MAX_INPUT_BYTES", "1048576"),
            ("MSGKIT_MAX_LINE_BYTES", " 4096 "),
            ("MSGKIT_MAX_MESSAGES", "7"),
            ("MSGKIT_MAX_JSON_DEPTH", "9"),
        ]))
        .unwrap();
        assert_eq!(
            (l.max_input_bytes, l.max_line_bytes, l.max_messages, l.max_json_depth),
            (Some(1_048_576), Some(4096), Some(7), Some(9))
        );
        assert!(!l.unlimited);
    }

    #[test]
    fn zero_off_and_bad_values_are_refused_by_name_without_echoing_them() {
        for (name, value) in [
            ("MSGKIT_MAX_MESSAGES", "0"),
            ("MSGKIT_MAX_MESSAGES", "off"),
            ("MSGKIT_MAX_MESSAGES", "lots-of-secret-text"),
            ("MSGKIT_MAX_INPUT_BYTES", "-1"),
            ("MSGKIT_MAX_JSON_DEPTH", "1.5"),
            ("MSGKIT_MAX_LINE_BYTES", "0"),
        ] {
            let e = limits_from(&env(&[(name, value)])).unwrap_err();
            assert_eq!(e, format!("{name} must be a whole number of at least 1"));
        }
    }

    #[test]
    fn no_limits_is_the_explicit_opt_out() {
        for value in ["true", "TRUE", "1", " true "] {
            assert!(limits_from(&env(&[("MSGKIT_NO_LIMITS", value)])).unwrap().unlimited, "{value}");
        }
        for value in ["false", "0"] {
            assert!(!limits_from(&env(&[("MSGKIT_NO_LIMITS", value)])).unwrap().unlimited, "{value}");
        }
        assert_eq!(
            limits_from(&env(&[("MSGKIT_NO_LIMITS", "maybe-secret")])).unwrap_err(),
            "MSGKIT_NO_LIMITS must be `true` or `false`"
        );
    }

    #[test]
    fn no_limit_is_the_largest_body() {
        assert_eq!(body_limit(None), usize::MAX);
        assert_eq!(body_limit(Some(1024)), 1024);
    }

    #[test]
    fn a_zero_limit_in_a_config_is_refused_at_startup() {
        let zero = Config::default().with_limits(Limits::default().with_max_messages(0));
        assert!(AppState::new(zero).is_err());
    }

    #[test]
    fn the_host_part_drops_the_port_and_a_trailing_dot() {
        for (value, host) in [
            ("localhost", "localhost"),
            ("localhost:8080", "localhost"),
            ("example.com.", "example.com"),
            ("example.com.:443", "example.com"),
            ("127.0.0.1:80", "127.0.0.1"),
            ("[::1]", "[::1]"),
            ("[::1]:8080", "[::1]"),
            ("::1", "::1"),
            (" example.com ", "example.com"),
        ] {
            assert_eq!(host_without_port(value), host, "{value}");
        }
    }

    #[test]
    fn loopback_hosts_are_the_machines_own_names_only() {
        for host in ["localhost", "a.localhost", "127.0.0.1", "127.9.9.9", "[::1]", "::1"] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in [
            "notlocalhost",
            "localhost.evil.example",
            "127.0.0.1.evil.example",
            "10.0.0.1",
            "0.0.0.0",
            "[::]",
            "example.com",
            "",
        ] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn allowed_hosts_are_read_trimmed_lower_cased_and_without_ports() {
        let hosts = allowed_hosts_from(&env(&[(
            "MSGKIT_ALLOWED_HOSTS",
            " Msgkit.Example.com , 10.0.0.7:8080,,[::1] ",
        )]))
        .unwrap();
        assert_eq!(hosts, ["msgkit.example.com", "10.0.0.7", "[::1]"]);
        assert_eq!(allowed_hosts_from(&env(&[])).unwrap(), Vec::<String>::new());
        assert_eq!(allowed_hosts_from(&env(&[("MSGKIT_ALLOWED_HOSTS", "")])).unwrap(), Vec::<String>::new());
        let with_at_sign = format!("user{}a.example", '@');
        for bad in
            ["https://a.example", "*.a.example", "a.example/x", "a b", "a.example?x=1", with_at_sign.as_str()]
        {
            let e = allowed_hosts_from(&env(&[("MSGKIT_ALLOWED_HOSTS", bad)])).unwrap_err();
            assert!(!e.contains(bad), "{e}");
        }
    }

    #[test]
    fn retention_days_are_read_and_bounded() {
        let day = Duration::from_secs(86_400);
        assert_eq!(days_var(&env(&[]), "MSGKIT_ID_RETENTION_DAYS").unwrap(), None);
        assert_eq!(
            days_var(&env(&[("MSGKIT_ID_RETENTION_DAYS", "")]), "MSGKIT_ID_RETENTION_DAYS").unwrap(),
            None
        );
        assert_eq!(
            days_var(&env(&[("MSGKIT_ID_RETENTION_DAYS", " 45 ")]), "MSGKIT_ID_RETENTION_DAYS").unwrap(),
            Some(45 * day)
        );
        for bad in ["0", "-1", "1.5", "soon", "36501", "99999999999999999999"] {
            let e = days_var(&env(&[("MSGKIT_PENDING_MAX_AGE_DAYS", bad)]), "MSGKIT_PENDING_MAX_AGE_DAYS")
                .unwrap_err();
            assert!(e.starts_with("MSGKIT_PENDING_MAX_AGE_DAYS must be"), "{e}");
            assert!(!e.contains(bad) || bad.len() < 2, "the error must not repeat the value: {e}");
        }
    }

    #[test]
    fn a_maintenance_pass_removes_acknowledged_text_from_the_write_ahead_log() {
        use msgkit_core::store::conformance::event_from;
        const MARKER: &str = "zq9x7k2-log-marker-text";
        let bytes = |path: &Path| {
            let mut name = path.as_os_str().to_owned();
            name.push("-wal");
            std::fs::read(Path::new(&name)).unwrap_or_default()
        };
        let has = |hay: &[u8]| hay.windows(MARKER.len()).any(|w| w == MARKER.as_bytes());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        // The service's own connection stays open, as it does while running.
        let mut db = SqliteStore::open(&path).unwrap();
        let mut event = event_from("a", 1, "+94700000000", "+94711111111");
        if let Event::Message(m) = &mut event {
            m.body = Some(MARKER.to_owned());
            m.raw = None;
        }
        db.insert(event).unwrap();
        db.acknowledge(&["a".to_owned()]).unwrap();
        assert!(has(&bytes(&path)), "acknowledging alone leaves the text in the log; the test must see that");

        // Nothing is old enough to delete, yet the pass still empties the log.
        let done = maintain_database(&path, &Config::default()).unwrap();
        assert_eq!(done, Maintenance { expired: 0, forgotten: 0 });
        assert!(!has(&bytes(&path)), "the maintenance pass left acknowledged text in the write-ahead log");
        drop(db);
    }

    #[test]
    fn a_maintenance_pass_expires_held_events_and_forgets_old_ids() {
        use msgkit_core::store::conformance::sample_event;
        let now = i64::try_from(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        {
            let mut db = SqliteStore::open(&path).unwrap();
            db.insert_all(vec![sample_event("held", 1), sample_event("done", 2)]).unwrap();
            db.acknowledge(&["done".to_owned()]).unwrap();
        }
        // Backdate the acknowledgement of `done` by 40 days and the arrival of `held` by 10 days.
        let raw = rusqlite::Connection::open(&path).unwrap();
        let ago = |days: i64| now - days * 86_400;
        raw.execute("UPDATE events SET acked_at = ?1 WHERE id = 'done'", [ago(40)]).unwrap();
        raw.execute("UPDATE events SET received_at = ?1 WHERE id = 'held'", [ago(10)]).unwrap();
        drop(raw);

        // Defaults: nothing is older than 30 days except the id backdated 40 days; held events are not expired.
        let config = Config::default();
        assert_eq!(maintain_database(&path, &config).unwrap(), Maintenance { expired: 0, forgotten: 1 });
        // With a pending limit of 7 days the held event (backdated 10 days) goes too.
        let config = Config::default().with_pending_max_age(Duration::from_secs(7 * 86_400));
        assert_eq!(maintain_database(&path, &config).unwrap(), Maintenance { expired: 1, forgotten: 0 });
        let mut db = SqliteStore::open(&path).unwrap();
        assert!(!db.insert(sample_event("held", 1)).unwrap(), "the expired event's id is kept");
        assert!(db.insert(sample_event("done", 2)).unwrap(), "the forgotten id is accepted again");
    }
}
