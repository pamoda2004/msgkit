//! Normalizer for records collected on a device (Android SMS and call log).
//!
//! Reading the device is platform-specific and lives in thin native
//! "collectors". A collector serializes what it read into [`DeviceBatch`]
//! (field names mirror Android's `Telephony.Sms` and `CallLog.Calls`
//! columns) and this module turns it into normalized events.

use chrono::DateTime;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::id::{derive_id, Ordinals};
use crate::limits::Limits;
use crate::model::{Call, CallOutcome, Channel, Direction, Event, ImportReport, Message, Participant};
use crate::phone::normalize_phone;

/// A row from `Telephony.Sms`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RawSms {
    /// `_id` column, if available. Only stable on one device.
    #[serde(default)]
    pub id: Option<String>,
    /// Remote party: a phone number or an alphanumeric sender id.
    pub address: String,
    pub body: String,
    /// Milliseconds since the Unix epoch.
    pub date_ms: i64,
    /// `type` column: `1` inbox, `2` sent. Other values map to `unknown`.
    #[serde(rename = "type")]
    pub kind: i32,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// A row from `CallLog.Calls`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RawCall {
    #[serde(default)]
    pub id: Option<String>,
    pub number: String,
    /// Cached contact name, if any.
    #[serde(default)]
    pub name: Option<String>,
    pub date_ms: i64,
    #[serde(default)]
    pub duration_secs: u64,
    /// `type` column: `1` incoming, `2` outgoing, `3` missed, `4` voicemail,
    /// `5` rejected, `6` blocked.
    #[serde(rename = "type")]
    pub kind: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DeviceBatch {
    pub sms: Vec<RawSms>,
    pub calls: Vec<RawCall>,
}

/// Options for [`normalize`].
///
/// `#[non_exhaustive]`, like [`WhatsAppExportOptions`](crate::whatsapp_export::WhatsAppExportOptions): start from
/// [`Default`] and chain the `with_*` setters or assign fields.
///
/// ```
/// use msgkit_core::device::DeviceOptions;
///
/// let opts = DeviceOptions::default().with_own_number("+94700000000").with_default_country_code("94");
/// assert_eq!(opts.default_country_code.as_deref(), Some("94"));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[non_exhaustive]
#[schemars(description = "Options for normalizing Android SMS and call-log rows.")]
pub struct DeviceOptions {
    /// The device owner's own number, recorded as the other party of each
    /// message. Optional.
    pub own_number: Option<String>,
    /// Calling code (digits) for national numbers such as `0700000000`.
    pub default_country_code: Option<String>,
    /// Upper bounds for this import. The defaults suit real devices. Only `max_messages` (SMS rows plus call
    /// rows) and the size and depth of the whole request apply to a batch. Over a limit the import fails with
    /// `limit_exceeded`.
    pub limits: Limits,
}

impl DeviceOptions {
    /// Sets [`own_number`](Self::own_number).
    #[must_use]
    pub fn with_own_number(mut self, number: impl Into<String>) -> Self {
        self.own_number = Some(number.into());
        self
    }

    /// Sets [`default_country_code`](Self::default_country_code).
    #[must_use]
    pub fn with_default_country_code(mut self, code: impl Into<String>) -> Self {
        self.default_country_code = Some(code.into());
        self
    }

    /// Sets [`limits`](Self::limits).
    #[must_use]
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
}

/// Infallible: when the batch is over `opts.limits`, the report has no events and one warning that names the
/// limit. Use [`try_normalize`] to get `limit_exceeded` as an error instead.
#[must_use]
pub fn normalize(batch: &DeviceBatch, opts: &DeviceOptions) -> ImportReport {
    try_normalize(batch, opts).unwrap_or_else(|e| {
        let mut report = ImportReport::default();
        report.warnings.push(format!("nothing was imported: {e}"));
        report
    })
}

/// Like [`normalize`], but refuses a batch over `opts.limits`.
///
/// # Errors
/// [`Error::LimitExceeded`](crate::Error::LimitExceeded) when SMS rows plus call rows are more than `max_messages`.
pub fn try_normalize(batch: &DeviceBatch, opts: &DeviceOptions) -> Result<ImportReport> {
    opts.limits.validate()?;
    opts.limits.check_messages(batch.sms.len().saturating_add(batch.calls.len()))?;
    let mut report = ImportReport::default();
    let mut ordinals = Ordinals::default();
    let cc = opts.default_country_code.as_deref();
    let own = opts
        .own_number
        .as_deref()
        .map(|n| normalize_phone(n, cc).unwrap_or_else(|| n.to_owned()))
        .map(Participant::new);
    let party =
        |raw: &str| Participant::new(normalize_phone(raw, cc).unwrap_or_else(|| raw.trim().to_owned()));
    let mut skipped = 0usize;

    for s in &batch.sms {
        let Some(timestamp) = DateTime::from_timestamp_millis(s.date_ms) else {
            skipped += 1;
            continue;
        };
        let remote = party(&s.address);
        let (direction, sender, recipients) = match s.kind {
            1 => (Direction::Inbound, remote, own.iter().cloned().collect()),
            2 => (Direction::Outbound, own.clone().unwrap_or_else(|| Participant::new("self")), vec![remote]),
            _ => (Direction::Unknown, remote, vec![]),
        };
        let ts = s.date_ms.to_string();
        // The device row id is not stable across devices or after a restore, so the derived id is
        // content-based and the row id is only kept as `external_id`. The address is used as written,
        // not normalized, so `default_country_code` cannot change ids.
        let address = s.address.trim();
        let n = ordinals.next(&format!("sms|{ts}|{}|{address}|{}", s.kind, s.body));
        let id = derive_id(&["sms", "msg", &ts, &s.kind.to_string(), address, &s.body, &n.to_string()]);
        report.events.push(Event::Message(Message {
            id,
            channel: Channel::Sms,
            external_id: s.id.clone(),
            conversation_id: s.thread_id.clone(),
            direction,
            timestamp,
            sender,
            recipients,
            body: Some(s.body.clone()),
            attachments: vec![],
            raw: None,
        }));
    }

    for c in &batch.calls {
        let Some(timestamp) = DateTime::from_timestamp_millis(c.date_ms) else {
            skipped += 1;
            continue;
        };
        let (direction, outcome) = match c.kind {
            1 => (Direction::Inbound, CallOutcome::Answered),
            2 if c.duration_secs > 0 => (Direction::Outbound, CallOutcome::Answered),
            2 => (Direction::Outbound, CallOutcome::Unknown),
            3 => (Direction::Inbound, CallOutcome::Missed),
            4 => (Direction::Inbound, CallOutcome::Voicemail),
            5 => (Direction::Inbound, CallOutcome::Rejected),
            6 => (Direction::Inbound, CallOutcome::Blocked),
            _ => (Direction::Unknown, CallOutcome::Unknown),
        };
        let mut remote = party(&c.number);
        remote.display_name = c.name.clone().filter(|n| !n.is_empty());
        let ts = c.date_ms.to_string();
        let number = c.number.trim();
        let n = ordinals.next(&format!("call|{ts}|{}|{number}", c.kind));
        report.events.push(Event::Call(Call {
            id: derive_id(&["cellular", "call", &ts, &c.kind.to_string(), number, &n.to_string()]),
            channel: Channel::Cellular,
            external_id: c.id.clone(),
            conversation_id: None,
            direction,
            timestamp,
            duration_secs: c.duration_secs,
            outcome,
            video: false,
            participants: vec![remote],
            raw: None,
        }));
    }

    if skipped > 0 {
        report.warnings.push(format!("{skipped} record(s) skipped because their timestamp was invalid"));
    }
    report.sort();
    Ok(report)
}
