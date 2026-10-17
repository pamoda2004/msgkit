//! Parser for WhatsApp "Export chat" text files (`_chat.txt`).
//!
//! WhatsApp's export format is undocumented and varies by platform, app
//! version and phone locale. This parser handles the common shapes:
//!
//! ```text
//! 31/12/2023, 21:41 - Alice: Happy new year          (Android, 24h)
//! 12/31/23, 9:41 PM - Alice: Happy new year          (Android, 12h)
//! [31/12/2023, 21:41:05] Alice: Happy new year       (iOS)
//! ```
//!
//! Known limits (also documented in `docs/SUPPORTED_SOURCES.md`):
//! * Timestamps have no time zone; supply `utc_offset_minutes`.
//! * Participants are display names, not phone numbers.
//! * Day/month order is guessed from the data when `date_order` is `auto`.

use std::sync::OnceLock;

use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::id::{derive_id, Ordinals};
use crate::model::{
    Attachment, AttachmentKind, Call, CallOutcome, Channel, Direction, Event, ImportReport, Message,
    Participant,
};
use crate::phone::normalize_phone;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DateOrder {
    /// Infer from the file; falls back to day-first with a warning.
    #[default]
    Auto,
    Dmy,
    Mdy,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WhatsAppExportOptions {
    /// The exporting user's name as it appears in the chat. Used to decide
    /// message direction; without it direction is `unknown`.
    pub self_name: Option<String>,
    pub date_order: DateOrder,
    /// Offset of the phone's local time zone from UTC, in minutes
    /// (e.g. `330` for UTC+05:30). Defaults to `0`.
    pub utc_offset_minutes: i32,
    pub conversation_id: Option<String>,
    /// Calling code (digits, e.g. `"94"`) used to normalize sender names that
    /// are national phone numbers.
    pub default_country_code: Option<String>,
}

struct Header {
    date: String,
    time: String,
    ampm: Option<String>,
}

struct Entry {
    header: Header,
    text: String,
}

fn header_regexes() -> &'static [Regex; 2] {
    static RE: OnceLock<[Regex; 2]> = OnceLock::new();
    RE.get_or_init(|| {
        let date = r"(\d{1,4}[/.\-]\d{1,2}[/.\-]\d{1,4})";
        let time = r"(\d{1,2}[:.]\d{2}(?:[:.]\d{2})?)";
        let ampm = r"(?:\s?([AaPp]\.?[Mm]\.?))?";
        [
            // Android: `date, time - text`
            Regex::new(&format!(r"^{date},?\s+{time}{ampm}\s+-\s+(.*)$")).unwrap(),
            // iOS: `[date, time] text`
            Regex::new(&format!(r"^\[{date},?\s+{time}{ampm}\]\s+(.*)$")).unwrap(),
        ]
    })
}

/// Removes invisible direction marks and normalizes exotic spaces that
/// WhatsApp inserts around timestamps and names.
fn clean_line(line: &str) -> String {
    line.chars()
        .filter(|c| !matches!(c, '\u{200e}' | '\u{200f}' | '\u{feff}'))
        .map(|c| if matches!(c, '\u{202f}' | '\u{00a0}') { ' ' } else { c })
        .collect()
}

fn split_date(s: &str) -> Option<(&str, &str, &str)> {
    let mut it = s.split(['/', '.', '-']).filter(|p| !p.is_empty());
    Some((it.next()?, it.next()?, it.next()?))
}

fn resolve_order(entries: &[Entry], requested: DateOrder, warnings: &mut Vec<String>) -> DateOrder {
    if requested != DateOrder::Auto {
        return requested;
    }
    let (mut first_gt_12, mut second_gt_12) = (false, false);
    for e in entries {
        if let Some((a, b, _)) = split_date(&e.header.date) {
            if a.len() == 4 {
                continue; // ISO order, unambiguous
            }
            first_gt_12 |= a.parse::<u32>().is_ok_and(|n| n > 12);
            second_gt_12 |= b.parse::<u32>().is_ok_and(|n| n > 12);
        }
    }
    match (first_gt_12, second_gt_12) {
        (true, false) => DateOrder::Dmy,
        (false, true) => DateOrder::Mdy,
        (true, true) => {
            warnings.push("dates contain both day-first and month-first values; assuming day-first".into());
            DateOrder::Dmy
        }
        (false, false) => {
            warnings.push(
                "date order is ambiguous (all values <= 12); assuming day-first. \
                 Set date_order explicitly if this is wrong"
                    .into(),
            );
            DateOrder::Dmy
        }
    }
}

fn parse_timestamp(h: &Header, order: DateOrder, offset: FixedOffset) -> Option<chrono::DateTime<Utc>> {
    let (first, second, third) = split_date(&h.date)?;
    let (year, month, day) = if first.len() == 4 {
        (first.parse::<i32>().ok()?, second.parse::<u32>().ok()?, third.parse::<u32>().ok()?)
    } else {
        let mut year = third.parse::<i32>().ok()?;
        if third.len() <= 2 {
            year += 2000;
        }
        let (x, y) = (first.parse::<u32>().ok()?, second.parse::<u32>().ok()?);
        match order {
            DateOrder::Mdy => (year, x, y),
            _ => (year, y, x),
        }
    };

    let (hour, minute, second) = parse_clock(h)?;
    let naive = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(year, month, day)?,
        NaiveTime::from_hms_opt(hour, minute, second)?,
    );
    offset.from_local_datetime(&naive).single().map(|dt| dt.with_timezone(&Utc))
}

/// The time of a header as (hour, minute, second) on a 24-hour clock.
fn parse_clock(h: &Header) -> Option<(u32, u32, u32)> {
    let mut t = h.time.split([':', '.']);
    let mut hour = t.next()?.parse::<u32>().ok()?;
    let minute = t.next()?.parse::<u32>().ok()?;
    let second = t.next().map_or(Some(0), |s| s.parse::<u32>().ok())?;
    if let Some(m) = &h.ampm {
        let pm = m.starts_with(['p', 'P']);
        if hour == 12 {
            hour = if pm { 12 } else { 0 };
        } else if pm {
            hour += 12;
        }
    }
    Some((hour, minute, second))
}

/// The moment a line was written, exactly as the file wrote it: local wall-clock time with no offset.
///
/// Ids are derived from this key instead of the UTC timestamp. The two leading date numbers are kept in
/// the order they appear, so neither `utc_offset_minutes` nor the way the day/month order was resolved
/// can change an id, and the same chat gets the same ids from overlapping exports.
fn local_key(h: &Header) -> Option<String> {
    let (first, second, third) = split_date(&h.date)?;
    let (hour, minute, sec) = parse_clock(h)?;
    let date = if first.len() == 4 {
        format!(
            "{:04}-{:02}-{:02}",
            first.parse::<i32>().ok()?,
            second.parse::<u32>().ok()?,
            third.parse::<u32>().ok()?
        )
    } else {
        let mut year = third.parse::<i32>().ok()?;
        if third.len() <= 2 {
            year += 2000;
        }
        format!("{:02}.{:02}.{:04}", first.parse::<u32>().ok()?, second.parse::<u32>().ok()?, year)
    };
    Some(format!("{date}T{hour:02}:{minute:02}:{sec:02}"))
}

fn kind_from_name(name: &str) -> AttachmentKind {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" => AttachmentKind::Image,
        "mp4" | "mov" | "3gp" | "mkv" | "avi" => AttachmentKind::Video,
        "opus" | "mp3" | "m4a" | "aac" | "ogg" | "amr" | "wav" => AttachmentKind::Audio,
        _ => AttachmentKind::File,
    }
}

/// Recognizes the placeholders WhatsApp writes instead of media.
fn parse_media(body: &str) -> Option<Attachment> {
    let t = body.trim();
    let lower = t.to_ascii_lowercase();
    if lower == "<media omitted>" {
        return Some(Attachment::new(AttachmentKind::Other));
    }
    if let Some(kind) = lower.strip_suffix(" omitted") {
        let kind = match kind {
            "image" | "sticker" | "gif" => AttachmentKind::Image,
            "video" => AttachmentKind::Video,
            "audio" => AttachmentKind::Audio,
            "document" => AttachmentKind::File,
            _ => AttachmentKind::Other,
        };
        return Some(Attachment::new(kind));
    }
    let file = t
        .strip_prefix("<attached:")
        .and_then(|r| r.strip_suffix('>'))
        .map(str::trim)
        .or_else(|| t.strip_suffix("(file attached)").map(str::trim));
    file.filter(|n| !n.is_empty()).map(|name| {
        let mut a = Attachment::new(kind_from_name(name));
        a.file_name = Some(name.to_owned());
        a
    })
}

fn call_from_system_text(text: &str) -> Option<bool> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("missed voice call") {
        Some(false)
    } else if lower.contains("missed video call") {
        Some(true)
    } else {
        None
    }
}

/// Parses the contents of a WhatsApp chat export.
#[must_use]
pub fn parse(text: &str, opts: &WhatsAppExportOptions) -> ImportReport {
    let mut report = ImportReport::default();
    let [android, ios] = header_regexes();

    let mut entries: Vec<Entry> = Vec::new();
    let mut orphan_lines = 0usize;
    for raw in text.lines() {
        let line = clean_line(raw);
        let caps = android.captures(&line).or_else(|| ios.captures(&line));
        match caps {
            Some(c) => entries.push(Entry {
                header: Header {
                    date: c[1].to_owned(),
                    time: c[2].to_owned(),
                    ampm: c.get(3).map(|m| m.as_str().to_owned()),
                },
                text: c[4].to_owned(),
            }),
            None => match entries.last_mut() {
                // A line without a header continues the previous message.
                Some(last) => {
                    last.text.push('\n');
                    last.text.push_str(&line);
                }
                None if line.trim().is_empty() => {}
                None => orphan_lines += 1,
            },
        }
    }
    if orphan_lines > 0 {
        report.warnings.push(format!("{orphan_lines} line(s) before the first message were ignored"));
    }
    if entries.is_empty() {
        report.warnings.push("no WhatsApp messages found; is this a chat export?".into());
        return report;
    }

    let order = resolve_order(&entries, opts.date_order, &mut report.warnings);
    let offset = FixedOffset::east_opt(opts.utc_offset_minutes.saturating_mul(60)).unwrap_or_else(|| {
        report.warnings.push(format!("invalid utc_offset_minutes {}; using UTC", opts.utc_offset_minutes));
        FixedOffset::east_opt(0).expect("zero offset is valid")
    });
    build_events(entries, opts, order, offset, report)
}

fn build_events(
    entries: Vec<Entry>,
    opts: &WhatsAppExportOptions,
    order: DateOrder,
    offset: FixedOffset,
    mut report: ImportReport,
) -> ImportReport {
    let mut ordinals = Ordinals::default();
    let mut bad_dates = 0usize;

    for entry in entries {
        let (Some(timestamp), Some(local)) =
            (parse_timestamp(&entry.header, order, offset), local_key(&entry.header))
        else {
            bad_dates += 1;
            continue;
        };

        let (name, body) = match entry.text.split_once(": ") {
            Some((n, b)) if !n.is_empty() && n.chars().count() <= 100 => (n.trim(), b),
            _ => {
                // System notice. Missed calls are the only ones worth keeping.
                if let Some(video) = call_from_system_text(&entry.text) {
                    let n = ordinals.next(&format!("call|{local}|{video}"));
                    let id = derive_id(&["whatsapp", "call", &local, &video.to_string(), &n.to_string()]);
                    report.events.push(Event::Call(Call {
                        id,
                        channel: Channel::Whatsapp,
                        external_id: None,
                        conversation_id: opts.conversation_id.clone(),
                        direction: Direction::Inbound,
                        timestamp,
                        duration_secs: 0,
                        outcome: CallOutcome::Missed,
                        video,
                        participants: vec![],
                        raw: None,
                    }));
                }
                continue;
            }
        };

        let direction = match &opts.self_name {
            Some(me) if me.trim().eq_ignore_ascii_case(name) => Direction::Outbound,
            Some(_) => Direction::Inbound,
            None => Direction::Unknown,
        };
        let sender_id =
            normalize_phone(name, opts.default_country_code.as_deref()).unwrap_or_else(|| name.to_owned());

        let body = body.trim_end().trim_end_matches("<This message was edited>").trim_end();
        let attachment = parse_media(body);
        let text = (attachment.is_none() && !body.is_empty()).then(|| body.to_owned());

        // Identical lines in the same minute are told apart by their occurrence index. The sender is the
        // name as written, not the normalized phone number, so `default_country_code` cannot change ids.
        let n = ordinals.next(&format!("msg|{local}|{name}|{body}"));
        let id = derive_id(&["whatsapp", "msg", &local, name, body, &n.to_string()]);
        report.events.push(Event::Message(Message {
            id,
            channel: Channel::Whatsapp,
            external_id: None,
            conversation_id: opts.conversation_id.clone(),
            direction,
            timestamp,
            sender: Participant::with_name(sender_id, name),
            recipients: vec![],
            body: text,
            attachments: attachment.into_iter().collect(),
            raw: None,
        }));
    }

    if bad_dates > 0 {
        report
            .warnings
            .push(format!("{bad_dates} message(s) skipped because their date could not be parsed"));
    }
    report.sort();
    report
}
