//! Covers the JSON entry point, the model helpers and the less common webhook and device branches
//! in the core crate itself (the language bindings only exercise them through other test suites).

use msgkit_core::webhook;
use msgkit_core::{api, Channel, Error, Event};
use serde_json::{json, Value};

fn call(op: &str, input: &Value) -> Value {
    serde_json::from_str(&api::dispatch(op, &input.to_string())).unwrap()
}

fn data(op: &str, input: &Value) -> Value {
    let out = call(op, input);
    assert_eq!(out["ok"], true, "{op} failed: {out}");
    out["data"].clone()
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e[key].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn every_operation_is_reachable_through_dispatch() {
    let version = data("version", &json!({}));
    for op in version["operations"].as_array().unwrap() {
        let op = op.as_str().unwrap();
        // An empty input may be rejected, but an advertised operation is never "unknown".
        let out = call(op, &json!({}));
        assert_ne!(out["error"]["code"], "unknown_op", "{op} is advertised but not routed");
    }
    assert_eq!(call("no_such_op", &json!({}))["error"]["code"], "unknown_op");
}

#[test]
fn dispatch_parses_each_source() {
    let thread: Value = serde_json::from_str(include_str!("fixtures/facebook_thread.json")).unwrap();
    let facebook = data("parse_facebook_export", &json!({ "thread": thread }));
    assert_ne!(facebook["events"].as_array().unwrap().len(), 0);

    let whatsapp = data("parse_whatsapp_export", &json!({ "text": "1/2/24, 10:00 - Alex: hello\n" }));
    assert_eq!(whatsapp["events"].as_array().unwrap().len(), 1);

    let device = data(
        "normalize_device",
        &json!({ "batch": {
            "sms": [{ "address": "+15550100111", "body": "hi", "date_ms": 1_700_000_000_000_i64, "type": 1 }],
            "calls": [{ "number": "+15550100111", "date_ms": 1_700_000_100_000_i64, "type": 1 }]
        }}),
    );
    assert_eq!(device["events"].as_array().unwrap().len(), 2);

    let bad = call("parse_facebook_export", &json!({ "thread": 5 }));
    assert_eq!(bad["error"]["code"], "parse_error");
    let bad = call("normalize_whatsapp_webhook", &json!({ "payload": { "object": "page" } }));
    assert_eq!(bad["error"]["code"], "parse_error");
    let bad =
        call("normalize_messenger_webhook", &json!({ "payload": { "object": "whatsapp_business_account" } }));
    assert_eq!(bad["error"]["code"], "parse_error");
}

#[test]
fn error_codes_are_stable() {
    assert_eq!(Error::InvalidInput(String::new()).code(), "invalid_input");
    assert_eq!(Error::Parse(String::new()).code(), "parse_error");
    assert_eq!(Error::UnknownOp(String::new()).code(), "unknown_op");
    assert_eq!(Error::Storage(String::new()).code(), "storage_error");
    let from_json: Error = serde_json::from_str::<Value>("{").unwrap_err().into();
    assert_eq!(from_json.code(), "invalid_input");
}

#[test]
fn channels_and_event_accessors() {
    assert_eq!(Channel::Sms.as_str(), "sms");
    assert_eq!(Channel::Cellular.as_str(), "cellular");
    assert_eq!(Channel::Whatsapp.as_str(), "whatsapp");
    assert_eq!(Channel::Messenger.as_str(), "messenger");

    let batch = serde_json::from_value(json!({
        "sms": [{ "address": "+15550100111", "body": "hi", "date_ms": 1_700_000_000_000_i64, "type": 1 }],
        "calls": [{ "number": "+15550100111", "date_ms": 1_700_000_100_000_i64, "type": 3 }]
    }))
    .unwrap();
    let report = msgkit_core::device::normalize(&batch, &msgkit_core::device::DeviceOptions::default());
    let kinds: Vec<_> = report.events.iter().map(|e| (e.kind(), e.channel())).collect();
    assert_eq!(kinds, [("message", Channel::Sms), ("call", Channel::Cellular)]);
    for event in &report.events {
        assert_ne!(event.id().len(), 0);
        assert!(event.timestamp().timestamp() > 1_600_000_000);
    }
    assert!(matches!(report.events[1], Event::Call(_)));
}

#[test]
fn every_call_type_is_mapped() {
    let mut calls: Vec<Value> = (0..=7)
        .map(|kind| {
            json!({ "number": "+15550100111", "date_ms": 1_700_000_000_000_i64 + i64::from(kind),
                    "type": kind, "duration_secs": if kind == 2 { 30 } else { 0 } })
        })
        .collect();
    calls.push(json!({ "number": "+15550100111", "date_ms": 1_700_000_050_000_i64, "type": 2 }));
    let out = data("normalize_device", &json!({ "batch": { "calls": calls } }));
    let outcomes = strings(&out, "outcome");
    assert_eq!(outcomes.len(), 9);
    for expected in ["answered", "missed", "voicemail", "rejected", "blocked", "unknown"] {
        assert!(outcomes.iter().any(|o| o == expected), "missing outcome {expected}: {outcomes:?}");
    }
}

#[test]
fn device_rows_with_invalid_timestamps_and_unknown_types_are_handled() {
    let out = data(
        "normalize_device",
        &json!({ "batch": {
            "sms": [
                { "address": "x", "body": "a", "date_ms": i64::MAX, "type": 1 },
                { "address": "+15550100111", "body": "b", "date_ms": 1_700_000_000_000_i64, "type": 2 },
                { "address": "+15550100111", "body": "c", "date_ms": 1_700_000_000_001_i64, "type": 9 }
            ],
            "calls": [{ "number": "+15550100111", "date_ms": i64::MAX, "type": 1 }]
        }, "options": { "own_number": "+15550100222" } }),
    );
    let directions = strings(&out, "direction");
    assert_eq!(directions.len(), 2);
    assert!(directions.iter().any(|d| d == "outbound") && directions.iter().any(|d| d == "unknown"));
}

fn whatsapp_payload(messages: &Value) -> Value {
    json!({ "object": "whatsapp_business_account", "entry": [{ "changes": [{ "value": {
        "metadata": { "display_phone_number": "15550100999" },
        "contacts": [{ "wa_id": "15550100111", "profile": { "name": "Alex" } }],
        "messages": messages
    }}]}]})
}

#[test]
fn whatsapp_webhook_message_kinds_and_problems() {
    let messages = json!([
        { "from": "15550100111", "id": "m1", "timestamp": "1700000000", "type": "text", "text": { "body": "hello" } },
        { "from": "15550100111", "id": "m2", "timestamp": "1700000001", "type": "button", "button": { "text": "Yes" } },
        { "from": "15550100111", "id": "m3", "timestamp": "1700000002", "type": "interactive",
          "interactive": { "button_reply": { "title": "A" } } },
        { "from": "15550100111", "id": "m4", "timestamp": "1700000003", "type": "interactive",
          "interactive": { "list_reply": { "title": "B" } } },
        { "from": "15550100111", "id": "m5", "timestamp": "1700000004", "type": "image",
          "image": { "id": "i", "mime_type": "image/jpeg", "caption": "pic" } },
        { "from": "15550100111", "id": "m6", "timestamp": "1700000005", "type": "video", "video": { "id": "v" } },
        { "from": "15550100111", "id": "m7", "timestamp": "1700000006", "type": "audio", "audio": { "id": "a" } },
        { "from": "15550100111", "id": "m8", "timestamp": "1700000007", "type": "voice", "voice": { "id": "a2" } },
        { "from": "15550100111", "id": "m9", "timestamp": "1700000008", "type": "document",
          "document": { "id": "d", "filename": "f.pdf" } },
        { "from": "15550100111", "id": "m10", "timestamp": "1700000009", "type": "sticker", "sticker": { "id": "s" } },
        { "from": "15550100111", "id": "m11", "timestamp": "1700000010", "type": "location" },
        { "from": "15550100111", "id": "m12", "timestamp": "soon", "type": "text" },
        { "id": "m13", "type": "text" }
    ]);
    let report = webhook::normalize_whatsapp(&whatsapp_payload(&messages)).unwrap();
    assert_eq!(report.events.len(), 11);
    assert_eq!(report.warnings.len(), 3, "{:?}", report.warnings);
    let attachments: usize = report
        .events
        .iter()
        .map(|e| match e {
            Event::Message(m) => m.attachments.len(),
            Event::Call(_) => 0,
        })
        .sum();
    assert_eq!(attachments, 6);
}

#[test]
fn messenger_webhook_problems_are_warnings() {
    let payload = json!({ "object": "page", "entry": [{ "messaging": [
        { "sender": { "id": "1" }, "recipient": { "id": "2" }, "timestamp": 1_700_000_000_000_i64,
          "message": { "mid": "m1", "text": "hi" } },
        { "sender": { "id": "1" }, "timestamp": 1_700_000_000_000_i64, "message": { "mid": "m2" } },
        { "sender": { "id": "1" }, "recipient": { "id": "2" }, "timestamp": i64::MAX, "message": { "mid": "m3" } },
        { "sender": { "id": "1" }, "recipient": { "id": "2" } }
    ]}]});
    let report = webhook::normalize_messenger(&payload).unwrap();
    assert_eq!(report.events.len(), 1);
    assert_eq!(report.warnings.len(), 2, "{:?}", report.warnings);
}

#[test]
fn malformed_signature_headers_are_rejected() {
    assert!(!webhook::verify_signature(b"secret", b"body", "deadbeef"));
    assert!(!webhook::verify_signature(b"secret", b"body", "sha256=not-hex"));
    assert!(!webhook::verify_signature(b"secret", b"body", "sha256="));
}

#[test]
fn facebook_invalid_timestamps_are_counted() {
    let thread = json!({ "participants": [{ "name": "A" }, { "name": "B" }], "messages": [
        { "sender_name": "A", "timestamp_ms": i64::MAX, "content": "x" },
        { "sender_name": "B", "timestamp_ms": 1_700_000_000_000_i64, "content": "y" }
    ]});
    let out = data("parse_facebook_export", &json!({ "thread": thread }));
    assert_eq!(out["events"].as_array().unwrap().len(), 1);
    assert!(out["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("skipped")));
}
