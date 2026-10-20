//! Runs the HTTP-reachable part of the language-independent conformance suite (`conformance/`) against the
//! service router: the three import endpoints must return exactly the `data` of the expected envelope, and the
//! import cases that fail must be refused with the same error `code` as the library. Operations without an HTTP
//! endpoint (webhook normalization without a store, signatures, the other errors) are covered by the other runners.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use msgkit_core::limits::Limits;
use msgkit_service::{router, AppState, Config};
use serde_json::Value;
use tower::ServiceExt;

#[path = "../../msgkit-core/tests/support/canonical.rs"]
mod canonical;

fn cases_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases")
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn query(options: &Value) -> String {
    let Some(map) = options.as_object() else { return String::new() };
    let pairs: Vec<String> = map
        .iter()
        // Limits are the operator's setting on the service, not a request option (see `limits_of`).
        .filter(|(k, v)| !v.is_null() && k.as_str() != "limits")
        .map(|(k, v)| {
            let text = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
            format!("{k}={}", percent_encode(&text))
        })
        .collect();
    if pairs.is_empty() {
        String::new()
    } else {
        format!("?{}", pairs.join("&"))
    }
}

#[tokio::test]
async fn the_http_service_matches_the_import_cases() {
    let default_app = router(AppState::new(Config::default()).unwrap());
    let mut names: Vec<PathBuf> =
        fs::read_dir(cases_dir()).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    names.sort();

    let mut checked = 0;
    let mut checked_errors = 0;
    let mut failures = Vec::new();
    for dir in names {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let spec: Value = serde_json::from_str(&fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
        let expected: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
        let expects_error = expected["ok"] != true;
        let read = |files: &Value, key: &str| {
            files.get(key).and_then(Value::as_str).map(|n| fs::read_to_string(dir.join(n)).unwrap())
        };
        let options = &spec["input"]["options"];
        let (path, content_type, body) = match spec["op"].as_str().unwrap() {
            "parse_whatsapp_export" => {
                let text = read(&spec["files"], "text")
                    .or_else(|| spec["input"]["text"].as_str().map(str::to_owned));
                // Error cases that fail before the text is read (a missing `text`) have no HTTP equivalent.
                let Some(text) = text else { continue };
                ("/v1/import/whatsapp-export", "text/plain; charset=utf-8", text)
            }
            "parse_facebook_export" => {
                let thread = read(&spec["json_files"], "thread")
                    .unwrap_or_else(|| spec["input"]["thread"].to_string());
                ("/v1/import/facebook-export", "application/json", thread)
            }
            "normalize_device" => {
                let batch =
                    read(&spec["json_files"], "batch").unwrap_or_else(|| spec["input"]["batch"].to_string());
                ("/v1/import/device", "application/json", batch)
            }
            _ => continue,
        };
        // A case that sets `limits` runs against a service configured with them, as an operator would. A limit
        // the service refuses to start with (0) has no HTTP equivalent: `AppState::new` rejects it at startup.
        let app = match options.get("limits") {
            Some(limits) => {
                let limits: Limits = serde_json::from_value(limits.clone()).unwrap();
                if limits.validate().is_err() {
                    continue;
                }
                router(AppState::new(Config::default().with_limits(limits)).unwrap())
            }
            None => default_app.clone(),
        };
        let request = Request::post(format!("{path}{}", query(options)))
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        if expects_error {
            // The service must refuse the request with the same stable code as the library (docs/ERRORS.md).
            let actual: Value = serde_json::from_slice(&bytes).unwrap();
            if !status.is_client_error() || actual["code"] != expected["error"]["code"] {
                failures.push(format!(
                    "{name}: HTTP {status}, code {} (want {})",
                    actual["code"], expected["error"]["code"]
                ));
            }
            checked_errors += 1;
            continue;
        }
        if status != StatusCode::OK {
            failures.push(format!("{name}: HTTP {status}"));
            continue;
        }
        let actual: Value = serde_json::from_slice(&bytes).unwrap();
        let (mut got, mut want) = (String::new(), String::new());
        canonical::canonical(&actual, &mut got);
        canonical::canonical(&expected["data"], &mut want);
        if got != want {
            failures.push(format!("{name}: response differs from expected data"));
        }
        checked += 1;
    }
    assert!(checked >= 26, "only {checked} cases were checked over HTTP");
    assert!(checked_errors >= 12, "only {checked_errors} error cases were checked over HTTP");
    assert!(failures.is_empty(), "HTTP conformance failures:\n{}", failures.join("\n"));
}
