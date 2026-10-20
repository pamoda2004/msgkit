//! The operator's input limits (`Config::limits`, `MSGKIT_MAX_*`) on the HTTP service: one default shared with the
//! library (32 MiB bodies), refusals with the JSON `limit_exceeded` error (413), and no way for a request to raise
//! a limit.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use msgkit_core::limits::Limits;
use msgkit_service::{router, AppState, Config, WebhookConfig};
use serde_json::Value;
use tower::ServiceExt;

/// `bytes` bytes of short lines, so only the body size (not the line length) is in question.
fn filler(bytes: usize) -> String {
    let line = format!("{}\n", "a".repeat(63));
    let mut text = line.repeat(bytes / line.len());
    text.push_str(&"b".repeat(bytes - text.len()));
    text
}

const CHAT: &str = "01/02/2024, 10:00 - Alice: PRIVATE-VALUE-MUST-NOT-APPEAR\n01/02/2024, 10:01 - Bob: hi\n";

fn app(limits: Limits) -> Router {
    router(AppState::new(Config::default().with_limits(limits)).unwrap())
}

fn whatsapp(query: &str, body: impl Into<Body>) -> Request<Body> {
    Request::post(format!("/v1/import/whatsapp-export{query}"))
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(body.into())
        .unwrap()
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn assert_limit_exceeded(status: StatusCode, body: &str) {
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    let json: Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["code"], "limit_exceeded", "{body}");
    assert!(
        !body.contains("PRIVATE-VALUE") && !body.contains("Alice"),
        "the error repeats the input: {body}"
    );
}

#[tokio::test]
async fn the_default_body_limit_is_the_library_default_of_32_mib() {
    let app = app(Limits::default());
    let at = filler(32 * 1024 * 1024);
    // At the limit the body is accepted by the transport (and is just not a chat), over it is refused.
    let (status, _) = send(&app, whatsapp("", at)).await;
    assert_eq!(status, StatusCode::OK);
    let over = filler(32 * 1024 * 1024 + 1);
    let (status, body) = send(&app, whatsapp("", over)).await;
    assert_limit_exceeded(status, &body);
}

#[tokio::test]
async fn an_operator_can_lower_and_raise_the_body_limit() {
    let lower = app(Limits::default().with_max_input_bytes(64));
    let (status, body) = send(&lower, whatsapp("", "a".repeat(65))).await;
    assert_limit_exceeded(status, &body);

    let raised = app(Limits::default().with_max_input_bytes(40 * 1024 * 1024));
    let (status, _) = send(&raised, whatsapp("", filler(33 * 1024 * 1024))).await;
    assert_eq!(status, StatusCode::OK);

    let off = app(Limits::unlimited());
    let (status, _) = send(&off, whatsapp("", filler(33 * 1024 * 1024))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn the_other_limits_are_refused_with_the_same_json_error() {
    let count = app(Limits::default().with_max_messages(1));
    let (status, body) = send(&count, whatsapp("", CHAT)).await;
    assert_limit_exceeded(status, &body);

    let line = app(Limits::default().with_max_line_bytes(20));
    let (status, body) = send(&line, whatsapp("", CHAT)).await;
    assert_limit_exceeded(status, &body);

    let depth = app(Limits::default().with_max_json_depth(2));
    let req = Request::post("/v1/import/facebook-export")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"participants":[{"name":"PRIVATE-VALUE-MUST-NOT-APPEAR"}],"messages":[]}"#))
        .unwrap();
    let (status, body) = send(&depth, req).await;
    assert_limit_exceeded(status, &body);

    let rows = app(Limits::default().with_max_messages(1));
    let batch = r#"{"sms":[{"address":"+94700000000","body":"a","date_ms":1,"type":1},{"address":"+94700000000","body":"b","date_ms":2,"type":1}]}"#;
    let req = Request::post("/v1/import/device")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(batch))
        .unwrap();
    let (status, body) = send(&rows, req).await;
    assert_limit_exceeded(status, &body);
}

#[tokio::test]
async fn a_request_cannot_raise_a_limit() {
    let tight = app(Limits::default().with_max_messages(1));
    // A `limits` option in the query string is not understood; it never replaces the operator's setting.
    let (status, body) = send(&tight, whatsapp("?limits=%7B%22max_messages%22%3A0%7D", CHAT)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "invalid_input");
    // Loose query parameters that merely look like limits are ignored.
    let (status, body) = send(&tight, whatsapp("?max_messages=0&max_input_bytes=0", CHAT)).await;
    assert_limit_exceeded(status, &body);
}

#[tokio::test]
async fn webhooks_are_capped_by_the_lower_of_2_mib_and_the_body_limit() {
    let wh = WebhookConfig { app_secret: "app-secret".into(), verify_token: "verify-me".into() };
    let lowered = router(
        AppState::new(
            Config::default().with_whatsapp(wh).with_limits(Limits::default().with_max_input_bytes(100)),
        )
        .unwrap(),
    );
    let body = vec![b'a'; 101];
    let req = Request::post("/webhooks/whatsapp")
        .header("x-hub-signature-256", msgkit_core::webhook::sign(b"app-secret", &body))
        .body(Body::from(body))
        .unwrap();
    let (status, text) = send(&lowered, req).await;
    assert_limit_exceeded(status, &text);
}

/// Every way a client could try to change a limit: none of them has any effect.
#[tokio::test]
async fn a_client_cannot_lower_raise_or_disable_a_limit_in_any_way() {
    // Operator: at most 1 message and a 1 KiB body.
    let app = app(Limits::default().with_max_messages(1).with_max_input_bytes(1024));
    let lift = r#"{"max_input_bytes":0,"max_messages":0,"max_line_bytes":0,"max_json_depth":0}"#;

    // 1. WhatsApp: query parameters, flat or as a `limits` option (the latter is refused outright).
    for query in [
        "?max_messages=0&max_input_bytes=0&max_line_bytes=0&max_json_depth=0".to_owned(),
        "?limits.max_messages=0".to_owned(),
        "?limits%5Bmax_messages%5D=0".to_owned(),
    ] {
        let (status, body) = send(&app, whatsapp(&query, CHAT)).await;
        assert_limit_exceeded(status, &body);
    }
    let encoded = lift
        .replace('{', "%7B")
        .replace('}', "%7D")
        .replace('"', "%22")
        .replace(':', "%3A")
        .replace(',', "%2C");
    let (status, body) = send(&app, whatsapp(&format!("?limits={encoded}"), CHAT)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // 2. A `limits` / `options` object inside a JSON body is not read: the body is the document itself.
    let device = r#"{"limits":{"max_messages":0},"options":{"limits":{"max_messages":0}},"sms":[
        {"address":"+94700000000","body":"a","date_ms":1,"type":1},
        {"address":"+94700000000","body":"b","date_ms":2,"type":1}]}"#;
    let req = Request::post("/v1/import/device")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(device))
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_limit_exceeded(status, &body);

    let facebook = r#"{"limits":{"max_messages":0},"participants":[],"messages":[
        {"sender_name":"A","timestamp_ms":1,"content":"x"},{"sender_name":"A","timestamp_ms":2,"content":"y"}]}"#;
    let req = Request::post("/v1/import/facebook-export?max_messages=0")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(facebook))
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_limit_exceeded(status, &body);

    // 3. Headers do not matter either (a body over the operator's byte cap stays refused).
    let req = Request::post("/v1/import/whatsapp-export")
        .header(header::CONTENT_TYPE, "text/plain")
        .header("x-msgkit-max-input-bytes", "0")
        .body(Body::from("a\n".repeat(2048)))
        .unwrap();
    let (status, body) = send(&app, req).await;
    assert_limit_exceeded(status, &body);

    // 4. A limit set by the operator can still be lower than the client's wish: nothing was lowered or raised.
    let (status, _) = send(&app, whatsapp("", "01/02/2024, 10:00 - Alice: hi\n")).await;
    assert_eq!(status, StatusCode::OK);
}
