//! Event ids are derived from what the source says (local wall-clock time, sender as written, body and an
//! occurrence index), never from how the importer was configured.
//!
//! Expected ids were computed independently with Python's `hashlib`
//! (`sha256("msgkit:v1" + 0x1f + part ...)[..16]` as hex), not by this crate.

use msgkit_core::device::{self, DeviceBatch, DeviceOptions};
use msgkit_core::whatsapp_export::{parse, DateOrder, WhatsAppExportOptions};
use msgkit_core::{Event, ImportReport};

fn ids(report: &ImportReport) -> Vec<String> {
    report.events.iter().map(|e| e.id().to_owned()).collect()
}

fn options(offset: i32, order: DateOrder, country: Option<&str>) -> WhatsAppExportOptions {
    let mut opts = WhatsAppExportOptions::default().with_utc_offset_minutes(offset).with_date_order(order);
    opts.default_country_code = country.map(str::to_owned);
    opts
}

const TWO_OKS: &str = "01/02/2024, 10:00 - Alex: ok\n01/02/2024, 10:00 - Alex: ok\n";

#[test]
fn two_identical_messages_in_the_same_minute_keep_separate_ids() {
    let report = parse(TWO_OKS, &options(0, DateOrder::Dmy, None));
    assert_eq!(report.events.len(), 2);
    assert_eq!(
        ids(&report),
        ["59fd4b87b49c82f9fae3ce5d748cba43", "2cc3cba8c48b57a2542280189436fa82"],
        "ids of \"ok\" #0 and #1 at 01/02/2024 10:00 from Alex"
    );
}

#[test]
fn the_occurrence_index_keeps_counting() {
    let text = format!("{TWO_OKS}01/02/2024, 10:00 - Alex: ok\n01/02/2024, 10:00 - Alex: ok, thanks\n01/02/2024, 10:01 - Alex: ok\n");
    let all = ids(&parse(&text, &options(0, DateOrder::Dmy, None)));
    assert_eq!(all.len(), 5);
    assert_eq!(all[2], "6f65c6121c29ed8e8ec54d118597747a", "third identical \"ok\" in the same minute");
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(unique.len(), 5, "different body, different minute and different occurrence all differ");
}

#[test]
fn re_importing_gives_the_same_ids() {
    let a = ids(&parse(TWO_OKS, &options(0, DateOrder::Dmy, None)));
    let b = ids(&parse(TWO_OKS, &options(0, DateOrder::Dmy, None)));
    assert_eq!(a, b);
}

#[test]
fn ids_do_not_change_with_utc_offset_minutes() {
    let baseline = ids(&parse(TWO_OKS, &options(0, DateOrder::Dmy, None)));
    for offset in [330, -480, 840, -720, 5 * 60 + 45] {
        let report = parse(TWO_OKS, &options(offset, DateOrder::Dmy, None));
        assert_eq!(ids(&report), baseline, "offset {offset}");
        // The UTC timestamps do move with the offset; only the ids are independent of it.
        let utc = report.events[0].timestamp().timestamp();
        assert_eq!(utc, 1_706_781_600 - i64::from(offset) * 60, "offset {offset}");
    }
}

#[test]
fn ids_do_not_change_with_default_country_code() {
    let text = "01/02/2024, 10:00 - 0700000000: hello\n01/02/2024, 10:01 - +94 70 000 0000: hello\n";
    let baseline = ids(&parse(text, &options(0, DateOrder::Dmy, None)));
    for country in ["94", "+94", "1", "44"] {
        let report = parse(text, &options(0, DateOrder::Dmy, Some(country)));
        assert_eq!(ids(&report), baseline, "country code {country}");
    }
}

#[test]
fn ids_do_not_change_with_date_order() {
    let text = "01/02/24, 10:00 - Alex: ok\n01/02/24, 10:00 - Alex: ok\n";
    let baseline = ids(&parse(text, &options(0, DateOrder::Dmy, None)));
    for order in [DateOrder::Mdy, DateOrder::Auto] {
        assert_eq!(ids(&parse(text, &options(0, order, None))), baseline, "{order:?}");
    }
    // The parsed instants do differ (1 February vs 2 January); the ids must not.
    let dmy = parse(text, &options(0, DateOrder::Dmy, None));
    let mdy = parse(text, &options(0, DateOrder::Mdy, None));
    assert_ne!(dmy.events[0].timestamp(), mdy.events[0].timestamp());
}

#[test]
fn the_clock_style_does_not_change_ids() {
    let h24 = "01/02/2024, 22:30 - Alex: ok\n";
    let h12 = "01/02/2024, 10:30 PM - Alex: ok\n";
    assert_eq!(
        ids(&parse(h24, &options(0, DateOrder::Dmy, None))),
        ids(&parse(h12, &options(0, DateOrder::Dmy, None)))
    );
}

#[test]
fn a_different_local_time_sender_or_body_gives_a_different_id() {
    let base = ids(&parse("01/02/2024, 10:00 - Alex: ok\n", &options(0, DateOrder::Dmy, None)));
    for variant in [
        "01/02/2024, 10:01 - Alex: ok\n",
        "01/02/2024, 11:00 - Alex: ok\n",
        "02/02/2024, 10:00 - Alex: ok\n",
        "01/02/2025, 10:00 - Alex: ok\n",
        "01/02/2024, 10:00 - Sam: ok\n",
        "01/02/2024, 10:00 - Alex: okay\n",
    ] {
        assert_ne!(ids(&parse(variant, &options(0, DateOrder::Dmy, None))), base, "{variant:?}");
    }
}

#[test]
fn missed_call_ids_ignore_the_offset_too() {
    let text = "01/02/2024, 10:05 - Missed voice call\n";
    let a = parse(text, &options(0, DateOrder::Dmy, None));
    let b = parse(text, &options(330, DateOrder::Dmy, None));
    assert_eq!(ids(&a), ["6f47f2aec9f282dec8130227cfda207a"]);
    assert_eq!(ids(&a), ids(&b));
    assert!(matches!(a.events[0], Event::Call(_)));
}

#[test]
fn device_ids_ignore_the_default_country_code() {
    let batch: DeviceBatch = serde_json::from_value(serde_json::json!({
        "sms": [{ "address": "+15550100111", "body": "hi", "date_ms": 1_700_000_000_000_i64, "type": 1 },
                { "address": "0700000000", "body": "hi", "date_ms": 1_700_000_000_000_i64, "type": 1 },
                { "address": "0700000000", "body": "hi", "date_ms": 1_700_000_000_000_i64, "type": 1 }],
        "calls": [{ "number": "+15550100111", "date_ms": 1_700_000_100_000_i64, "type": 3 },
                  { "number": "0700000000", "date_ms": 1_700_000_100_000_i64, "type": 3 }]
    }))
    .unwrap();
    let with = |country: Option<&str>| {
        let mut opts = DeviceOptions::default();
        opts.default_country_code = country.map(str::to_owned);
        device::normalize(&batch, &opts)
    };
    let baseline = with(None);
    for country in ["94", "+94", "1"] {
        assert_eq!(ids(&with(Some(country))), ids(&baseline), "country code {country}");
    }
    let all = ids(&baseline);
    assert_eq!(all.len(), 5);
    assert!(all.contains(&"c37db18a1aeea85051ddfa0dbc57b843".to_owned()), "sms id pinned");
    assert!(all.contains(&"5692c4cdbb5c89e78050d36adb60bc42".to_owned()), "call id pinned");
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(unique.len(), 5, "identical SMS rows keep separate ids through the occurrence index");
}
