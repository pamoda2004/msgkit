//! Redacting events and import reports.

use msgkit_core::{Event, ImportReport};
use msgkit_redact::{IdPolicy, Kind, Mask, RedactConfig, Redactor};
use serde_json::{json, Value};

const KEY: &[u8] = b"a-test-key-that-is-not-a-secret";

fn message() -> Event {
    serde_json::from_value(json!({
        "type": "message",
        "id": "0123456789abcdef0123456789abcdef",
        "channel": "whatsapp",
        "external_id": "wamid.TEST1",
        "conversation_id": "+94710000000",
        "direction": "inbound",
        "timestamp": "2024-02-01T10:00:00Z",
        "sender": {"id": "+94700000000", "display_name": "Kamal"},
        "recipients": [{"id": "Nimal", "display_name": "Nimal"}],
        "body": "Kamal here, call +94700000000, NIC 850010000V, kamal@example.org",
        "attachments": [{"kind": "image", "uri": "https://example.org/files/0700000000.jpg", "file_name": "card 4111111111111111.png", "source_id": "media-1"}],
        "raw": {"from": "+94700000000", "text": {"body": "call 0710000000"}, "n": 5}
    }))
    .unwrap()
}

fn call() -> Event {
    serde_json::from_value(json!({
        "type": "call",
        "id": "fedcba9876543210fedcba9876543210",
        "channel": "cellular",
        "direction": "outbound",
        "timestamp": "2024-02-01T10:00:00Z",
        "duration_secs": 42,
        "outcome": "answered",
        "video": false,
        "participants": [{"id": "+94700000000"}, {"id": "self"}]
    }))
    .unwrap()
}

fn as_json(e: &Event) -> Value {
    serde_json::to_value(e).unwrap()
}

#[test]
fn a_message_is_masked_where_identifiers_can_be_and_kept_elsewhere() {
    let r = Redactor::new(RedactConfig::default()).unwrap();
    let out = as_json(&r.redact_event(&message()));
    assert_eq!(out["body"], "Kamal here, call [PHONE], NIC [NIC], [EMAIL]");
    assert_eq!(out["sender"], json!({"id": "[PHONE]", "display_name": "Kamal"}));
    assert_eq!(out["recipients"][0], json!({"id": "Nimal", "display_name": "Nimal"}));
    assert_eq!(out["conversation_id"], "[PHONE]");
    assert_eq!(out["attachments"][0]["file_name"], "card [CARD].png");
    assert_eq!(out["attachments"][0]["uri"], "https://example.org/files/[PHONE].jpg");
    // Kept: structure, time, platform ids and, by default, the event id.
    assert_eq!(out["id"], "0123456789abcdef0123456789abcdef");
    assert_eq!(out["external_id"], "wamid.TEST1");
    assert_eq!(out["attachments"][0]["source_id"], "media-1");
    assert_eq!(out["timestamp"], "2024-02-01T10:00:00Z");
    assert_eq!(out["direction"], "inbound");
}

#[test]
fn raw_is_dropped_by_default_and_masked_when_kept() {
    let dropped = as_json(&Redactor::new(RedactConfig::default()).unwrap().redact_event(&message()));
    assert!(dropped.get("raw").is_none(), "{dropped}");

    let kept = Redactor::new(RedactConfig::default().with_drop_raw(false)).unwrap();
    let out = as_json(&kept.redact_event(&message()));
    assert_eq!(out["raw"], json!({"from": "[PHONE]", "text": {"body": "call [PHONE]"}, "n": 5}));
}

#[test]
fn calls_are_masked_too() {
    let r = Redactor::new(RedactConfig::default()).unwrap();
    let out = as_json(&r.redact_event(&call()));
    assert_eq!(out["participants"], json!([{"id": "[PHONE]"}, {"id": "self"}]));
    assert_eq!(out["duration_secs"], 42);
    assert_eq!(out["outcome"], "answered");
}

#[test]
fn display_names_and_listed_names_are_masked_on_request() {
    let r =
        Redactor::new(RedactConfig::default().with_names(["Kamal", "Nimal"]).with_redact_display_names(true))
            .unwrap();
    let out = as_json(&r.redact_event(&message()));
    assert_eq!(out["body"], "[NAME] here, call [PHONE], NIC [NIC], [EMAIL]");
    assert_eq!(out["sender"], json!({"id": "[PHONE]", "display_name": "[NAME]"}));
    // An id that is the display name is masked as a name.
    assert_eq!(out["recipients"][0], json!({"id": "[NAME]", "display_name": "[NAME]"}));
}

#[test]
fn pseudonyms_keep_participants_distinguishable_and_ids_can_be_pseudonymized() {
    let r = Redactor::new(
        RedactConfig::default()
            .with_mask(Mask::Pseudonym(KEY.to_vec()))
            .with_id_policy(IdPolicy::Pseudonymize),
    )
    .unwrap();
    let a = as_json(&r.redact_event(&message()));
    let b = as_json(&r.redact_event(&message()));
    assert_eq!(a, b, "deterministic");
    assert_ne!(a["id"], "0123456789abcdef0123456789abcdef");
    assert_eq!(a["id"].as_str().unwrap().len(), 32);
    assert_ne!(as_json(&r.redact_event(&call()))["id"], a["id"]);
    // The same number is the same token in the sender and in the body.
    let sender = a["sender"]["id"].as_str().unwrap().to_owned();
    assert!(a["body"].as_str().unwrap().contains(&sender), "{a}");
    // Different numbers are different tokens.
    assert_ne!(a["sender"]["id"], a["conversation_id"]);
}

#[test]
fn a_report_is_redacted_with_a_count_per_kind_and_its_warnings_are_kept() {
    let r = Redactor::new(RedactConfig::default()).unwrap();
    let report =
        ImportReport { events: vec![message(), call()], warnings: vec!["2 line(s) were ignored".into()] };
    let (out, summary) = r.redact_report(&report);
    assert_eq!(out.events.len(), 2);
    assert_eq!(out.warnings, report.warnings);
    assert_eq!(summary.counts[&Kind::Phone], 5, "{summary:?}");
    assert_eq!(summary.counts[&Kind::Nic], 1);
    assert_eq!(summary.counts[&Kind::Email], 1);
    assert_eq!(summary.counts[&Kind::Card], 1);
    assert_eq!(summary.total(), summary.counts.values().sum::<usize>());
    // Nothing that was masked is left anywhere in the output.
    let text = serde_json::to_string(&out).unwrap();
    for original in
        ["94700000000", "94710000000", "850010000V", "kamal@example.org", "4111111111111111", "0700000000"]
    {
        assert!(!text.contains(original), "{original} survived in {text}");
    }
}

#[test]
fn redacting_an_event_twice_changes_nothing() {
    let r = Redactor::new(RedactConfig::default().with_names(["Kamal"])).unwrap();
    let once = r.redact_event(&message());
    assert_eq!(r.redact_event(&once), once);
}
