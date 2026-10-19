//! The options structs are `#[non_exhaustive]` (new options are added over time), so Rust users build them with
//! `Default` plus the `with_*` setters, or by assigning fields. Each setter sets exactly one field.

use msgkit_core::device::DeviceOptions;
use msgkit_core::facebook::FacebookExportOptions;
use msgkit_core::whatsapp_export::{DateOrder, WhatsAppExportOptions};

#[test]
fn whatsapp_setters_set_exactly_their_field() {
    let base = WhatsAppExportOptions::default();
    assert!(base.self_name.is_none() && base.conversation_id.is_none());
    assert!(base.default_country_code.is_none() && base.time_zone.is_none());
    assert_eq!((base.date_order, base.utc_offset_minutes), (DateOrder::Auto, 0));

    let o = WhatsAppExportOptions::default().with_self_name("Alice");
    assert_eq!(o.self_name.as_deref(), Some("Alice"));
    assert!(o.conversation_id.is_none() && o.default_country_code.is_none() && o.time_zone.is_none());

    let o = WhatsAppExportOptions::default().with_date_order(DateOrder::Mdy);
    assert_eq!((o.date_order, o.utc_offset_minutes, o.self_name), (DateOrder::Mdy, 0, None));

    let o = WhatsAppExportOptions::default().with_utc_offset_minutes(-330);
    assert_eq!((o.utc_offset_minutes, o.date_order), (-330, DateOrder::Auto));

    let o = WhatsAppExportOptions::default().with_conversation_id("chat-1");
    assert_eq!(o.conversation_id.as_deref(), Some("chat-1"));
    assert!(o.self_name.is_none());

    let o = WhatsAppExportOptions::default().with_default_country_code("94");
    assert_eq!(o.default_country_code.as_deref(), Some("94"));
    assert!(o.time_zone.is_none());

    let o = WhatsAppExportOptions::default().with_time_zone(String::from("Asia/Colombo"));
    assert_eq!(o.time_zone.as_deref(), Some("Asia/Colombo"));
    assert!(o.default_country_code.is_none());
}

#[test]
fn setters_chain_and_equal_field_assignment() {
    let chained = WhatsAppExportOptions::default()
        .with_self_name("Alice")
        .with_date_order(DateOrder::Dmy)
        .with_utc_offset_minutes(330)
        .with_conversation_id("c")
        .with_default_country_code("94")
        .with_time_zone("Asia/Colombo");

    let mut assigned = WhatsAppExportOptions::default();
    assigned.self_name = Some("Alice".into());
    assigned.date_order = DateOrder::Dmy;
    assigned.utc_offset_minutes = 330;
    assigned.conversation_id = Some("c".into());
    assigned.default_country_code = Some("94".into());
    assigned.time_zone = Some("Asia/Colombo".into());

    assert_eq!(serde_json::to_value(&chained).unwrap(), serde_json::to_value(&assigned).unwrap());
}

#[test]
fn facebook_and_device_setters() {
    let f = FacebookExportOptions::default().with_self_name("Alice").with_conversation_id("c");
    assert_eq!((f.self_name.as_deref(), f.conversation_id.as_deref()), (Some("Alice"), Some("c")));
    let only = FacebookExportOptions::default().with_conversation_id("c");
    assert!(only.self_name.is_none());

    let d = DeviceOptions::default().with_own_number("+94700000000").with_default_country_code("94");
    assert_eq!(
        (d.own_number.as_deref(), d.default_country_code.as_deref()),
        (Some("+94700000000"), Some("94"))
    );
    let only = DeviceOptions::default().with_default_country_code("94");
    assert!(only.own_number.is_none());
}

#[test]
fn json_options_are_unchanged_by_non_exhaustive() {
    // Deserialization (the path every binding uses) ignores `#[non_exhaustive]`.
    let o: WhatsAppExportOptions =
        serde_json::from_str(r#"{"self_name":"Alice","time_zone":"Asia/Colombo","utc_offset_minutes":330}"#)
            .unwrap();
    assert_eq!(o.time_zone.as_deref(), Some("Asia/Colombo"));
    let empty: WhatsAppExportOptions = serde_json::from_str("{}").unwrap();
    assert_eq!(
        serde_json::to_value(&empty).unwrap(),
        serde_json::to_value(WhatsAppExportOptions::default()).unwrap()
    );
}
