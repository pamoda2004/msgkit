#![no_main]
//! The first two bytes pick the options (UTC offset and date order); the rest is the exported text.

use libfuzzer_sys::fuzz_target;
use msgkit_core::whatsapp_export::{parse, DateOrder, WhatsAppExportOptions};

fuzz_target!(|data: &[u8]| {
    let Some((head, rest)) = data.split_at_checked(2) else { return };
    let order = match head[1] % 3 {
        0 => DateOrder::Auto,
        1 => DateOrder::Dmy,
        _ => DateOrder::Mdy,
    };
    let opts = WhatsAppExportOptions {
        utc_offset_minutes: i32::from(head[0]) * 10 - 1270,
        date_order: order,
        ..WhatsAppExportOptions::default()
    };
    let report = parse(&String::from_utf8_lossy(rest), &opts);
    // Re-importing must give the same ids.
    assert_eq!(report, parse(&String::from_utf8_lossy(rest), &opts));
});
