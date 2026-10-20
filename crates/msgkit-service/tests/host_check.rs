//! The Host-header check (DNS rebinding defence) and the refusal to listen beyond loopback without a token.

use std::net::SocketAddr;
use std::process::Command;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use msgkit_service::{check_bind, router, AppState, Config, WebhookConfig};
use serde_json::Value;
use tower::ServiceExt;

const TOKEN: &str = "a-long-random-token";

fn app(config: Config) -> Router {
    router(AppState::new(config).unwrap())
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn get(path: &str, host: Option<&str>) -> Request<Body> {
    let mut req = Request::get(path);
    if let Some(host) = host {
        req = req.header(header::HOST, host);
    }
    req.body(Body::empty()).unwrap()
}

fn assert_refused(status: StatusCode, body: &str, host: &str) {
    assert_eq!(status, StatusCode::FORBIDDEN, "{host}: {body}");
    let json: Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["code"], "host_not_allowed", "{host}: {body}");
    assert!(!body.contains(host), "the error repeats the rejected host: {body}");
}

#[tokio::test]
async fn the_machines_own_names_are_accepted_without_a_token() {
    let app = app(Config::default());
    for host in [
        "localhost",
        "localhost:8080",
        "LOCALHOST:8080",
        "localhost.",
        "app.localhost:3000",
        "127.0.0.1",
        "127.0.0.1:8080",
        "127.0.0.2:9",
        "[::1]",
        "[::1]:8080",
    ] {
        let (status, body) = send(&app, get("/v1/version", Some(host))).await;
        assert_eq!(status, StatusCode::OK, "{host}: {body}");
    }
}

#[tokio::test]
async fn a_request_without_a_host_header_is_let_through() {
    // Browsers always send a Host header, so a request without one cannot be a rebinding attack.
    let (status, _) = send(&app(Config::default()), get("/v1/version", None)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn any_other_host_is_refused_on_every_v1_route_without_a_token() {
    let app = app(Config::default());
    for host in [
        "evil.example.com",
        "evil.example.com:8080",
        "EVIL.EXAMPLE.COM",
        "localhost.evil.example.com",
        "127.0.0.1.evil.example.com",
        "notlocalhost",
        "192.168.1.5:8080",
        "10.0.0.1",
        "[2001:db8::1]:8080",
        "0.0.0.0:8080",
    ] {
        for (method, path) in [
            ("GET", "/v1/version"),
            ("GET", "/v1/events"),
            ("GET", "/v1/events?drain=true"),
            ("POST", "/v1/events/ack"),
            ("POST", "/v1/import/whatsapp-export"),
            ("POST", "/v1/import/facebook-export"),
            ("POST", "/v1/import/device"),
        ] {
            let req = Request::builder()
                .method(method)
                .uri(path)
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap();
            let (status, body) = send(&app, req).await;
            assert_refused(status, &body, host);
        }
    }
}

#[tokio::test]
async fn allowed_hosts_are_accepted_ignoring_case_and_port() {
    let app = app(Config::default().with_allowed_hosts(["Msgkit.Example.com", "10.0.0.7:9999"]));
    for host in ["msgkit.example.com", "MSGKIT.example.com:443", "10.0.0.7", "10.0.0.7:1"] {
        let (status, body) = send(&app, get("/v1/version", Some(host))).await;
        assert_eq!(status, StatusCode::OK, "{host}: {body}");
    }
    let (status, body) = send(&app, get("/v1/version", Some("other.example.com"))).await;
    assert_refused(status, &body, "other.example.com");
    // A suffix or a prefix of an allowed name is not the name.
    let (status, body) = send(&app, get("/v1/version", Some("evil-msgkit.example.com"))).await;
    assert_refused(status, &body, "evil-msgkit.example.com");
}

#[tokio::test]
async fn with_a_token_the_host_does_not_matter_but_the_token_does() {
    let app = app(Config::default().with_api_token(TOKEN));
    let authed = Request::get("/v1/version")
        .header(header::HOST, "msgkit.example.com")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, authed).await.0, StatusCode::OK);
    let (status, body) = send(&app, get("/v1/version", Some("msgkit.example.com"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "unauthorized");
}

#[tokio::test]
async fn health_and_webhooks_are_never_host_checked() {
    let wh = WebhookConfig { app_secret: "app-secret".into(), verify_token: "verify-me".into() };
    let app = app(Config::default().with_whatsapp(wh));
    let host = "graph.facebook.example";
    assert_eq!(send(&app, get("/healthz", Some(host))).await.0, StatusCode::OK);
    // The handshake and an unsigned delivery reach their own checks (403 verification_failed, 401 signature).
    let (status, body) = send(
        &app,
        get("/webhooks/whatsapp?hub.mode=subscribe&hub.verify_token=x&hub.challenge=1", Some(host)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "verification_failed");
    let post = Request::post("/webhooks/whatsapp").header(header::HOST, host).body(Body::from("{}")).unwrap();
    let (status, body) = send(&app, post).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["code"], "signature_invalid");
}

// ---- refusing to listen beyond loopback without a token ----

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

#[test]
fn a_non_loopback_bind_needs_a_token() {
    let open = Config::default();
    let token = Config::default().with_api_token(TOKEN);
    for bind in ["127.0.0.1:8080", "[::1]:8080", "127.0.0.2:1"] {
        assert!(check_bind(addr(bind), &open).is_ok(), "{bind}");
    }
    for bind in
        ["0.0.0.0:8080", "[::]:8080", "192.168.1.5:8080", "10.0.0.1:80", "203.0.113.9:80", "[2001:db8::1]:80"]
    {
        let e = check_bind(addr(bind), &open).unwrap_err();
        assert!(
            e.contains("MSGKIT_API_TOKEN") && e.contains(bind) && e.contains("openssl rand -hex 32"),
            "{e}"
        );
        assert!(check_bind(addr(bind), &token).is_ok(), "{bind} with a token");
    }
}

#[test]
fn the_binary_refuses_to_start_on_a_non_loopback_address_without_a_token() {
    let out = Command::new(env!("CARGO_BIN_EXE_msgkit-service"))
        .env("MSGKIT_BIND", "0.0.0.0:0")
        .env_remove("MSGKIT_API_TOKEN")
        .env_remove("MSGKIT_DB_PATH")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("refusing to listen on 0.0.0.0:0") && err.contains("MSGKIT_API_TOKEN"), "{err}");
}

#[test]
fn a_bad_allowed_hosts_entry_stops_the_service_at_startup() {
    for bad in ["http://evil.example.com", "*.example.com", "evil.example.com/path", "a b"] {
        let out = Command::new(env!("CARGO_BIN_EXE_msgkit-service"))
            .env("MSGKIT_BIND", "127.0.0.1:0")
            .env("MSGKIT_ALLOWED_HOSTS", bad)
            .env_remove("MSGKIT_API_TOKEN")
            .output()
            .unwrap();
        assert!(!out.status.success(), "{bad}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("MSGKIT_ALLOWED_HOSTS entries must be host names"), "{err}");
        assert!(!err.contains(bad), "the entry is echoed: {err}");
    }
}
