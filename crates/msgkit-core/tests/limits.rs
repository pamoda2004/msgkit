//! Input limits (`msgkit_core::limits`): every entry point refuses input over a limit with `limit_exceeded`, at
//! the limit and not before, never repeats the input in the error, and the defaults accept every input the
//! repository ships (fixtures, conformance cases and the fuzz corpus).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use msgkit_core::device::{self, DeviceBatch, DeviceOptions, RawCall, RawSms};
use msgkit_core::error::codes;
use msgkit_core::facebook::{self, FacebookExportOptions};
use msgkit_core::limits::{
    Limits, DEFAULT_MAX_INPUT_BYTES, DEFAULT_MAX_JSON_DEPTH, DEFAULT_MAX_LINE_BYTES, DEFAULT_MAX_MESSAGES,
};
use msgkit_core::webhook;
use msgkit_core::whatsapp_export::{self, WhatsAppExportOptions};
use msgkit_core::{api, Error};
use serde_json::{json, Value};

const SECRET_TEXT: &str = "PRIVATE-VALUE-MUST-NOT-APPEAR";

fn chat(lines: usize) -> String {
    let mut out = String::new();
    for i in 0..lines {
        let _ = writeln!(out, "01/02/2024, 10:{:02} - Alice: message {i}", i % 60);
    }
    out
}

fn wa(limits: Limits) -> WhatsAppExportOptions {
    WhatsAppExportOptions::default().with_limits(limits)
}

fn assert_limit(result: Result<impl std::fmt::Debug, Error>) -> String {
    match result {
        Err(e @ Error::LimitExceeded(_)) => {
            assert_eq!(e.code(), codes::LIMIT_EXCEEDED);
            e.to_string()
        }
        other => panic!("expected limit_exceeded, got {other:?}"),
    }
}

#[test]
fn the_defaults_are_the_documented_numbers() {
    let d = Limits::default();
    // Absent limits have their defaults.
    assert_eq!(d.input_bytes_limit(), Some(32 * 1024 * 1024));
    assert_eq!(d.max_input_bytes, None);
    assert_eq!(
        (DEFAULT_MAX_INPUT_BYTES, DEFAULT_MAX_LINE_BYTES, DEFAULT_MAX_MESSAGES, DEFAULT_MAX_JSON_DEPTH),
        (33_554_432, 262_144, 1_000_000, 32)
    );
    // They are the ones enforced: one byte, line, message or level over each default is refused.
    assert!(d.check_input_bytes(33_554_432).is_ok() && d.check_input_bytes(33_554_433).is_err());
    assert!(d.check_line_bytes(1, 262_144).is_ok() && d.check_line_bytes(1, 262_145).is_err());
    assert!(d.check_messages(1_000_000).is_ok() && d.check_messages(1_000_001).is_err());
    let levels = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
    assert!(d.check_json_depth_str(&levels(32)).is_ok() && d.check_json_depth_str(&levels(33)).is_err());
    assert_eq!(WhatsAppExportOptions::default().limits, d);
    assert_eq!(FacebookExportOptions::default().limits, d);
    assert_eq!(DeviceOptions::default().limits, d);
}

#[test]
fn whatsapp_input_size_line_length_and_message_count() {
    let text = chat(3);
    let longest = text.lines().map(str::len).max().unwrap() as u64;

    let at = Limits::default()
        .with_max_input_bytes(text.len() as u64)
        .with_max_messages(3)
        .with_max_line_bytes(longest);
    assert_eq!(whatsapp_export::try_parse(&text, &wa(at)).unwrap().events.len(), 3);

    let size = Limits::default().with_max_input_bytes(text.len() as u64 - 1);
    assert!(assert_limit(whatsapp_export::try_parse(&text, &wa(size))).contains("max_input_bytes"));
    let line = Limits::default().with_max_line_bytes(longest - 1);
    assert!(assert_limit(whatsapp_export::try_parse(&text, &wa(line))).contains("max_line_bytes"));
    let count = Limits::default().with_max_messages(2);
    assert!(assert_limit(whatsapp_export::try_parse(&text, &wa(count))).contains("max_messages"));
}

#[test]
fn a_continuation_line_counts_towards_the_line_limit_but_not_the_message_count() {
    let text = "01/02/2024, 10:00 - Alice: first\nthe second line of the very same message, a bit longer\n";
    let ok = Limits::default().with_max_messages(1).with_max_line_bytes(70);
    assert_eq!(whatsapp_export::try_parse(text, &wa(ok)).unwrap().events.len(), 1);
    let long = Limits::default().with_max_line_bytes(40);
    assert!(assert_limit(whatsapp_export::try_parse(text, &wa(long))).contains("line 2"));
}

#[test]
fn only_unlimited_switches_limits_off_and_zero_is_invalid_input() {
    let text = chat(5);
    let tight = Limits::default().with_max_input_bytes(1);
    assert!(whatsapp_export::try_parse(&text, &wa(tight)).is_err());
    assert_eq!(whatsapp_export::try_parse(&text, &wa(Limits::unlimited())).unwrap().events.len(), 5);

    // A limit of 0 is refused, never read as "no limit", in every entry point, and nothing is echoed.
    for zero in [
        Limits::default().with_max_input_bytes(0),
        Limits::default().with_max_line_bytes(0),
        Limits::default().with_max_messages(0),
        Limits::default().with_max_json_depth(0),
    ] {
        let e = whatsapp_export::try_parse(&text, &wa(zero.clone())).unwrap_err();
        assert!(matches!(e, Error::InvalidInput(_)) && e.code() == codes::INVALID_INPUT, "{e:?}");
        assert!(e.to_string().contains("at least 1") && !e.to_string().contains("message 0"), "{e}");
        let thread = facebook_thread(1);
        assert!(facebook::parse(
            &thread.to_string(),
            &FacebookExportOptions::default().with_limits(zero.clone())
        )
        .is_err());
        assert!(facebook::parse_value(thread, &FacebookExportOptions::default().with_limits(zero.clone()))
            .is_err());
        assert!(device::try_normalize(
            &DeviceBatch::default(),
            &DeviceOptions::default().with_limits(zero.clone())
        )
        .is_err());
        assert!(webhook::normalize_messenger_with_limits(&json!({"object": "page"}), &zero).is_err());
        assert!(webhook::normalize_whatsapp_with_limits(&json!({"object": "x"}), &zero).is_err());
        // The infallible entry point reports it in a warning and imports nothing.
        let report = whatsapp_export::parse(&text, &wa(zero));
        assert_eq!(report.events, vec![]);
        assert!(report.warnings[0].contains("at least 1"));
    }
}

#[test]
fn the_infallible_parse_returns_no_events_and_a_warning() {
    let report = whatsapp_export::parse(&chat(3), &wa(Limits::default().with_max_messages(1)));
    assert_eq!(report.events, vec![]);
    assert_eq!(report.warnings.len(), 1);
    assert!(
        report.warnings[0].contains("nothing was imported") && report.warnings[0].contains("max_messages")
    );
}

#[test]
fn the_error_text_never_repeats_the_input() {
    let text = format!("01/02/2024, 10:00 - Alice: {SECRET_TEXT}\n");
    let e = assert_limit(whatsapp_export::try_parse(&text, &wa(Limits::default().with_max_line_bytes(5))));
    assert!(!e.contains(SECRET_TEXT) && !e.contains("Alice"), "{e}");
    let e = assert_limit(whatsapp_export::try_parse(&text, &wa(Limits::default().with_max_input_bytes(5))));
    assert!(!e.contains(SECRET_TEXT), "{e}");
}

fn facebook_thread(messages: usize) -> Value {
    let list: Vec<Value> = (0..messages)
        .map(|i| json!({"sender_name": "Alice", "timestamp_ms": 1_700_000_000_000_i64 + i64::try_from(i).unwrap(), "content": format!("m{i}")}))
        .collect();
    json!({"participants": [{"name": "Alice"}], "messages": list})
}

#[test]
fn facebook_size_depth_and_message_count() {
    let thread = facebook_thread(3);
    let text = thread.to_string();
    let opts = |l: Limits| FacebookExportOptions::default().with_limits(l);

    let at =
        Limits::default().with_max_input_bytes(text.len() as u64).with_max_messages(3).with_max_json_depth(4);
    assert_eq!(facebook::parse(&text, &opts(at.clone())).unwrap().events.len(), 3);
    assert_eq!(facebook::parse_value(thread.clone(), &opts(at)).unwrap().events.len(), 3);

    let size = Limits::default().with_max_input_bytes(text.len() as u64 - 1);
    assert!(assert_limit(facebook::parse(&text, &opts(size))).contains("max_input_bytes"));
    let count = Limits::default().with_max_messages(2);
    assert!(assert_limit(facebook::parse(&text, &opts(count.clone()))).contains("max_messages"));
    assert!(assert_limit(facebook::parse_value(thread.clone(), &opts(count))).contains("max_messages"));
    // thread object, messages array, message object: 3 levels.
    let depth = Limits::default().with_max_json_depth(2);
    assert!(assert_limit(facebook::parse(&text, &opts(depth.clone()))).contains("max_json_depth"));
    assert!(assert_limit(facebook::parse_value(thread, &opts(depth))).contains("max_json_depth"));
}

#[test]
fn a_hand_built_value_nested_beyond_the_json_parsers_own_depth_is_refused() {
    // serde_json refuses to parse text this deep, but a caller can build the value by hand. The check is iterative.
    let mut value = json!(1);
    for _ in 0..500 {
        value = Value::Array(vec![value]);
    }
    let e = assert_limit(facebook::parse_value(value, &FacebookExportOptions::default()));
    assert!(e.contains("max_json_depth"));
}

#[test]
fn device_counts_sms_and_call_rows_together() {
    let sms = |i: i64| RawSms {
        id: None,
        address: "+94700000000".into(),
        body: "a".into(),
        date_ms: 1_700_000_000_000 + i,
        kind: 1,
        thread_id: None,
    };
    let call = RawCall {
        id: None,
        number: "+94700000000".into(),
        name: None,
        date_ms: 1_700_000_001_000,
        duration_secs: 5,
        kind: 1,
    };
    let batch = DeviceBatch { sms: vec![sms(0), sms(1)], calls: vec![call] };
    let opts = |n: u64| DeviceOptions::default().with_limits(Limits::default().with_max_messages(n));

    assert_eq!(device::try_normalize(&batch, &opts(3)).unwrap().events.len(), 3);
    assert!(assert_limit(device::try_normalize(&batch, &opts(2))).contains("max_messages"));
    let report = device::normalize(&batch, &opts(2));
    assert!(report.events.is_empty() && report.warnings[0].contains("nothing was imported"));
}

#[test]
fn webhooks_check_depth_and_event_count() {
    let payload = json!({"object": "page", "entry": [{"messaging": [
        {"sender": {"id": "1"}, "recipient": {"id": "2"}, "timestamp": 1_700_000_000_000_i64, "message": {"mid": "m1", "text": "one"}},
        {"sender": {"id": "1"}, "recipient": {"id": "2"}, "timestamp": 1_700_000_001_000_i64, "message": {"mid": "m2", "text": "two"}}]}]});
    let at = Limits::default().with_max_messages(2).with_max_json_depth(6);
    assert_eq!(webhook::normalize_messenger_with_limits(&payload, &at).unwrap().events.len(), 2);
    assert!(assert_limit(webhook::normalize_messenger_with_limits(
        &payload,
        &at.clone().with_max_messages(1)
    ))
    .contains("max_messages"));
    assert!(assert_limit(webhook::normalize_messenger_with_limits(&payload, &at.with_max_json_depth(5)))
        .contains("max_json_depth"));
    // The plain functions use the default limits.
    assert_eq!(webhook::normalize_messenger(&payload).unwrap().events.len(), 2);
}

#[test]
fn the_json_entry_point_reads_limits_from_the_request() {
    let run = |op: &str, input: &Value| -> Value {
        serde_json::from_str(&api::dispatch(op, &input.to_string())).unwrap()
    };
    let code = |v: &Value| v["error"]["code"].as_str().map(str::to_owned);

    let v =
        run("parse_whatsapp_export", &json!({"text": chat(3), "options": {"limits": {"max_messages": 2}}}));
    assert_eq!(code(&v).as_deref(), Some("limit_exceeded"));
    let v =
        run("parse_whatsapp_export", &json!({"text": chat(3), "options": {"limits": {"max_messages": 3}}}));
    assert_eq!(v["ok"], true);

    // Only the named limit changes: the others keep their defaults; null means the default too.
    let v =
        run("parse_whatsapp_export", &json!({"text": chat(3), "options": {"limits": {"max_json_depth": 1}}}));
    assert_eq!(v["ok"], true);
    let v = run(
        "parse_whatsapp_export",
        &json!({"text": chat(3), "options": {"limits": {"max_messages": null}}}),
    );
    assert_eq!(v["ok"], true);

    // 0 is invalid_input; `unlimited` is the explicit opt-out.
    let v =
        run("parse_whatsapp_export", &json!({"text": chat(3), "options": {"limits": {"max_messages": 0}}}));
    assert_eq!(code(&v).as_deref(), Some("invalid_input"));
    let v = run(
        "parse_whatsapp_export",
        &json!({"text": chat(3), "options": {"limits": {"unlimited": true, "max_messages": 1}}}),
    );
    assert_eq!(v["ok"], true);

    // For the JSON imports the whole request is measured, wrapper included.
    let request = json!({"thread": facebook_thread(1), "options": {"limits": {"max_json_depth": 3}}});
    assert_eq!(code(&run("parse_facebook_export", &request)).as_deref(), Some("limit_exceeded"));
    let request = json!({"thread": facebook_thread(1), "options": {"limits": {"max_json_depth": 4}}});
    assert_eq!(run("parse_facebook_export", &request)["ok"], true);
    let size = serde_json::to_string(&request).unwrap().len() as u64;
    let tight = json!({"thread": facebook_thread(1), "options": {"limits": {"max_json_depth": 4, "max_input_bytes": size - 1}}});
    assert_eq!(code(&run("parse_facebook_export", &tight)).as_deref(), Some("limit_exceeded"));

    let v = run(
        "normalize_device",
        &json!({"batch": {"sms": [], "calls": []}, "options": {"limits": {"unlimited": true}}}),
    );
    assert_eq!(v["ok"], true);
    let v = run(
        "normalize_device",
        &json!({"batch": {"sms": [], "calls": []}, "options": {"limits": {"max_messages": 0}}}),
    );
    assert_eq!(code(&v).as_deref(), Some("invalid_input"));
    // A limit that is not a number is a bad option, not a limit error.
    let v =
        run("parse_whatsapp_export", &json!({"text": "", "options": {"limits": {"max_messages": "many"}}}));
    assert_eq!(code(&v).as_deref(), Some("invalid_input"));
}

#[test]
fn a_realistic_whatsapp_export_is_far_below_the_defaults() {
    // WhatsApp itself stops an export at 40,000 messages without media.
    let text = chat(40_000);
    assert!((text.len() as u64) < DEFAULT_MAX_INPUT_BYTES / 4, "40,000 messages are {} bytes", text.len());
    assert_eq!(
        whatsapp_export::try_parse(&text, &WhatsAppExportOptions::default()).unwrap().events.len(),
        40_000
    );
}

// ---- the defaults accept everything the repository ships ----

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(path)
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> =
        fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_file()).collect();
    out.sort();
    out
}

fn assert_not_a_limit_error(name: &str, result: Result<impl std::fmt::Debug, Error>) {
    if let Err(Error::LimitExceeded(m)) = result {
        panic!("{name} hits a default limit: {m}");
    }
}

#[test]
fn the_defaults_accept_every_fixture_conformance_input_and_fuzz_seed() {
    let mut checked = 0;
    let wa_opts = WhatsAppExportOptions::default();
    let fb_opts = FacebookExportOptions::default();

    // Fixtures.
    for path in files_in(&repo("crates/msgkit-core/tests/fixtures")) {
        let name = path.display().to_string();
        let text = fs::read_to_string(&path).unwrap();
        if path.extension().and_then(|e| e.to_str()) == Some("txt") {
            assert_not_a_limit_error(&name, whatsapp_export::try_parse(&text, &wa_opts));
        } else {
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_not_a_limit_error(&name, facebook::parse(&text, &fb_opts));
            assert_not_a_limit_error(&name, webhook::normalize_whatsapp(&value));
            assert_not_a_limit_error(&name, webhook::normalize_messenger(&value));
        }
        checked += 1;
    }

    // Conformance cases: every non-limit case must not answer limit_exceeded (their expected files are what every
    // binding reproduces, so this is the same statement for all of them).
    for dir in fs::read_dir(repo("conformance/cases")).unwrap().map(|e| e.unwrap().path()) {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("limit_") {
            continue;
        }
        let expected = fs::read_to_string(dir.join("expected.json")).unwrap();
        assert!(!expected.contains("limit_exceeded"), "{name} expects limit_exceeded");
        checked += 1;
    }

    // Fuzz seeds, through the same entry points the fuzz targets use.
    for path in files_in(&repo("fuzz/corpus/whatsapp_export")) {
        let data = fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&data[data.len().min(2)..]).into_owned();
        assert_not_a_limit_error(&path.display().to_string(), whatsapp_export::try_parse(&text, &wa_opts));
        checked += 1;
    }
    for path in files_in(&repo("fuzz/corpus/facebook_export")) {
        let text = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
        assert_not_a_limit_error(&path.display().to_string(), facebook::parse(&text, &fb_opts));
        checked += 1;
    }
    for path in files_in(&repo("fuzz/corpus/device")) {
        let batch: DeviceBatch = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_not_a_limit_error(
            &path.display().to_string(),
            device::try_normalize(&batch, &DeviceOptions::default()),
        );
        checked += 1;
    }
    for path in files_in(&repo("fuzz/corpus/webhooks")) {
        if let Ok(value) = serde_json::from_slice::<Value>(&fs::read(&path).unwrap()) {
            assert_not_a_limit_error(&path.display().to_string(), webhook::normalize_whatsapp(&value));
            assert_not_a_limit_error(&path.display().to_string(), webhook::normalize_messenger(&value));
        }
        checked += 1;
    }
    for path in files_in(&repo("fuzz/corpus/dispatch")) {
        let data = fs::read(&path).unwrap();
        let Some((&first, rest)) = data.split_first() else { continue };
        let ops = [
            "version",
            "parse_whatsapp_export",
            "parse_facebook_export",
            "normalize_device",
            "normalize_whatsapp_webhook",
            "normalize_messenger_webhook",
            "verify_signature",
        ];
        let response = api::dispatch(ops[usize::from(first) % ops.len()], &String::from_utf8_lossy(rest));
        assert!(!response.contains("limit_exceeded"), "{} hits a default limit", path.display());
        checked += 1;
    }
    assert!(checked > 100, "only {checked} inputs were checked");
}
