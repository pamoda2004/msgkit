use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use msgkit_service::{router, AppState, Config, WebhookConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

const SECRET: &str = "app-secret";
const TOKEN: &str = "verify-me";

fn config(api_token: Option<&str>, db_path: Option<&std::path::Path>) -> Config {
    let wh = WebhookConfig { app_secret: SECRET.into(), verify_token: TOKEN.into() };
    Config {
        whatsapp: Some(wh.clone()),
        messenger: Some(wh),
        api_token: api_token.map(str::to_owned),
        db_path: db_path.map(std::path::Path::to_path_buf),
    }
}

fn app(api_token: Option<&str>) -> Router {
    router(AppState::new(config(api_token, None)).unwrap())
}

/// An app backed by the SQLite file at `path`. Calling it again with the same
/// path simulates restarting the service.
fn app_with_db(path: &std::path::Path) -> Router {
    router(AppState::new(config(None, Some(path))).unwrap())
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

fn sign(body: &str) -> String {
    msgkit_core::webhook::sign(SECRET.as_bytes(), body.as_bytes())
}

const WA_PAYLOAD: &str = include_str!("../../msgkit-core/tests/fixtures/whatsapp_webhook.json");

#[tokio::test]
async fn health_is_open() {
    let (s, b) = send(&app(Some("t")), Request::get("/healthz").body(Body::empty()).unwrap()).await;
    assert_eq!((s, b.as_str()), (StatusCode::OK, "ok"));
}

#[tokio::test]
async fn whatsapp_export_import() {
    let req = Request::post("/v1/import/whatsapp-export?self_name=Alice&utc_offset_minutes=-60")
        .body(Body::from("31/12/2023, 21:41 - Alice: hi\n"))
        .unwrap();
    let (s, b) = send(&app(None), req).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let v = json_of(&b);
    assert_eq!(v["events"][0]["direction"], "outbound");
    assert_eq!(v["events"][0]["timestamp"], "2023-12-31T22:41:00Z");
}

#[tokio::test]
async fn facebook_import_rejects_non_exports() {
    let req = Request::post("/v1/import/facebook-export")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("[1,2]"))
        .unwrap();
    let (s, _) = send(&app(None), req).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn bearer_token_protects_v1_only() {
    let app = app(Some("s3cret"));
    let (s, _) = send(&app, Request::get("/v1/version").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let req = Request::get("/v1/version")
        .header(header::AUTHORIZATION, "Bearer wrong")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::UNAUTHORIZED);

    let req = Request::get("/v1/version")
        .header(header::AUTHORIZATION, "Bearer s3cret")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::OK);
}

#[tokio::test]
async fn webhook_requires_a_valid_signature() {
    let app = app(None);
    let req = Request::post("/webhooks/whatsapp").body(Body::from(WA_PAYLOAD)).unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::UNAUTHORIZED);

    let req = Request::post("/webhooks/whatsapp")
        .header("x-hub-signature-256", sign("something else"))
        .body(Body::from(WA_PAYLOAD))
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_is_refused_when_unconfigured() {
    let app = router(AppState::new(Config::default()).unwrap());
    let req = Request::post("/webhooks/whatsapp")
        .header("x-hub-signature-256", sign(WA_PAYLOAD))
        .body(Body::from(WA_PAYLOAD))
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn signed_webhook_is_stored_deduplicated_and_drainable() {
    let app = app(None);
    let post = || {
        Request::post("/webhooks/whatsapp")
            .header("x-hub-signature-256", sign(WA_PAYLOAD))
            .body(Body::from(WA_PAYLOAD))
            .unwrap()
    };

    let (s, b) = send(&app, post()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(json_of(&b)["accepted"], 3);

    // Meta retries deliveries; replays must not create duplicates.
    let (_, b) = send(&app, post()).await;
    let v = json_of(&b);
    assert_eq!((v["accepted"].as_u64(), v["duplicates"].as_u64()), (Some(0), Some(3)));

    let (_, b) = send(&app, Request::get("/v1/events").body(Body::empty()).unwrap()).await;
    assert_eq!(json_of(&b).as_array().unwrap().len(), 3);

    let (_, b) = send(&app, Request::get("/v1/events?drain=true").body(Body::empty()).unwrap()).await;
    assert_eq!(json_of(&b).as_array().unwrap().len(), 3);
    let (_, b) = send(&app, Request::get("/v1/events").body(Body::empty()).unwrap()).await;
    assert!(json_of(&b).as_array().unwrap().is_empty());
}

fn signed_wa_post() -> Request<Body> {
    Request::post("/webhooks/whatsapp")
        .header("x-hub-signature-256", sign(WA_PAYLOAD))
        .body(Body::from(WA_PAYLOAD))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn ack(ids: &[&str]) -> Request<Body> {
    Request::post("/v1/events/ack")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "ids": ids }).to_string()))
        .unwrap()
}

fn ids_of(body: &str) -> Vec<String> {
    json_of(body).as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn acknowledge_removes_only_named_events() {
    let app = app(None);
    send(&app, signed_wa_post()).await;
    let (_, b) = send(&app, get("/v1/events")).await;
    let ids = ids_of(&b);
    assert_eq!(ids.len(), 3);

    let (s, b) = send(&app, ack(&[&ids[0], "unknown"])).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(json_of(&b)["acknowledged"], 1);

    let (_, b) = send(&app, get("/v1/events")).await;
    assert_eq!(ids_of(&b), ids[1..]);
}

#[tokio::test]
async fn events_survive_a_service_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("msgkit.db");

    let first = app_with_db(&db);
    let (s, b) = send(&first, signed_wa_post()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(json_of(&b)["accepted"], 3);
    let (_, b) = send(&first, get("/v1/events")).await;
    let before = ids_of(&b);
    drop(first); // "restart"

    let second = app_with_db(&db);
    let (_, b) = send(&second, get("/v1/events")).await;
    assert_eq!(ids_of(&b), before, "events are still there after a restart");

    // Meta redelivers the same webhook after the restart: all duplicates.
    let (_, b) = send(&second, signed_wa_post()).await;
    let v = json_of(&b);
    assert_eq!((v["accepted"].as_u64(), v["duplicates"].as_u64()), (Some(0), Some(3)));
}

#[tokio::test]
async fn acknowledgements_survive_a_restart_and_keep_deduplicating() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("msgkit.db");

    let first = app_with_db(&db);
    send(&first, signed_wa_post()).await;
    let (_, b) = send(&first, get("/v1/events")).await;
    let ids = ids_of(&b);
    send(&first, ack(&[&ids[0], &ids[1]])).await;
    drop(first);

    let second = app_with_db(&db);
    let (_, b) = send(&second, get("/v1/events")).await;
    assert_eq!(ids_of(&b), ids[2..]);
    let (_, b) = send(&second, signed_wa_post()).await;
    assert_eq!(json_of(&b)["accepted"], 0, "acknowledged events are not re-accepted");
}

#[tokio::test]
async fn drain_still_works_on_the_durable_store() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_db(&dir.path().join("msgkit.db"));
    send(&app, signed_wa_post()).await;
    let (_, b) = send(&app, get("/v1/events?drain=true")).await;
    assert_eq!(ids_of(&b).len(), 3);
    let (_, b) = send(&app, get("/v1/events")).await;
    assert!(ids_of(&b).is_empty());
}

#[tokio::test]
async fn ack_requires_the_bearer_token() {
    let app = app(Some("s3cret"));
    assert_eq!(send(&app, ack(&["x"])).await.0, StatusCode::UNAUTHORIZED);
}

#[test]
fn unusable_db_path_fails_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("no-such-dir").join("msgkit.db");
    assert!(AppState::new(config(None, Some(&bad))).is_err());
}

#[tokio::test]
async fn messenger_webhook() {
    let body = json!({"object":"page","entry":[{"id":"P","messaging":[
        {"sender":{"id":"U"},"recipient":{"id":"P"},"timestamp":1704067200000i64,"message":{"mid":"m1","text":"yo"}}
    ]}]})
    .to_string();
    let req = Request::post("/webhooks/messenger")
        .header("x-hub-signature-256", sign(&body))
        .body(Body::from(body))
        .unwrap();
    let (s, b) = send(&app(None), req).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(json_of(&b)["accepted"], 1);
}

#[tokio::test]
async fn subscription_handshake() {
    let app = app(None);
    let uri = format!("/webhooks/whatsapp?hub.mode=subscribe&hub.verify_token={TOKEN}&hub.challenge=12345");
    let (s, b) = send(&app, Request::get(uri).body(Body::empty()).unwrap()).await;
    assert_eq!((s, b.as_str()), (StatusCode::OK, "12345"));

    let uri = "/webhooks/whatsapp?hub.mode=subscribe&hub.verify_token=nope&hub.challenge=12345";
    assert_eq!(send(&app, Request::get(uri).body(Body::empty()).unwrap()).await.0, StatusCode::FORBIDDEN);
}
