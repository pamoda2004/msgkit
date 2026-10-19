//! Guards the published JSON Schema against drifting from the Rust types.
//!
//! Regenerate after an intentional model change with:
//!   `UPDATE_SCHEMA=1 cargo test -p msgkit-core --test schema`

use std::path::PathBuf;

use msgkit_core::device::{DeviceBatch, DeviceOptions};
use msgkit_core::facebook::FacebookExportOptions;
use msgkit_core::whatsapp_export::WhatsAppExportOptions;
use msgkit_core::{Event, ImportReport, SCHEMA_VERSION};
use schemars::schema_for;
use serde_json::json;

const VERSION_KEY: &str = "x-msgkit-schema-version";

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema")
}

/// Every schema file declares the `SCHEMA_VERSION` it was generated for (`docs/SCHEMA_POLICY.md`).
fn with_version(mut schema: serde_json::Value) -> serde_json::Value {
    schema[VERSION_KEY] = json!(SCHEMA_VERSION);
    schema
}

fn check(file: &str, schema: &serde_json::Value) {
    let path = schema_dir().join(file);
    let generated = format!("{}\n", serde_json::to_string_pretty(&with_version(schema.clone())).unwrap());

    if std::env::var_os("UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, generated).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; run with UPDATE_SCHEMA=1", path.display()))
        .replace("\r\n", "\n");
    assert_eq!(
        on_disk, generated,
        "{file} is out of date; run `UPDATE_SCHEMA=1 cargo test -p msgkit-core --test schema`"
    );
}

#[test]
fn event_schema_is_current() {
    check("event.schema.json", &json!(schema_for!(Event)));
}

#[test]
fn import_report_schema_is_current() {
    check("import-report.schema.json", &json!(schema_for!(ImportReport)));
}

#[test]
fn options_schema_is_current() {
    check(
        "options.schema.json",
        &json!({
            "whatsapp_export": schema_for!(WhatsAppExportOptions),
            "facebook_export": schema_for!(FacebookExportOptions),
            "device_options": schema_for!(DeviceOptions),
            "device_batch": schema_for!(DeviceBatch),
        }),
    );
}
