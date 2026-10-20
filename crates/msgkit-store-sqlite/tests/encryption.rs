//! Encryption at rest. The store fails closed in every build: a key is never silently ignored and an encrypted
//! file is never opened as if it were plain. The tests marked `sqlcipher` need a build with that feature.

// Some helpers are only used by the tests that need a build with SQLCipher.
#![cfg_attr(not(feature = "sqlcipher"), allow(unused_imports, dead_code))]

use std::path::{Path, PathBuf};

use msgkit_core::store::conformance::{event_from, run, run_erase, sample_event};
use msgkit_core::store::{EraseOptions, EraseStore, EventStore};
use msgkit_core::Error;
use msgkit_store_sqlite::{encrypt_database, DbKey, SqliteStore, StoreOptions};

const KEY: &str = "abababababababababababababababababababababababababababababababab";
const OTHER_KEY: &str = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
const MARKER: &str = "zq9x7k2-encryption-marker-text";

fn key() -> DbKey {
    DbKey::from_hex(KEY).unwrap()
}

fn with_key() -> StoreOptions {
    StoreOptions::default().with_key(key())
}

fn side(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The file, its write-ahead log and its shared memory, as bytes.
fn disk_bytes(path: &Path) -> Vec<u8> {
    ["", "-wal", "-shm"].iter().filter_map(|s| std::fs::read(side(path, s)).ok()).flatten().collect()
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn event_with_text(id: &str, text: &str) -> msgkit_core::Event {
    let mut event = event_from(id, 1, "+94700000000", "+94711111111");
    if let msgkit_core::Event::Message(m) = &mut event {
        m.body = Some(text.to_owned());
        m.raw = None;
    }
    event
}

/// A file that looks like an encrypted database: no SQLite header, random-looking bytes.
fn encrypted_looking_file(path: &Path) {
    let bytes: Vec<u8> =
        (0..8192_u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13).to_le_bytes()[0]).collect();
    std::fs::write(path, bytes).unwrap();
}

fn plain_database_with(path: &Path, events: Vec<msgkit_core::Event>) {
    let mut store = SqliteStore::open(path).unwrap();
    store.insert_all(events).unwrap();
}

// ---- every build -------------------------------------------------------------------------------------------

#[test]
fn an_encrypted_looking_file_without_a_key_is_refused_and_the_error_names_only_the_setting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    encrypted_looking_file(&path);
    let err = SqliteStore::open(&path).unwrap_err();
    assert!(matches!(err, Error::Storage(_)), "{err}");
    assert!(err.to_string().contains("MSGKIT_DB_KEY_FILE"), "{err}");
    assert!(!err.to_string().contains(KEY));
}

#[test]
fn a_key_with_a_plain_database_is_refused_and_the_database_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    plain_database_with(&path, vec![sample_event("a", 1)]);

    let err = SqliteStore::open_with(&path, &with_key()).unwrap_err();
    assert!(err.to_string().contains("not encrypted"), "{err}");
    assert!(err.to_string().contains("msgkit store encrypt"), "{err}");
    assert!(!err.to_string().contains(KEY));

    // Still a plain database, still readable without the key.
    assert_eq!(SqliteStore::open(&path).unwrap().list().unwrap(), vec![sample_event("a", 1)]);
}

#[test]
fn encrypting_checks_its_inputs_and_never_touches_the_plain_file_when_it_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let plain = dir.path().join("plain.db");
    plain_database_with(&plain, vec![sample_event("a", 1)]);
    let before = std::fs::read(&plain).unwrap();

    // Missing input, an output that exists, and an input that is not a plain database.
    let missing = encrypt_database(&dir.path().join("nope.db"), &dir.path().join("out.db"), &key(), false);
    assert!(matches!(missing, Err(Error::InvalidInput(_))));
    let taken = dir.path().join("taken.db");
    std::fs::write(&taken, b"already here").unwrap();
    assert!(matches!(encrypt_database(&plain, &taken, &key(), false), Err(Error::InvalidInput(_))));
    assert_eq!(std::fs::read(&taken).unwrap(), b"already here");
    let looks_encrypted = dir.path().join("enc.db");
    encrypted_looking_file(&looks_encrypted);
    let out = dir.path().join("out2.db");
    assert!(matches!(encrypt_database(&looks_encrypted, &out, &key(), false), Err(Error::InvalidInput(_))));

    assert_eq!(std::fs::read(&plain).unwrap(), before, "a refused call leaves the plain file untouched");
}

// ---- a build without SQLCipher -----------------------------------------------------------------------------

#[cfg(not(feature = "sqlcipher"))]
mod without_sqlcipher {
    use super::*;

    #[test]
    fn a_key_with_a_build_that_cannot_encrypt_is_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        let err = SqliteStore::open_with(&path, &with_key()).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{err}");
        assert!(err.to_string().contains("no SQLCipher support"), "{err}");
        assert!(!err.to_string().contains(KEY));
        assert!(!path.exists(), "no database file may be left behind (it would be plain)");
        assert!(SqliteStore::open_in_memory_with(&with_key()).is_err());
    }

    #[test]
    fn encrypting_without_support_is_refused_and_the_plain_file_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.db");
        plain_database_with(&plain, vec![sample_event("a", 1)]);
        let out = dir.path().join("out.db");
        let err = encrypt_database(&plain, &out, &key(), false).unwrap_err();
        assert!(err.to_string().contains("no SQLCipher support"), "{err}");
        assert!(!out.exists());
        assert_eq!(SqliteStore::open(&plain).unwrap().list().unwrap(), vec![sample_event("a", 1)]);
    }
}

// ---- a build with SQLCipher ----------------------------------------------------------------------------------

#[cfg(feature = "sqlcipher")]
mod with_sqlcipher {
    use super::*;

    #[test]
    fn a_new_database_with_a_key_is_encrypted_and_leaves_no_text_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        let mut store = SqliteStore::open_with(&path, &with_key()).unwrap();
        store
            .insert_all(vec![event_with_text("a", MARKER), event_with_text("b", "other text 5e2f")])
            .unwrap();
        store.acknowledge(&["b".to_owned()]).unwrap();

        // The file does not start like SQLite, and neither it nor its log holds any text, sender or id.
        let bytes = disk_bytes(&path);
        assert_ne!(bytes.len(), 0, "the encrypted database has content on disk");
        assert!(!contains(&bytes, MARKER) && !contains(&bytes, "other text 5e2f"));
        assert!(!contains(&bytes, "+94700000000") && !contains(&bytes, "SQLite format 3"));
        drop(store);
        assert!(!contains(&std::fs::read(&path).unwrap()[..16], "SQLite format 3"));

        let store = SqliteStore::open_with(&path, &with_key()).unwrap();
        assert_eq!(store.list().unwrap(), vec![event_with_text("a", MARKER)]);
    }

    #[test]
    fn the_right_key_is_required_and_the_errors_do_not_repeat_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        SqliteStore::open_with(&path, &with_key()).unwrap().insert(sample_event("a", 1)).unwrap();

        let wrong = StoreOptions::default().with_key(DbKey::from_hex(OTHER_KEY).unwrap());
        let err = SqliteStore::open_with(&path, &wrong).unwrap_err();
        assert!(err.to_string().contains("wrong key"), "{err}");
        assert!(!err.to_string().contains(KEY) && !err.to_string().contains(OTHER_KEY));

        let err = SqliteStore::open(&path).unwrap_err();
        assert!(err.to_string().contains("MSGKIT_DB_KEY_FILE"), "{err}");

        // Neither refusal damaged the database.
        assert_eq!(SqliteStore::open_with(&path, &with_key()).unwrap().len().unwrap(), 1);
    }

    #[test]
    fn an_encrypted_store_passes_the_same_conformance_and_erase_suites() {
        let dir = tempfile::tempdir().unwrap();
        let counter = std::cell::Cell::new(0);
        let make = || {
            counter.set(counter.get() + 1);
            SqliteStore::open_with(dir.path().join(format!("e{}.db", counter.get())), &with_key()).unwrap()
        };
        run(make);
        run_erase(make);
        let mut store = SqliteStore::open_in_memory_with(&with_key()).unwrap();
        store.insert(sample_event("a", 1)).unwrap();
        assert_eq!(store.erase_person("+94700000000", &EraseOptions::default()).unwrap(), 1);
    }

    #[test]
    fn erased_text_is_gone_from_an_encrypted_database_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        let mut store = SqliteStore::open_with(&path, &with_key()).unwrap();
        store.insert(event_with_text("a", MARKER)).unwrap();
        assert_eq!(store.erase_person("+94700000000", &EraseOptions::default()).unwrap(), 1);
        assert_eq!(store.len().unwrap(), 0);
        assert!(!contains(&disk_bytes(&path), MARKER));
    }

    #[test]
    fn encrypting_a_plain_database_copies_verifies_and_then_overwrites_and_deletes_the_plain_file() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.db");
        let out = dir.path().join("encrypted.db");
        {
            let mut store = SqliteStore::open(&plain).unwrap();
            store
                .insert_all(vec![
                    event_with_text("held", MARKER),
                    sample_event("done", 2),
                    sample_event("gone", 3),
                ])
                .unwrap();
            store.acknowledge(&["done".to_owned()]).unwrap();
            assert!(contains(&disk_bytes(&plain), MARKER), "the plain database holds the text before");
            // Left open on purpose: the write-ahead log is still there, as when a service was stopped badly.
            let report = encrypt_database(&plain, &out, &key(), false).unwrap();
            assert_eq!(report.rows, 3);
            assert!(report.plain_deleted);
        }
        assert!(!plain.exists() && !side(&plain, "-wal").exists() && !side(&plain, "-shm").exists());

        let bytes = disk_bytes(&out);
        assert!(!contains(&bytes, MARKER) && !contains(&bytes, "SQLite format 3"));
        let mut store = SqliteStore::open_with(&out, &with_key()).unwrap();
        let held: Vec<_> = store.list().unwrap().iter().map(|e| e.id().to_owned()).collect();
        assert_eq!(held, ["held", "gone"]);
        assert!(!store.insert(sample_event("done", 2)).unwrap(), "acknowledged ids survive the conversion");
        assert!(SqliteStore::open(&out).is_err(), "the copy does not open without the key");
    }

    #[test]
    fn keep_plain_keeps_the_plain_file_and_a_failure_leaves_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.db");
        plain_database_with(&plain, vec![sample_event("a", 1)]);

        let kept = dir.path().join("kept.db");
        let report = encrypt_database(&plain, &kept, &key(), true).unwrap();
        assert!(!report.plain_deleted);
        assert_eq!(SqliteStore::open(&plain).unwrap().list().unwrap(), vec![sample_event("a", 1)]);

        // An output that cannot be written: the call fails, the plain file is intact, no output remains.
        let bad = dir.path().join("no-such-dir").join("out.db");
        assert!(encrypt_database(&plain, &bad, &key(), false).is_err());
        assert!(!bad.exists());
        assert_eq!(SqliteStore::open(&plain).unwrap().list().unwrap(), vec![sample_event("a", 1)]);
    }
}
