//! Durable SQLite implementation of [`msgkit_core::store::EventStore`].
//!
//! ```no_run
//! use msgkit_core::store::EventStore;
//! use msgkit_store_sqlite::SqliteStore;
//!
//! let mut store = SqliteStore::open("msgkit.db")?;
//! // store.insert_all(events)?;      // atomic, de-duplicated by event id
//! // let pending = store.list()?;    // everything not yet acknowledged
//! // store.acknowledge(&ids)?;       // after you processed them
//! # Ok::<(), msgkit_core::Error>(())
//! ```
//!
//! Behaviour worth knowing:
//!
//! * **Durable.** WAL journal with `synchronous = FULL`: once `insert`/
//!   `insert_all` returns, the events survive a crash or power loss.
//! * **Atomic batches.** `insert_all` runs in one transaction.
//! * **Erased on acknowledge.** Acknowledging (or draining) an event deletes
//!   its content immediately and keeps only its id, so the database does not
//!   become a permanent copy of personal messages. The id is remembered to
//!   reject redelivered webhooks until [`SqliteStore::prune_acknowledged`]
//!   removes it.
//! * **Multi-process safe.** Several processes may open the same file; writers
//!   wait up to five seconds for each other.
//! * **Not encrypted.** Content is stored as plain JSON. Use full-disk
//!   encryption or an encrypted volume, and restrict file access (the file is
//!   created owner-only on Unix).

use std::fmt::Display;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use msgkit_core::store::EventStore;
use msgkit_core::{Error, Event, Result};
use rusqlite::{params, Connection, TransactionBehavior};

/// Version of the on-disk layout, kept in `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 1;

/// Suggested retention for acknowledged ids. Meta retries failed webhook
/// deliveries for several days, so keep ids at least that long.
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

const SCHEMA_SQL: &str = "
    BEGIN IMMEDIATE;
    CREATE TABLE IF NOT EXISTS events (
        seq          INTEGER PRIMARY KEY AUTOINCREMENT,
        id           TEXT    NOT NULL UNIQUE,
        timestamp_ms INTEGER NOT NULL,
        channel      TEXT    NOT NULL,
        kind         TEXT    NOT NULL,
        json         TEXT,
        acked_at     INTEGER
    );
    CREATE INDEX IF NOT EXISTS events_pending ON events(seq) WHERE acked_at IS NULL;
    CREATE INDEX IF NOT EXISTS events_acked ON events(acked_at) WHERE acked_at IS NOT NULL;
    PRAGMA user_version = 1;
    COMMIT;";

fn storage(e: impl Display) -> Error {
    Error::Storage(e.to_string())
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[derive(Debug)]
pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Opens (creating if needed) the database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let is_new = !path.exists();
        let conn =
            Connection::open(path).map_err(|e| storage(format!("cannot open {}: {e}", path.display())))?;
        if is_new {
            restrict_permissions(path);
        }
        Self::init(conn)
    }

    /// A private in-memory database, mainly for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(storage)?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5)).map_err(storage)?;
        // Reading user_version first fails fast with "file is not a database"
        // if the path points at something that is not SQLite.
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(storage)?;
        if version > SCHEMA_VERSION {
            return Err(storage(format!(
                "database layout version {version} is newer than this library supports ({SCHEMA_VERSION}); upgrade msgkit"
            )));
        }
        conn.pragma_update(None, "journal_mode", "WAL").map_err(storage)?;
        conn.pragma_update(None, "synchronous", "FULL").map_err(storage)?;
        if version < SCHEMA_VERSION {
            conn.execute_batch(SCHEMA_SQL).map_err(storage)?;
        }
        Ok(SqliteStore { conn })
    }

    /// Permanently forgets ids acknowledged more than `older_than` ago.
    ///
    /// After this a redelivery of such an event would be accepted as new, so
    /// keep `older_than` above the platforms' retry window
    /// ([`DEFAULT_RETENTION`]). Returns the number of ids removed.
    pub fn prune_acknowledged(&mut self, older_than: Duration) -> Result<usize> {
        let cutoff = now_secs().saturating_sub(i64::try_from(older_than.as_secs()).unwrap_or(i64::MAX));
        self.conn
            .execute("DELETE FROM events WHERE acked_at IS NOT NULL AND acked_at < ?1", params![cutoff])
            .map_err(storage)
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    // Best effort: the database holds personal messages.
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

const INSERT_SQL: &str =
    "INSERT OR IGNORE INTO events (id, timestamp_ms, channel, kind, json) VALUES (?1, ?2, ?3, ?4, ?5)";

fn insert_row(conn: &Connection, event: &Event) -> Result<bool> {
    let json = serde_json::to_string(event).map_err(storage)?;
    let changed = conn
        .prepare_cached(INSERT_SQL)
        .and_then(|mut stmt| {
            stmt.execute(params![
                event.id(),
                event.timestamp().timestamp_millis(),
                event.channel().as_str(),
                event.kind(),
                json
            ])
        })
        .map_err(storage)?;
    Ok(changed == 1)
}

fn parse_row(json: &str) -> Result<Event> {
    serde_json::from_str(json).map_err(|e| storage(format!("corrupt event row: {e}")))
}

impl EventStore for SqliteStore {
    fn insert(&mut self, event: Event) -> Result<bool> {
        // A single statement is already atomic; no explicit transaction needed.
        insert_row(&self.conn, &event)
    }

    fn insert_all(&mut self, events: Vec<Event>) -> Result<usize> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
        let mut added = 0;
        for event in &events {
            added += usize::from(insert_row(&tx, event)?);
        }
        tx.commit().map_err(storage)?;
        Ok(added)
    }

    fn len(&self) -> Result<usize> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events WHERE acked_at IS NULL", [], |r| r.get(0))
            .map_err(storage)?;
        Ok(usize::try_from(n).unwrap_or(0))
    }

    fn list(&self) -> Result<Vec<Event>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT json FROM events WHERE acked_at IS NULL ORDER BY seq")
            .map_err(storage)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(storage)?;
        rows.map(|row| parse_row(&row.map_err(storage)?)).collect()
    }

    fn drain(&mut self) -> Result<Vec<Event>> {
        // Read and acknowledge in one transaction so no event can slip in
        // between and be acknowledged without being returned.
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
        let events = {
            let mut stmt =
                tx.prepare("SELECT json FROM events WHERE acked_at IS NULL ORDER BY seq").map_err(storage)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(storage)?;
            rows.map(|row| parse_row(&row.map_err(storage)?)).collect::<Result<Vec<_>>>()?
        };
        tx.execute(
            "UPDATE events SET json = NULL, acked_at = ?1 WHERE acked_at IS NULL",
            params![now_secs()],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(events)
    }

    fn acknowledge(&mut self, ids: &[String]) -> Result<usize> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
        let now = now_secs();
        let mut acked = 0;
        {
            let mut stmt = tx
                .prepare_cached(
                    "UPDATE events SET json = NULL, acked_at = ?1 WHERE id = ?2 AND acked_at IS NULL",
                )
                .map_err(storage)?;
            for id in ids {
                acked += stmt.execute(params![now, id]).map_err(storage)?;
            }
        }
        tx.commit().map_err(storage)?;
        Ok(acked)
    }
}

#[cfg(test)]
mod tests {
    use msgkit_core::store::conformance::{run, sample_event};

    use super::*;

    #[test]
    fn conforms_in_memory() {
        run(|| SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn conforms_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let counter = std::cell::Cell::new(0);
        run(|| {
            counter.set(counter.get() + 1);
            SqliteStore::open(dir.path().join(format!("s{}.db", counter.get()))).unwrap()
        });
    }

    #[test]
    fn events_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");

        let mut store = SqliteStore::open(&path).unwrap();
        store.insert_all(vec![sample_event("a", 1), sample_event("b", 2)]).unwrap();
        drop(store); // "restart"

        let mut store = SqliteStore::open(&path).unwrap();
        assert_eq!(store.list().unwrap(), vec![sample_event("a", 1), sample_event("b", 2)]);
        assert!(!store.insert(sample_event("a", 1)).unwrap(), "redelivery after restart is a duplicate");
    }

    #[test]
    fn acknowledgements_and_dedupe_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");

        let mut store = SqliteStore::open(&path).unwrap();
        store.insert_all(vec![sample_event("a", 1), sample_event("b", 2)]).unwrap();
        assert_eq!(store.acknowledge(&["a".to_owned()]).unwrap(), 1);
        drop(store);

        let mut store = SqliteStore::open(&path).unwrap();
        let ids: Vec<_> = store.list().unwrap().iter().map(|e| e.id().to_owned()).collect();
        assert_eq!(ids, ["b"]);
        assert!(!store.insert(sample_event("a", 1)).unwrap());
    }

    #[test]
    fn content_is_erased_when_acknowledged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        let mut store = SqliteStore::open(&path).unwrap();
        store.insert_all(vec![sample_event("a", 1), sample_event("b", 2), sample_event("c", 3)]).unwrap();
        store.acknowledge(&["a".to_owned()]).unwrap();
        store.drain().unwrap();

        let raw = Connection::open(&path).unwrap();
        let with_content: i64 =
            raw.query_row("SELECT COUNT(*) FROM events WHERE json IS NOT NULL", [], |r| r.get(0)).unwrap();
        let rows: i64 = raw.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap();
        assert_eq!((with_content, rows), (0, 3), "ids kept for dedupe, content gone");
    }

    #[test]
    fn prune_forgets_only_old_acknowledged_ids() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        store
            .insert_all(vec![sample_event("old", 1), sample_event("new", 2), sample_event("pending", 3)])
            .unwrap();
        store.acknowledge(&["old".to_owned(), "new".to_owned()]).unwrap();
        // Backdate "old" to 40 days ago.
        store
            .conn
            .execute("UPDATE events SET acked_at = ?1 WHERE id = 'old'", params![now_secs() - 40 * 86_400])
            .unwrap();

        assert_eq!(store.prune_acknowledged(DEFAULT_RETENTION).unwrap(), 1);
        assert!(store.insert(sample_event("old", 1)).unwrap(), "pruned id is forgotten");
        assert!(!store.insert(sample_event("new", 2)).unwrap(), "recent id still remembered");
        assert!(!store.insert(sample_event("pending", 3)).unwrap(), "pending never pruned");
    }

    #[test]
    fn two_handles_on_one_file_share_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        let mut a = SqliteStore::open(&path).unwrap();
        let mut b = SqliteStore::open(&path).unwrap();

        assert!(a.insert(sample_event("x", 1)).unwrap());
        assert!(!b.insert(sample_event("x", 1)).unwrap(), "unique id enforced across handles");
        assert!(b.insert(sample_event("y", 2)).unwrap());
        assert_eq!(a.len().unwrap(), 2);
    }

    #[test]
    fn refuses_a_database_from_the_future() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 99).unwrap();
        let err = SqliteStore::open(&path).unwrap_err();
        assert!(err.to_string().contains("newer"), "{err}");
    }

    #[test]
    fn rejects_files_that_are_not_databases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "this is definitely not a sqlite database, just some text ".repeat(20))
            .unwrap();
        assert!(matches!(SqliteStore::open(&path), Err(Error::Storage(_))));
    }

    #[test]
    fn unwritable_location_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing-dir").join("msgkit.db");
        assert!(matches!(SqliteStore::open(path), Err(Error::Storage(_))));
    }

    #[test]
    fn corrupt_rows_are_reported() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        store.insert(sample_event("a", 1)).unwrap();
        store.conn.execute("UPDATE events SET json = '{not json'", []).unwrap();
        assert!(matches!(store.list(), Err(Error::Storage(_))));
    }
}
