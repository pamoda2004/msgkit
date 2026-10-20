//! `msgkit store erase`: any spelling of a number erases the same person, the output is a count only, and the
//! address never appears in the output.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use msgkit_core::store::conformance::event_from;
use msgkit_core::store::EventStore;
use msgkit_store_sqlite::SqliteStore;

fn database(path: &Path) {
    let mut store = SqliteStore::open(path).unwrap();
    store
        .insert_all(vec![
            event_from("a", 1, "+94700000000", "+94711111111"),
            event_from("b", 2, "+94722222222", "+94700000000"),
            event_from("c", 3, "+94700000003", "+94700000004"),
        ])
        .unwrap();
}

fn msgkit(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_msgkit"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(text) = stdin {
        child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn erases_one_person_in_any_spelling_and_prints_only_a_count() {
    for spelling in ["0700000000", "070 000 0000", "+94700000000"] {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("msgkit.db");
        database(&db);

        let out = msgkit(
            &["store", "erase", db.to_str().unwrap(), "--address", spelling, "--default-country-code", "94"],
            None,
        );
        assert!(out.status.success(), "{spelling}: {}", text(&out));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "erased 2 event(s)", "{spelling}");
        assert!(!text(&out).contains("700000000") && !text(&out).contains("070"), "the address was echoed");

        let store = SqliteStore::open(&db).unwrap();
        let ids: Vec<_> = store.list().unwrap().iter().map(|e| e.id().to_owned()).collect();
        assert_eq!(ids, ["c"], "{spelling}");
    }
}

#[test]
fn the_address_can_come_from_standard_input() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("msgkit.db");
    database(&db);
    let out = msgkit(
        &[
            "store",
            "erase",
            db.to_str().unwrap(),
            "--address-stdin",
            "--default-country-code",
            "94",
            "--compact",
        ],
        Some("070 000 0000\n"),
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "erased 2 event(s)");
}

#[test]
fn an_unknown_person_erases_nothing_without_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("msgkit.db");
    database(&db);
    let out = msgkit(&["store", "erase", db.to_str().unwrap(), "--address", "+94700000009"], None);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "erased 0 event(s)");
}

#[test]
fn a_missing_or_empty_address_and_a_missing_database_are_errors_that_do_not_echo_input() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("msgkit.db");
    database(&db);
    let path = db.to_str().unwrap();

    let none = msgkit(&["store", "erase", path], None);
    assert!(!none.status.success());
    let empty = msgkit(&["store", "erase", path, "--address", "  "], None);
    assert!(!empty.status.success());
    let missing = msgkit(
        &["store", "erase", dir.path().join("nope.db").to_str().unwrap(), "--address", "SECRET-ADDR"],
        None,
    );
    assert!(!missing.status.success());
    assert!(!text(&missing).contains("SECRET-ADDR"), "{}", text(&missing));

    // Nothing was erased by the failed calls, and the missing database was not created.
    assert_eq!(SqliteStore::open(&db).unwrap().len().unwrap(), 3);
    assert!(!dir.path().join("nope.db").exists());
}

// ---- encryption ------------------------------------------------------------------------------------------

const KEY: &str = "abababababababababababababababababababababababababababababababab";

fn key_file(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("db.key");
    std::fs::write(&path, format!("{KEY}\n")).unwrap();
    path
}

#[test]
fn a_bad_key_is_refused_without_repeating_it_and_the_key_never_comes_from_an_argument() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("msgkit.db");
    database(&db);
    let bad = dir.path().join("bad.key");
    std::fs::write(&bad, "not-a-key-SECRET").unwrap();
    let path = db.to_str().unwrap();

    let out = msgkit(
        &["store", "erase", path, "--address", "+94700000000", "--key-file", bad.to_str().unwrap()],
        None,
    );
    assert!(!out.status.success());
    assert!(text(&out).contains("64 hexadecimal"), "{}", text(&out));
    assert!(!text(&out).contains("SECRET"), "{}", text(&out));

    let missing =
        msgkit(&["store", "erase", path, "--address", "+94700000000", "--key-file", "/no/such/key"], None);
    assert!(!missing.status.success() && text(&missing).contains("key file cannot be read"));

    // There is no `--key` argument, and the two standard-input flags cannot be combined.
    assert!(!msgkit(&["store", "erase", path, "--address", "x", "--key", KEY], None).status.success());
    let both = msgkit(&["store", "erase", path, "--address-stdin", "--key-stdin"], Some("x\n"));
    assert!(!both.status.success());
    assert_eq!(SqliteStore::open(&db).unwrap().len().unwrap(), 3, "nothing was erased");
}

#[test]
fn erasing_with_a_key_on_a_plain_database_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("msgkit.db");
    database(&db);
    let key = key_file(dir.path());
    let out = msgkit(
        &[
            "store",
            "erase",
            db.to_str().unwrap(),
            "--address",
            "+94700000000",
            "--key-file",
            key.to_str().unwrap(),
        ],
        None,
    );
    assert!(!out.status.success());
    assert!(!text(&out).contains(KEY), "{}", text(&out));
    assert_eq!(SqliteStore::open(&db).unwrap().len().unwrap(), 3);
}

#[cfg(not(feature = "sqlcipher"))]
#[test]
fn encrypting_with_a_build_that_cannot_encrypt_fails_and_keeps_the_plain_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("msgkit.db");
    database(&db);
    let key = key_file(dir.path());
    let out_path = dir.path().join("encrypted.db");
    let out = msgkit(
        &[
            "store",
            "encrypt",
            db.to_str().unwrap(),
            "--output",
            out_path.to_str().unwrap(),
            "--key-file",
            key.to_str().unwrap(),
        ],
        None,
    );
    assert!(!out.status.success());
    assert!(text(&out).contains("no SQLCipher support"), "{}", text(&out));
    assert!(!out_path.exists());
    assert_eq!(SqliteStore::open(&db).unwrap().len().unwrap(), 3);
}

#[cfg(feature = "sqlcipher")]
#[test]
fn encrypt_then_erase_with_the_key_from_a_file_or_from_standard_input() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("msgkit.db");
    database(&db);
    let key = key_file(dir.path());
    let encrypted = dir.path().join("encrypted.db");

    let out = msgkit(
        &["store", "encrypt", db.to_str().unwrap(), "--output", encrypted.to_str().unwrap(), "--key-stdin"],
        Some(&format!("{KEY}\n")),
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        text(&out).contains("encrypted 3 row(s)") && text(&out).contains("overwritten and deleted"),
        "{}",
        text(&out)
    );
    assert!(!text(&out).contains(KEY));
    assert!(!db.exists(), "the plain database was removed after the copy was verified");

    // Without the key the encrypted file is refused; with it, erase works.
    let refused = msgkit(&["store", "erase", encrypted.to_str().unwrap(), "--address", "+94700000000"], None);
    assert!(!refused.status.success() && text(&refused).contains("MSGKIT_DB_KEY_FILE"), "{}", text(&refused));
    let ok = msgkit(
        &[
            "store",
            "erase",
            encrypted.to_str().unwrap(),
            "--address",
            "+94700000000",
            "--key-file",
            key.to_str().unwrap(),
        ],
        None,
    );
    assert!(ok.status.success(), "{}", text(&ok));
    assert_eq!(String::from_utf8_lossy(&ok.stdout).trim(), "erased 2 event(s)");
}
