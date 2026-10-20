//! Every error response of the HTTP service carries a stable `code` (docs/ERRORS.md) next to the status and the
//! text. This test pins all three, so a change to any of them is deliberate.

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use msgkit_core::error::codes;
use msgkit_service::{router, AppState, Config, WebhookConfig};
use serde_json::Value;
use tower::ServiceExt;

const SECRET: &str = "app-secret";
const TOKEN: &str = "verify-me";

fn config(api_token: Option<&str>, webhooks: bool) -> Config {
    let wh = WebhookConfig { app_secret: SECRET.into(), verify_token: TOKEN.into() };
    let mut config = Config::default();
    if webhooks {
        config = config.with_whatsapp(wh.clone()).with_messenger(wh);
    }
    match api_token {
        Some(token) => config.with_api_token(token),
        None => config,
    }
}

fn post(path: &str, content_type: &str, body: &[u8]) -> Request<Body> {
    Request::post(path).header(header::CONTENT_TYPE, content_type).body(Body::from(body.to_vec())).unwrap()
}

fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

fn signed(path: &str, body: &[u8]) -> Request<Body> {
    Request::post(path)
        .header("x-hub-signature-256", msgkit_core::webhook::sign(SECRET.as_bytes(), body))
        .body(Body::from(body.to_vec()))
        .unwrap()
}

struct Case {
    name: &'static str,
    token: Option<&'static str>,
    webhooks: bool,
    request: Request<Body>,
    status: StatusCode,
    code: &'static str,
    error: &'static str,
}

// A table of cases: splitting it would only scatter the data.
#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn http_error_responses_carry_stable_codes() {
    let big_webhook = vec![b'a'; 2 * 1024 * 1024 + 1];
    let big_import = vec![b' '; 32 * 1024 * 1024 + 1];
    let case = |name, token, webhooks, request, status, code, error| Case {
        name,
        token,
        webhooks,
        request,
        status,
        code,
        error,
    };
    let cases = vec![
        case(
            "facebook export that is not an export",
            None,
            true,
            post("/v1/import/facebook-export", "application/json", b"[1,2,3]"),
            StatusCode::BAD_REQUEST,
            codes::PARSE_ERROR,
            "parse error: not a Facebook messages export: the JSON does not match the expected shape",
        ),
        case(
            "body that is not JSON",
            None,
            true,
            post("/v1/import/facebook-export", "application/json", b"{nope"),
            StatusCode::BAD_REQUEST,
            codes::INVALID_INPUT,
            "the request body is not valid JSON",
        ),
        case(
            "wrong content type",
            None,
            true,
            post("/v1/import/facebook-export", "text/plain", b"{}"),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            codes::INVALID_INPUT,
            "expected a request with content-type application/json",
        ),
        case(
            "device batch of the wrong shape",
            None,
            true,
            post("/v1/import/device", "application/json", br#"{"sms":"x"}"#),
            StatusCode::UNPROCESSABLE_ENTITY,
            codes::INVALID_INPUT,
            "the JSON body does not match the expected shape",
        ),
        case(
            "bad option value in the query",
            None,
            true,
            post("/v1/import/whatsapp-export?date_order=zz", "text/plain", b"x"),
            StatusCode::BAD_REQUEST,
            codes::INVALID_INPUT,
            "the query string is invalid",
        ),
        case(
            "unknown time zone in the query",
            None,
            true,
            post("/v1/import/whatsapp-export?time_zone=Mars%2FPhobos", "text/plain", b"x"),
            StatusCode::BAD_REQUEST,
            codes::INVALID_INPUT,
            "invalid input: unknown or unsupported time_zone (use an exact IANA name such as Asia/Colombo)",
        ),
        case(
            "missing bearer token",
            Some("tok"),
            true,
            get("/v1/events"),
            StatusCode::UNAUTHORIZED,
            codes::UNAUTHORIZED,
            "missing or invalid bearer token",
        ),
        case(
            "webhook without a signature",
            None,
            true,
            post("/webhooks/whatsapp", "application/json", b"{}"),
            StatusCode::UNAUTHORIZED,
            codes::SIGNATURE_INVALID,
            "invalid signature",
        ),
        case(
            "webhook that is not configured",
            None,
            false,
            post("/webhooks/whatsapp", "application/json", b"{}"),
            StatusCode::SERVICE_UNAVAILABLE,
            codes::NOT_CONFIGURED,
            "webhook not configured",
        ),
        case(
            "handshake with the wrong token",
            None,
            true,
            get("/webhooks/whatsapp?hub.mode=subscribe&hub.verify_token=no&hub.challenge=1"),
            StatusCode::FORBIDDEN,
            codes::VERIFICATION_FAILED,
            "verification failed",
        ),
        case(
            "handshake on a webhook that is not configured",
            None,
            false,
            get("/webhooks/whatsapp?hub.mode=subscribe&hub.verify_token=no&hub.challenge=1"),
            StatusCode::SERVICE_UNAVAILABLE,
            codes::NOT_CONFIGURED,
            "webhook not configured",
        ),
        case(
            "signed webhook that is not JSON",
            None,
            true,
            signed("/webhooks/whatsapp", b"{nope"),
            StatusCode::BAD_REQUEST,
            codes::INVALID_INPUT,
            "invalid JSON: invalid JSON syntax at line 1 column 2",
        ),
        case(
            "signed webhook for the wrong platform",
            None,
            true,
            signed("/webhooks/whatsapp", br#"{"object":"x"}"#),
            StatusCode::BAD_REQUEST,
            codes::PARSE_ERROR,
            "parse error: not a WhatsApp Business webhook (object != whatsapp_business_account)",
        ),
        case(
            "signed Messenger webhook for the wrong platform",
            None,
            true,
            signed("/webhooks/messenger", br#"{"object":"x"}"#),
            StatusCode::BAD_REQUEST,
            codes::PARSE_ERROR,
            "parse error: not a Messenger webhook (object != page)",
        ),
        case(
            "acknowledgement of the wrong shape",
            None,
            true,
            post("/v1/events/ack", "application/json", br#"{"ids":1}"#),
            StatusCode::UNPROCESSABLE_ENTITY,
            codes::INVALID_INPUT,
            "the JSON body does not match the expected shape",
        ),
        case(
            "JSON body over the limit",
            None,
            true,
            post("/v1/events/ack", "application/json", &vec![b' '; 3 * 1024 * 1024]),
            StatusCode::PAYLOAD_TOO_LARGE,
            codes::LIMIT_EXCEEDED,
            "the request body could not be read (too large?)",
        ),
        case(
            "webhook body over the limit",
            None,
            true,
            signed("/webhooks/whatsapp", &big_webhook),
            StatusCode::PAYLOAD_TOO_LARGE,
            codes::LIMIT_EXCEEDED,
            "the request body is too large",
        ),
        case(
            "text import over the limit",
            None,
            true,
            post("/v1/import/whatsapp-export", "text/plain", &big_import),
            StatusCode::PAYLOAD_TOO_LARGE,
            codes::LIMIT_EXCEEDED,
            "the request body is too large",
        ),
        case(
            "JSON import over the limit",
            None,
            true,
            post("/v1/import/device", "application/json", &big_import),
            StatusCode::PAYLOAD_TOO_LARGE,
            codes::LIMIT_EXCEEDED,
            "the request body could not be read (too large?)",
        ),
    ];

    let mut seen = std::collections::BTreeSet::new();
    for c in cases {
        let app = router(AppState::new(config(c.token, c.webhooks)).unwrap());
        let response = app.oneshot(c.request).await.unwrap();
        assert_eq!(response.status(), c.status, "{}", c.name);
        let content_type = response.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok());
        assert_eq!(content_type, Some("application/json"), "{}", c.name);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], c.code, "{}", c.name);
        assert_eq!(body["error"], c.error, "{}", c.name);
        assert_eq!(body.as_object().unwrap().len(), 2, "{}: only `error` and `code`", c.name);
        seen.insert(c.code);
    }
    // Every code the service can return is exercised above; `unknown_op` and `internal_error` never come from a
    // request the service can be sent in a test, and `storage_error` needs a failing store.
    for code in [
        codes::INVALID_INPUT,
        codes::PARSE_ERROR,
        codes::LIMIT_EXCEEDED,
        codes::UNAUTHORIZED,
        codes::SIGNATURE_INVALID,
        codes::VERIFICATION_FAILED,
        codes::NOT_CONFIGURED,
    ] {
        assert!(seen.contains(code), "no case returns {code}");
    }
}

/// Routes and methods that do not exist answer with a bare status and no body, as before.
#[tokio::test]
async fn unknown_routes_and_methods_have_no_body() {
    for (request, status) in
        [(get("/v1/nope"), StatusCode::NOT_FOUND), (get("/v1/import/device"), StatusCode::METHOD_NOT_ALLOWED)]
    {
        let app = router(AppState::new(config(None, true)).unwrap());
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), status);
        assert!(response.into_body().collect().await.unwrap().to_bytes().is_empty());
    }
}

/// The authentication errors say only that authentication failed. They must never contain the secret, the verify
/// token, the API token, the signature the service expected or computed, what the caller supplied, the challenge,
/// or any part of the request body.
#[tokio::test]
async fn authentication_errors_do_not_leak_secrets_or_the_request() {
    const APP_SECRET: &str = "app-secret-QQ7F2K9";
    const VERIFY_TOKEN: &str = "verify-token-QQ7F2K9";
    const API_TOKEN: &str = "api-token-QQ7F2K9";
    const SUPPLIED_BEARER: &str = "supplied-bearer-ZXQMARKER";
    const SUPPLIED_SIGNATURE: &str = "sha256=ZXQMARKERSIGNATURE";
    const SUPPLIED_VERIFY_TOKEN: &str = "supplied-verify-ZXQMARKER";
    const CHALLENGE: &str = "challenge-ZXQMARKER";
    const BODY: &str = r#"{"object":"whatsapp_business_account","note":"body-ZXQMARKER"}"#;

    let wh = WebhookConfig { app_secret: APP_SECRET.into(), verify_token: VERIFY_TOKEN.into() };
    let config = Config::default().with_whatsapp(wh.clone()).with_messenger(wh).with_api_token(API_TOKEN);
    let computed = msgkit_core::webhook::sign(APP_SECRET.as_bytes(), BODY.as_bytes());
    let computed_hex = computed.trim_start_matches("sha256=").to_owned();

    let handshake = format!(
        "/webhooks/whatsapp?hub.mode=subscribe&hub.verify_token={SUPPLIED_VERIFY_TOKEN}&hub.challenge={CHALLENGE}"
    );
    let requests = vec![
        (
            "wrong bearer token",
            Request::get("/v1/events")
                .header(header::AUTHORIZATION, format!("Bearer {SUPPLIED_BEARER}"))
                .body(Body::empty())
                .unwrap(),
            StatusCode::UNAUTHORIZED,
            codes::UNAUTHORIZED,
        ),
        (
            "wrong signature",
            Request::post("/webhooks/whatsapp")
                .header("x-hub-signature-256", SUPPLIED_SIGNATURE)
                .body(Body::from(BODY))
                .unwrap(),
            StatusCode::UNAUTHORIZED,
            codes::SIGNATURE_INVALID,
        ),
        (
            "signature of the wrong body",
            Request::post("/webhooks/whatsapp")
                .header(
                    "x-hub-signature-256",
                    msgkit_core::webhook::sign(APP_SECRET.as_bytes(), b"another body"),
                )
                .body(Body::from(BODY))
                .unwrap(),
            StatusCode::UNAUTHORIZED,
            codes::SIGNATURE_INVALID,
        ),
        (
            "wrong verify token",
            Request::get(handshake).body(Body::empty()).unwrap(),
            StatusCode::FORBIDDEN,
            codes::VERIFICATION_FAILED,
        ),
    ];

    let forbidden = [
        APP_SECRET,
        VERIFY_TOKEN,
        API_TOKEN,
        SUPPLIED_BEARER,
        SUPPLIED_SIGNATURE,
        SUPPLIED_VERIFY_TOKEN,
        CHALLENGE,
        "ZXQMARKER",
        "body-",
        computed.as_str(),
        computed_hex.as_str(),
    ];
    for (name, request, status, code) in requests {
        let app = router(AppState::new(config.clone()).unwrap());
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), status, "{name}");
        let mut headers = String::new();
        for (key, value) in response.headers() {
            writeln!(headers, "{key}: {}", value.to_str().unwrap_or("")).unwrap();
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["code"], code, "{name}");
        for value in forbidden {
            assert!(!text.contains(value), "{name}: the body contains {value}: {text}");
            assert!(!headers.contains(value), "{name}: a header contains {value}: {headers}");
        }
    }
}
