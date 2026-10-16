//! The language-neutral entry point: one function, JSON in, JSON out.
//!
//! The C ABI (`msgkit-ffi`), WebAssembly and native bindings all go through
//! [`dispatch`], so every language gets identical behaviour. The wire format
//! is documented in `docs/API.md`.
//!
//! Response envelope:
//! ```json
//! {"ok": true,  "data": { ... }}
//! {"ok": false, "error": {"code": "invalid_input", "message": "..."}}
//! ```

use serde::Deserialize;
use serde_json::{json, Value};

use crate::device::{self, DeviceBatch, DeviceOptions};
use crate::error::{Error, Result};
use crate::facebook::{self, FacebookExportOptions};
use crate::model::ImportReport;
use crate::webhook;
use crate::whatsapp_export::{self, WhatsAppExportOptions};
use crate::SCHEMA_VERSION;

/// Operations accepted by [`dispatch`].
pub const OPERATIONS: &[&str] = &[
    "version",
    "parse_whatsapp_export",
    "parse_facebook_export",
    "normalize_device",
    "normalize_whatsapp_webhook",
    "normalize_messenger_webhook",
    "verify_signature",
];

#[derive(Deserialize)]
struct WhatsAppExportInput {
    text: String,
    #[serde(default)]
    options: WhatsAppExportOptions,
}

#[derive(Deserialize)]
struct FacebookExportInput {
    thread: Value,
    #[serde(default)]
    options: FacebookExportOptions,
}

#[derive(Deserialize)]
struct DeviceInput {
    batch: DeviceBatch,
    #[serde(default)]
    options: DeviceOptions,
}

#[derive(Deserialize)]
struct WebhookInput {
    payload: Value,
}

#[derive(Deserialize)]
struct SignatureInput {
    secret: String,
    body: String,
    signature: String,
}

fn report_json(report: ImportReport) -> Result<Value> {
    Ok(serde_json::to_value(report)?)
}

fn run(op: &str, input: &str) -> Result<Value> {
    match op {
        "version" => Ok(json!({
            "crate_version": env!("CARGO_PKG_VERSION"),
            "schema_version": SCHEMA_VERSION,
            "operations": OPERATIONS,
        })),
        "parse_whatsapp_export" => {
            let i: WhatsAppExportInput = serde_json::from_str(input)?;
            report_json(whatsapp_export::parse(&i.text, &i.options))
        }
        "parse_facebook_export" => {
            let i: FacebookExportInput = serde_json::from_str(input)?;
            report_json(facebook::parse_value(i.thread, &i.options)?)
        }
        "normalize_device" => {
            let i: DeviceInput = serde_json::from_str(input)?;
            report_json(device::normalize(&i.batch, &i.options))
        }
        "normalize_whatsapp_webhook" => {
            let i: WebhookInput = serde_json::from_str(input)?;
            report_json(webhook::normalize_whatsapp(&i.payload)?)
        }
        "normalize_messenger_webhook" => {
            let i: WebhookInput = serde_json::from_str(input)?;
            report_json(webhook::normalize_messenger(&i.payload)?)
        }
        "verify_signature" => {
            let i: SignatureInput = serde_json::from_str(input)?;
            let valid = webhook::verify_signature(i.secret.as_bytes(), i.body.as_bytes(), &i.signature);
            Ok(json!({ "valid": valid }))
        }
        other => Err(Error::UnknownOp(other.to_owned())),
    }
}

/// Runs `op` with a JSON `input` document and returns the JSON envelope as a
/// string. Never panics on bad input.
pub fn dispatch(op: &str, input: &str) -> String {
    let envelope = match run(op, input) {
        Ok(data) => json!({ "ok": true, "data": data }),
        Err(e) => json!({ "ok": false, "error": { "code": e.code(), "message": e.to_string() } }),
    };
    envelope.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn version_lists_operations() {
        let v = parse(&dispatch("version", "{}"));
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["schema_version"], SCHEMA_VERSION);
    }

    #[test]
    fn unknown_op_is_an_error_envelope() {
        let v = parse(&dispatch("nope", "{}"));
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "unknown_op");
    }

    #[test]
    fn malformed_input_is_an_error_envelope() {
        let v = parse(&dispatch("parse_whatsapp_export", "not json"));
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "invalid_input");
    }
}
