//! The log line of an erase request has counts, a request id and a UTC time, never the address. This is the only
//! test in this file on purpose: tracing caches which call sites are enabled for the whole process, so a
//! scoped subscriber is only reliable when no other test runs beside it.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, Request};
use http_body_util::BodyExt;
use msgkit_core::store::conformance::event_from;
use msgkit_core::store::EventStore;
use msgkit_service::{router, AppState, Config};
use msgkit_store_sqlite::SqliteStore;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "operator-token";
const SECRET_ADDRESS: &str = "secret.person@example.com";

fn database(path: &Path) {
    let mut db = SqliteStore::open(path).unwrap();
    db.insert_all(vec![
        event_from("a", 1, "+94700000000", "+94711111111"),
        event_from("n", 3, "0700000000", "+94711111111"),
        event_from("m", 5, SECRET_ADDRESS, "+94700000004"),
    ])
    .unwrap();
}

fn erase_request(body: &str, token: &str) -> Request<Body> {
    Request::post("/v1/store/erase")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body.to_owned()))
        .unwrap()
}

/// A log writer the test can read back.
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
async fn the_log_line_has_counts_an_id_and_a_time_and_never_the_address() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    database(&path);
    let app = router(
        AppState::new(
            Config::default().with_db_path(&path).with_api_token(TOKEN).with_default_country_code("94"),
        )
        .unwrap(),
    );
    let send = |req: Request<Body>| {
        let app = app.clone();
        async move {
            let res = app.oneshot(req).await.unwrap();
            let bytes = res.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null)
        }
    };

    // A success, a refused body that holds the address, and a wrong token that holds it too.
    let ok = send(erase_request(&json!({ "address": SECRET_ADDRESS }).to_string(), TOKEN)).await;
    send(erase_request(&json!({ "address": SECRET_ADDRESS, "x": 1 }).to_string(), TOKEN)).await;
    send(erase_request(&json!({ "address": SECRET_ADDRESS }).to_string(), "bad")).await;
    send(erase_request(&json!({ "address": "0700000000" }).to_string(), TOKEN)).await;

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    let request_id = ok["request_id"].as_str().unwrap();
    let line = log
        .lines()
        .find(|l| l.contains("erase request handled"))
        .unwrap_or_else(|| panic!("no log line: {log}"));
    assert!(line.contains(request_id), "{line}");
    assert!(line.contains(ok["handled_at"].as_str().unwrap()), "{line}");
    assert!(line.contains("erased=1"), "{line}");
    assert!(line.contains("drop_ids=false"), "{line}");
    assert_eq!(log.matches("erase request handled").count(), 2, "one line per handled request: {log}");
    for needle in [SECRET_ADDRESS, "secret.person", "0700000000", "700000000", "+94700000000"] {
        assert!(!log.contains(needle), "the log repeats the address ({needle}): {log}");
    }
}
