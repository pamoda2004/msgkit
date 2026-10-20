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

use std::time::Duration;

use chrono::DateTime;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha2::Sha256;

use crate::error::{Error, Result};
use crate::id::derive_id;
use crate::limits::Limits;
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

/// How an event's timestamp compares with the clock, under a [`ReplayPolicy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Freshness {
    /// Within the window (and not from the future beyond the allowed clock skew).
    Fresh,
    /// Older than the window.
    Stale,
    /// Further in the future than the allowed clock skew.
    Future,
}

/// Which webhook events are too old, or too far ahead, to be believed (replay protection, F14).
///
/// Meta signs the whole body, so a replayer cannot change the timestamp inside it. Together with the de-duplication
/// by event id this closes the gap after the ids are forgotten: an event older than the window is refused whatever
/// the store remembers, so the id retention must be at least as long as the window.
///
/// **A real message can be late.** Meta retries a failed WhatsApp delivery for up to 7 days, and the timestamp is
/// when the message was *sent*, so a user whose phone was offline can produce an old but genuine message. The
/// default window (8 days) covers Meta's longest retry; a shorter one drops such messages.
///
/// The check is pure: the caller passes the current time (`msgkit-core` never reads a clock), and the server's
/// clock must be synchronised (NTP) for it to mean anything.
///
/// ```
/// use std::time::Duration;
/// use msgkit_core::webhook::ReplayPolicy;
///
/// let policy = ReplayPolicy::new(Duration::from_secs(3600)).with_future_skew(Duration::from_secs(60));
/// assert_eq!(policy.window, Duration::from_secs(3600));
/// assert_eq!(ReplayPolicy::default().window, ReplayPolicy::DEFAULT_WINDOW);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReplayPolicy {
    /// The oldest an event may be.
    pub window: Duration,
    /// How far ahead of the clock an event may be, for clocks that differ a little.
    pub future_skew: Duration,
}

impl ReplayPolicy {
    /// 8 days: Meta's longest WhatsApp retry (7 days) and a day of margin.
    pub const DEFAULT_WINDOW: Duration = Duration::from_secs(8 * 24 * 60 * 60);
    /// 10 minutes.
    pub const DEFAULT_FUTURE_SKEW: Duration = Duration::from_secs(10 * 60);

    /// A policy with this window and the default clock skew.
    #[must_use]
    pub fn new(window: Duration) -> Self {
        ReplayPolicy { window, future_skew: Self::DEFAULT_FUTURE_SKEW }
    }

    /// Sets [`future_skew`](Self::future_skew).
    #[must_use]
    pub fn with_future_skew(mut self, skew: Duration) -> Self {
        self.future_skew = skew;
        self
    }

    /// Compares `event`'s timestamp with `now_unix_millis`. The edges are inclusive: an event exactly one window
    /// old, or exactly one skew ahead, is fresh.
    #[must_use]
    pub fn classify(&self, event: &Event, now_unix_millis: i64) -> Freshness {
        let millis = |d: Duration| i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
        let age = now_unix_millis.saturating_sub(event.timestamp().timestamp_millis());
        if age > millis(self.window) {
            Freshness::Stale
        } else if age < millis(self.future_skew).saturating_neg() {
            Freshness::Future
        } else {
            Freshness::Fresh
        }
    }
}

impl Default for ReplayPolicy {
    fn default() -> Self {
        Self::new(Self::DEFAULT_WINDOW)
    }
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
    normalize_whatsapp_with_limits(payload, &Limits::default())
}

/// Like [`normalize_whatsapp`], with the given limits: the nesting depth of `payload` and the number of
/// events it yields are checked (the byte size of an already parsed value is not known here).
///
/// # Errors
/// [`Error::Parse`] for a payload of another platform; [`Error::LimitExceeded`] when over `limits`.
pub fn normalize_whatsapp_with_limits(payload: &Value, limits: &Limits) -> Result<ImportReport> {
    limits.validate()?;
    limits.check_json_depth_value(payload)?;
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
    limits.check_messages(report.events.len())?;
    report.sort();
    Ok(report)
}

/// Normalizes a Messenger Platform webhook (`object: page`).
///
/// Echoes of messages the page itself sent are marked outbound.
pub fn normalize_messenger(payload: &Value) -> Result<ImportReport> {
    normalize_messenger_with_limits(payload, &Limits::default())
}

/// Like [`normalize_messenger`], with the given limits: the nesting depth of `payload` and the number of
/// events it yields are checked (the byte size of an already parsed value is not known here).
///
/// # Errors
/// [`Error::Parse`] for a payload of another platform; [`Error::LimitExceeded`] when over `limits`.
pub fn normalize_messenger_with_limits(payload: &Value, limits: &Limits) -> Result<ImportReport> {
    limits.validate()?;
    limits.check_json_depth_value(payload)?;
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
    limits.check_messages(report.events.len())?;
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

    #[test]
    fn replay_policy_edges_are_inclusive_and_cover_seconds_and_milliseconds() {
        use crate::store::conformance::event_from;
        let now_ms = 1_800_000_000_000_i64;
        let at = |offset_ms: i64| {
            // event_from takes seconds; the sub-second part is checked separately below.
            event_from("e", (now_ms + offset_ms) / 1000, "+94700000000", "+94711111111")
        };
        let policy = ReplayPolicy::new(Duration::from_secs(3600)).with_future_skew(Duration::from_secs(60));

        assert_eq!(policy.classify(&at(0), now_ms), Freshness::Fresh);
        assert_eq!(policy.classify(&at(-3_600_000), now_ms), Freshness::Fresh, "exactly one window old");
        assert_eq!(policy.classify(&at(-3_601_000), now_ms), Freshness::Stale);
        assert_eq!(policy.classify(&at(60_000), now_ms), Freshness::Fresh, "exactly one skew ahead");
        assert_eq!(policy.classify(&at(61_000), now_ms), Freshness::Future);
        // Far away in both directions, and the default policy.
        assert_eq!(policy.classify(&at(-86_400_000 * 400), now_ms), Freshness::Stale);
        assert_eq!(policy.classify(&at(86_400_000 * 400), now_ms), Freshness::Future);
        let d = ReplayPolicy::default();
        assert_eq!(d.window, Duration::from_secs(8 * 86_400));
        assert_eq!(d.future_skew, Duration::from_secs(600));
        assert_eq!(d.classify(&at(-7 * 86_400_000), now_ms), Freshness::Fresh, "Meta's longest retry");
        assert_eq!(d.classify(&at(-9 * 86_400_000), now_ms), Freshness::Stale);
        assert_eq!(
            d.classify(&at(9 * 60_000), now_ms),
            Freshness::Fresh,
            "9 minutes ahead is within the skew"
        );
        assert_eq!(d.classify(&at(11 * 60_000), now_ms), Freshness::Future);
        // A huge window or skew never overflows.
        let huge = ReplayPolicy::new(Duration::MAX).with_future_skew(Duration::MAX);
        assert_eq!(huge.classify(&at(0), now_ms), Freshness::Fresh);
        assert_eq!(huge.classify(&at(i64::MAX / 2000), i64::MIN / 2), Freshness::Fresh);
    }
}
