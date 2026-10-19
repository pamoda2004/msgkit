//! The optional IANA `time_zone` option of the WhatsApp importer (decision D7). The conformance suite pins the
//! numbers (`conformance/cases/tz_*`); these tests pin the guarantees around them.

use msgkit_core::whatsapp_export::{parse, try_parse, DateOrder, WhatsAppExportOptions};
use msgkit_core::{api, Error, Event};
use serde_json::{json, Value};

const CHAT: &str =
    "01/06/2024, 12:00 - Alex: hello\n01/06/2024, 12:05 - Sam: hi\n01/06/2024, 12:05 - Sam: hi\n";

fn options(zone: Option<&str>, offset: i32) -> WhatsAppExportOptions {
    let mut opts =
        WhatsAppExportOptions::default().with_date_order(DateOrder::Dmy).with_utc_offset_minutes(offset);
    opts.time_zone = zone.map(str::to_owned);
    opts
}

fn ids(chat: &str, opts: &WhatsAppExportOptions) -> Vec<String> {
    try_parse(chat, opts).unwrap().events.iter().map(|e| e.id().to_owned()).collect()
}

fn instants(chat: &str, opts: &WhatsAppExportOptions) -> Vec<String> {
    try_parse(chat, opts)
        .unwrap()
        .events
        .iter()
        .map(|e| e.timestamp().format("%Y-%m-%dT%H:%MZ").to_string())
        .collect()
}

#[test]
fn ids_do_not_depend_on_the_time_zone() {
    let without = ids(CHAT, &options(None, 0));
    for zone in ["Asia/Colombo", "America/New_York", "Pacific/Apia", "UTC"] {
        assert_eq!(ids(CHAT, &options(Some(zone), 0)), without, "{zone}");
    }
    assert_eq!(ids(CHAT, &options(None, 330)), without, "the offset never mattered either");
}

#[test]
fn without_a_time_zone_nothing_changes() {
    let default = WhatsAppExportOptions::default().with_date_order(DateOrder::Dmy);
    assert_eq!(try_parse(CHAT, &default).unwrap(), parse(CHAT, &default));
    assert_eq!(
        instants(CHAT, &options(None, 330)),
        ["2024-06-01T06:30Z", "2024-06-01T06:35Z", "2024-06-01T06:35Z"]
    );
}

#[test]
fn a_zone_matches_the_equivalent_offset_where_there_is_no_daylight_saving() {
    assert_eq!(instants(CHAT, &options(Some("Asia/Colombo"), 0)), instants(CHAT, &options(None, 330)));
}

#[test]
fn the_zone_wins_over_the_offset() {
    assert_eq!(instants(CHAT, &options(Some("Asia/Colombo"), 600)), instants(CHAT, &options(None, 330)));
}

#[test]
fn a_zone_follows_daylight_saving_through_the_year() {
    let chat = "15/01/2024, 12:00 - Alex: winter\n15/07/2024, 12:00 - Alex: summer\n";
    assert_eq!(
        instants(chat, &options(Some("America/New_York"), 0)),
        ["2024-01-15T17:00Z", "2024-07-15T16:00Z"],
        "UTC-5 in winter, UTC-4 in summer"
    );
}

#[test]
fn transitions_are_counted_in_one_warning_that_quotes_nothing() {
    let chat = "10/03/2024, 02:30 - Alex: secret-marker-one\n03/11/2024, 01:30 - Sam: secret-marker-two\n";
    let report = try_parse(chat, &options(Some("America/New_York"), 0)).unwrap();
    assert_eq!(report.warnings.len(), 1);
    let warning = &report.warnings[0];
    assert!(
        warning.starts_with("2 message time(s) fell where the time zone's UTC offset changes"),
        "{warning}"
    );
    assert!(
        warning.contains("1 in a repeated period") && warning.contains("1 in a skipped period"),
        "{warning}"
    );
    assert!(!warning.contains("secret-marker") && !warning.contains("Alex"), "{warning}");

    let quiet = try_parse(CHAT, &options(Some("America/New_York"), 0)).unwrap();
    assert!(quiet.warnings.is_empty(), "{:?}", quiet.warnings);
}

#[test]
fn an_unknown_zone_is_refused_without_repeating_it() {
    for bad in ["Mars/Phobos", "asia/colombo", "+05:30", "", " ", "Asia/Colombo\n", "IST"] {
        match try_parse(CHAT, &options(Some(bad), 0)) {
            Err(Error::InvalidInput(message)) => {
                // Empty and blank names are trivially "contained" in any text, so only real names are checked.
                if !bad.trim().is_empty() {
                    assert!(!message.contains(bad), "{message}");
                }
            }
            other => panic!("{bad:?} was accepted: {other:?}"),
        }
    }
    let marker = "ZXQMARKERZONE";
    let Err(Error::InvalidInput(message)) = try_parse(CHAT, &options(Some(marker), 0)) else { panic!() };
    assert!(!message.contains(marker), "{message}");
}

#[test]
fn the_infallible_entry_point_warns_and_falls_back_to_the_offset() {
    let report = parse(CHAT, &options(Some("Mars/Phobos"), 330));
    assert_eq!(report.warnings, ["unknown or unsupported time_zone; using utc_offset_minutes"]);
    let times: Vec<_> =
        report.events.iter().map(|e| e.timestamp().format("%Y-%m-%dT%H:%MZ").to_string()).collect();
    assert_eq!(times, ["2024-06-01T06:30Z", "2024-06-01T06:35Z", "2024-06-01T06:35Z"]);
}

fn dispatch(input: &Value) -> Value {
    serde_json::from_str(&api::dispatch("parse_whatsapp_export", &input.to_string())).unwrap()
}

#[test]
fn the_json_entry_point_accepts_a_zone_and_refuses_wrong_types() {
    let ok =
        dispatch(&json!({ "text": CHAT, "options": { "date_order": "dmy", "time_zone": "Asia/Colombo" } }));
    assert_eq!(ok["ok"], true);
    assert_eq!(ok["data"]["events"][0]["timestamp"], "2024-06-01T06:30:00Z");

    let null = dispatch(&json!({ "text": CHAT, "options": { "date_order": "dmy", "time_zone": null } }));
    assert_eq!(null["ok"], true, "null means no time zone");

    for bad in [json!(5), json!(["Asia/Colombo"]), json!({ "name": "Asia/Colombo" }), json!("Mars/Phobos")] {
        let r = dispatch(&json!({ "text": CHAT, "options": { "time_zone": bad } }));
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"]["code"], "invalid_input");
    }
}

#[test]
fn the_version_operation_reports_the_bundled_time_zone_database() {
    let v: Value = serde_json::from_str(&api::dispatch("version", "{}")).unwrap();
    assert_eq!(v["data"]["tzdata_version"], msgkit_core::tzdata_version());
    assert_eq!(v["data"]["schema_version"], "1.1.0");
}

#[test]
fn every_event_still_serializes_with_a_utc_timestamp() {
    let report = try_parse(CHAT, &options(Some("America/New_York"), 0)).unwrap();
    for event in &report.events {
        let Event::Message(m) = event else { panic!() };
        assert!(serde_json::to_string(m).unwrap().contains("Z\""));
    }
}
