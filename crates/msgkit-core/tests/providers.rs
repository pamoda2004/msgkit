use msgkit_core::device::{self, DeviceBatch, DeviceOptions};
use msgkit_core::facebook::{self, FacebookExportOptions};
use msgkit_core::webhook;
use msgkit_core::*;

fn calls(r: &ImportReport) -> Vec<&Call> {
    r.events
        .iter()
        .filter_map(|e| match e {
            Event::Call(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn messages(r: &ImportReport) -> Vec<&Message> {
    r.events
        .iter()
        .filter_map(|e| match e {
            Event::Message(m) => Some(m),
            _ => None,
        })
        .collect()
}

// ---- Facebook export ------------------------------------------------------

const FB: &str = include_str!("fixtures/facebook_thread.json");

#[test]
fn facebook_messages_are_sorted_oldest_first_and_repaired() {
    let opts = FacebookExportOptions { self_name: Some("Alice Perera".into()), ..Default::default() };
    let r = facebook::parse(FB, &opts).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(r.events.len(), 5);

    let msgs = messages(&r);
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0].body.as_deref(), Some("Hi"));
    assert_eq!(msgs[0].direction, Direction::Inbound);
    assert_eq!(msgs[1].attachments[0].kind, AttachmentKind::Image);
    assert_eq!(msgs[2].body.as_deref(), Some("café ❤"));
    assert_eq!(msgs[2].direction, Direction::Outbound);
    assert_eq!(msgs[2].conversation_id.as_deref(), Some("inbox/bob_abc123"));
    assert_eq!(msgs[2].recipients[0].id, "Bob");
}

#[test]
fn facebook_calls() {
    let r = facebook::parse(FB, &FacebookExportOptions::default()).unwrap();
    let c = calls(&r);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].outcome, CallOutcome::Missed);
    assert!(c[0].video);
    assert_eq!(c[1].outcome, CallOutcome::Answered);
    assert_eq!(c[1].duration_secs, 125);
}

#[test]
fn facebook_rejects_non_exports() {
    assert!(facebook::parse("[1,2,3]", &FacebookExportOptions::default()).is_err());
    assert!(facebook::parse("not json", &FacebookExportOptions::default()).is_err());
}

// ---- WhatsApp Cloud API webhook -------------------------------------------

const WA: &str = include_str!("fixtures/whatsapp_webhook.json");

#[test]
fn whatsapp_webhook_messages() {
    let payload: serde_json::Value = serde_json::from_str(WA).unwrap();
    let r = webhook::normalize_whatsapp(&payload).unwrap();
    let msgs = messages(&r);
    assert_eq!(msgs.len(), 3, "statuses are ignored, reaction is kept");

    assert_eq!(msgs[0].body.as_deref(), Some("Hello"));
    assert_eq!(msgs[0].sender.id, "+94771234567");
    assert_eq!(msgs[0].sender.display_name.as_deref(), Some("Kamal"));
    assert_eq!(msgs[0].recipients[0].id, "+15550001111");
    assert_eq!(msgs[0].direction, Direction::Inbound);
    assert_eq!(msgs[0].external_id.as_deref(), Some("wamid.AAA"));
    assert!(msgs[0].raw.is_some());

    assert_eq!(msgs[1].body.as_deref(), Some("look"));
    assert_eq!(msgs[1].attachments[0].source_id.as_deref(), Some("MEDIA1"));
    assert_eq!(msgs[1].attachments[0].mime_type.as_deref(), Some("image/jpeg"));

    assert!(r.warnings.iter().any(|w| w.contains("reaction")));
}

#[test]
fn whatsapp_webhook_ids_are_stable_across_redelivery() {
    let payload: serde_json::Value = serde_json::from_str(WA).unwrap();
    let a = webhook::normalize_whatsapp(&payload).unwrap();
    let b = webhook::normalize_whatsapp(&payload).unwrap();
    assert_eq!(a, b);
}

// ---- Messenger webhook ----------------------------------------------------

const MSGR: &str = include_str!("fixtures/messenger_webhook.json");

#[test]
fn messenger_webhook_handles_echoes_and_ignores_receipts() {
    let payload: serde_json::Value = serde_json::from_str(MSGR).unwrap();
    let r = webhook::normalize_messenger(&payload).unwrap();
    let msgs = messages(&r);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].direction, Direction::Inbound);
    assert_eq!(msgs[0].conversation_id.as_deref(), Some("USER1"));
    assert_eq!(msgs[1].direction, Direction::Outbound);
    assert_eq!(msgs[1].conversation_id.as_deref(), Some("USER1"));
    assert_eq!(msgs[1].attachments[0].uri.as_deref(), Some("https://example.com/a.png"));
}

// ---- Device collector rows ------------------------------------------------

const DEVICE: &str = include_str!("fixtures/device_batch.json");

#[test]
fn device_sms_and_calls() {
    let batch: DeviceBatch = serde_json::from_str(DEVICE).unwrap();
    let opts =
        DeviceOptions { own_number: Some("+94710000000".into()), default_country_code: Some("94".into()) };
    let r = device::normalize(&batch, &opts);
    assert!(r.warnings.is_empty());

    let msgs = messages(&r);
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0].direction, Direction::Inbound);
    assert_eq!(msgs[0].sender.id, "+94771234567");
    assert_eq!(msgs[0].recipients[0].id, "+94710000000");
    assert_eq!(msgs[0].conversation_id.as_deref(), Some("5"));
    assert_eq!(msgs[1].direction, Direction::Outbound);
    assert_eq!(msgs[1].sender.id, "+94710000000");
    assert_eq!(msgs[1].recipients[0].id, "+94771234567", "national number normalized");
    assert_eq!(msgs[2].sender.id, "DIALOG", "alphanumeric sender kept as is");

    let c = calls(&r);
    let outcomes: Vec<_> = c.iter().map(|c| (c.direction, c.outcome)).collect();
    assert_eq!(
        outcomes,
        [
            (Direction::Inbound, CallOutcome::Answered),
            (Direction::Inbound, CallOutcome::Missed),
            (Direction::Outbound, CallOutcome::Answered),
            (Direction::Inbound, CallOutcome::Rejected),
        ]
    );
    assert_eq!(c[0].participants[0].display_name.as_deref(), Some("Kamal"));
    assert!(c.iter().all(|c| c.channel == Channel::Cellular));
}

#[test]
fn device_normalization_is_deterministic() {
    let batch: DeviceBatch = serde_json::from_str(DEVICE).unwrap();
    let a = device::normalize(&batch, &DeviceOptions::default());
    let b = device::normalize(&batch, &DeviceOptions::default());
    assert_eq!(a, b);
}

#[test]
fn device_invalid_timestamps_are_reported() {
    let batch: DeviceBatch = serde_json::from_str(
        r#"{"sms":[{"address":"x","body":"y","date_ms":9223372036854775807,"type":1}]}"#,
    )
    .unwrap();
    let r = device::normalize(&batch, &DeviceOptions::default());
    assert!(r.events.is_empty());
    assert_eq!(r.warnings.len(), 1);
}

// ---- JSON dispatcher ------------------------------------------------------

#[test]
fn dispatch_end_to_end() {
    let input = serde_json::json!({ "text": "01/02/2024, 10:00 - A: hi\n" }).to_string();
    let out: serde_json::Value =
        serde_json::from_str(&api::dispatch("parse_whatsapp_export", &input)).unwrap();
    assert_eq!(out["ok"], true);
    assert_eq!(out["data"]["events"][0]["type"], "message");
    assert_eq!(out["data"]["events"][0]["body"], "hi");

    let sig = webhook::sign(b"s", b"body");
    let input = serde_json::json!({ "secret": "s", "body": "body", "signature": sig }).to_string();
    let out: serde_json::Value = serde_json::from_str(&api::dispatch("verify_signature", &input)).unwrap();
    assert_eq!(out["data"]["valid"], true);
}
