//! Property-based tests: invariants that must hold for *any* input, not just the handwritten cases.
//!
//! * no input makes a parser or the JSON entry point panic;
//! * results are deterministic and re-importing yields the same ids (so a store de-duplicates);
//! * every timestamp is UTC (serialized with a `Z`);
//! * ids never depend on `utc_offset_minutes`, `date_order` or `default_country_code`;
//! * error text never repeats the caller's data;
//! * a signature produced by `sign` verifies, and any change to the body or signature does not.

use std::collections::HashSet;

use msgkit_core::device::{self, DeviceBatch, DeviceOptions};
use msgkit_core::facebook::{self, FacebookExportOptions};
use msgkit_core::store::{EventStore, MemoryStore};
use msgkit_core::webhook::{self, sign, verify_signature};
use msgkit_core::whatsapp_export::{parse, DateOrder, WhatsAppExportOptions};
use msgkit_core::{api, Event, ImportReport};
use proptest::prelude::*;
use serde_json::{json, Value};

fn config() -> ProptestConfig {
    ProptestConfig { cases: 256, failure_persistence: None, ..ProptestConfig::default() }
}

fn ids(report: &ImportReport) -> Vec<String> {
    report.events.iter().map(|e| e.id().to_owned()).collect()
}

fn options(order: DateOrder, offset: i32, country: Option<&str>) -> WhatsAppExportOptions {
    WhatsAppExportOptions {
        date_order: order,
        utc_offset_minutes: offset,
        default_country_code: country.map(str::to_owned),
        ..WhatsAppExportOptions::default()
    }
}

// ---- generators -----------------------------------------------------------------------------------------

/// A header-shaped line with a random body, so the deeper parser paths run.
fn header_line() -> impl Strategy<Value = String> {
    let when = (1u32..=31, 1u32..=12, 2019u32..=2026, 0u32..=23, 0u32..=59, prop::option::of(0u32..=59));
    let sender = prop::sample::select(vec![
        "Alex",
        "Sam",
        "Priya J",
        "+94 70 000 0000",
        "0700000000",
        "~ Maya",
        "a: b",
    ]);
    let body = prop_oneof![
        "[^\n\r]{0,30}",
        Just("<Media omitted>".to_owned()),
        Just("IMG-20240101-WA0001.jpg (file attached)".to_owned()),
        Just("This message was deleted".to_owned()),
        Just("ok".to_owned()),
    ];
    (when, sender, body, any::<bool>(), any::<bool>()).prop_map(
        |((d, m, y, h, mi, s), who, text, ios, ampm)| {
            let (hour, suffix) =
                if ampm { (h % 12 + 1, if h < 12 { " AM" } else { " PM" }) } else { (h, "") };
            let clock = match s {
                Some(s) => format!("{hour:02}:{mi:02}:{s:02}{suffix}"),
                None => format!("{hour:02}:{mi:02}{suffix}"),
            };
            if ios {
                format!("[{d:02}/{m:02}/{y}, {clock}] {who}: {text}")
            } else {
                format!("{d:02}/{m:02}/{y}, {clock} - {who}: {text}")
            }
        },
    )
}

fn notice_line() -> impl Strategy<Value = String> {
    (1u32..=28, 1u32..=12, 2019u32..=2026, 0usize..3).prop_map(|(d, m, y, kind)| {
        let text = ["Missed voice call", "Missed video call", "Alex created group \"G\""][kind];
        format!("{d:02}/{m:02}/{y}, 10:00 - {text}")
    })
}

fn chat_line() -> impl Strategy<Value = String> {
    prop_oneof![4 => header_line(), 1 => notice_line(), 2 => "[^\n\r]{0,60}"]
}

fn chat() -> impl Strategy<Value = String> {
    (prop::collection::vec(chat_line(), 0..24), prop::sample::select(vec!["\n", "\r\n"]), any::<bool>())
        .prop_map(|(lines, eol, bom)| format!("{}{}", if bom { "\u{feff}" } else { "" }, lines.join(eol)))
}

fn date_order() -> impl Strategy<Value = DateOrder> {
    prop::sample::select(vec![DateOrder::Auto, DateOrder::Dmy, DateOrder::Mdy])
}

/// Arbitrary JSON, nested a few levels deep.
fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|n| json!(n)),
        any::<u64>().prop_map(|n| json!(n)),
        "[^\u{0}]{0,12}".prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            prop::collection::btree_map("[a-z_]{1,10}", inner, 0..5)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn time_zone() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "Asia/Colombo",
        "America/New_York",
        "Europe/London",
        "Australia/Lord_Howe",
        "Pacific/Apia",
        "UTC",
    ])
}

// ---- properties -----------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    #[test]
    fn ids_ignore_the_time_zone(text in chat(), zone in time_zone()) {
        let with_zone = WhatsAppExportOptions { time_zone: Some(zone.to_owned()), ..options(DateOrder::Dmy, 0, None) };
        let (a, b) = (parse(&text, &options(DateOrder::Dmy, 0, None)), parse(&text, &with_zone));
        prop_assert_eq!(a.events.len(), b.events.len());
        prop_assert_eq!(ids(&a), ids(&b));
    }

    #[test]
    fn any_time_zone_text_is_refused_without_panicking_or_being_repeated(name in any::<String>()) {
        let marked = format!("ZXQMARKER{name}");
        let input = json!({ "text": "01/02/2024, 10:00 - A: x\n", "options": { "time_zone": marked } }).to_string();
        let out: Value = serde_json::from_str(&api::dispatch("parse_whatsapp_export", &input)).unwrap();
        prop_assert_eq!(&out["ok"], &json!(false));
        prop_assert_eq!(&out["error"]["code"], &json!("invalid_input"));
        prop_assert!(!out["error"]["message"].as_str().unwrap().contains("ZXQMARKER"));
        // And without the marker the name may even be valid, but nothing panics.
        let plain = json!({ "text": "01/02/2024, 10:00 - A: x\n", "options": { "time_zone": name } }).to_string();
        let _ = api::dispatch("parse_whatsapp_export", &plain);
    }

    #[test]
    fn whatsapp_parser_never_panics_on_any_text(text in any::<String>(), offset in -2000i32..=2000, order in date_order()) {
        let report = parse(&text, &options(order, offset, None));
        prop_assert!(report.events.len() <= text.lines().count());
    }

    #[test]
    fn whatsapp_parser_never_panics_on_chat_like_text(text in chat(), offset in -2000i32..=2000, order in date_order()) {
        let _ = parse(&text, &options(order, offset, None));
    }

    #[test]
    fn importing_twice_gives_identical_reports(text in chat(), order in date_order()) {
        let opts = options(order, 0, None);
        prop_assert_eq!(parse(&text, &opts), parse(&text, &opts));
    }

    #[test]
    fn ids_within_one_report_are_unique(text in chat(), order in date_order()) {
        let all = ids(&parse(&text, &options(order, 0, None)));
        let unique: HashSet<_> = all.iter().collect();
        prop_assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn re_importing_is_idempotent_in_a_store(text in chat()) {
        let report = parse(&text, &WhatsAppExportOptions::default());
        let mut store = MemoryStore::new();
        let first = store.insert_all(report.events.clone()).unwrap();
        let second = store.insert_all(report.events.clone()).unwrap();
        prop_assert_eq!(first, report.events.len());
        prop_assert_eq!(second, 0);
        prop_assert_eq!(store.len().unwrap(), report.events.len());
    }

    #[test]
    fn ids_ignore_offset_and_country_code(
        text in chat(),
        offset in -2000i32..=2000,
        country in prop::option::of(prop::sample::select(vec!["94", "+94", "1", "44"])),
    ) {
        let (a, b) = (parse(&text, &options(DateOrder::Dmy, 0, None)), parse(&text, &options(DateOrder::Dmy, offset, country)));
        prop_assert_eq!(a.events.len(), b.events.len());
        prop_assert_eq!(ids(&a), ids(&b));
    }

    #[test]
    fn ids_ignore_the_date_order_when_both_orders_accept_every_line(text in chat()) {
        let (dmy, mdy) = (parse(&text, &options(DateOrder::Dmy, 0, None)), parse(&text, &options(DateOrder::Mdy, 0, None)));
        // Lines with a first number above 12 (or a second above 12) are valid in only one order, so only
        // compare the lines both orders accept: those keep their position and their id.
        if dmy.events.len() == mdy.events.len() {
            let (mut a, mut b) = (ids(&dmy), ids(&mdy));
            a.sort();
            b.sort();
            prop_assert_eq!(a, b);
        }
    }

    #[test]
    fn serialized_timestamps_are_always_utc(text in chat(), offset in -2000i32..=2000) {
        let report = parse(&text, &options(DateOrder::Auto, offset, None));
        for event in &report.events {
            let value = serde_json::to_value(event).unwrap();
            let stamp = value["timestamp"].as_str().unwrap();
            prop_assert!(stamp.ends_with('Z'), "{stamp}");
        }
    }

    #[test]
    fn facebook_parser_never_panics(text in any::<String>(), value in json_value()) {
        let _ = facebook::parse(&text, &FacebookExportOptions::default());
        let _ = facebook::parse_value(value, &FacebookExportOptions::default());
    }

    #[test]
    fn device_normalizer_never_panics(
        rows in prop::collection::vec((any::<i64>(), any::<i32>(), ".{0,12}", ".{0,12}", any::<u64>()), 0..12),
        country in prop::option::of("[0-9+]{0,4}"),
    ) {
        let sms: Vec<Value> = rows.iter().map(|(ms, kind, addr, body, _)| json!({ "address": addr, "body": body, "date_ms": ms, "type": kind })).collect();
        let calls: Vec<Value> = rows.iter().map(|(ms, kind, num, _, secs)| json!({ "number": num, "date_ms": ms, "type": kind, "duration_secs": secs })).collect();
        let batch: DeviceBatch = serde_json::from_value(json!({ "sms": sms, "calls": calls })).unwrap();
        let report = device::normalize(&batch, &DeviceOptions { default_country_code: country, ..DeviceOptions::default() });
        for event in &report.events {
            let value = serde_json::to_value(event).unwrap();
            prop_assert!(value["timestamp"].as_str().unwrap().ends_with('Z'));
        }
    }

    #[test]
    fn webhook_normalizers_never_panic(payload in json_value()) {
        let _ = webhook::normalize_whatsapp(&payload);
        let _ = webhook::normalize_messenger(&payload);
        let page = json!({ "object": "page", "entry": [payload.clone()] });
        let _ = webhook::normalize_messenger(&page);
        let whatsapp = json!({ "object": "whatsapp_business_account", "entry": [{ "changes": [{ "value": payload }] }] });
        let _ = webhook::normalize_whatsapp(&whatsapp);
    }

    #[test]
    fn dispatch_never_panics_and_always_answers_with_an_envelope(op in ".{0,24}", input in any::<String>()) {
        let response = api::dispatch(&op, &input);
        let envelope: Value = serde_json::from_str(&response).expect("the response is JSON");
        prop_assert!(envelope["ok"].is_boolean());
    }

    #[test]
    fn dispatch_never_echoes_caller_data_in_errors(
        marker in "[A-Z]{12}",
        op in prop::sample::select(vec![
            "parse_whatsapp_export", "parse_facebook_export", "normalize_device",
            "normalize_whatsapp_webhook", "normalize_messenger_webhook", "verify_signature",
        ]),
        shape in 0usize..8,
    ) {
        let m = marker.as_str();
        let input = match shape {
            0 => format!("{{\"{m}\": 1}}"),
            1 => format!("{{\"text\": {{\"{m}\": 1}}}}"),
            2 => format!("{{\"options\": \"{m}\"}}"),
            3 => format!("{{\"thread\": \"{m}\", \"batch\": \"{m}\", \"payload\": \"{m}\", \"secret\": 1, \"body\": \"{m}\"}}"),
            4 => format!("[\"{m}\"]"),
            5 => format!("{{\"text\": \"x\", \"options\": {{\"date_order\": \"{m}\", \"self_name\": [\"{m}\"]}}}}"),
            6 => format!("{{\"batch\": {{\"sms\": [{{\"address\": \"{m}\", \"body\": \"{m}\", \"date_ms\": \"{m}\", \"type\": \"{m}\"}}]}}}}"),
            _ => format!("{{\"text\": \"{m}\" "),
        };
        let response = api::dispatch(op, &input);
        prop_assert!(!response.contains(m), "{} echoed the input: {}", op, response);
    }

    #[test]
    fn signatures_round_trip_and_reject_any_change(
        secret in prop::collection::vec(any::<u8>(), 0..64),
        body in prop::collection::vec(any::<u8>(), 0..128),
        flip in any::<prop::sample::Index>(),
    ) {
        let header = sign(&secret, &body);
        prop_assert!(verify_signature(&secret, &body, &header));
        if !body.is_empty() {
            let mut tampered = body.clone();
            tampered[flip.index(body.len())] ^= 1;
            prop_assert!(!verify_signature(&secret, &tampered, &header));
        }
        let mut wrong = header.into_bytes();
        let at = 7 + flip.index(wrong.len() - 7);
        wrong[at] = if wrong[at] == b'0' { b'1' } else { b'0' };
        prop_assert!(!verify_signature(&secret, &body, &String::from_utf8(wrong).unwrap()));
    }

    #[test]
    fn verify_signature_never_panics(
        secret in prop::collection::vec(any::<u8>(), 0..32),
        body in prop::collection::vec(any::<u8>(), 0..32),
        header in any::<String>(),
    ) {
        let _ = verify_signature(&secret, &body, &header);
    }
}

#[test]
fn reports_serialize_and_deserialize_losslessly() {
    let text = "01/02/2024, 10:00 - Alex: hi\n01/02/2024, 10:01 - Missed voice call\n";
    let report = parse(text, &WhatsAppExportOptions::default());
    let json = serde_json::to_string(&report).unwrap();
    let back: ImportReport = serde_json::from_str(&json).unwrap();
    assert_eq!(report, back);
    assert!(matches!(back.events[0], Event::Message(_)));
}
