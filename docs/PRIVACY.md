# Privacy and responsible use

Messages and call logs are some of the most sensitive personal data there is. msgkit is a parsing toolkit, but how you use it carries legal and ethical obligations.

## What msgkit does and does not do

* It processes data **in memory** and returns it to you. It does not persist, upload or log message content.
* The core makes **no network calls**. The HTTP sidecar only receives requests you point at it.
* Webhook events buffered by the sidecar live in process memory only and are lost on restart, **unless** you set `MSGKIT_DB_PATH`. Then they are written to a local SQLite file (plain JSON, **not encrypted**) until your application acknowledges them; acknowledging erases the content (overwritten, not just unlinked: SQLite `secure_delete` is on; the service empties the write-ahead log every six hours, so acknowledged text can remain in that log for up to about six hours) and keeps only the event id for 30 days (`MSGKIT_ID_RETENTION_DAYS`) to reject redelivered webhooks; that id is derived from sender and text, so it is pseudonymous personal data. To delete one person, use `msgkit store erase <db> --address-stdin --default-country-code <code>` (or `EraseStore::erase_person`), and set `MSGKIT_PENDING_MAX_AGE_DAYS` so events nobody collects do not stay forever. **Do not put the database in a cloud-synced folder** (OneDrive, Google Drive, Dropbox): the sync copies the file, its write-ahead log and old versions where msgkit cannot erase them. For encryption of the file itself see [ENCRYPTION.md](ENCRYPTION.md) (optional SQLCipher; losing the key loses the data, so keep the key separately from the database). Protect that file like any other store of personal messages: encrypted disk or volume, restricted permissions, backups under the same rules.

## Your responsibilities as an integrator

1. **Consent and lawful basis.** Only import data for accounts you own or where each affected person has consented or another lawful basis applies (GDPR, CCPA, local telecom and wiretap laws). Messages involve third parties who never agreed to anything.
2. **Minimize.** Import only what the feature needs; drop `raw` payloads you do not need.
3. **Protect at rest.** Encrypt the store that holds events; restrict access; keep backups under the same rules.
4. **Retention and deletion.** Define how long data is kept and support deletion on request. Deterministic ids and participant ids make "delete everything for contact X" a simple query.
5. **Secrets.** Keep app secrets and tokens out of source control; rotate them. The sidecar reads them from environment variables.
6. **Transport.** Serve webhooks over HTTPS only; if the sidecar is reachable beyond localhost, set `MSGKIT_API_TOKEN` and put it behind TLS.
7. **Platform terms.** Use only the official APIs and user-initiated exports described in [SUPPORTED_SOURCES.md](SUPPORTED_SOURCES.md).

## Redaction (optional, best effort)

The optional `msgkit-redact` crate masks phone numbers, Sri Lankan NIC numbers, card and bank account numbers, IBANs and email addresses in text and in events, for example before you log, share or test with imported data. It is **not anonymisation**: names, places and free text are not detected, and every detector has false negatives. What it finds, what it leaves alone and how to configure it: [REDACTION.md](REDACTION.md).

## Do not use msgkit to

* read someone else's messages without their knowledge or consent (stalkerware),
* circumvent platform protections or terms of service.

Contributions that add such capabilities will be declined.
