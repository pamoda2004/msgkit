#![no_main]
//! The JSON entry point every binding uses: the first byte picks the operation, the rest is the input.
//! Whatever happens, the answer must be a JSON envelope.

use libfuzzer_sys::fuzz_target;
use msgkit_core::api;

const OPS: [&str; 7] = [
    "version",
    "parse_whatsapp_export",
    "parse_facebook_export",
    "normalize_device",
    "normalize_whatsapp_webhook",
    "normalize_messenger_webhook",
    "verify_signature",
];

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else { return };
    let response = api::dispatch(OPS[usize::from(first) % OPS.len()], &String::from_utf8_lossy(rest));
    let envelope: serde_json::Value = serde_json::from_str(&response).expect("the response is JSON");
    assert!(envelope["ok"].is_boolean());
});
