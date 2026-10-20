//! The warning for events outside the replay window has counts and an age, never an id, a sender or text. This is
//! the only test in this file on purpose: tracing caches which call sites are enabled for the whole process, so a
//! scoped subscriber is only reliable when no other test runs beside it.

use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, Request};
use http_body_util::BodyExt;
use msgkit_service::{router, AppState, Config, ReplayMode, WebhookConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

const SECRET: &str = "app-secret";

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn the_log_has_counts_and_an_age_and_no_content() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let wh = WebhookConfig { app_secret: SECRET.into(), verify_token: "v".into() };
    let make = |mode| {
        router(
            AppState::new(Config::default().with_whatsapp(wh.clone()).with_replay_protection(mode)).unwrap(),
        )
    };
    let body = json!({ "object": "whatsapp_business_account", "entry": [{ "id": "W", "changes": [{ "field": "messages",
        "value": { "metadata": { "display_phone_number": "15550100111" }, "messages": [
            { "from": "94700000000", "id": "wamid.SECRETID", "timestamp": "1000000", "type": "text",
              "text": { "body": "SECRET-MESSAGE-TEXT" } } ] } }] }] })
    .to_string();

    for (mode, message) in [
        (ReplayMode::Enforce, "webhook events outside the replay window were dropped"),
        (ReplayMode::Log, "webhook events outside the replay window would be dropped in enforce mode"),
    ] {
        let signature = msgkit_core::webhook::sign(SECRET.as_bytes(), body.as_bytes());
        let request = Request::post("/webhooks/whatsapp")
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-hub-signature-256", signature)
            .body(Body::from(body.clone()))
            .unwrap();
        let response = make(mode).oneshot(request).await.unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let answer: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(!answer.to_string().contains("SECRET"), "{answer}");

        let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        let line = log
            .lines()
            .find(|l| l.contains(message))
            .unwrap_or_else(|| panic!("no line for {mode:?}: {log}"));
        assert!(line.contains("platform=\"whatsapp\"") || line.contains("platform=whatsapp"), "{line}");
        assert!(line.contains("stale=1") && line.contains("future=0"), "{line}");
        assert!(line.contains("oldest_age_hours="), "{line}");
        let count = if mode == ReplayMode::Enforce { "dropped=1" } else { "would_drop=1" };
        assert!(line.contains(count), "{line}");
    }
    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    for needle in ["SECRET", "wamid", "94700000000", "15550100111"] {
        assert!(!log.contains(needle), "the log repeats content ({needle}): {log}");
    }
}
