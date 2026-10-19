//! Tests that pin behaviour `cargo mutants` showed to be unchecked: call outcomes and warnings in the device
//! collector, phone-number validation, event ordering, the `EventStore` helpers and the JSON error
//! descriptions. All data is synthetic.

use std::panic::{catch_unwind, AssertUnwindSafe};

use msgkit_core::device::{self, DeviceBatch, DeviceOptions, RawSms};
use msgkit_core::error::describe_json_error;
use msgkit_core::phone::normalize_phone;
use msgkit_core::store::{conformance, EventStore, MemoryStore};
use msgkit_core::*;
use serde_json::json;

fn batch(value: serde_json::Value) -> DeviceBatch {
    serde_json::from_value(value).unwrap()
}

fn calls(r: &ImportReport) -> Vec<&Call> {
    r.events
        .iter()
        .filter_map(|e| match e {
            Event::Call(c) => Some(c),
            Event::Message(_) => None,
        })
        .collect()
}

// ---- device: call outcome and skipped rows ---------------------------------

#[test]
fn an_outgoing_call_is_answered_only_when_it_has_a_duration() {
    let r = device::normalize(
        &batch(json!({ "calls": [
            { "number": "0771234567", "date_ms": 1_000, "duration_secs": 0, "type": 2 },
            { "number": "0771234567", "date_ms": 2_000, "duration_secs": 1, "type": 2 },
        ]})),
        &DeviceOptions::default(),
    );
    let got: Vec<_> = calls(&r).iter().map(|c| (c.direction, c.outcome)).collect();
    assert_eq!(
        got,
        [(Direction::Outbound, CallOutcome::Unknown), (Direction::Outbound, CallOutcome::Answered)],
        "an unanswered outgoing call is still outbound, and one second is enough to count as answered"
    );
}

#[test]
fn a_call_with_an_invalid_timestamp_is_counted_in_the_warning() {
    let r = device::normalize(
        &batch(json!({ "calls": [{ "number": "0771234567", "date_ms": i64::MAX, "type": 1 }] })),
        &DeviceOptions::default(),
    );
    assert_eq!(r.events, []);
    assert_eq!(r.warnings, ["1 record(s) skipped because their timestamp was invalid"]);
}

// ---- model: events are ordered by time, ties keep source order ----------------

#[test]
fn events_are_sorted_by_time_and_ties_keep_source_order() {
    let r = device::normalize(
        &batch(json!({
            "sms": [
                { "address": "a", "body": "late", "date_ms": 3_000, "type": 1 },
                { "address": "a", "body": "tie-first", "date_ms": 1_000, "type": 1 },
                { "address": "a", "body": "tie-second", "date_ms": 1_000, "type": 1 },
            ],
            "calls": [{ "number": "a", "date_ms": 2_000, "type": 3 }],
        })),
        &DeviceOptions::default(),
    );
    let order: Vec<String> = r
        .events
        .iter()
        .map(|e| match e {
            Event::Message(m) => m.body.clone().unwrap(),
            Event::Call(_) => "call".to_owned(),
        })
        .collect();
    assert_eq!(order, ["tie-first", "tie-second", "call", "late"]);
}

// ---- phone: both digit and length checks apply on every route ---------------

#[test]
fn international_prefix_00_requires_digits_and_a_plausible_length() {
    assert_eq!(normalize_phone("0094771234567", None).as_deref(), Some("+94771234567"));
    assert_eq!(normalize_phone("0012345", None), None, "digits only but too short");
    assert_eq!(normalize_phone("00123456a89", None), None, "plausible length but not all digits");
}

#[test]
fn national_numbers_with_a_country_code_require_digits_and_a_plausible_length() {
    assert_eq!(normalize_phone("0771234567", Some("94")).as_deref(), Some("+94771234567"));
    assert_eq!(normalize_phone("0123", Some("94")), None, "digits only but too short");
    assert_eq!(normalize_phone("07712a4567", Some("94")), None, "plausible length but not all digits");
}

// ---- store: the default is_empty and the conformance suite itself -----------

#[test]
fn is_empty_reflects_the_number_of_held_events() {
    let mut s = MemoryStore::new();
    assert!(s.is_empty().unwrap());
    s.insert(conformance::sample_event("a", 1)).unwrap();
    assert!(!s.is_empty().unwrap());
    s.drain().unwrap();
    assert!(s.is_empty().unwrap());
}

/// A store that reports every insert as new, so it fails to de-duplicate.
#[derive(Default)]
struct ForgetfulStore(MemoryStore);

impl EventStore for ForgetfulStore {
    fn insert(&mut self, event: Event) -> Result<bool> {
        self.0.insert(event)?;
        Ok(true)
    }
    fn len(&self) -> Result<usize> {
        self.0.len()
    }
    fn list(&self) -> Result<Vec<Event>> {
        self.0.list()
    }
    fn drain(&mut self) -> Result<Vec<Event>> {
        self.0.drain()
    }
    fn acknowledge(&mut self, ids: &[String]) -> Result<usize> {
        self.0.acknowledge(ids)
    }
}

#[test]
fn the_conformance_suite_rejects_a_store_that_does_not_deduplicate() {
    conformance::run(MemoryStore::new);
    let outcome = catch_unwind(AssertUnwindSafe(|| conformance::run(ForgetfulStore::default)));
    assert!(outcome.is_err(), "a broken store must fail the suite");
}

// ---- error: missing-field names with underscores, and positions -------------

#[test]
fn a_missing_field_name_with_an_underscore_is_kept() {
    let e = serde_json::from_value::<RawSms>(json!({ "address": "x", "body": "y", "type": 1 })).unwrap_err();
    assert_eq!(
        describe_json_error(&e, false),
        "the JSON does not match the expected shape (missing field `date_ms`)"
    );
}

#[test]
fn positions_are_added_only_when_asked_for_and_known() {
    // Parsed from text: the error has a position.
    let from_text = serde_json::from_str::<RawSms>(r#"{"address":"x","body":"y","type":1}"#).unwrap_err();
    assert!(describe_json_error(&from_text, true).contains(" at line 1 column "));
    assert!(!describe_json_error(&from_text, false).contains("line"));

    // Built from an already-parsed value: no position, even when asked for.
    let from_value = serde_json::from_value::<RawSms>(json!({ "address": "x" })).unwrap_err();
    assert_eq!(from_value.line(), 0);
    assert!(!describe_json_error(&from_value, true).contains("line"));
}
