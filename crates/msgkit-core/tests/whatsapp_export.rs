use msgkit_core::whatsapp_export::{parse, DateOrder, WhatsAppExportOptions};
use msgkit_core::*;

const ANDROID_DMY: &str = include_str!("fixtures/whatsapp_android_dmy.txt");
const ANDROID_MDY: &str = include_str!("fixtures/whatsapp_android_mdy_12h.txt");

fn messages(r: &ImportReport) -> Vec<&Message> {
    r.events
        .iter()
        .filter_map(|e| match e {
            Event::Message(m) => Some(m),
            _ => None,
        })
        .collect()
}

fn ts(m: &Message) -> String {
    m.timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[test]
fn android_day_first_is_detected_without_warnings() {
    let r = parse(ANDROID_DMY, &WhatsAppExportOptions::default());
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let msgs = messages(&r);
    assert_eq!(msgs.len(), 7);
    assert_eq!(r.events.len(), 8, "7 messages + 1 missed call");
    assert_eq!(ts(msgs[0]), "2023-12-31T21:41:00Z");
    assert_eq!(msgs[0].body.as_deref(), Some("Happy new year!"));
    assert_eq!(msgs[0].channel, Channel::Whatsapp);
}

#[test]
fn identical_messages_in_the_same_minute_get_distinct_ids() {
    let r = parse(ANDROID_DMY, &WhatsAppExportOptions::default());
    let oks: Vec<_> = messages(&r).into_iter().filter(|m| m.body.as_deref() == Some("ok")).collect();
    assert_eq!(oks.len(), 2);
    assert_ne!(oks[0].id, oks[1].id);
}

#[test]
fn reimport_produces_identical_ids() {
    let a = parse(ANDROID_DMY, &WhatsAppExportOptions::default());
    let b = parse(ANDROID_DMY, &WhatsAppExportOptions::default());
    assert_eq!(a, b);
}

#[test]
fn multiline_messages_are_joined() {
    let r = parse(ANDROID_DMY, &WhatsAppExportOptions::default());
    let m = messages(&r)
        .into_iter()
        .find(|m| m.body.as_deref().is_some_and(|b| b.starts_with("line one")))
        .unwrap();
    assert_eq!(m.body.as_deref(), Some("line one\nline two"));
}

#[test]
fn media_placeholders_become_attachments() {
    let r = parse(ANDROID_DMY, &WhatsAppExportOptions::default());
    let msgs = messages(&r);
    let omitted = msgs.iter().find(|m| m.sender.id == "Alice" && m.body.is_none()).unwrap();
    assert_eq!(omitted.attachments.len(), 1);
    let file = msgs.iter().find(|m| !m.attachments.is_empty() && m.sender.id == "Bob").unwrap();
    assert_eq!(file.attachments[0].kind, AttachmentKind::Image);
    assert_eq!(file.attachments[0].file_name.as_deref(), Some("IMG-20240101-WA0001.jpg"));
    assert!(file.body.is_none());
}

#[test]
fn missed_call_notice_becomes_a_call() {
    let r = parse(ANDROID_DMY, &WhatsAppExportOptions::default());
    let call = r
        .events
        .iter()
        .find_map(|e| match e {
            Event::Call(c) => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(call.outcome, CallOutcome::Missed);
    assert!(!call.video);
}

#[test]
fn self_name_sets_direction() {
    let opts = WhatsAppExportOptions { self_name: Some("alice".into()), ..Default::default() };
    let r = parse(ANDROID_DMY, &opts);
    for m in messages(&r) {
        let expected = if m.sender.id == "Alice" { Direction::Outbound } else { Direction::Inbound };
        assert_eq!(m.direction, expected);
    }
    let r = parse(ANDROID_DMY, &WhatsAppExportOptions::default());
    assert!(messages(&r).iter().all(|m| m.direction == Direction::Unknown));
}

#[test]
fn utc_offset_is_applied() {
    let opts = WhatsAppExportOptions { utc_offset_minutes: 330, ..Default::default() };
    let r = parse(ANDROID_DMY, &opts);
    assert_eq!(ts(messages(&r)[0]), "2023-12-31T16:11:00Z");
}

#[test]
fn month_first_12_hour_clock() {
    let r = parse(ANDROID_MDY, &WhatsAppExportOptions::default());
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let t: Vec<_> = messages(&r).iter().map(|m| ts(m)).collect();
    assert_eq!(t, ["2023-12-30T23:59:00Z", "2023-12-31T00:05:00Z", "2023-12-31T12:30:00Z"]);
}

#[test]
fn ambiguous_dates_warn_and_can_be_overridden() {
    let text = "03/04/2024, 10:00 - A: x\n";
    let auto = parse(text, &WhatsAppExportOptions::default());
    assert!(auto.warnings.iter().any(|w| w.contains("ambiguous")));
    assert_eq!(ts(messages(&auto)[0]), "2024-04-03T10:00:00Z");

    let mdy = parse(text, &WhatsAppExportOptions { date_order: DateOrder::Mdy, ..Default::default() });
    assert!(mdy.warnings.is_empty());
    assert_eq!(ts(messages(&mdy)[0]), "2024-03-04T10:00:00Z");
}

#[test]
fn ios_format_with_invisible_marks_and_narrow_nbsp() {
    let text = "\u{200e}[31/12/2023, 9:41:05\u{202f}PM] Alice: Hello\n\u{200e}[31/12/2023, 21:42:10] Bob: \u{200e}image omitted\n";
    let r = parse(text, &WhatsAppExportOptions::default());
    let msgs = messages(&r);
    assert_eq!(msgs.len(), 2);
    assert_eq!(ts(msgs[0]), "2023-12-31T21:41:05Z");
    assert_eq!(msgs[0].sender.id, "Alice");
    assert_eq!(msgs[1].attachments[0].kind, AttachmentKind::Image);
}

#[test]
fn phone_number_senders_are_normalized() {
    let text = "01/02/2024, 10:00 - +94 77 123 4567: hi\n";
    let r = parse(text, &WhatsAppExportOptions::default());
    let m = messages(&r)[0];
    assert_eq!(m.sender.id, "+94771234567");
    assert_eq!(m.sender.display_name.as_deref(), Some("+94 77 123 4567"));
}

#[test]
fn garbage_input_yields_a_warning_not_a_panic() {
    let r = parse("this is not a chat export\nat all", &WhatsAppExportOptions::default());
    assert!(r.events.is_empty());
    assert!(!r.warnings.is_empty());
    let r = parse("", &WhatsAppExportOptions::default());
    assert!(r.events.is_empty());
}

#[test]
fn impossible_dates_are_skipped_with_a_warning() {
    let text = "31/02/2024, 10:00 - A: bad\n01/03/2024, 10:00 - A: good\n";
    let r = parse(text, &WhatsAppExportOptions::default());
    assert_eq!(messages(&r).len(), 1);
    assert!(r.warnings.iter().any(|w| w.contains("could not be parsed")));
}
