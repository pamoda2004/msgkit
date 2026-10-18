//! Error text must never repeat the input: messages can contain personal data and error strings end up in
//! logs, crash reports and HTTP responses.
//!
//! Every malformed input below carries a unique marker. If the marker appears anywhere in a response, an
//! error message quotes caller data.

use msgkit_core::api;
use serde_json::{json, Value};

const MARKER: &str = "ZXQMARKERZXQ";

fn assert_no_leak(op: &str, input: &str) {
    let response = api::dispatch(op, input);
    assert!(!response.contains(MARKER), "{op} echoed the input: {response}");
}

fn inputs() -> Vec<(&'static str, String)> {
    let m = MARKER;
    let mut cases: Vec<(&'static str, String)> = Vec::new();
    let mut add = |op: &'static str, value: Value| cases.push((op, value.to_string()));

    // Wrong types and unknown enum values in every operation.
    add("parse_whatsapp_export", json!({ "text": { m: 1 } }));
    add("parse_whatsapp_export", json!({ "text": [m] }));
    add("parse_whatsapp_export", json!({ "text": "x", "options": { "date_order": m } }));
    add("parse_whatsapp_export", json!({ "text": "x", "options": { "utc_offset_minutes": m } }));
    add("parse_whatsapp_export", json!({ "text": "x", "options": { "self_name": { m: 1 } } }));
    add("parse_whatsapp_export", json!({ "text": "x", "options": { m: 1 } }));
    add("parse_facebook_export", json!({ "thread": m }));
    add("parse_facebook_export", json!({ "thread": { "messages": m } }));
    add("parse_facebook_export", json!({ "thread": { "messages": [{ "sender_name": 1, "content": m }] } }));
    add("parse_facebook_export", json!({ "thread": { "messages": [{ "timestamp_ms": m }] } }));
    add("normalize_device", json!({ "batch": m }));
    add("normalize_device", json!({ "batch": { "sms": [{ "address": m }] } }));
    add(
        "normalize_device",
        json!({ "batch": { "sms": [{ "address": 1, "body": m, "date_ms": m, "type": 1 }] } }),
    );
    add("normalize_device", json!({ "batch": { "calls": [{ "number": m, "date_ms": 1, "type": m }] } }));
    add("normalize_whatsapp_webhook", json!({ "payload": m }));
    add("normalize_whatsapp_webhook", json!({ "payload": { "object": m } }));
    add("normalize_messenger_webhook", json!({ "payload": { "object": m } }));
    add("normalize_messenger_webhook", json!({ "payload": [m] }));
    add("verify_signature", json!({ "secret": m }));
    add("verify_signature", json!({ "secret": 1, "body": m, "signature": m }));
    add("verify_signature", json!({ "secret": m, "body": m, "signature": { m: 1 } }));

    // Not JSON at all, and JSON of the wrong top-level type.
    for op in [
        "parse_whatsapp_export",
        "parse_facebook_export",
        "normalize_device",
        "normalize_whatsapp_webhook",
        "normalize_messenger_webhook",
        "verify_signature",
    ] {
        cases.push((op, format!("{{{m}")));
        cases.push((op, format!("[\"{m}\"]")));
        cases.push((op, format!("\"{m}\"")));
        cases.push((op, format!("{{\"text\": \"{m}\", ")));
        cases.push((op, m.to_owned()));
    }
    cases
}

#[test]
fn malformed_inputs_never_echo_caller_data() {
    for (op, input) in inputs() {
        assert_no_leak(op, &input);
    }
}

#[test]
fn a_facebook_export_that_is_not_json_does_not_echo_it() {
    let report = msgkit_core::facebook::parse(
        &format!("{{\"messages\": [\"{MARKER}\""),
        &msgkit_core::facebook::FacebookExportOptions::default(),
    );
    assert!(!report.unwrap_err().to_string().contains(MARKER));
    let report = msgkit_core::facebook::parse(
        &format!("{{\"messages\": \"{MARKER}\"}}"),
        &msgkit_core::facebook::FacebookExportOptions::default(),
    );
    assert!(!report.unwrap_err().to_string().contains(MARKER));
}

#[test]
fn unknown_operations_report_only_a_code_and_the_operation_name() {
    let response: Value = serde_json::from_str(&api::dispatch("no_such_op", "{}")).unwrap();
    assert_eq!(response["error"]["code"], "unknown_op");
}

#[test]
fn json_error_descriptions_keep_only_the_category_and_a_missing_field_name() {
    let missing = api::dispatch("parse_whatsapp_export", "{\"options\":{}}");
    assert!(missing.contains("missing field `text`"), "{missing}");
    assert!(
        !missing.contains("column"),
        "positions inside request documents differ between bindings: {missing}"
    );
    let syntax: Value = serde_json::from_str(&api::dispatch("parse_whatsapp_export", "{nope")).unwrap();
    assert_eq!(syntax["error"]["code"], "invalid_input");
    assert!(syntax["error"]["message"].as_str().unwrap().starts_with("invalid input: invalid JSON syntax"));
}
