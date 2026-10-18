#![no_main]
//! Feeds arbitrary bytes to the Facebook export parser, both as a document and as an already-parsed value.

use libfuzzer_sys::fuzz_target;
use msgkit_core::facebook::{self, FacebookExportOptions};

fuzz_target!(|data: &[u8]| {
    let opts = FacebookExportOptions::default();
    let text = String::from_utf8_lossy(data);
    let _ = facebook::parse(&text, &opts);
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) {
        let _ = facebook::parse_value(value, &opts);
    }
});
