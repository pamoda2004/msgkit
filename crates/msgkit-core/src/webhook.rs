//! Signature verification and payload normalization for the official
//! WhatsApp Business Cloud API and Messenger Platform webhooks.
//!
//! Both platforms sign the raw request body with HMAC-SHA256 using the app
//! secret and send it as `X-Hub-Signature-256: sha256=<hex>`. Always verify
//! the signature over the *unparsed* body before calling a `normalize_*`
//! function.
//!
//! Not covered: WhatsApp call webhooks (the Cloud API calling payloads are not
//! normalized yet), delivery/read receipts and other non-message events.

use chrono::DateTime;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;

use crate::error::{Error, Result};
use crate::id::derive_id;
use crate::model::{
    Attachment, AttachmentKind, Channel, Direction, Event, ImportReport, Message, Participant,
};

type HmacSha256 = Hmac<Sha256>;

/// Checks an `X-Hub-Signature-256` header against the raw body, in constant time.
#[must_use]
pub fn verify_signature(app_secret: &[u8], body: &[u8], header: &str) -> bool {
    let Some(hex_sig) = header.trim().strip_prefix("sha256=") else {
        return false;
    };
    let Ok(sig) = hex::decode(hex_sig) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(app_secret) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

/// Computes the header value Meta would send. Useful for tests and for
/// clients that simulate webhooks.
#[must_use]
pub fn sign(app_secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(app_secret).expect("HMAC accepts any key length");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for key in path {
        cur = cur.get(key)?;
    }
    cur.as_str()
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn kind_from_str(s: &str) -> AttachmentKind {
    match s {
        "image" | "sticker" => AttachmentKind::Image,
        "video" => AttachmentKind::Video,
        "audio" | "voice" => AttachmentKind::Audio,
        "document" | "file" => AttachmentKind::File,
        _ => AttachmentKind::Other,
    }
}

/// Normalizes a WhatsApp Cloud API webhook (`object: whatsapp_business_account`).
///
/// Messages are inbound: they were sent to the business number. Status updates
/// are ignored.
pub fn normalize_whatsapp(payload: &Value) -> Result<ImportReport> {
    if str_at(payload, &["object"]) != Some("whatsapp_business_account") {
        return Err(Error::Parse(
            "not a WhatsApp Business webhook (object != whatsapp_business_account)".into(),
        ));
    }
    let mut report = ImportReport::default();

    for entry in array(payload, "entry") {
        for change in array(entry, "changes") {
            let Some(value) = change.get("value") else { continue };
            let business = str_at(value, &["metadata", "display_phone_number"])
                .map(|n| Participant::new(format!("+{}", n.trim_start_matches('+').replace([' ', '-'], ""))));

            for msg in array(value, "messages") {
                let (Some(from), Some(ext_id), Some(kind)) =
                    (str_at(msg, &["from"]), str_at(msg, &["id"]), str_at(msg, &["type"]))
                else {
                    report.warnings.push("skipped a message missing from/id/type".into());
                    continue;
                };
                let Some(timestamp) = str_at(msg, &["timestamp"])
                    .and_then(|t| t.parse::<i64>().ok())
                    .and_then(|t| DateTime::from_timestamp(t, 0))
                else {
                    report.warnings.push(format!("message {ext_id} has no valid timestamp"));
                    continue;
                };

                let profile = array(value, "contacts")
                    .iter()
                    .find(|c| str_at(c, &["wa_id"]) == Some(from))
                    .and_then(|c| str_at(c, &["profile", "name"]));
                let sender_id = format!("+{}", from.trim_start_matches('+'));
                let sender = match profile {
                    Some(name) => Participant::with_name(sender_id, name),
                    None => Participant::new(sender_id),
                };

                let mut attachments = Vec::new();
                let body = match kind {
                    "text" => str_at(msg, &["text", "body"]).map(str::to_owned),
                    "button" => str_at(msg, &["button", "text"]).map(str::to_owned),
                    "interactive" => str_at(msg, &["interactive", "button_reply", "title"])
                        .or_else(|| str_at(msg, &["interactive", "list_reply", "title"]))
                        .map(str::to_owned),
                    "image" | "video" | "audio" | "voice" | "document" | "sticker" => {
                        let m = msg.get(kind);
                        let mut a = Attachment::new(kind_from_str(kind));
                        a.source_id = m.and_then(|m| str_at(m, &["id"])).map(str::to_owned);
                        a.mime_type = m.and_then(|m| str_at(m, &["mime_type"])).map(str::to_owned);
                        a.file_name = m.and_then(|m| str_at(m, &["filename"])).map(str::to_owned);
                        attachments.push(a);
                        m.and_then(|m| str_at(m, &["caption"])).map(str::to_owned)
                    }
                    other => {
                        report.warnings.push(format!(
                            "message {ext_id}: unsupported type '{other}', body not extracted"
                        ));
                        None
                    }
                };

                report.events.push(Event::Message(Message {
                    id: derive_id(&["whatsapp", "msg", ext_id]),
                    channel: Channel::Whatsapp,
                    external_id: Some(ext_id.to_owned()),
                    conversation_id: Some(from.to_owned()),
                    direction: Direction::Inbound,
                    timestamp,
                    sender,
                    recipients: business.iter().cloned().collect(),
                    body,
                    attachments,
                    raw: Some(msg.clone()),
                }));
            }
        }
    }
    report.sort();
    Ok(report)
}

/// Normalizes a Messenger Platform webhook (`object: page`).
///
/// Echoes of messages the page itself sent are marked outbound.
pub fn normalize_messenger(payload: &Value) -> Result<ImportReport> {
    if str_at(payload, &["object"]) != Some("page") {
        return Err(Error::Parse("not a Messenger webhook (object != page)".into()));
    }
    let mut report = ImportReport::default();

    for entry in array(payload, "entry") {
        for item in array(entry, "messaging") {
            let Some(message) = item.get("message") else { continue };
            let (Some(sender), Some(recipient), Some(mid)) = (
                str_at(item, &["sender", "id"]),
                str_at(item, &["recipient", "id"]),
                str_at(message, &["mid"]),
            ) else {
                report.warnings.push("skipped a message missing sender/recipient/mid".into());
                continue;
            };
            let Some(timestamp) =
                item.get("timestamp").and_then(Value::as_i64).and_then(DateTime::from_timestamp_millis)
            else {
                report.warnings.push(format!("message {mid} has no valid timestamp"));
                continue;
            };

            let echo = message.get("is_echo").and_then(Value::as_bool).unwrap_or(false);
            let (direction, conversation) =
                if echo { (Direction::Outbound, recipient) } else { (Direction::Inbound, sender) };

            let attachments = array(message, "attachments")
                .iter()
                .map(|a| {
                    let mut att = Attachment::new(kind_from_str(str_at(a, &["type"]).unwrap_or("")));
                    att.uri = str_at(a, &["payload", "url"]).map(str::to_owned);
                    att
                })
                .collect();

            report.events.push(Event::Message(Message {
                id: derive_id(&["messenger", "msg", mid]),
                channel: Channel::Messenger,
                external_id: Some(mid.to_owned()),
                conversation_id: Some(conversation.to_owned()),
                direction,
                timestamp,
                sender: Participant::new(sender),
                recipients: vec![Participant::new(recipient)],
                body: str_at(message, &["text"]).map(str::to_owned),
                attachments,
                raw: Some(item.clone()),
            }));
        }
    }
    report.sort();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_round_trip() {
        let sig = sign(b"secret", b"{\"a\":1}");
        assert!(verify_signature(b"secret", b"{\"a\":1}", &sig));
    }

    #[test]
    fn rejects_tampered_body_wrong_key_and_garbage() {
        let sig = sign(b"secret", b"body");
        assert!(!verify_signature(b"secret", b"bodY", &sig));
        assert!(!verify_signature(b"other", b"body", &sig));
        assert!(!verify_signature(b"secret", b"body", "sha256=zz"));
        assert!(!verify_signature(b"secret", b"body", "md5=abcd"));
        assert!(!verify_signature(b"secret", b"body", ""));
    }

    #[test]
    fn rejects_wrong_object_type() {
        let v: Value = serde_json::json!({"object": "page"});
        assert!(normalize_whatsapp(&v).is_err());
        let v: Value = serde_json::json!({"object": "whatsapp_business_account"});
        assert!(normalize_messenger(&v).is_err());
    }
}
