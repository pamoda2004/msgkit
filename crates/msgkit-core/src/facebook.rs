//! Parser for Facebook "Download Your Information" Messenger exports (JSON).
//!
//! Each conversation lives in `messages/inbox/<thread>/message_N.json`. Pass
//! the contents of one file at a time to [`parse`] and merge the reports.
//!
//! Quirks handled here:
//! * Facebook writes UTF-8 bytes as Latin-1 escapes ("mojibake"), so `é`
//!   arrives as `Ã©`. Text is repaired when it round-trips cleanly.
//! * Messages are stored newest-first; output is sorted oldest-first.
//! * Calls are messages with `type: "Call"` and a `call_duration`.

use chrono::DateTime;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::id::{derive_id, Ordinals};
use crate::model::{
    Attachment, AttachmentKind, Call, CallOutcome, Channel, Direction, Event, ImportReport, Message,
    Participant,
};

/// Options for [`parse`] and [`parse_value`].
///
/// `#[non_exhaustive]`, like [`WhatsAppExportOptions`](crate::whatsapp_export::WhatsAppExportOptions): start from
/// [`Default`] and chain the `with_*` setters or assign fields.
///
/// ```
/// use msgkit_core::facebook::FacebookExportOptions;
///
/// let opts = FacebookExportOptions::default().with_self_name("Alice Perera");
/// assert_eq!(opts.self_name.as_deref(), Some("Alice Perera"));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
#[schemars(description = "Options for parsing Messenger `message_N.json` files.")]
pub struct FacebookExportOptions {
    /// The exporting user's name, used to decide message direction.
    pub self_name: Option<String>,
    /// Overrides the conversation id (defaults to the export's `thread_path`).
    pub conversation_id: Option<String>,
}

impl FacebookExportOptions {
    /// Sets [`self_name`](Self::self_name).
    #[must_use]
    pub fn with_self_name(mut self, name: impl Into<String>) -> Self {
        self.self_name = Some(name.into());
        self
    }

    /// Sets [`conversation_id`](Self::conversation_id).
    #[must_use]
    pub fn with_conversation_id(mut self, id: impl Into<String>) -> Self {
        self.conversation_id = Some(id.into());
        self
    }
}

#[derive(Deserialize)]
struct Thread {
    #[serde(default)]
    participants: Vec<Person>,
    #[serde(default)]
    messages: Vec<FbMessage>,
    #[serde(default, rename = "thread_path")]
    path: Option<String>,
}

#[derive(Deserialize)]
struct Person {
    name: String,
}

#[derive(Deserialize)]
struct Media {
    uri: String,
}

#[derive(Deserialize)]
struct Share {
    #[serde(default)]
    link: Option<String>,
}

#[derive(Deserialize)]
struct FbMessage {
    sender_name: String,
    timestamp_ms: i64,
    #[serde(default)]
    content: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    call_duration: Option<u64>,
    #[serde(default)]
    is_unsent: bool,
    #[serde(default)]
    photos: Vec<Media>,
    #[serde(default)]
    videos: Vec<Media>,
    #[serde(default)]
    audio_files: Vec<Media>,
    #[serde(default)]
    files: Vec<Media>,
    #[serde(default)]
    sticker: Option<Media>,
    #[serde(default)]
    share: Option<Share>,
}

/// Repairs Facebook's Latin-1-escaped UTF-8. Returns the input unchanged if it
/// is not mojibake (plain ASCII, or text that is genuinely non-Latin-1).
#[must_use]
pub fn fix_encoding(s: &str) -> String {
    let mut bytes = Vec::with_capacity(s.len());
    for c in s.chars() {
        match u8::try_from(u32::from(c)) {
            Ok(b) => bytes.push(b),
            Err(_) => return s.to_owned(),
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|_| s.to_owned())
}

/// Parses one `message_N.json` file.
pub fn parse(json: &str, opts: &FacebookExportOptions) -> Result<ImportReport> {
    let thread: Thread = serde_json::from_str(json).map_err(|e| {
        Error::Parse(format!(
            "not a Facebook messages export: {}",
            crate::error::describe_json_error(&e, true)
        ))
    })?;
    Ok(build(thread, opts))
}

/// Like [`parse`], for callers that already hold a JSON value.
pub fn parse_value(value: Value, opts: &FacebookExportOptions) -> Result<ImportReport> {
    let thread: Thread = serde_json::from_value(value).map_err(|e| {
        Error::Parse(format!(
            "not a Facebook messages export: {}",
            crate::error::describe_json_error(&e, false)
        ))
    })?;
    Ok(build(thread, opts))
}

fn media(items: &[Media], kind: AttachmentKind, out: &mut Vec<Attachment>) {
    for m in items {
        let mut a = Attachment::new(kind);
        a.uri = Some(m.uri.clone());
        out.push(a);
    }
}

fn build(thread: Thread, opts: &FacebookExportOptions) -> ImportReport {
    let mut report = ImportReport::default();
    let mut ordinals = Ordinals::default();
    let conversation_id = opts.conversation_id.clone().or_else(|| thread.path.clone());
    let participants: Vec<String> = thread.participants.iter().map(|p| fix_encoding(&p.name)).collect();
    let mut skipped = 0usize;

    // The file is newest-first; walk it oldest-first so ordinals are stable.
    for m in thread.messages.into_iter().rev() {
        let Some(timestamp) = DateTime::from_timestamp_millis(m.timestamp_ms) else {
            skipped += 1;
            continue;
        };
        if m.is_unsent {
            continue;
        }
        let sender_name = fix_encoding(&m.sender_name);
        let direction = match &opts.self_name {
            Some(me) if me.trim().eq_ignore_ascii_case(&sender_name) => Direction::Outbound,
            Some(_) => Direction::Inbound,
            None => Direction::Unknown,
        };
        let ts = m.timestamp_ms.to_string();
        let content = m.content.as_deref().map(fix_encoding);
        let others = || {
            participants
                .iter()
                .filter(|p| **p != sender_name)
                .map(|p| Participant::with_name(p.clone(), p.clone()))
        };

        let is_call = m.kind.as_deref() == Some("Call") || m.call_duration.is_some();
        if is_call {
            let duration = m.call_duration.unwrap_or(0);
            let video = content.as_deref().is_some_and(|c| c.to_ascii_lowercase().contains("video"));
            let n = ordinals.next(&format!("call|{ts}|{sender_name}"));
            report.events.push(Event::Call(Call {
                id: derive_id(&["messenger", "call", &ts, &sender_name, &n.to_string()]),
                channel: Channel::Messenger,
                external_id: None,
                conversation_id: conversation_id.clone(),
                direction,
                timestamp,
                duration_secs: duration,
                outcome: if duration > 0 { CallOutcome::Answered } else { CallOutcome::Missed },
                video,
                participants: participants
                    .iter()
                    .map(|p| Participant::with_name(p.clone(), p.clone()))
                    .collect(),
                raw: None,
            }));
            continue;
        }

        let mut attachments = Vec::new();
        media(&m.photos, AttachmentKind::Image, &mut attachments);
        media(&m.videos, AttachmentKind::Video, &mut attachments);
        media(&m.audio_files, AttachmentKind::Audio, &mut attachments);
        media(&m.files, AttachmentKind::File, &mut attachments);
        if let Some(s) = &m.sticker {
            media(std::slice::from_ref(s), AttachmentKind::Image, &mut attachments);
        }
        let body = content.filter(|c| !c.is_empty()).or_else(|| m.share.and_then(|s| s.link));

        let key_body = body.as_deref().unwrap_or("");
        let n = ordinals.next(&format!("msg|{ts}|{sender_name}|{key_body}"));
        report.events.push(Event::Message(Message {
            id: derive_id(&["messenger", "msg", &ts, &sender_name, key_body, &n.to_string()]),
            channel: Channel::Messenger,
            external_id: None,
            conversation_id: conversation_id.clone(),
            direction,
            timestamp,
            sender: Participant::with_name(sender_name.clone(), sender_name.clone()),
            recipients: others().collect(),
            body,
            attachments,
            raw: None,
        }));
    }

    if skipped > 0 {
        report.warnings.push(format!("{skipped} message(s) skipped because their timestamp was invalid"));
    }
    report.sort();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repairs_mojibake() {
        // "café ❤" as Facebook exports it.
        assert_eq!(fix_encoding("cafÃ© â\u{9d}¤"), "café ❤");
    }

    #[test]
    fn leaves_clean_text_alone() {
        assert_eq!(fix_encoding("hello"), "hello");
        assert_eq!(fix_encoding("café"), "café");
        assert_eq!(fix_encoding("සිංහල"), "සිංහල");
    }
}
