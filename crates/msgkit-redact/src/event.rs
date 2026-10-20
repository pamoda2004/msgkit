//! Redacting whole events and import reports.

use std::collections::BTreeMap;

use msgkit_core::{Attachment, Call, Event, ImportReport, Message, Participant};
use serde_json::Value;

use crate::config::{IdPolicy, Kind};
use crate::{Redacted, Redactor};

/// How much was masked: a count per kind, never the values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Findings per kind.
    pub counts: BTreeMap<Kind, usize>,
}

impl Summary {
    fn add(&mut self, redacted: &Redacted) {
        for (kind, n) in redacted.counts() {
            *self.counts.entry(kind).or_insert(0) += n;
        }
    }

    /// The total number of findings.
    #[must_use]
    pub fn total(&self) -> usize {
        self.counts.values().sum()
    }
}

/// Values deeper than `serde_json` itself parses are cut off instead of followed.
const MAX_VALUE_DEPTH: usize = 128;

impl Redactor {
    fn text(&self, text: &str, summary: &mut Summary) -> String {
        let redacted = self.redact_text(text);
        summary.add(&redacted);
        redacted.text
    }

    fn opt(&self, text: Option<&String>, summary: &mut Summary) -> Option<String> {
        text.map(|t| self.text(t, summary))
    }

    fn name_token(&self, name: &str, summary: &mut Summary) -> String {
        *summary.counts.entry(Kind::Name).or_insert(0) += 1;
        self.mask_value(Kind::Name, name)
    }

    fn participant(&self, p: &Participant, summary: &mut Summary) -> Participant {
        let masks_names = self.config().redact_display_names;
        // In exports a sender's id is often its display name: mask both the same way.
        let id = match &p.display_name {
            Some(name) if masks_names && *name == p.id => self.name_token(&p.id, summary),
            _ => self.text(&p.id, summary),
        };
        let display_name = p.display_name.as_ref().map(|name| {
            if masks_names {
                self.name_token(name, summary)
            } else {
                self.text(name, summary)
            }
        });
        Participant { id, display_name }
    }

    fn attachment(&self, a: &Attachment, summary: &mut Summary) -> Attachment {
        Attachment {
            kind: a.kind,
            uri: self.opt(a.uri.as_ref(), summary),
            mime_type: a.mime_type.clone(),
            file_name: self.opt(a.file_name.as_ref(), summary),
            // The platform's own media id is opaque, not personal; it is kept like `external_id`.
            source_id: a.source_id.clone(),
        }
    }

    fn value(&self, value: &Value, depth: usize, summary: &mut Summary) -> Value {
        if depth >= MAX_VALUE_DEPTH {
            return Value::Null;
        }
        match value {
            Value::String(s) => Value::String(self.text(s, summary)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|v| self.value(v, depth + 1, summary)).collect())
            }
            Value::Object(map) => Value::Object(
                map.iter().map(|(k, v)| (self.text(k, summary), self.value(v, depth + 1, summary))).collect(),
            ),
            other => other.clone(),
        }
    }

    fn raw(&self, raw: Option<&Value>, summary: &mut Summary) -> Option<Value> {
        if self.config().drop_raw {
            None
        } else {
            raw.map(|v| self.value(v, 0, summary))
        }
    }

    fn event_id(&self, id: &str) -> String {
        match self.config().id_policy {
            IdPolicy::Keep => id.to_owned(),
            IdPolicy::Pseudonymize => self.pseudonym_id(id).unwrap_or_else(|| id.to_owned()),
        }
    }

    /// Redacts one event, adding what was masked to `summary`.
    ///
    /// Masked: the message body, attachment file names and addresses, participant ids (and, with
    /// `redact_display_names`, display names), the conversation id, and, with `drop_raw` off, every string in `raw`.
    /// Dropped by default: `raw`. Kept: timestamps, direction, channel, call duration and outcome, attachment kinds,
    /// the platform's own ids (`external_id`, `source_id`) and, unless `id_policy` says otherwise, the event id.
    #[must_use]
    pub fn redact_event_counted(&self, event: &Event, summary: &mut Summary) -> Event {
        match event {
            Event::Message(m) => Event::Message(Message {
                id: self.event_id(&m.id),
                channel: m.channel,
                external_id: m.external_id.clone(),
                conversation_id: self.opt(m.conversation_id.as_ref(), summary),
                direction: m.direction,
                timestamp: m.timestamp,
                sender: self.participant(&m.sender, summary),
                recipients: m.recipients.iter().map(|p| self.participant(p, summary)).collect(),
                body: self.opt(m.body.as_ref(), summary),
                attachments: m.attachments.iter().map(|a| self.attachment(a, summary)).collect(),
                raw: self.raw(m.raw.as_ref(), summary),
            }),
            Event::Call(c) => Event::Call(Call {
                id: self.event_id(&c.id),
                channel: c.channel,
                external_id: c.external_id.clone(),
                conversation_id: self.opt(c.conversation_id.as_ref(), summary),
                direction: c.direction,
                timestamp: c.timestamp,
                duration_secs: c.duration_secs,
                outcome: c.outcome,
                video: c.video,
                participants: c.participants.iter().map(|p| self.participant(p, summary)).collect(),
                raw: self.raw(c.raw.as_ref(), summary),
            }),
        }
    }

    /// Redacts one event (see [`Redactor::redact_event_counted`]).
    #[must_use]
    pub fn redact_event(&self, event: &Event) -> Event {
        self.redact_event_counted(event, &mut Summary::default())
    }

    /// Redacts every event of an import report. The warnings are kept as they are: msgkit never puts message
    /// content into a warning.
    #[must_use]
    pub fn redact_report(&self, report: &ImportReport) -> (ImportReport, Summary) {
        let mut summary = Summary::default();
        let events = report.events.iter().map(|e| self.redact_event_counted(e, &mut summary)).collect();
        (ImportReport { events, warnings: report.warnings.clone() }, summary)
    }
}
