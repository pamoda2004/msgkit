#![no_main]
//! Feeds arbitrary batches of device rows to the SMS and call-log normalizer.

use libfuzzer_sys::fuzz_target;
use msgkit_core::device::{self, DeviceBatch, DeviceOptions};

fuzz_target!(|data: &[u8]| {
    if let Ok(batch) = serde_json::from_slice::<DeviceBatch>(data) {
        let opts = DeviceOptions::default().with_default_country_code("94");
        let _ = device::normalize(&batch, &opts);
        let _ = device::normalize(&batch, &DeviceOptions::default());
    }
});
