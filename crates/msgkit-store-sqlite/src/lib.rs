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
//!   removes it. SQLite's `secure_delete` is on by default (see
//!   [`StoreOptions`]), so the freed space is overwritten with zeros rather
//!   than left holding the old text.
//! * **Deletion and retention** ([`EraseStore`], [`RetentionStore`]). `erase_person` removes the
//!   content of every held event that involves one person (any spelling of a
//!   phone number matches), `expire_pending` removes events nobody collected,
//!   and `prune_acknowledged` forgets old ids. After a deletion the write-ahead
//!   log is checkpointed and truncated; [`SqliteStore::compact`] also runs
//!   `VACUUM`. Acknowledging does not truncate the log (it would make every
//!   acknowledge expensive): acknowledged text can stay in the `-wal` file
//!   until [`SqliteStore::checkpoint`] runs. `msgkit-service` calls it every
//!   six hours; a library user should do the same.
//! * **Ids are personal data too.** An id is derived from the sender and the
//!   text (webhook ids from the platform's message id). It holds no readable
//!   content, but anyone who can guess the content can confirm it, so it is
//!   pseudonymous personal data. An acknowledged or erased event keeps its id
//!   for [`DEFAULT_RETENTION`] (30 days) after the moment it was acknowledged
//!   or erased, so redelivered webhooks are still recognised, and is then
//!   removed by `prune_acknowledged`. The service runs that at start-up and
//!   every six hours; a library user must call it.
//! * **Multi-process safe.** Several processes may open the same file; writers
//!   wait up to five seconds for each other.
//! * **Encrypted only on request.** By default content is stored as plain
//!   JSON: use full-disk encryption or an encrypted volume, and restrict file
//!   access (the file is created owner-only on Unix). Built with the
//!   `sqlcipher` feature and given a [`DbKey`] in [`StoreOptions`], the whole
//!   file (and its log) is encrypted with SQLCipher. The store fails closed: a
//!   key with a plain file or a build without SQLCipher, and an encrypted file
//!   without a key, are all refused. Losing the key loses the data. Convert an
//!   existing database once with [`encrypt_database`]. See `docs/ENCRYPTION.md`.
//! * **Keep it out of cloud-synced folders.** A production database must not
//!   live in `OneDrive`, Google Drive, Dropbox or any similar synced folder:
//!   those copy the file, its `-wal` file and every earlier version to other
//!   machines and to the provider, where msgkit cannot erase them and a
//!   half-synced WAL can corrupt the database. Use a local, non-synced path.
//! * **What erasure cannot reach.** Deletion covers this database file and its
//!   write-ahead log. It cannot recall backups, snapshots, copies made by the
//!   operating system or a sync client, or bytes a flash drive's wear levelling
//!   keeps (use an encrypted volume for that).

mod encrypt;
mod key;

pub use encrypt::{encrypt_database, EncryptReport};
pub use key::{cloud_synced_folder, key_file_too_open, DbKey};

use std::fmt::Display;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use msgkit_core::store::{erase_key, involves, EraseOptions, EraseStore, EventStore, RetentionStore};
use msgkit_core::{Error, Event, Result};
use rusqlite::{params, Connection, TransactionBehavior};

/// Version of the on-disk layout, kept in `PRAGMA user_version`.
///
/// 1: the first layout. 2: adds `received_at`, the second an event was inserted, for `expire_pending`.
/// A database at version 1 is upgraded in place when opened; an older msgkit refuses a version 2 database.
const SCHEMA_VERSION: i64 = 2;

/// Suggested retention for acknowledged ids: 30 days. Meta now retries a failed WhatsApp delivery for up to
/// 7 days (it used to be 30), so 8 to 14 days is enough, and enough for replay protection with its default 8-day
/// window; 30 is the conservative default kept from before. The id is pseudonymous personal data, so a shorter
/// retention keeps less of it. Keep it at least as long as `MSGKIT_REPLAY_WINDOW_HOURS` when replay protection is
/// enforced (the service checks).
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

const SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS events (
        seq          INTEGER PRIMARY KEY AUTOINCREMENT,
        id           TEXT    NOT NULL UNIQUE,
        timestamp_ms INTEGER NOT NULL,
        channel      TEXT    NOT NULL,
        kind         TEXT    NOT NULL,
        json         TEXT,
        acked_at     INTEGER,
        received_at  INTEGER
    );
    CREATE INDEX IF NOT EXISTS events_pending ON events(seq) WHERE acked_at IS NULL;
    CREATE INDEX IF NOT EXISTS events_acked ON events(acked_at) WHERE acked_at IS NOT NULL;";

/// Layout 1 to 2. Events already held get `received_at` = now, the earliest moment this store can vouch for.
const MIGRATE_1_TO_2_SQL: &str = "ALTER TABLE events ADD COLUMN received_at INTEGER;";

/// Options for opening a [`SqliteStore`].
///
/// `#[non_exhaustive]`: start from [`StoreOptions::default`] and use the `with_*` setters.
///
/// ```
/// use msgkit_store_sqlite::StoreOptions;
///
/// assert!(StoreOptions::default().secure_delete);
/// assert!(!StoreOptions::default().with_secure_delete(false).secure_delete);
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct StoreOptions {
    /// The key of an encrypted database (SQLCipher; build with the `sqlcipher` or `sqlcipher-vendored` feature).
    /// The store **fails closed**: a key with a build that cannot encrypt, a key with a plain database file, and an
    /// encrypted file without a key are all refused, never opened unencrypted. A new file is created encrypted.
    pub key: Option<DbKey>,
    /// Overwrite deleted content with zeros (SQLite `secure_delete = ON`). On by default; it makes deletes and
    /// updates a little slower. The setting belongs to this connection: another process that opens the same file
    /// without it, such as an older msgkit, does not overwrite.
    pub secure_delete: bool,
}

impl Default for StoreOptions {
    fn default() -> Self {
        StoreOptions { key: None, secure_delete: true }
    }
}

impl StoreOptions {
    /// Sets [`key`](Self::key).
    #[must_use]
    pub fn with_key(mut self, key: DbKey) -> Self {
        self.key = Some(key);
        self
    }

    /// Sets [`secure_delete`](Self::secure_delete).
    #[must_use]
    pub fn with_secure_delete(mut self, on: bool) -> Self {
        self.secure_delete = on;
        self
    }
}

fn storage(e: impl Display) -> Error {
    Error::Storage(e.to_string())
}

/// The first bytes of every plain SQLite file. An encrypted file starts with random bytes instead.
const PLAIN_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// Whether the file at `path` starts like a plain SQLite database.
fn has_plain_header(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0_u8; 16];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && &head == PLAIN_HEADER
}

/// Whether the file exists and holds something (an empty file is a database that was never written to).
fn has_content(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() > 0)
}

/// Refuses a connection whose SQLite is not SQLCipher. Plain SQLite ignores `PRAGMA key`, so without this check a
/// build without the feature would silently write an unencrypted database while a key is configured.
fn require_cipher(conn: &Connection) -> Result<()> {
    let version: Option<String> = conn.query_row("PRAGMA cipher_version", [], |r| r.get(0)).ok();
    if version.is_some_and(|v| !v.is_empty()) {
        Ok(())
    } else {
        Err(Error::InvalidInput(
            "a database key is set, but this build has no SQLCipher support (build with the `sqlcipher` feature)"
                .to_owned(),
        ))
    }
}

fn user_version(conn: &Connection) -> Result<i64> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(storage)
}

/// Creates or upgrades the layout inside one write transaction. The version is read again inside it, so two
/// processes opening an old database at once upgrade it only once.
fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE").map_err(storage)?;
    let result = (|| -> Result<()> {
        let version = user_version(conn)?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }
        if version == 0 {
            conn.execute_batch(SCHEMA_SQL).map_err(storage)?;
        } else {
            conn.execute_batch(MIGRATE_1_TO_2_SQL).map_err(storage)?;
            conn.execute("UPDATE events SET received_at = ?1 WHERE received_at IS NULL", params![now_secs()])
                .map_err(storage)?;
        }
        conn.pragma_update(None, "user_version", SCHEMA_VERSION).map_err(storage)
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(storage),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// The moment `older_than` ago, in seconds: things dated before it are "older than" the window.
fn cutoff_secs(older_than: Duration) -> i64 {
    now_secs().saturating_sub(i64::try_from(older_than.as_secs()).unwrap_or(i64::MAX))
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[derive(Debug)]
pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Opens (creating if needed) the database at `path` with the default [`StoreOptions`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, &StoreOptions::default())
    }

    /// Opens (creating if needed) the database at `path`.
    pub fn open_with(path: impl AsRef<Path>, options: &StoreOptions) -> Result<Self> {
        let path = path.as_ref();
        // Fail closed before touching the file: what is on disk must agree with the key setting.
        if has_content(path) {
            match (options.key.is_some(), has_plain_header(path)) {
                (false, false) => {
                    return Err(storage(
                        "the database file is not a plain SQLite database: if it is encrypted, a key is required \
                         (MSGKIT_DB_KEY_FILE)",
                    ));
                }
                (true, true) => {
                    return Err(Error::InvalidInput(
                        "a database key is set, but the database is not encrypted: encrypt it once with \
                         `msgkit store encrypt`, or unset the key (it is not used on a plain database)"
                            .to_owned(),
                    ));
                }
                _ => {}
            }
        }
        let is_new = !path.exists();
        let conn =
            Connection::open(path).map_err(|e| storage(format!("cannot open {}: {e}", path.display())))?;
        if is_new {
            restrict_permissions(path);
        }
        let opened = Self::init(conn, options);
        if opened.is_err() && is_new && !has_content(path) {
            // Nothing was written: do not leave an empty file behind that the next start would take for a database.
            let _ = std::fs::remove_file(path);
        }
        opened
    }

    /// A private in-memory database, mainly for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::open_in_memory_with(&StoreOptions::default())
    }

    /// A private in-memory database with the given options.
    pub fn open_in_memory_with(options: &StoreOptions) -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(storage)?, options)
    }

    fn init(conn: Connection, options: &StoreOptions) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5)).map_err(storage)?;
        if let Some(key) = &options.key {
            conn.execute_batch(&key.pragma()).map_err(storage)?;
            require_cipher(&conn)?;
        }
        // Reading user_version first fails fast with "file is not a database"
        // if the path points at something that is not SQLite (or, with a key, at a database that key does not open).
        let version = user_version(&conn).map_err(|e| {
            if options.key.is_some() {
                storage("the database cannot be read with the configured key (wrong key, or a damaged file)")
            } else {
                e
            }
        })?;
        if version > SCHEMA_VERSION {
            return Err(storage(format!(
                "database layout version {version} is newer than this library supports ({SCHEMA_VERSION}); upgrade msgkit"
            )));
        }
        conn.pragma_update(None, "journal_mode", "WAL").map_err(storage)?;
        conn.pragma_update(None, "synchronous", "FULL").map_err(storage)?;
        conn.pragma_update(None, "secure_delete", options.secure_delete).map_err(storage)?;
        if version < SCHEMA_VERSION {
            migrate(&conn)?;
        }
        Ok(SqliteStore { conn })
    }

    /// Truncates the write-ahead log: its frames, which may still hold the text of deleted events, are folded
    /// into the database file and the log is emptied. Returns `false` when another connection was reading and the
    /// log could not be emptied this time (nothing is lost; try again later). Deleting calls this itself.
    pub fn checkpoint(&self) -> Result<bool> {
        let busy: i64 =
            self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0)).map_err(storage)?;
        Ok(busy == 0)
    }

    /// Rebuilds the database file (`VACUUM`) so no free pages remain, then truncates the log. This can take a
    /// while and needs free disk space about the size of the database; it blocks other writers meanwhile.
    pub fn compact(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM").map_err(storage)?;
        self.checkpoint()?;
        Ok(())
    }

    /// Permanently forgets ids acknowledged more than `older_than` ago.
    ///
    /// After this a redelivery of such an event would be accepted as new, so
    /// keep `older_than` above the platforms' retry window
    /// ([`DEFAULT_RETENTION`]). Returns the number of ids removed.
    pub fn prune_acknowledged(&mut self, older_than: Duration) -> Result<usize> {
        let n = self
            .conn
            .execute(
                "DELETE FROM events WHERE acked_at IS NOT NULL AND acked_at < ?1",
                params![cutoff_secs(older_than)],
            )
            .map_err(storage)?;
        self.after_delete(n)?;
        Ok(n)
    }

    /// Empties the write-ahead log after rows were deleted, so the deleted text does not linger in it.
    fn after_delete(&self, deleted: usize) -> Result<()> {
        if deleted > 0 {
            self.checkpoint()?;
        }
        Ok(())
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
    "INSERT OR IGNORE INTO events (id, timestamp_ms, channel, kind, json, received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

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
                json,
                now_secs()
            ])
        })
        .map_err(storage)?;
    Ok(changed == 1)
}

fn parse_row(json: &str) -> Result<Event> {
    serde_json::from_str(json).map_err(|e| {
        storage(format!("corrupt event row: {}", msgkit_core::error::describe_json_error(&e, true)))
    })
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

impl EraseStore for SqliteStore {
    fn erase_person(&mut self, address: &str, options: &EraseOptions) -> Result<usize> {
        let key = erase_key(address, options)?;
        let cc = options.default_country_code.as_deref();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(storage)?;
        let mut matching = Vec::new();
        {
            let mut stmt =
                tx.prepare("SELECT seq, json FROM events WHERE acked_at IS NULL").map_err(storage)?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))).map_err(storage)?;
            for row in rows {
                let (seq, json) = row.map_err(storage)?;
                // A row that cannot be read cannot be matched; fail rather than report an erasure that did not
                // look at it. The error does not quote the row.
                if involves(&parse_row(&json)?, &key, cc) {
                    matching.push(seq);
                }
            }
        }
        let now = now_secs();
        for seq in &matching {
            if options.drop_ids {
                tx.execute("DELETE FROM events WHERE seq = ?1", params![seq]).map_err(storage)?;
            } else {
                tx.execute("UPDATE events SET json = NULL, acked_at = ?1 WHERE seq = ?2", params![now, seq])
                    .map_err(storage)?;
            }
        }
        tx.commit().map_err(storage)?;
        self.after_delete(matching.len())?;
        Ok(matching.len())
    }
}

impl RetentionStore for SqliteStore {
    fn expire_pending(&mut self, older_than: Duration) -> Result<usize> {
        let n = self
            .conn
            .execute(
                "UPDATE events SET json = NULL, acked_at = ?1 WHERE acked_at IS NULL AND received_at < ?2",
                params![now_secs(), cutoff_secs(older_than)],
            )
            .map_err(storage)?;
        self.after_delete(n)?;
        Ok(n)
    }

    fn prune_acknowledged(&mut self, older_than: Duration) -> Result<usize> {
        SqliteStore::prune_acknowledged(self, older_than)
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

    // ---- deletion and retention -------------------------------------------------------------------------

    use msgkit_core::store::conformance::{event_from, run_erase};

    const MARKER: &str = "zq9x7k2-unique-marker-text";

    /// Everything on disk that belongs to the database: the file, its write-ahead log and shared memory.
    fn disk_bytes(path: &Path) -> Vec<u8> {
        let mut all = Vec::new();
        for suffix in ["", "-wal", "-shm"] {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            if let Ok(bytes) = std::fs::read(Path::new(&name)) {
                all.extend(bytes);
            }
        }
        all
    }

    fn contains(haystack: &[u8], needle: &str) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    fn event_with_text(id: &str, sender: &str, text: &str) -> Event {
        let mut event = event_from(id, 1, sender, "+94711111111");
        if let Event::Message(m) = &mut event {
            m.body = Some(text.to_owned());
            m.raw = Some(serde_json::json!({ "text": text }));
        }
        event
    }

    #[test]
    fn erases_in_memory() {
        run_erase(|| SqliteStore::open_in_memory().unwrap());
    }

    #[test]
    fn erases_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let counter = std::cell::Cell::new(0);
        run_erase(|| {
            counter.set(counter.get() + 1);
            SqliteStore::open(dir.path().join(format!("e{}.db", counter.get()))).unwrap()
        });
    }

    #[test]
    fn erased_text_is_gone_from_the_file_and_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        let mut store = SqliteStore::open(&path).unwrap();
        store
            .insert_all(vec![
                event_with_text("gone", "+94700000000", MARKER),
                event_with_text("kept", "+94722222222", "kept text 8c1d"),
            ])
            .unwrap();
        let before = disk_bytes(&path);
        assert!(
            contains(&before, MARKER),
            "the test must see the text on disk before erasing, or it proves nothing"
        );

        let opts = EraseOptions::default().with_default_country_code("94");
        assert_eq!(store.erase_person("0700000000", &opts).unwrap(), 1);

        // Still open, so the log file is still there: nothing of the erased text is in either file.
        let after = disk_bytes(&path);
        assert!(!contains(&after, MARKER), "erased text is still on disk");
        assert!(contains(&after, "kept text 8c1d"), "other events are untouched");
        assert_eq!(store.len().unwrap(), 1);
    }

    #[test]
    fn secure_delete_is_on_by_default_and_can_be_switched_off() {
        let pragma = |s: &SqliteStore| -> i64 {
            s.conn.pragma_query_value(None, "secure_delete", |r| r.get(0)).unwrap()
        };
        assert_eq!(pragma(&SqliteStore::open_in_memory().unwrap()), 1);
        let off =
            SqliteStore::open_in_memory_with(&StoreOptions::default().with_secure_delete(false)).unwrap();
        assert_eq!(pragma(&off), 0);
    }

    #[test]
    fn acknowledged_text_is_gone_from_the_file_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        let mut store = SqliteStore::open(&path).unwrap();
        store.insert(event_with_text("a", "+94700000000", MARKER)).unwrap();
        assert!(contains(&disk_bytes(&path), MARKER));
        store.acknowledge(&["a".to_owned()]).unwrap();
        store.checkpoint().unwrap();
        assert!(!contains(&disk_bytes(&path), MARKER), "acknowledged text is still on disk");
    }

    #[test]
    fn compact_keeps_the_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SqliteStore::open(dir.path().join("msgkit.db")).unwrap();
        store.insert_all(vec![sample_event("a", 1), sample_event("b", 2)]).unwrap();
        store.acknowledge(&["a".to_owned()]).unwrap();
        store.compact().unwrap();
        assert_eq!(store.list().unwrap(), vec![sample_event("b", 2)]);
        assert!(!store.insert(sample_event("a", 1)).unwrap());
    }

    #[test]
    fn an_erased_id_is_kept_for_the_retention_window_and_then_pruned() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        store.insert_all(vec![sample_event("a", 1), sample_event("b", 2)]).unwrap();
        let opts = EraseOptions::default().with_default_country_code("94");
        // sample_event's sender is +94700000000.
        assert_eq!(store.erase_person("070 000 0000", &opts).unwrap(), 2);

        // Kept: nothing but the id and the erase time, and a redelivery is a duplicate.
        let (json_null, acked_at): (i64, i64) = store
            .conn
            .query_row("SELECT COUNT(*) FILTER (WHERE json IS NULL), MAX(acked_at) FROM events", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(json_null, 2);
        assert!((now_secs() - acked_at).abs() <= 2, "the retention clock starts when the event was erased");
        assert!(!store.insert(sample_event("a", 1)).unwrap());

        // 29 days after the erasure the id is still kept; 31 days after it is removed.
        let backdate = |store: &SqliteStore, days: i64| {
            store
                .conn
                .execute("UPDATE events SET acked_at = ?1", params![now_secs() - days * 86_400])
                .unwrap();
        };
        backdate(&store, 29);
        assert_eq!(store.prune_acknowledged(DEFAULT_RETENTION).unwrap(), 0);
        backdate(&store, 31);
        assert_eq!(RetentionStore::prune_acknowledged(&mut store, DEFAULT_RETENTION).unwrap(), 2);
        assert!(store.insert(sample_event("a", 1)).unwrap(), "a pruned id is forgotten");
    }

    #[test]
    fn expire_pending_erases_by_arrival_not_by_event_time() {
        let mut store = SqliteStore::open_in_memory().unwrap();
        // The events are dated 1970; they only count as old by when they were inserted.
        store
            .insert_all(vec![sample_event("old", 1), sample_event("new", 2), sample_event("acked", 3)])
            .unwrap();
        store.acknowledge(&["acked".to_owned()]).unwrap();
        store
            .conn
            .execute("UPDATE events SET received_at = ?1 WHERE id = 'old'", params![now_secs() - 10 * 86_400])
            .unwrap();

        assert_eq!(store.expire_pending(Duration::from_secs(7 * 86_400)).unwrap(), 1);
        let ids: Vec<_> = store.list().unwrap().iter().map(|e| e.id().to_owned()).collect();
        assert_eq!(ids, ["new"]);
        assert!(!store.insert(sample_event("old", 1)).unwrap(), "the id of an expired event is kept");
        let content: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM events WHERE id = 'old' AND json IS NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(content, 1);
    }

    #[test]
    fn a_version_1_database_is_upgraded_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        {
            // The layout 0.2.0 wrote before this change.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (
                    seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE, timestamp_ms INTEGER NOT NULL,
                    channel TEXT NOT NULL, kind TEXT NOT NULL, json TEXT, acked_at INTEGER);
                 CREATE INDEX events_pending ON events(seq) WHERE acked_at IS NULL;
                 CREATE INDEX events_acked ON events(acked_at) WHERE acked_at IS NOT NULL;
                 PRAGMA user_version = 1;",
            )
            .unwrap();
            let json = serde_json::to_string(&sample_event("held", 1)).unwrap();
            conn.execute(
                "INSERT INTO events (id, timestamp_ms, channel, kind, json) VALUES ('held', 1000, 'whatsapp', 'message', ?1)",
                params![json],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events (id, timestamp_ms, channel, kind, json, acked_at) VALUES ('done', 2000, 'whatsapp', 'message', NULL, 5)",
                [],
            )
            .unwrap();
        }

        let mut store = SqliteStore::open(&path).unwrap();
        assert_eq!(user_version(&store.conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(store.list().unwrap(), vec![sample_event("held", 1)]);
        assert!(!store.insert(sample_event("done", 2)).unwrap(), "acknowledged ids survive the upgrade");
        let stamped: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM events WHERE id = 'held' AND received_at IS NOT NULL", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stamped, 1, "rows already held are stamped with the upgrade time");

        // The upgraded database works for the new operations, and a second open changes nothing.
        let opts = EraseOptions::default().with_default_country_code("94");
        assert_eq!(store.erase_person("+94700000000", &opts).unwrap(), 1);
        drop(store);
        let store = SqliteStore::open(&path).unwrap();
        assert_eq!(user_version(&store.conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(store.len().unwrap(), 0);
    }

    #[test]
    fn an_interrupted_upgrade_leaves_a_valid_version_1_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        let json = serde_json::to_string(&sample_event("held", 1)).unwrap();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (
                    seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE, timestamp_ms INTEGER NOT NULL,
                    channel TEXT NOT NULL, kind TEXT NOT NULL, json TEXT, acked_at INTEGER);
                 PRAGMA journal_mode = WAL;
                 PRAGMA user_version = 1;
                 CREATE TRIGGER stop_upgrade BEFORE UPDATE ON events BEGIN SELECT RAISE(ABORT, 'interrupted'); END;",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events (id, timestamp_ms, channel, kind, json) VALUES ('held', 1000, 'whatsapp', 'message', ?1)",
                params![json],
            )
            .unwrap();
        }

        // The trigger makes the second step of the upgrade (stamping the rows) fail after the column was added.
        assert!(matches!(SqliteStore::open(&path), Err(Error::Storage(_))), "the upgrade must fail here");

        // Nothing of the upgrade stayed: still version 1, no new column, the row untouched.
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), 1);
        let has_column: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('events') WHERE name = 'received_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_column, 0, "the half-finished upgrade was not rolled back");
        let kept: String =
            conn.query_row("SELECT json FROM events WHERE id = 'held'", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, json);

        // Once the obstacle is gone the same file upgrades normally.
        conn.execute_batch("DROP TRIGGER stop_upgrade").unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).unwrap();
        assert_eq!(user_version(&store.conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(store.list().unwrap(), vec![sample_event("held", 1)]);
    }

    #[test]
    fn two_processes_opening_an_old_database_upgrade_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (
                    seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE, timestamp_ms INTEGER NOT NULL,
                    channel TEXT NOT NULL, kind TEXT NOT NULL, json TEXT, acked_at INTEGER);
                 PRAGMA journal_mode = WAL;
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || SqliteStore::open(path).map(|_| ()))
            })
            .collect();
        for h in handles {
            h.join().unwrap().unwrap();
        }
    }
}
