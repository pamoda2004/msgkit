//! Storage abstraction.
//!
//! The core never decides where data lives. Hosts implement [`EventStore`]
//! and msgkit hands them events; the store de-duplicates by event id.
//! [`MemoryStore`] is built in; `msgkit-store-sqlite` provides a durable one.

use std::collections::HashSet;
use std::io::{self, Write};

use crate::error::Result;
use crate::model::Event;

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

/// An in-memory store, useful for tests and short-lived processes.
#[derive(Debug, Default)]
pub struct MemoryStore {
    seen: HashSet<String>,
    events: Vec<Event>,
}

impl MemoryStore {
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

    use super::EventStore;
    use crate::model::*;

    pub fn sample_event(id: &str, secs: i64) -> Event {
        Event::Message(Message {
            id: id.into(),
            channel: Channel::Whatsapp,
            external_id: Some(format!("ext-{id}")),
            conversation_id: Some("conv".into()),
            direction: Direction::Inbound,
            timestamp: DateTime::from_timestamp(secs, 0).unwrap(),
            sender: Participant::with_name("+94771234567", "Kamal"),
            recipients: vec![Participant::new("+15550001111")],
            body: Some(format!("body of {id} සිංහල 🎉")),
            attachments: vec![Attachment::new(AttachmentKind::Image)],
            raw: Some(serde_json::json!({"k": [1, 2, {"x": null}]})),
        })
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
        assert!(s.list().unwrap().is_empty());
        assert!(!s.insert(sample_event("a", 1)).unwrap(), "drained ids must stay deduplicated");
        assert!(s.insert(sample_event("d", 4)).unwrap());
        assert_eq!(s.drain().unwrap().len(), 1);
        assert!(s.drain().unwrap().is_empty());

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
    fn ndjson_has_one_line_per_event() {
        let mut buf = Vec::new();
        write_ndjson(&mut buf, &[sample_event("a", 1), sample_event("b", 2)]).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap().lines().count(), 2);
    }
}
