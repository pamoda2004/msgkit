//! `POST /v1/store/erase`: it needs the token, takes the address only in the JSON body, reads it the way
//! `erase_person` does, and answers and logs counts, a request id and a time, never the address.

use std::path::Path;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use msgkit_core::store::conformance::event_from;
use msgkit_core::store::{EventStore, MemoryStore};
use msgkit_service::{router, AppState, Config};
use msgkit_store_sqlite::SqliteStore;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "operator-token";
/// A recognisable address that must never appear in any output.
const SECRET_ADDRESS: &str = "secret.person@example.com";

/// Three held events: `a` and `b` involve +94700000000 (stored as written by an import), `n` involves the same
/// person stored in national form, `c` is someone else, `m` is the person with an email address.
fn database(path: &Path) {
    let mut db = SqliteStore::open(path).unwrap();
    db.insert_all(vec![
        event_from("a", 1, "+94700000000", "+94711111111"),
        event_from("b", 2, "+94722222222", "+94700000000"),
        event_from("n", 3, "0700000000", "+94711111111"),
        event_from("c", 4, "+94700000003", "+94700000004"),
        event_from("m", 5, SECRET_ADDRESS, "+94700000004"),
    ])
    .unwrap();
}

fn app_on(path: &Path, token: Option<&str>, country: Option<&str>) -> Router {
    let mut config = Config::default().with_db_path(path);
    if let Some(token) = token {
        config = config.with_api_token(token);
    }
    if let Some(code) = country {
        config = config.with_default_country_code(code);
    }
    router(AppState::new(config).unwrap())
}

fn erase_request(body: &str, token: Option<&str>) -> Request<Body> {
    let mut req = Request::post("/v1/store/erase").header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    req.body(Body::from(body.to_owned())).unwrap()
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn erase(app: &Router, address: &str) -> Value {
    let (status, body) =
        send(app, erase_request(&json!({ "address": address }).to_string(), Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_str(&body).unwrap()
}

fn held_ids(path: &Path) -> Vec<String> {
    SqliteStore::open(path).unwrap().list().unwrap().iter().map(|e| e.id().to_owned()).collect()
}

#[tokio::test]
async fn any_spelling_of_a_number_erases_the_same_person_and_the_answer_is_counts_and_an_id() {
    for spelling in ["0700000000", "070 000 0000", "+94700000000", " +94 70 000 0000 "] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        database(&path);
        let app = app_on(&path, Some(TOKEN), Some("94"));

        let body = erase(&app, spelling).await;
        assert_eq!(body["erased"], 3, "{spelling}");
        assert_eq!(held_ids(&path), ["c", "m"], "{spelling}");

        let id = body["request_id"].as_str().unwrap();
        assert_eq!(id.len(), 32, "{spelling}");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        let at = body["handled_at"].as_str().unwrap();
        assert!(at.ends_with('Z') && chrono_parses(at), "{at}");
        assert_eq!(body.as_object().unwrap().len(), 3, "counts, an id and a time, nothing else");
        assert!(!body.to_string().contains("0700000000"), "the answer must not repeat the address");
    }
}

fn chrono_parses(at: &str) -> bool {
    // 2026-10-20T12:00:00Z
    let b = at.as_bytes();
    at.len() == 20 && b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && b[16] == b':'
}

#[tokio::test]
async fn a_request_id_is_new_each_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    database(&path);
    let app = app_on(&path, Some(TOKEN), None);
    let first = erase(&app, "+94700000009").await;
    let second = erase(&app, "+94700000009").await;
    assert_ne!(first["request_id"], second["request_id"]);
}

#[tokio::test]
async fn an_unknown_person_erases_nothing_without_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    database(&path);
    let app = app_on(&path, Some(TOKEN), Some("94"));
    assert_eq!(erase(&app, "+94700000008").await["erased"], 0);
    assert_eq!(erase(&app, "nobody@example.com").await["erased"], 0);
    assert_eq!(held_ids(&path).len(), 5);
}

#[tokio::test]
async fn an_email_address_or_id_is_matched_trimmed_and_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    database(&path);
    let app = app_on(&path, Some(TOKEN), None);
    assert_eq!(erase(&app, "  SECRET.Person@Example.COM ").await["erased"], 1);
    assert_eq!(held_ids(&path), ["a", "b", "n", "c"]);
}

#[tokio::test]
async fn only_the_operator_sets_the_country_code() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    database(&path);

    // No operator code: a national number is only itself, and a `+` number is only the E.164 rows.
    let app = app_on(&path, Some(TOKEN), None);
    assert_eq!(erase(&app, "0700000000").await["erased"], 1, "only the row stored in national form");

    // A client cannot send the code, in the body or anywhere else.
    let body = json!({ "address": "+94700000009", "default_country_code": "94" }).to_string();
    let (status, text) = send(&app, erase_request(&body, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
    assert!(text.contains("invalid_input"), "{text}");
    let mut req = erase_request(&json!({ "address": "0700000000" }).to_string(), Some(TOKEN));
    *req.uri_mut() = "/v1/store/erase?default_country_code=94".parse().unwrap();
    req.headers_mut().insert("x-default-country-code", "94".parse().unwrap());
    let (status, text) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["erased"],
        0,
        "the national row is already gone"
    );
}

#[tokio::test]
async fn it_needs_the_token_and_without_one_configured_it_is_off_even_on_loopback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    database(&path);
    let body = json!({ "address": "+94700000000" }).to_string();

    let open = app_on(&path, None, None);
    for req in
        [erase_request(&body, None), erase_request(&body, Some("anything")), erase_request("{nope", None)]
    {
        let (status, text) = send(&open, req).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["code"], "not_configured");
        assert_eq!(v["error"], "erasing a person needs MSGKIT_API_TOKEN to be set");
    }

    let guarded = app_on(&path, Some(TOKEN), None);
    for req in [erase_request(&body, None), erase_request(&body, Some("wrong"))] {
        let (status, text) = send(&guarded, req).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["code"], "unauthorized");
    }
    assert_eq!(held_ids(&path).len(), 5, "no refused request erased anything");
}

#[tokio::test]
async fn bad_requests_get_the_documented_errors_and_never_echo_the_address() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    database(&path);
    let app = app_on(&path, Some(TOKEN), None);
    let long = "x".repeat(257);
    let huge = format!(r#"{{"address": "{}"}}"#, "y".repeat(5000));
    let wrong_type = format!(r#"{{"address": ["{SECRET_ADDRESS}"]}}"#);
    let not_json = format!("{{{SECRET_ADDRESS}");

    let cases: Vec<(&str, Request<Body>, StatusCode, &str)> = vec![
        ("empty", erase_request(r#"{"address": ""}"#, Some(TOKEN)), StatusCode::BAD_REQUEST, "invalid_input"),
        (
            "blank",
            erase_request(r#"{"address": "  \t"}"#, Some(TOKEN)),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        (
            "too long",
            erase_request(&json!({ "address": long }).to_string(), Some(TOKEN)),
            StatusCode::BAD_REQUEST,
            "invalid_input",
        ),
        ("missing", erase_request("{}", Some(TOKEN)), StatusCode::UNPROCESSABLE_ENTITY, "invalid_input"),
        (
            "wrong type",
            erase_request(&wrong_type, Some(TOKEN)),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_input",
        ),
        ("not json", erase_request(&not_json, Some(TOKEN)), StatusCode::BAD_REQUEST, "invalid_input"),
        (
            "unknown field",
            erase_request(&json!({ "address": SECRET_ADDRESS, "all": true }).to_string(), Some(TOKEN)),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_input",
        ),
        ("over 4 KiB", erase_request(&huge, Some(TOKEN)), StatusCode::PAYLOAD_TOO_LARGE, "limit_exceeded"),
        (
            "wrong content type",
            Request::post("/v1/store/erase")
                .header(header::CONTENT_TYPE, "text/plain")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::from(SECRET_ADDRESS))
                .unwrap(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "invalid_input",
        ),
    ];
    for (name, req, status, code) in cases {
        let (got, text) = send(&app, req).await;
        assert_eq!(got, status, "{name}: {text}");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["code"], code, "{name}");
        assert_eq!(v.as_object().unwrap().len(), 2, "{name}: only `error` and `code`");
        assert!(
            !text.contains(SECRET_ADDRESS) && !text.contains("xxxx") && !text.contains("yyyy"),
            "{name}: {text}"
        );
    }
    assert_eq!(held_ids(&path).len(), 5, "no refused request erased anything");

    // The address belongs in the body only: a GET, or the address in the URL, is not an erase.
    let (status, _) = send(
        &app,
        Request::get("/v1/store/erase")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let (status, _) = send(
        &app,
        Request::delete(format!("/v1/store/erase/{SECRET_ADDRESS}"))
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(held_ids(&path).len(), 5);
}

#[tokio::test]
async fn drop_ids_forgets_the_ids_and_the_default_keeps_them() {
    let event = || event_from("a", 1, "+94700000000", "+94711111111");
    for (drop_ids, accepted_again) in [(false, false), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        database(&path);
        let app = app_on(&path, Some(TOKEN), None);
        let body = json!({ "address": "+94700000000", "drop_ids": drop_ids }).to_string();
        let (status, text) = send(&app, erase_request(&body, Some(TOKEN))).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(
            SqliteStore::open(&path).unwrap().insert(event()).unwrap(),
            accepted_again,
            "drop_ids={drop_ids}"
        );
    }
}

#[tokio::test]
async fn the_memory_store_can_erase_too() {
    let app = router(AppState::new(Config::default().with_api_token(TOKEN)).unwrap());
    let (status, text) =
        send(&app, erase_request(&json!({ "address": "+94700000000" }).to_string(), Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["erased"], 0);
}

#[tokio::test]
async fn a_custom_store_that_cannot_erase_answers_501_not_supported() {
    let state = AppState::with_store(Config::default().with_api_token(TOKEN), Box::new(MemoryStore::new()));
    let app = router(state);
    let (status, text) =
        send(&app, erase_request(&json!({ "address": "+94700000000" }).to_string(), Some(TOKEN))).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["code"], "not_supported");
    assert_eq!(v["error"], "this service's store cannot erase a person");
}
