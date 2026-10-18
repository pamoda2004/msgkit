#![no_main]
//! Feeds arbitrary JSON to both webhook normalizers, bare and wrapped in the expected envelope.

use libfuzzer_sys::fuzz_target;
use msgkit_core::webhook;
use serde_json::{json, Value};

fuzz_target!(|data: &[u8]| {
    let Ok(payload) = serde_json::from_slice::<Value>(data) else { return };
    let _ = webhook::normalize_whatsapp(&payload);
    let _ = webhook::normalize_messenger(&payload);
    let page = json!({ "object": "page", "entry": [payload.clone()] });
    let _ = webhook::normalize_messenger(&page);
    let whatsapp = json!({ "object": "whatsapp_business_account", "entry": [{ "changes": [{ "value": payload }] }] });
    let _ = webhook::normalize_whatsapp(&whatsapp);
});
