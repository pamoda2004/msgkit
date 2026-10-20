# Encryption at rest

The SQLite store (`MSGKIT_DB_PATH`) can encrypt the whole database file with [SQLCipher](https://www.zetetic.net/sqlcipher/): AES-256 on every page, so the tables, indexes, event ids, timestamps, the write-ahead log and the free pages are all unreadable without the key. It is **optional and off by default**; a default build is plain SQLite and needs no OpenSSL.

> **Losing the key means losing the data.** There is no recovery, no backdoor and no key escrow. **Back the key up separately from the database** (a password manager, a hardware token, a sealed copy in another place): anyone who holds both can read everything, and a backup of the database without the key is worthless. Test that you can read the backup before you rely on it.

## What it covers and what it does not

| Covered | Not covered |
|---|---|
| The database file, its `-wal` file and free pages (events, ids, times, channels) | The running process's memory, and messages in transit (use TLS in front of the service) |
| A stolen or copied database file, a disk image, a backup of the file | Anyone who can read the key (the service's environment or key file) |
| A database that ends up in a sync client or an unencrypted backup (the content stays encrypted) | Backups of the *plain* database made before you encrypted it |
| | Metadata the file system keeps (the file's name, size and modification time) |

Disk or volume encryption (BitLocker, LUKS, FileVault, an encrypted Docker volume) is a good **extra layer** and protects the things above that SQLCipher does not (swap, logs, temporary files). Use both.

**Keep the database out of cloud-synced folders** (OneDrive, Google Drive, Dropbox, iCloud Drive and similar), even when it is encrypted: the sync copies the file and its write-ahead log and every earlier version to other machines and to the provider, where msgkit cannot erase them, and a half-synced log can corrupt the database. The service, and the `msgkit store` commands, **warn** when the path has a folder with one of these names (a name check: it cannot recognise a synced folder with an unusual name).

## Building with encryption

| Feature | What it does | Needs |
|---|---|---|
| `sqlcipher` | Bundles SQLCipher and **links the system OpenSSL** | OpenSSL development files (Linux: `libssl-dev` and `pkg-config`) |
| `sqlcipher-vendored` | Bundles SQLCipher and **builds OpenSSL from source** | Perl and a C compiler (more build time, a larger binary) |

Both features exist on `msgkit-store-sqlite`, `msgkit-service` and `msgkit-cli`:

```bash
cargo build --release -p msgkit-service -p msgkit-cli --features msgkit-service/sqlcipher,msgkit-cli/sqlcipher
```

**Where it is tested.** CI builds and tests the `sqlcipher` feature on Linux (`ubuntu-latest`, system OpenSSL) in the job *SQLCipher (build, test, size)* and prints the build time and binary sizes in the job summary. **Windows and macOS builds are not tested by CI.** `sqlcipher-vendored` is the way to try them; it failed to configure OpenSSL on the Windows development machine used for this project (its Perl could not run OpenSSL's `Configure`), so treat Windows as unverified. The Android, Python, Node.js (WebAssembly) and Kotlin packages do not contain the store, so they are not affected. The official Docker image will enable `sqlcipher` when it is built (Phase 5).

## The key

* **Format:** 64 hexadecimal characters (256 bits), given to SQLCipher as a raw key. Generate one with `openssl rand -hex 32` or `python -c "import secrets; print(secrets.token_hex(32))"`.
* **Service:** `MSGKIT_DB_KEY_FILE` is the path of a file that holds the key (for Docker or Kubernetes, a secret mounted at `/run/secrets/...`). `MSGKIT_DB_KEY` holds the key itself and is **less safe**: environment variables show up in process listings, crash reports and child processes. Set only one of them; both need `MSGKIT_DB_PATH`.
* **Command line:** `--key-file <file>` or `--key-stdin`, never an argument (an argument is visible in the process list and the shell history). Without either, `MSGKIT_DB_KEY_FILE` names the file.
* The key is never logged or printed, `Debug` output shows `<redacted>`, and an error names the setting, not its value.
* Rotating the key and keeping it in the operating system's secret store (Windows Credential Manager, macOS Keychain, Linux Secret Service) are backlog items.

## Fail closed

msgkit never silently uses a plain database where you expect an encrypted one:

| Situation | Result |
|---|---|
| A key is set, but the build has no SQLCipher | The service refuses to start (`no SQLCipher support`); no database file is created |
| A key is set and the database file is plain | Refused: `msgkit store encrypt` it first. The key is not used on a plain database |
| The file is encrypted (it does not start like SQLite) and there is no key | Refused, naming `MSGKIT_DB_KEY_FILE` |
| The key is wrong, or the file is damaged | Refused: "cannot be read with the configured key" |
| No key, a plain file | Opens as before (a default deployment is unchanged) |
| A key and a new path | A new encrypted database is created |

## Encrypting an existing database

Nothing is converted automatically when a database is opened. Do it once, deliberately:

1. **Stop the service.**
2. **Back up the plain database** (the file and any `-wal` file) to a safe place. This is your way back if anything goes wrong.
3. Create a key and **store a copy of it separately** from the database.
4. Run (with a build that has the feature):
   ```bash
   msgkit store encrypt /var/lib/msgkit/events.db --output /var/lib/msgkit/events.enc.db --key-file ./db.key
   ```
   It writes a **new** file, opens it with the key and checks that the layout and every row match, checks that it does not read as plain SQLite, and only then **overwrites the plain file with zeros and deletes it** (and its `-wal` and `-shm`). Add `--keep-plain` to keep it and delete it yourself later. If anything fails, the plain file is untouched and the half-written output is removed.
5. Point `MSGKIT_DB_PATH` at the new file, set `MSGKIT_DB_KEY_FILE`, and start the service.
6. Delete the plain **backup** from step 2 when you no longer need it. Overwriting does not reach copies made by a snapshot, a backup, a sync client or an SSD that has remapped its blocks, so the plain backup is the part to destroy carefully (and an encrypted volume makes this easier).

## Licences

SQLCipher Community Edition is under a BSD-3-Clause licence (Zetetic LLC); its notice is kept in [`third_party/sqlcipher/LICENSE`](../third_party/sqlcipher/LICENSE) and must accompany a binary that includes it. OpenSSL 3 is under the Apache License 2.0. `cargo deny check` covers the Rust crates (`openssl-sys`, `openssl-src`, `libsqlite3-sys`), which pass the allowed licences in `deny.toml`; it cannot see the C sources inside them, which is why the SQLCipher notice is kept here.
