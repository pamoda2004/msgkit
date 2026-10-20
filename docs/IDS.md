# How event ids are derived

Every event has an `id`. Stores use it to de-duplicate, so importing the same chat twice must produce the same ids, and two different messages must not share one. This page says exactly which inputs go into an id, what changes it, what does not, and what msgkit promises.

The code is in `crates/msgkit-core/src/id.rs` (the hash and the occurrence counter) and in each importer. `crates/msgkit-core/tests/` and the `ids_*` conformance cases pin the behaviour described here.

## The function

An id is the first 16 bytes (32 lowercase hex characters) of `SHA-256` over:

```
"msgkit:v1"  0x1f  part1  0x1f  part2  0x1f  ...
```

The unit separator `0x1f` keeps `("ab", "c")` distinct from `("a", "bc")`. The id contains no readable text, but it is derived from message content, so treat it as personal data of the same kind as the message (it can be confirmed by anyone who can guess the content).

## The inputs, per source

All text inputs are used exactly as the importer reads them (after the encoding repair described below), never the normalized form.

| Source | Parts, in order |
|---|---|
| WhatsApp export, message | `whatsapp`, `msg`, local time, sender name as written, body, occurrence index |
| WhatsApp export, missed-call notice | `whatsapp`, `call`, local time, `true`/`false` (video), occurrence index |
| Messenger export, message | `messenger`, `msg`, `timestamp_ms`, sender name, body, occurrence index |
| Messenger export, call | `messenger`, `call`, `timestamp_ms`, sender name, occurrence index |
| Android SMS | `sms`, `msg`, `date` in ms, `type` number, address (trimmed), body, occurrence index |
| Android call log | `cellular`, `call`, `date` in ms, `type` number, number (trimmed), occurrence index |
| WhatsApp webhook | `whatsapp`, `msg`, the platform's message id |
| Messenger webhook | `messenger`, `msg`, the platform's message id (`mid`) |

Details that matter:

* **WhatsApp local time** is the date and time of the line exactly as the file wrote it, with no offset: `2025-03-14T09:05:00` for a year-first date, `14.03.2025T09:05:00` for a day-first or month-first date (the two leading numbers keep the order the file used). A two-digit year gets 2000 added. The body is the text after `Name: `, with trailing whitespace and the `<This message was edited>` marker removed. For media lines the body is the placeholder text (for example `<Media omitted>`), so media messages get ids too.
* **Messenger** text goes through the same mojibake repair as the parsed event (the export stores UTF-8 as Latin-1 escapes), so the id is computed from the repaired sender and body. A message with no text uses its shared link, or the empty string. Messages the export marks as unsent are skipped and take no occurrence index.
* **SMS and call log:** the device row id (`_id`) is *not* an input, because it is not stable across devices or after a restore. It is kept only as `external_id`.
* **Webhooks** depend on the platform id alone. Timestamps, text and sender do not matter, so redelivery of the same message gives the same id even if the payload around it differs.

## The occurrence index

WhatsApp exports have minute precision, so two identical lines from the same person in the same minute (say two "ok") would hash to the same id. To keep them apart the importer counts, per import, how many times it has already seen the same key (the parts above, without the index). The first occurrence gets `0`, the next `1`, and so on, in the order they appear in the input. For the Messenger export the file is newest-first, so it is walked oldest-first to make the order natural.

The index is counted **within one call**, over the input it was given. It does not look at earlier imports or at the store.

## What does not change an id

* `utc_offset_minutes`, `time_zone` and `date_order` (WhatsApp). The id uses the local wall-clock text, so a different zone, a different way of resolving day versus month, or a clock change cannot alter it. (`date_order` can still change `timestamp`; it does not change the id for the same file because the leading numbers are kept in file order.)
* `default_country_code` and `own_number`. Phone numbers are normalized into `sender`/`recipients`, but the id uses the address as written.
* `self_name`, `conversation_id` and therefore `direction`.
* Everything the event carries beyond the listed inputs: attachments, `raw`, participants, warnings.
* The binding, the platform or the msgkit version, as long as the version number of the id scheme (`msgkit:v1`) is the same.
* Re-importing the same file, or importing an overlapping export in which a line appears with the same neighbours, as far as the occurrence index allows (next section).

## What does change an id

* Any change to a listed input: the minute or second a line was written at, the sender name as written (a contact renamed in a later export is a different sender), the body (an edited message, a different caption), the SMS type or address, the call type or number.
* The occurrence index: removing or adding an *identical* earlier line in the same minute shifts the index of the later ones.
* A different source. The same real conversation exported from WhatsApp and received as a webhook has different ids by design; msgkit does not try to match them.
* A change of scheme version (`msgkit:v1`). That is a breaking change and is described below.

## Known limits

* **Overlapping exports and identical messages.** Index `0` is "the first identical line this file contains". If export A contains two identical "ok" lines in one minute and export B (a later, shorter export) starts between them, B's single "ok" gets index `0` and collides with A's first one, and A's second one is not matched. Messages that are unique in their minute, which is almost all of them, are unaffected.
* **Edits and renames** produce new ids. msgkit does not link an edited message to its original (tracking edits is planned work).
* **Time-zone shifts of the phone** between two exports do not matter for the id, but a WhatsApp export written in a different language or date format yields different local-time text for the same message, and therefore a different id.

## Stability guarantee

* Within a release series that carries the same `msgkit:v1` prefix, the same input and the same options-that-matter (none of those listed under "does not change" count) produce the same id, in every binding and on every platform. The conformance suite checks this byte for byte.
* A change to any of the inputs above, to their order, to the separator or to the hash is a **breaking change**: it needs a new prefix (`msgkit:v2`), a major-version bump, an entry under "Breaking" in `CHANGELOG.md`, and migration notes, because stored ids would stop matching.
* Fixing a bug in an importer so that a *field value* (not an id input) becomes more correct does not change ids. If a fix does change an id input (for example how a body is trimmed), it is treated as breaking, whatever the size of the change.
* Ids are derived deterministically but are not secret and not reversible by design; do not rely on them for authentication or anonymization.

Until the first release the ids are still allowed to change (they did once, for decision D1: local time plus an occurrence index, recorded in the CHANGELOG); after the first release this page is the contract.
