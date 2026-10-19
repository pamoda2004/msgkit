//! Replays the committed fuzz seed corpus (`fuzz/corpus/<target>/`) through the same code the cargo-fuzz targets
//! run, on the stable toolchain and on every platform. Fuzzing itself needs nightly and runs in CI on Linux; this
//! test keeps the seeds valid, makes sure they never panic, and doubles as a regression suite for crashes: when
//! a fuzz run finds one, add its input to the matching corpus directory.

use std::fs;
use std::path::{Path, PathBuf};

use msgkit_core::api;
use msgkit_core::device::{self, DeviceBatch, DeviceOptions};
use msgkit_core::facebook::{self, FacebookExportOptions};
use msgkit_core::webhook::{self, sign, verify_signature};
use msgkit_core::whatsapp_export::{parse, DateOrder, WhatsAppExportOptions};
use serde_json::{json, Value};

fn files(target: &str) -> Vec<(String, Vec<u8>)> {
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus").join(target);
    let mut out: Vec<(String, Vec<u8>)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), fs::read(&p).unwrap()))
        .collect();
    out.sort();
    assert!(!out.is_empty(), "the {target} corpus is empty");
    out
}

#[test]
fn whatsapp_export_corpus() {
    for (name, data) in files("whatsapp_export") {
        let Some((head, rest)) = data.split_at_checked(2) else { continue };
        let order = match head[1] % 3 {
            0 => DateOrder::Auto,
            1 => DateOrder::Dmy,
            _ => DateOrder::Mdy,
        };
        let opts = WhatsAppExportOptions {
            utc_offset_minutes: i32::from(head[0]) * 10 - 1270,
            date_order: order,
            time_zone: [
                None,
                Some("Asia/Colombo"),
                Some("America/New_York"),
                Some("Australia/Lord_Howe"),
                Some("Europe/London"),
            ][usize::from(head[1] / 3) % 5]
                .map(str::to_owned),
            ..WhatsAppExportOptions::default()
        };
        let text = String::from_utf8_lossy(rest);
        assert_eq!(parse(&text, &opts), parse(&text, &opts), "{name}");
    }
}

#[test]
fn facebook_export_corpus() {
    for (_, data) in files("facebook_export") {
        let opts = FacebookExportOptions::default();
        let _ = facebook::parse(&String::from_utf8_lossy(&data), &opts);
        if let Ok(value) = serde_json::from_slice::<Value>(&data) {
            let _ = facebook::parse_value(value, &opts);
        }
    }
}

#[test]
fn device_corpus() {
    for (name, data) in files("device") {
        let batch: DeviceBatch = serde_json::from_slice(&data).unwrap_or_else(|e| panic!("{name}: {e}"));
        let opts = DeviceOptions { default_country_code: Some("94".to_owned()), ..DeviceOptions::default() };
        let _ = device::normalize(&batch, &opts);
        let _ = device::normalize(&batch, &DeviceOptions::default());
    }
}

#[test]
fn webhook_corpus() {
    for (_, data) in files("webhooks") {
        let Ok(payload) = serde_json::from_slice::<Value>(&data) else { continue };
        let _ = webhook::normalize_whatsapp(&payload);
        let _ = webhook::normalize_messenger(&payload);
        let page = json!({ "object": "page", "entry": [payload.clone()] });
        let _ = webhook::normalize_messenger(&page);
        let whatsapp = json!({ "object": "whatsapp_business_account", "entry": [{ "changes": [{ "value": payload }] }] });
        let _ = webhook::normalize_whatsapp(&whatsapp);
    }
}

#[test]
fn verify_signature_corpus() {
    for (_, data) in files("verify_signature") {
        let Some((&split, rest)) = data.split_first() else { continue };
        let (secret, rest) = rest.split_at((usize::from(split) % (rest.len() + 1)).min(rest.len()));
        let (body, header) = rest.split_at(rest.len() / 2);
        let _ = verify_signature(secret, body, &String::from_utf8_lossy(header));
        let signature = sign(secret, body);
        assert!(verify_signature(secret, body, &signature));
        let mut other = body.to_vec();
        other.push(0);
        assert!(!verify_signature(secret, &other, &signature));
    }
}

#[test]
fn dispatch_corpus() {
    const OPS: [&str; 7] = [
        "version",
        "parse_whatsapp_export",
        "parse_facebook_export",
        "normalize_device",
        "normalize_whatsapp_webhook",
        "normalize_messenger_webhook",
        "verify_signature",
    ];
    for (name, data) in files("dispatch") {
        let Some((&first, rest)) = data.split_first() else { continue };
        let response = api::dispatch(OPS[usize::from(first) % OPS.len()], &String::from_utf8_lossy(rest));
        let envelope: Value = serde_json::from_str(&response).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(envelope["ok"].is_boolean(), "{name}");
    }
}
