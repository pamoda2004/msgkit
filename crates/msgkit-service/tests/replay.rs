//! Replay protection for webhooks (F14): off by default, `log` reports what `enforce` would drop, `enforce` drops
//! events whose signed timestamp is outside the window, and the answer is always `200` with counts.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use msgkit_service::{router, AppState, Config, ReplayMode, WebhookConfig};
use serde_json::{json, Value};
use tower::ServiceExt;

const SECRET: &str = "app-secret";
const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;

fn now_ms() -> i64 {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap()
}

fn config(mode: ReplayMode) -> Config {
    let wh = WebhookConfig { app_secret: SECRET.into(), verify_token: "v".into() };
    Config::default().with_whatsapp(wh.clone()).with_messenger(wh).with_replay_protection(mode)
}

fn app(config: Config) -> Router {
    router(AppState::new(config).unwrap())
}

/// A WhatsApp delivery with one text message per `(id, timestamp in ms)`; WhatsApp sends seconds as a string.
fn whatsapp(messages: &[(&str, i64)]) -> String {
    let messages: Vec<Value> = messages
        .iter()
        .map(|(id, ms)| {
            json!({ "from": "94700000000", "id": id, "timestamp": (ms / 1000).to_string(), "type": "text",
                    "text": { "body": "synthetic text" } })
        })
        .collect();
    json!({ "object": "whatsapp_business_account", "entry": [{ "id": "W", "changes": [{ "field": "messages",
        "value": { "metadata": { "display_phone_number": "15550100111" }, "messages": messages } }] }] })
    .to_string()
}

/// A Messenger delivery; Messenger sends milliseconds as a number.
fn messenger(messages: &[(&str, i64)]) -> String {
    let items: Vec<Value> = messages
        .iter()
        .map(|(mid, ms)| {
            json!({ "sender": { "id": "USER1" }, "recipient": { "id": "PAGE1" }, "timestamp": ms,
                    "message": { "mid": mid, "text": "synthetic text" } })
        })
        .collect();
    json!({ "object": "page", "entry": [{ "id": "PAGE1", "time": 0, "messaging": items }] }).to_string()
}

async fn deliver(app: &Router, path: &str, body: String) -> (StatusCode, Value) {
    let signature = msgkit_core::webhook::sign(SECRET.as_bytes(), body.as_bytes());
    let request = Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-hub-signature-256", signature)
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn held_ids(app: &Router) -> Vec<String> {
    let response =
        app.clone().oneshot(Request::get("/v1/events").body(Body::empty()).unwrap()).await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let events: Vec<Value> = serde_json::from_slice(&bytes).unwrap();
    events.iter().map(|e| e["external_id"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn off_is_the_default_and_changes_nothing_not_even_the_answer() {
    assert_eq!(Config::default().replay_protection, ReplayMode::Off);
    let app = app(config(ReplayMode::Off));
    // A message from 1970 and one from the future are both accepted, and the answer has only the old fields.
    let body = whatsapp(&[("wamid.OLD", 1_000_000), ("wamid.AHEAD", now_ms() + 400 * DAY_MS)]);
    let (status, answer) = deliver(&app, "/webhooks/whatsapp", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer, json!({ "accepted": 2, "duplicates": 0, "warnings": [] }));
}

#[tokio::test]
async fn enforce_drops_old_events_keeps_fresh_ones_and_still_answers_200_with_counts() {
    let app = app(config(ReplayMode::Enforce));
    let now = now_ms();
    let body = whatsapp(&[
        ("wamid.FRESH", now - 5 * MINUTE_MS),
        ("wamid.LATE", now - 7 * DAY_MS), // Meta's longest retry: still accepted
        ("wamid.STALE", now - 9 * DAY_MS),
        ("wamid.ANCIENT", 1_000_000),
    ]);
    let (status, answer) = deliver(&app, "/webhooks/whatsapp", body).await;
    assert_eq!(status, StatusCode::OK, "a rejected replay must not make Meta retry");
    assert_eq!(answer["accepted"], 2);
    assert_eq!(answer["duplicates"], 0);
    assert_eq!(answer["stale"], 2);
    assert_eq!(answer["future"], 0);
    assert_eq!(answer["replay_protection"], "enforce");
    assert_eq!(answer["warnings"], json!(["2 event(s) outside the replay window were dropped"]));
    let mut held = held_ids(&app).await;
    held.sort();
    assert_eq!(held, ["wamid.FRESH", "wamid.LATE"]);

    // An old payload replayed later is dropped again (and an accepted one is a duplicate, as before).
    let (_, again) =
        deliver(&app, "/webhooks/whatsapp", whatsapp(&[("wamid.STALE", now - 9 * DAY_MS)])).await;
    assert_eq!((again["accepted"].as_i64(), again["stale"].as_i64()), (Some(0), Some(1)));
    let (_, dup) =
        deliver(&app, "/webhooks/whatsapp", whatsapp(&[("wamid.FRESH", now - 5 * MINUTE_MS)])).await;
    assert_eq!((dup["accepted"].as_i64(), dup["duplicates"].as_i64()), (Some(0), Some(1)));
}

#[tokio::test]
async fn a_delivery_where_everything_is_stale_is_still_a_200() {
    let app = app(config(ReplayMode::Enforce));
    let (status, answer) = deliver(&app, "/webhooks/whatsapp", whatsapp(&[("wamid.OLD", 1_000_000)])).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((answer["accepted"].as_i64(), answer["stale"].as_i64()), (Some(0), Some(1)));
    assert_eq!(held_ids(&app).await.len(), 0);
}

#[tokio::test]
async fn the_window_edges_and_a_custom_window() {
    let now = now_ms();
    let app = app(config(ReplayMode::Enforce));
    // The default window is 8 days: a little inside is fresh, a little outside is stale.
    let body = whatsapp(&[
        ("wamid.IN", now - 8 * DAY_MS + 5 * MINUTE_MS),
        ("wamid.OUT", now - 8 * DAY_MS - 5 * MINUTE_MS),
    ]);
    let (_, answer) = deliver(&app, "/webhooks/whatsapp", body).await;
    assert_eq!((answer["accepted"].as_i64(), answer["stale"].as_i64()), (Some(1), Some(1)));

    // A one-hour window accepts a 30-minute-old event and refuses a two-hour-old one.
    let short = self::app(config(ReplayMode::Enforce).with_replay_window(Duration::from_secs(3600)));
    let body = whatsapp(&[("wamid.HALF", now - 30 * MINUTE_MS), ("wamid.TWO", now - 120 * MINUTE_MS)]);
    let (_, answer) = deliver(&short, "/webhooks/whatsapp", body).await;
    assert_eq!((answer["accepted"].as_i64(), answer["stale"].as_i64()), (Some(1), Some(1)));
}

#[tokio::test]
async fn a_future_timestamp_within_the_ten_minute_skew_is_accepted_and_beyond_it_is_not() {
    let now = now_ms();
    let app = app(config(ReplayMode::Enforce));
    let body = whatsapp(&[("wamid.A", now + 5 * MINUTE_MS), ("wamid.B", now + 9 * MINUTE_MS)]);
    let (status, answer) = deliver(&app, "/webhooks/whatsapp", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((answer["accepted"].as_i64(), answer["future"].as_i64()), (Some(2), Some(0)), "{answer}");

    let body = whatsapp(&[("wamid.C", now + 15 * MINUTE_MS), ("wamid.D", now + 2 * DAY_MS)]);
    let (_, answer) = deliver(&app, "/webhooks/whatsapp", body).await;
    assert_eq!(
        (answer["accepted"].as_i64(), answer["future"].as_i64(), answer["stale"].as_i64()),
        (Some(0), Some(2), Some(0))
    );
    assert_eq!(held_ids(&app).await.len(), 2);
}

#[tokio::test]
async fn log_mode_drops_nothing_and_reports_what_enforce_would_drop() {
    let app = app(config(ReplayMode::Log));
    let now = now_ms();
    let body =
        whatsapp(&[("wamid.FRESH", now), ("wamid.STALE", now - 9 * DAY_MS), ("wamid.AHEAD", now + DAY_MS)]);
    let (status, answer) = deliver(&app, "/webhooks/whatsapp", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(answer["accepted"], 3, "log mode stores everything");
    assert_eq!((answer["stale"].as_i64(), answer["future"].as_i64()), (Some(1), Some(1)));
    assert_eq!(answer["replay_protection"], "log");
    assert_eq!(
        answer["warnings"],
        json!(["2 event(s) outside the replay window would be dropped in enforce mode"])
    );
    assert_eq!(held_ids(&app).await.len(), 3);
}

#[tokio::test]
async fn messenger_uses_milliseconds() {
    let app = app(config(ReplayMode::Enforce));
    let now = now_ms();
    let body = messenger(&[
        ("m_fresh", now - MINUTE_MS),
        ("m_stale", now - 9 * DAY_MS),
        ("m_ahead", now + 9 * MINUTE_MS),
    ]);
    let (status, answer) = deliver(&app, "/webhooks/messenger", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (answer["accepted"].as_i64(), answer["stale"].as_i64(), answer["future"].as_i64()),
        (Some(2), Some(1), Some(0))
    );
    let mut held = held_ids(&app).await;
    held.sort();
    assert_eq!(held, ["m_ahead", "m_fresh"]);
}

#[test]
fn enforce_needs_the_id_retention_to_cover_the_window() {
    let refuse = |config: Config| AppState::new(config).expect_err("must refuse").to_string();

    // The default retention is 30 days, the default window 8: fine. A short retention is refused, by name.
    assert!(AppState::new(config(ReplayMode::Enforce)).is_ok());
    let text = refuse(config(ReplayMode::Enforce).with_id_retention(Duration::from_secs(2 * 86_400)));
    assert!(
        text.contains("MSGKIT_ID_RETENTION_DAYS") && text.contains("MSGKIT_REPLAY_WINDOW_HOURS"),
        "{text}"
    );
    // A wide window against the default retention is refused too, and 8 to 14 days of both are fine.
    let wide = config(ReplayMode::Enforce).with_replay_window(Duration::from_secs(40 * 86_400));
    assert!(refuse(wide).contains("replay window"));
    let tight = config(ReplayMode::Enforce)
        .with_id_retention(Duration::from_secs(8 * 86_400))
        .with_replay_window(Duration::from_secs(8 * 86_400));
    assert!(AppState::new(tight).is_ok(), "equal is enough");
    // Log and off drop nothing, so they do not need it.
    assert!(AppState::new(config(ReplayMode::Log).with_id_retention(Duration::from_secs(86_400))).is_ok());
    assert!(AppState::new(config(ReplayMode::Off).with_id_retention(Duration::from_secs(86_400))).is_ok());
}
