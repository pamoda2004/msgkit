#![no_main]
//! The first two bytes pick the options (UTC offset, date order and time zone); the rest is the exported text.

use libfuzzer_sys::fuzz_target;
use msgkit_core::whatsapp_export::{parse, DateOrder, WhatsAppExportOptions};

fuzz_target!(|data: &[u8]| {
    let Some((head, rest)) = data.split_at_checked(2) else { return };
    let order = match head[1] % 3 {
        0 => DateOrder::Auto,
        1 => DateOrder::Dmy,
        _ => DateOrder::Mdy,
    };
    let mut opts = WhatsAppExportOptions::default()
        .with_utc_offset_minutes(i32::from(head[0]) * 10 - 1270)
        .with_date_order(order);
    opts.time_zone = [None, Some("Asia/Colombo"), Some("America/New_York"), Some("Australia/Lord_Howe"), Some("Europe/London")]
        [usize::from(head[1] / 3) % 5]
        .map(str::to_owned);
    let report = parse(&String::from_utf8_lossy(rest), &opts);
    // Re-importing must give the same ids.
    assert_eq!(report, parse(&String::from_utf8_lossy(rest), &opts));
});
