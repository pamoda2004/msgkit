//! The normalized, source-independent data model.
//!
//! Every provider (exports, webhooks, device collectors) maps its own format
//! into these types. They are the public contract of msgkit: the JSON Schema
//! in `schema/` is generated from them, and every language binding consumes
//! that schema.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The network or transport a message or call travelled over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Text messages (SMS/MMS) on a mobile device.
    Sms,
    /// Ordinary cellular voice calls.
    Cellular,
    Whatsapp,
    Messenger,
}

impl Channel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Sms => "sms",
            Channel::Cellular => "cellular",
            Channel::Whatsapp => "whatsapp",
            Channel::Messenger => "messenger",
        }
    }
}

/// Whether the account owner received or sent the item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
    /// The source did not contain enough information to tell.
    Unknown,
}

/// How a call ended, from the account owner's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallOutcome {
    Answered,
    Missed,
    Rejected,
    Blocked,
    Voicemail,
    Unknown,
}

/// A person or address taking part in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Participant {
    /// Stable identifier. An E.164 phone number (`+94700000000`) when one is
    /// known, otherwise the platform's own id or the display name.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl Participant {
    pub fn new(id: impl Into<String>) -> Self {
        Participant { id: id.into(), display_name: None }
    }

    pub fn with_name(id: impl Into<String>, name: impl Into<String>) -> Self {
        Participant { id: id.into(), display_name: Some(name.into()) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    Video,
    Audio,
    File,
    Other,
}

/// A media item or file belonging to a message. msgkit records references,
/// never the bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Attachment {
    pub kind: AttachmentKind,
    /// Location of the media, if the source provided one (URL or export path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// The platform's own media id, used to download the bytes via its API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

impl Attachment {
    #[must_use]
    pub fn new(kind: AttachmentKind) -> Self {
        Attachment { kind, uri: None, mime_type: None, file_name: None, source_id: None }
    }
}

/// A text or media message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    /// Deterministic id: importing the same data twice yields the same id.
    pub id: String,
    pub channel: Channel,
    /// The id the source platform assigned, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub direction: Direction,
    /// ISO 8601, always UTC (for example `2024-01-01T00:00:00Z`).
    pub timestamp: DateTime<Utc>,
    pub sender: Participant,
    #[serde(default)]
    pub recipients: Vec<Participant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// The untouched source payload (webhook events only), kept so records can be re-parsed later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// A voice or video call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Call {
    pub id: String,
    pub channel: Channel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub direction: Direction,
    /// ISO 8601, always UTC (for example `2024-01-01T00:00:00Z`).
    pub timestamp: DateTime<Utc>,
    /// Talk time in seconds; `0` for calls that were never answered.
    pub duration_secs: u64,
    pub outcome: CallOutcome,
    #[serde(default)]
    pub video: bool,
    #[serde(default)]
    pub participants: Vec<Participant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// Anything msgkit can import: a message or a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Message(Message),
    Call(Call),
}

impl Event {
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Event::Message(m) => &m.id,
            Event::Call(c) => &c.id,
        }
    }

    #[must_use]
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Event::Message(m) => m.timestamp,
            Event::Call(c) => c.timestamp,
        }
    }

    #[must_use]
    pub fn channel(&self) -> Channel {
        match self {
            Event::Message(m) => m.channel,
            Event::Call(c) => c.channel,
        }
    }

    /// The `type` tag used in JSON: `"message"` or `"call"`.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Event::Message(_) => "message",
            Event::Call(_) => "call",
        }
    }
}

/// The result of an import: the events plus anything worth telling the user.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImportReport {
    pub events: Vec<Event>,
    /// Non-fatal problems, e.g. lines that could not be parsed.
    pub warnings: Vec<String>,
}

impl ImportReport {
    /// Orders events by time. The sort is stable, so events with equal
    /// timestamps (exports only have minute precision) keep source order.
    pub(crate) fn sort(&mut self) {
        self.events.sort_by_key(Event::timestamp);
    }
}
