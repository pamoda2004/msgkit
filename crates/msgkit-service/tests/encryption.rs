//! The service fails closed on the database key: it refuses to start rather than run unencrypted or open an
//! encrypted file without its key, and no error repeats the key.
#![cfg_attr(not(feature = "sqlcipher"), allow(unused_imports, dead_code))]

use std::path::Path;

use msgkit_core::store::conformance::{event_from, sample_event};
use msgkit_core::store::EventStore;
use msgkit_service::{maintain_database, AppState, Config, Maintenance};
use msgkit_store_sqlite::{DbKey, SqliteStore, StoreOptions};

const KEY: &str = "abababababababababababababababababababababababababababababababab";

fn key() -> DbKey {
    DbKey::from_hex(KEY).unwrap()
}

fn encrypted_looking_file(path: &Path) {
    let bytes: Vec<u8> =
        (0..8192_u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13).to_le_bytes()[0]).collect();
    std::fs::write(path, bytes).unwrap();
}

fn refusal(config: Config) -> String {
    let err = AppState::new(config).expect_err("the service must refuse to start");
    let text = err.to_string();
    assert!(!text.contains(KEY), "the error repeats the key: {text}");
    text
}

#[test]
fn the_key_is_never_shown_by_debug() {
    let config = Config::default().with_db_key(key());
    let shown = format!("{config:?}");
    assert!(!shown.contains(KEY) && !shown.contains("abab"), "{shown}");
}

#[test]
fn an_encrypted_looking_database_without_a_key_stops_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    encrypted_looking_file(&path);
    let text = refusal(Config::default().with_db_path(&path));
    assert!(text.contains("MSGKIT_DB_KEY_FILE"), "{text}");
}

#[test]
fn a_key_with_a_plain_database_stops_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    SqliteStore::open(&path).unwrap().insert(sample_event("a", 1)).unwrap();
    let text = refusal(Config::default().with_db_path(&path).with_db_key(key()));
    assert!(text.contains("not encrypted"), "{text}");
    assert_eq!(SqliteStore::open(&path).unwrap().len().unwrap(), 1, "the plain database is untouched");
}

#[cfg(not(feature = "sqlcipher"))]
#[test]
fn a_key_with_a_build_that_cannot_encrypt_stops_the_service() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("msgkit.db");
    let text = refusal(Config::default().with_db_path(&path).with_db_key(key()));
    assert!(text.contains("no SQLCipher support"), "{text}");
    assert!(!path.exists(), "no plain database may be created while a key is configured");
}

#[cfg(feature = "sqlcipher")]
mod with_sqlcipher {
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use msgkit_service::router;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn an_encrypted_database_works_end_to_end_and_stays_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        SqliteStore::open_with(&path, &StoreOptions::default().with_key(key()))
            .unwrap()
            .insert_all(vec![event_from("a", 1, "+94700000000", "+94711111111"), sample_event("b", 2)])
            .unwrap();

        let config = Config::default().with_db_path(&path).with_db_key(key()).with_api_token("t");
        let app = router(AppState::new(config.clone()).unwrap());
        let res = app
            .clone()
            .oneshot(
                Request::post("/v1/store/erase")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::AUTHORIZATION, "Bearer t")
                    .body(Body::from(json!({ "address": "+94700000000" }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["erased"], 2);

        // The maintenance pass opens the database with the key too.
        assert_eq!(maintain_database(&path, &config).unwrap(), Maintenance { expired: 0, forgotten: 0 });
        assert!(maintain_database(&path, &Config::default()).is_err(), "and refuses it without the key");
        let head = std::fs::read(&path).unwrap();
        assert!(!head.starts_with(b"SQLite format 3"));
    }

    #[test]
    fn an_encrypted_database_without_the_key_stops_the_service() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msgkit.db");
        SqliteStore::open_with(&path, &StoreOptions::default().with_key(key())).unwrap();
        let text = refusal(Config::default().with_db_path(&path));
        assert!(text.contains("MSGKIT_DB_KEY_FILE"), "{text}");
    }
}
