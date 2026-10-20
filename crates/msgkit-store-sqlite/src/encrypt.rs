//! One-time conversion of a plain database into an encrypted one (`msgkit store encrypt`).

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use msgkit_core::{Error, Result};
use rusqlite::types::Value;
use rusqlite::{params, Connection};

use crate::{has_plain_header, restrict_permissions, storage, DbKey, SqliteStore, StoreOptions};

/// What [`encrypt_database`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncryptReport {
    /// Rows copied (held events and remembered ids).
    pub rows: u64,
    /// Whether the plain file (and its `-wal` and `-shm`) was overwritten with zeros and deleted.
    pub plain_deleted: bool,
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Overwrites a file with zeros and deletes it. Best effort: it cannot reach copies the file system, a snapshot, a
/// backup or a sync client made, and an SSD may keep the old blocks until it reuses them.
fn shred(path: &Path) -> Result<()> {
    let Ok(meta) = fs::metadata(path) else { return Ok(()) };
    let io = |e: std::io::Error| storage(format!("cannot erase {}: {e}", path.display()));
    let mut file = OpenOptions::new().write(true).open(path).map_err(io)?;
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    let zeros = vec![0_u8; 64 * 1024];
    let mut left = meta.len();
    while left > 0 {
        let n = usize::try_from(left.min(zeros.len() as u64)).unwrap_or(zeros.len());
        file.write_all(&zeros[..n]).map_err(io)?;
        left -= n as u64;
    }
    file.sync_all().map_err(io)?;
    drop(file);
    fs::remove_file(path).map_err(io)
}

/// Copies the plain database at `plain` into a new encrypted database at `output`, checks that the copy opens with
/// `key` and holds the same rows, and (unless `keep_plain`) overwrites and deletes the plain file.
///
/// The plain file is only touched after the copy has been verified, and is never changed before that. Stop the
/// service first and **make a backup** of the plain file somewhere safe before you run this; and keep the key
/// separately from the database, because without it the data cannot be recovered.
///
/// # Errors
/// [`Error::InvalidInput`] when this build has no SQLCipher support, `plain` is missing or already encrypted, or
/// `output` already exists; a storage error when the copy cannot be written or does not match. On any error the plain
/// file is left as it was and a half-written `output` is removed.
pub fn encrypt_database(plain: &Path, output: &Path, key: &DbKey, keep_plain: bool) -> Result<EncryptReport> {
    if !plain.is_file() {
        return Err(Error::InvalidInput("the database to encrypt does not exist".to_owned()));
    }
    if !has_plain_header(plain) {
        return Err(Error::InvalidInput(
            "the database is not a plain SQLite file (it may already be encrypted)".to_owned(),
        ));
    }
    if output.exists() {
        return Err(Error::InvalidInput("the output file already exists".to_owned()));
    }
    let result = copy_and_verify(plain, output, key);
    let rows = match result {
        Ok(rows) => rows,
        Err(e) => {
            for part in [output.to_path_buf(), sibling(output, "-wal"), sibling(output, "-shm")] {
                let _ = fs::remove_file(part);
            }
            return Err(e);
        }
    };
    if !keep_plain {
        for part in [plain.to_path_buf(), sibling(plain, "-wal"), sibling(plain, "-shm")] {
            shred(&part)?;
        }
    }
    Ok(EncryptReport { rows, plain_deleted: !keep_plain })
}

fn copy_and_verify(plain: &Path, output: &Path, key: &DbKey) -> Result<u64> {
    let conn = Connection::open(plain).map_err(|e| storage(format!("cannot open the database: {e}")))?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
    crate::require_cipher(&conn)?;
    // Fold the write-ahead log into the file, so the copy sees every committed row.
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).map_err(storage)?;
    let version = crate::user_version(&conn)?;
    let out =
        output.to_str().ok_or_else(|| Error::InvalidInput("the output path is not valid text".to_owned()))?;
    conn.execute("ATTACH DATABASE ?1 AS enc KEY ?2", params![out, key.attach_key()]).map_err(storage)?;
    restrict_permissions(output);
    conn.query_row("SELECT sqlcipher_export('enc')", [], |_| Ok(())).map_err(storage)?;
    conn.execute_batch(&format!("PRAGMA enc.user_version = {version};")).map_err(storage)?;
    conn.execute_batch("DETACH DATABASE enc").map_err(storage)?;

    // The copy must open with the key, be the layout we copied, and hold exactly the same rows.
    let store = SqliteStore::open_with(output, &StoreOptions::default().with_key(key.clone()))?;
    if crate::user_version(&store.conn)? != version {
        return Err(storage("the encrypted copy has a different layout version"));
    }
    let rows = same_rows(&conn, &store.conn)?;
    drop(store);
    // And it must not read as a plain database.
    if has_plain_header(output) {
        return Err(storage("the copy is not encrypted"));
    }
    Ok(rows)
}

const ROWS_SQL: &str =
    "SELECT id, timestamp_ms, channel, kind, json, acked_at, received_at FROM events ORDER BY seq";

/// Walks both databases in step and fails at the first difference. Returns the number of rows.
fn same_rows(a: &Connection, b: &Connection) -> Result<u64> {
    let mismatch = || storage("the encrypted copy does not hold the same rows as the original");
    let (mut stmt_a, mut stmt_b) =
        (a.prepare(ROWS_SQL).map_err(storage)?, b.prepare(ROWS_SQL).map_err(storage)?);
    let (mut rows_a, mut rows_b) = (stmt_a.query([]).map_err(storage)?, stmt_b.query([]).map_err(storage)?);
    let mut count = 0_u64;
    loop {
        match (rows_a.next().map_err(storage)?, rows_b.next().map_err(storage)?) {
            (None, None) => return Ok(count),
            (Some(x), Some(y)) => {
                for column in 0..7 {
                    let (vx, vy): (Value, Value) =
                        (x.get(column).map_err(storage)?, y.get(column).map_err(storage)?);
                    if vx != vy {
                        return Err(mismatch());
                    }
                }
                count += 1;
            }
            _ => return Err(mismatch()),
        }
    }
}
