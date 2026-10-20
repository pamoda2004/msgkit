//! The database key and a check for cloud-synced folders.

use std::fmt;
use std::path::{Component, Path};

use msgkit_core::{Error, Result};

/// A 256-bit database key for SQLCipher, written as 64 hexadecimal characters.
///
/// Generate one with `openssl rand -hex 32` (or `python -c "import secrets; print(secrets.token_hex(32))"`) and keep
/// it **separately from the database**: whoever has both can read everything, and **without the key the data cannot
/// be recovered**. It is passed to SQLCipher as a raw key, so there is no passphrase stretching to wait for and no
/// weak passphrase to choose.
///
/// `Debug` never shows it, and no error message in this crate repeats it.
#[derive(Clone)]
pub struct DbKey {
    hex: String,
}

impl DbKey {
    /// Reads a key from 64 hexadecimal characters. Surrounding whitespace (such as the newline at the end of a key
    /// file) is ignored.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] when the text is not exactly 64 hexadecimal characters. The error does not repeat it.
    ///
    /// ```
    /// use msgkit_store_sqlite::DbKey;
    ///
    /// assert!(DbKey::from_hex(&"ab".repeat(32)).is_ok());
    /// assert!(DbKey::from_hex("too short").is_err());
    /// assert_eq!(format!("{:?}", DbKey::from_hex(&"ab".repeat(32)).unwrap()), "DbKey(<redacted>)");
    /// ```
    pub fn from_hex(text: &str) -> Result<Self> {
        let hex = text.trim();
        if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(DbKey { hex: hex.to_ascii_lowercase() })
        } else {
            Err(Error::InvalidInput(
                "the database key must be 64 hexadecimal characters (256 bits)".to_owned(),
            ))
        }
    }

    /// The `PRAGMA key` statement. Only hexadecimal digits go in, so nothing can be injected.
    pub(crate) fn pragma(&self) -> String {
        format!("PRAGMA key = \"x'{}'\";", self.hex)
    }

    /// The value for `ATTACH ... KEY`.
    pub(crate) fn attach_key(&self) -> String {
        format!("x'{}'", self.hex)
    }
}

impl fmt::Debug for DbKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DbKey(<redacted>)")
    }
}

/// Which cloud-sync client's folder `path` is inside, by the names of its directories, or `None`.
///
/// A production database must not live in such a folder: the sync copies the file, its write-ahead log and every
/// earlier version to other machines and to the provider, where msgkit cannot erase them, and a half-synced log can
/// corrupt the database. This is a name check, not a guarantee: a synced folder with an unusual name is not
/// recognised, and a folder that merely has such a name is. It is meant for a warning, never for a refusal.
///
/// ```
/// use msgkit_store_sqlite::cloud_synced_folder;
/// use std::path::Path;
///
/// assert_eq!(cloud_synced_folder(Path::new("/home/user/Dropbox/msgkit/events.db")), Some("Dropbox"));
/// assert_eq!(cloud_synced_folder(Path::new("/var/lib/msgkit/events.db")), None);
/// ```
#[must_use]
pub fn cloud_synced_folder(path: &Path) -> Option<&'static str> {
    // The first element is matched as a whole name or a prefix followed by a space or a dash, so that
    // `OneDrive - Contoso` is found but an unrelated `Dropboxes` is not.
    const NAMES: &[(&str, &str)] = &[
        ("onedrive", "OneDrive"),
        ("dropbox", "Dropbox"),
        ("google drive", "Google Drive"),
        ("googledrive", "Google Drive"),
        ("my drive", "Google Drive"),
        ("icloud drive", "iCloud Drive"),
        ("iclouddrive", "iCloud Drive"),
        ("com~apple~clouddocs", "iCloud Drive"),
        ("mobile documents", "iCloud Drive"),
        ("pcloud", "pCloud"),
        ("pcloud drive", "pCloud"),
        ("megasync", "MEGA"),
        ("nextcloud", "Nextcloud"),
        ("owncloud", "ownCloud"),
        ("box sync", "Box"),
    ];
    let canonical = path.canonicalize().ok();
    let by_name = |p: &Path| {
        p.components().find_map(|component| {
            let Component::Normal(name) = component else { return None };
            let name = name.to_string_lossy().to_lowercase();
            NAMES.iter().find_map(|(prefix, label)| {
                let hit = name == *prefix
                    || name.strip_prefix(prefix).is_some_and(|rest| rest.starts_with([' ', '-', '(']));
                hit.then_some(*label)
            })
        })
    };
    by_name(path).or_else(|| canonical.as_deref().and_then(by_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_64_hex_characters_and_never_shown() {
        let key = "0123456789abcdef".repeat(4);
        assert!(DbKey::from_hex(&key).is_ok());
        assert!(
            DbKey::from_hex(&format!("  {}\r\n", key.to_uppercase())).is_ok(),
            "case and whitespace are ignored"
        );
        for bad in ["", "abc", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65), &format!("{key} {key}")] {
            let e = DbKey::from_hex(bad).unwrap_err().to_string();
            assert_eq!(e, "invalid input: the database key must be 64 hexadecimal characters (256 bits)");
        }
        let shown = format!("{:?}", DbKey::from_hex(&key).unwrap());
        assert!(!shown.contains("0123"), "{shown}");
    }

    #[test]
    fn synced_folders_are_recognised_by_name() {
        let cases = [
            ("C:/Users/user/OneDrive/Documents/msgkit.db", Some("OneDrive")),
            ("C:/Users/user/OneDrive - Contoso/msgkit.db", Some("OneDrive")),
            ("/Users/user/Dropbox/x/msgkit.db", Some("Dropbox")),
            ("/Users/user/Google Drive/msgkit.db", Some("Google Drive")),
            ("/Users/user/Library/Mobile Documents/com~apple~CloudDocs/msgkit.db", Some("iCloud Drive")),
            ("/home/user/Nextcloud/msgkit.db", Some("Nextcloud")),
            ("/var/lib/msgkit/events.db", None),
            ("/srv/dropboxes/events.db", None),
            ("relative/events.db", None),
        ];
        for (path, want) in cases {
            assert_eq!(cloud_synced_folder(Path::new(path)), want, "{path}");
        }
    }
}
