//! Storage abstraction.
//!
//! The core never decides where data lives. Hosts implement [`EventStore`]
//! and msgkit hands them events; the store de-duplicates by event id.
//! [`MemoryStore`] is built in; `msgkit-store-sqlite` provides a durable one.

use std::collections::HashSet;
use std::io::{self, Write};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::model::Event;
use crate::phone::normalize_phone;

/// A de-duplicating buffer of events.
///
/// The trait is object-safe so applications can pick a backend at runtime
/// (`Box<dyn EventStore + Send>`).
pub trait EventStore {
    /// Stores `event` unless one with the same id has been seen before.
    /// Returns `true` if the event was new.
    fn insert(&mut self, event: Event) -> Result<bool>;

    /// Stores all `events` and returns how many were new.
    ///
    /// Durable backends should override this to make the batch atomic: either
    /// every new event is stored or the call fails and none are.
    fn insert_all(&mut self, events: Vec<Event>) -> Result<usize> {
        let mut added = 0;
        for event in events {
            added += usize::from(self.insert(event)?);
        }
        Ok(added)
    }

    /// Number of events currently held (not yet drained).
    fn len(&self) -> Result<usize>;

    fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Events currently held, oldest arrival first.
    fn list(&self) -> Result<Vec<Event>>;

    /// Removes and returns all held events, oldest arrival first.
    ///
    /// Their ids stay remembered, so a replay of the same webhook is still
    /// recognised as a duplicate afterwards.
    fn drain(&mut self) -> Result<Vec<Event>>;

    /// Marks the events with these ids as consumed: they are no longer
    /// returned by `list`/`drain`, but stay remembered for de-duplication.
    /// Unknown or already-acknowledged ids are ignored. Returns how many
    /// events were newly acknowledged.
    ///
    /// Prefer `list` + `acknowledge` over `drain` for reliable consumption:
    /// if the consumer crashes between `drain` and processing, drained events
    /// are lost, whereas unacknowledged events are simply listed again.
    fn acknowledge(&mut self, ids: &[String]) -> Result<usize>;
}

/// Options for [`EraseStore::erase_person`].
///
/// `#[non_exhaustive]`: build it with [`EraseOptions::default`] and the `with_*` setters.
///
/// ```
/// use msgkit_core::store::EraseOptions;
///
/// let opts = EraseOptions::default().with_default_country_code("94").with_drop_ids(true);
/// assert_eq!(opts.default_country_code.as_deref(), Some("94"));
/// assert!(opts.drop_ids);
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EraseOptions {
    /// Calling code (digits, e.g. `"94"`) used to read national-format phone numbers, exactly as at import: the
    /// same `default_country_code` the events were imported with. Without it only addresses with an international
    /// prefix are recognised as phone numbers.
    pub default_country_code: Option<String>,
    /// Also forget the ids of the erased events. By default an erased event keeps its id (and nothing else) so a
    /// redelivered webhook is still recognised as a duplicate; the id is then forgotten by the usual pruning of
    /// acknowledged ids. Dropping it means a redelivery of the same message would be accepted again.
    pub drop_ids: bool,
}

impl EraseOptions {
    /// Sets [`default_country_code`](Self::default_country_code).
    #[must_use]
    pub fn with_default_country_code(mut self, code: impl Into<String>) -> Self {
        self.default_country_code = Some(code.into());
        self
    }

    /// Sets [`drop_ids`](Self::drop_ids).
    #[must_use]
    pub fn with_drop_ids(mut self, drop_ids: bool) -> Self {
        self.drop_ids = drop_ids;
        self
    }
}

/// The form in which two addresses are compared by [`EraseStore::erase_person`].
///
/// Phone-like input is normalised with [`normalize_phone`] and `default_country_code`, the same way the importers
/// normalise participants, so `0700000000`, `070 000 0000` and `+94700000000` are one person when the code is `94`.
/// Anything else (a name, an email address, a platform id) is trimmed and lower-cased.
#[must_use]
pub fn address_key(address: &str, default_country_code: Option<&str>) -> String {
    normalize_phone(address, default_country_code).unwrap_or_else(|| address.trim().to_lowercase())
}

/// Whether `event` names the person with this [`address_key`] as a sender, recipient or call participant.
/// Stored addresses are put through [`address_key`] too, so rows imported without a country code still match.
#[must_use]
pub fn involves(event: &Event, key: &str, default_country_code: Option<&str>) -> bool {
    let is = |id: &str| address_key(id, default_country_code) == key;
    match event {
        Event::Message(m) => is(&m.sender.id) || m.recipients.iter().any(|p| is(&p.id)),
        Event::Call(c) => c.participants.iter().any(|p| is(&p.id)),
    }
}

/// The key to erase by.
///
/// # Errors
/// [`Error::InvalidInput`] when `address` is empty or only whitespace.
pub fn erase_key(address: &str, options: &EraseOptions) -> Result<String> {
    let key = address_key(address, options.default_country_code.as_deref());
    if key.is_empty() {
        return Err(Error::InvalidInput("the address to erase is empty".to_owned()));
    }
    Ok(key)
}

/// A store that can erase the events of one person.
///
/// A separate trait, so [`EventStore`] stays as it was; both built-in stores implement it. Nothing here echoes
/// content: the result is a count.
pub trait EraseStore: EventStore {
    /// Erases the content of every **held** (not yet acknowledged) event in which the person is the sender, a
    /// recipient or a call participant, matched by [`address_key`]. The whole event goes, including the other
    /// people in it and any `raw` payload. Returns how many events were erased; an address that matches nothing
    /// returns `Ok(0)`.
    ///
    /// The id of an erased event is kept (see [`EraseOptions::drop_ids`]). Events already acknowledged hold no
    /// content, and their ids cannot be tied to a person, so they are not touched: a [`RetentionStore`] forgets
    /// them when they are old enough.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] for an empty address, or a storage error.
    fn erase_person(&mut self, address: &str, options: &EraseOptions) -> Result<usize>;
}

/// A store that forgets things by age. Only durable stores implement it: `msgkit-core` never reads a clock, so a
/// store that keeps no time (such as [`MemoryStore`], which holds nothing across a restart anyway) has no
/// retention to apply.
pub trait RetentionStore: EventStore {
    /// Erases held events that arrived (were inserted) more than `older_than` ago and were never acknowledged.
    /// Their ids are kept, as with [`EraseStore::erase_person`]. Returns how many were erased.
    ///
    /// # Errors
    /// A storage error.
    fn expire_pending(&mut self, older_than: Duration) -> Result<usize>;

    /// Forgets the ids of events acknowledged (or erased or expired) more than `older_than` ago. After this a
    /// redelivery of such an event would be accepted as new, so keep `older_than` above the platforms' retry
    /// window. Returns the number of ids forgotten.
    ///
    /// # Errors
    /// A storage error.
    fn prune_acknowledged(&mut self, older_than: Duration) -> Result<usize>;
}

/// An in-memory store, useful for tests and short-lived processes.
#[derive(Debug, Default)]
pub struct MemoryStore {
    seen: HashSet<String>,
    events: Vec<Event>,
}

impl MemoryStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl EventStore for MemoryStore {
    fn insert(&mut self, event: Event) -> Result<bool> {
        Ok(if self.seen.insert(event.id().to_owned()) {
            self.events.push(event);
            true
        } else {
            false
        })
    }

    fn len(&self) -> Result<usize> {
        Ok(self.events.len())
    }

    fn list(&self) -> Result<Vec<Event>> {
        Ok(self.events.clone())
    }

    fn drain(&mut self) -> Result<Vec<Event>> {
        Ok(std::mem::take(&mut self.events))
    }

    fn acknowledge(&mut self, ids: &[String]) -> Result<usize> {
        let before = self.events.len();
        self.events.retain(|e| !ids.iter().any(|id| id == e.id()));
        Ok(before - self.events.len())
    }
}

impl EraseStore for MemoryStore {
    fn erase_person(&mut self, address: &str, options: &EraseOptions) -> Result<usize> {
        let key = erase_key(address, options)?;
        let cc = options.default_country_code.as_deref();
        let before = self.events.len();
        let mut dropped = Vec::new();
        self.events.retain(|e| {
            let hit = involves(e, &key, cc);
            if hit && options.drop_ids {
                dropped.push(e.id().to_owned());
            }
            !hit
        });
        for id in dropped {
            self.seen.remove(&id);
        }
        Ok(before - self.events.len())
    }
}

/// Writes events as newline-delimited JSON, one event per line.
pub fn write_ndjson<W: Write>(mut out: W, events: &[Event]) -> io::Result<()> {
    for event in events {
        serde_json::to_writer(&mut out, event).map_err(io::Error::other)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// A reusable conformance suite for [`EventStore`] implementations, so every
/// backend is held to the same behaviour. Call it from a backend's tests with
/// a function that creates a fresh, empty store.
pub mod conformance {
    use chrono::DateTime;

    use super::{EraseOptions, EraseStore, EventStore};
    use crate::model::{
        Attachment, AttachmentKind, Call, CallOutcome, Channel, Direction, Event, Message, Participant,
    };

    #[must_use]
    pub fn sample_event(id: &str, secs: i64) -> Event {
        Event::Message(Message {
            id: id.into(),
            channel: Channel::Whatsapp,
            external_id: Some(format!("ext-{id}")),
            conversation_id: Some("conv".into()),
            direction: Direction::Inbound,
            timestamp: DateTime::from_timestamp(secs, 0).unwrap(),
            sender: Participant::with_name("+94700000000", "Kamal"),
            recipients: vec![Participant::new("+15550100111")],
            body: Some(format!("body of {id} සිංහල 🎉")),
            attachments: vec![Attachment::new(AttachmentKind::Image)],
            raw: Some(serde_json::json!({"k": [1, 2, {"x": null}]})),
        })
    }

    /// A message from `sender` (an address as stored) to one recipient.
    #[must_use]
    pub fn event_from(id: &str, secs: i64, sender: &str, recipient: &str) -> Event {
        Event::Message(Message {
            id: id.into(),
            channel: Channel::Sms,
            external_id: None,
            conversation_id: None,
            direction: Direction::Inbound,
            timestamp: DateTime::from_timestamp(secs, 0).unwrap(),
            sender: Participant::new(sender),
            recipients: vec![Participant::new(recipient)],
            body: Some(format!("body of {id}")),
            attachments: vec![],
            raw: Some(serde_json::json!({"secret": id})),
        })
    }

    /// A call whose participants are `people`.
    #[must_use]
    pub fn call_with(id: &str, secs: i64, people: &[&str]) -> Event {
        Event::Call(Call {
            id: id.into(),
            channel: Channel::Cellular,
            external_id: None,
            conversation_id: None,
            direction: Direction::Outbound,
            timestamp: DateTime::from_timestamp(secs, 0).unwrap(),
            duration_secs: 5,
            outcome: CallOutcome::Answered,
            video: false,
            participants: people.iter().map(|p| Participant::new(*p)).collect(),
            raw: None,
        })
    }

    fn ids(s: &impl EventStore) -> Vec<String> {
        s.list().unwrap().iter().map(|e| e.id().to_owned()).collect()
    }

    /// Panics if `make()` does not erase correctly. Call it in addition to [`run`].
    pub fn run_erase<S: EraseStore>(make: impl Fn() -> S) {
        let cc94 = || EraseOptions::default().with_default_country_code("94");
        // The same person written four ways: stored as E.164, and stored the way an import without a country
        // code leaves a national number.
        let fill = || {
            let mut s = make();
            s.insert_all(vec![
                event_from("sent-by", 1, "+94700000000", "+94711111111"),
                event_from("sent-to", 2, "+94722222222", "+94700000000"),
                call_with("call-with", 3, &["+94700000003", "+94700000000"]),
                event_from("national", 4, "0700000000", "+94700000004"),
                event_from("other", 5, "+94700000005", "+94700000006"),
                event_from("named", 6, "Alice Smith", "+94700000007"),
            ])
            .unwrap();
            s
        };

        // Every spelling of the number erases the same four events, and only those.
        for spelling in ["0700000000", "070 000 0000", "+94700000000", " +94 70 000 0000 ", "0094-70-0000000"]
        {
            let mut s = fill();
            assert_eq!(s.erase_person(spelling, &cc94()).unwrap(), 4, "{spelling}");
            assert_eq!(ids(&s), ["other", "named"], "{spelling}");
            assert_eq!(s.len().unwrap(), 2);
            // The ids stay: a redelivery is still a duplicate, and nothing is listed for the erased ones.
            assert!(
                !s.insert(event_from("sent-by", 1, "+94700000000", "+94711111111")).unwrap(),
                "{spelling}"
            );
        }

        // Without the country code only international forms match a stored E.164 number.
        let mut s = fill();
        assert_eq!(
            s.erase_person("0700000000", &EraseOptions::default()).unwrap(),
            1,
            "only the row stored as 070..."
        );
        let mut s = fill();
        assert_eq!(s.erase_person("+94700000000", &EraseOptions::default()).unwrap(), 3);

        // A name or any other id: trimmed and case-insensitive.
        let mut s = fill();
        assert_eq!(s.erase_person("  ALICE smith ", &EraseOptions::default()).unwrap(), 1);
        assert_eq!(s.len().unwrap(), 5);

        // An address that matches nothing is not an error.
        let mut s = fill();
        assert_eq!(s.erase_person("+94700000009", &cc94()).unwrap(), 0);
        assert_eq!(s.erase_person("nobody@example.com", &cc94()).unwrap(), 0);
        assert_eq!(s.len().unwrap(), 6);

        // An empty address is refused, never "match everything".
        for empty in ["", "   ", "\t"] {
            let err = s.erase_person(empty, &cc94()).unwrap_err();
            assert_eq!(err.code(), "invalid_input", "{empty:?}");
        }
        assert_eq!(s.len().unwrap(), 6);

        // Erasing twice finds nothing the second time.
        assert_eq!(s.erase_person("0700000000", &cc94()).unwrap(), 4);
        assert_eq!(s.erase_person("0700000000", &cc94()).unwrap(), 0);

        // drop_ids: the id is forgotten too, so the same event is accepted again.
        let mut s = fill();
        let drop = cc94().with_drop_ids(true);
        assert_eq!(s.erase_person("+94700000000", &drop).unwrap(), 4);
        assert!(s.insert(event_from("sent-by", 1, "+94700000000", "+94711111111")).unwrap(), "id dropped");
        assert!(
            !s.insert(event_from("other", 5, "+94700000005", "+94700000006")).unwrap(),
            "others untouched"
        );

        // A person who is only in already-acknowledged events has nothing left to erase.
        let mut s = fill();
        s.acknowledge(&["sent-by".to_owned()]).unwrap();
        assert_eq!(s.erase_person("+94700000000", &cc94()).unwrap(), 3);
    }

    /// Panics if `make()` does not behave like a correct store.
    pub fn run<S: EventStore>(make: impl Fn() -> S) {
        // Duplicates are ignored.
        let mut s = make();
        assert!(s.insert(sample_event("a", 1)).unwrap());
        assert!(!s.insert(sample_event("a", 1)).unwrap());
        assert!(s.insert(sample_event("b", 2)).unwrap());
        assert_eq!(s.len().unwrap(), 2);

        // Events round-trip losslessly, in arrival order.
        let listed = s.list().unwrap();
        assert_eq!(listed, vec![sample_event("a", 1), sample_event("b", 2)]);

        // Arrival order wins over timestamp order.
        let mut s = make();
        s.insert(sample_event("late", 99)).unwrap();
        s.insert(sample_event("early", 1)).unwrap();
        let ids: Vec<_> = s.list().unwrap().iter().map(|e| e.id().to_owned()).collect();
        assert_eq!(ids, ["late", "early"]);

        // insert_all counts only new events, including duplicates inside the batch.
        let mut s = make();
        s.insert(sample_event("a", 1)).unwrap();
        let added = s
            .insert_all(vec![
                sample_event("a", 1),
                sample_event("b", 2),
                sample_event("b", 2),
                sample_event("c", 3),
            ])
            .unwrap();
        assert_eq!(added, 2);
        assert_eq!(s.len().unwrap(), 3);

        // Draining empties the store but keeps dedupe memory.
        let drained = s.drain().unwrap();
        assert_eq!(drained.len(), 3);
        assert_eq!(s.len().unwrap(), 0);
        assert_eq!(s.list().unwrap(), []);
        assert!(!s.insert(sample_event("a", 1)).unwrap(), "drained ids must stay deduplicated");
        assert!(s.insert(sample_event("d", 4)).unwrap());
        assert_eq!(s.drain().unwrap().len(), 1);
        assert_eq!(s.drain().unwrap(), []);

        // Acknowledging removes only the named events, counts only real ones,
        // and keeps dedupe memory.
        let mut s = make();
        s.insert_all(vec![sample_event("a", 1), sample_event("b", 2), sample_event("c", 3)]).unwrap();
        let acked = s.acknowledge(&["b".to_owned(), "nope".to_owned()]).unwrap();
        assert_eq!(acked, 1);
        assert_eq!(s.acknowledge(&["b".to_owned()]).unwrap(), 0, "already acknowledged");
        let ids: Vec<_> = s.list().unwrap().iter().map(|e| e.id().to_owned()).collect();
        assert_eq!(ids, ["a", "c"]);
        assert!(!s.insert(sample_event("b", 2)).unwrap(), "acknowledged ids stay deduplicated");
        assert_eq!(s.len().unwrap(), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::conformance::{run, sample_event};
    use super::*;

    #[test]
    fn memory_store_conforms() {
        run(MemoryStore::new);
    }

    #[test]
    fn memory_store_erases() {
        super::conformance::run_erase(MemoryStore::new);
    }

    #[test]
    fn address_keys_agree_across_spellings() {
        use super::address_key;
        let want = "+94700000000";
        for spelling in ["0700000000", "070 000 0000", "+94700000000", "0094 70 000 0000", " (070) 000-0000 "]
        {
            assert_eq!(address_key(spelling, Some("94")), want, "{spelling}");
        }
        assert_eq!(address_key("  Alice@Example.COM ", Some("94")), "alice@example.com");
        assert_eq!(address_key("0700000000", None), "0700000000", "a national number needs the code");
    }

    #[test]
    fn ndjson_has_one_line_per_event() {
        let mut buf = Vec::new();
        write_ndjson(&mut buf, &[sample_event("a", 1), sample_event("b", 2)]).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap().lines().count(), 2);
    }
}
