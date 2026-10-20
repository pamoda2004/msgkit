# Public API surface and stability

This page lists what each binding exposes and how far you can rely on it. [API.md](API.md) explains how the operations work; this page says what is covered by compatibility promises and what is not.

## Stability levels

| Level | Meaning |
|---|---|
| **Stable** | Part of the contract. Within one major version (SemVer for the packages, `SCHEMA_VERSION` for the JSON data) it changes only additively: new optional fields, new enum values, new operations, new error codes. Removing or renaming it, changing a type or changing what it means needs a new major version and a CHANGELOG entry marked breaking. |
| **Unstable** | May change or disappear in any release, including a patch release, without a deprecation period. Do not build on it. |

Notes that apply everywhere:

* **Nothing has been published yet** and every package is at `0.2.0`. Until `1.0.0`, "stable" states the intent: a change to a stable item is still listed in the CHANGELOG as breaking and needs the owner's approval, but SemVer does not yet forbid it.
* **Readers must be tolerant.** Ignore unknown JSON fields, and treat an unknown enum value or error `code` as "other". New enum values and error codes are additive and arrive in minor versions.
* **Text meant for people is unstable**: error `message` values and `warnings` strings may be reworded in any release. Branch on the error `code` and the structured fields, never on the wording. (Error and warning text never repeats message content.)
* **The data contract** is [`schema/`](../schema/README.md): the shapes of `Event`, `ImportReport` and the option objects are stable at the level of `SCHEMA_VERSION` (`1.0.0`). Exception: the contents of an event's `raw` field are the provider's own payload and are unstable.
* **Event ids** are stable values: the rules in [CHANGELOG.md](../CHANGELOG.md) (decision D1) and the conformance suite pin them. A change to how ids are derived is a breaking change.

## Operations (all bindings)

Stable. New operations may be added; `version().operations` lists what a build supports.

| Operation | Rust | Python | Node.js | Kotlin |
|---|---|---|---|---|
| `version` | `api::dispatch("version", "{}")` | `version()` | `version()` | `Msgkit.version()` |
| `parse_whatsapp_export` | `whatsapp_export::parse` | `parse_whatsapp_export` | `parseWhatsAppExport` | `Msgkit.parseWhatsAppExport` |
| `parse_facebook_export` | `facebook::parse`, `facebook::parse_value` | `parse_facebook_export` | `parseFacebookExport` | `Msgkit.parseFacebookExport` |
| `normalize_device` | `device::normalize` | `normalize_device` | `normalizeDevice` | `Msgkit.normalizeDevice` |
| `normalize_whatsapp_webhook` | `webhook::normalize_whatsapp` | `normalize_whatsapp_webhook` | `normalizeWhatsAppWebhook` | `Msgkit.normalizeWhatsAppWebhook` (takes the raw body) |
| `normalize_messenger_webhook` | `webhook::normalize_messenger` | `normalize_messenger_webhook` | `normalizeMessengerWebhook` | `Msgkit.normalizeMessengerWebhook` (takes the raw body) |
| `verify_signature` | `webhook::verify_signature` | `verify_signature` | `verifySignature` | `Msgkit.verifySignature` |

The `normalize_*_webhook` operations do not verify signatures; verify first.

**Integrator entry points are stable in every binding:** signature verification (`verify_signature`), the export importers (`parse_whatsapp_export`, `parse_facebook_export`), the device importer (`normalize_device`) and the webhook normalizers (`normalize_*_webhook`) are marked stable in the Rust, Python, Node.js and Kotlin tables below, in the C and WebAssembly ABIs (through `msgkit_call`), in the HTTP service (`/v1/import/*`, `/webhooks/*`) and in the command line.

## Rust (`msgkit-core`)

| Item | Level | Notes |
|---|---|---|
| `model::*` (re-exported at the crate root): `Event`, `Message`, `Call`, `Participant`, `Attachment`, `AttachmentKind`, `Channel`, `Direction`, `CallOutcome`, `ImportReport` | Stable | The data contract. `Event::id`, `timestamp`, `channel` and `kind` are stable. |
| `SCHEMA_VERSION` | Stable | |
| `Error`, `Result`, `Error::code` | Stable | The codes are listed under [Error codes](#error-codes). The variants of `Error` may grow, so match with a wildcard arm. |
| `api::dispatch`, `api::OPERATIONS` | Stable | The JSON entry point used by every other binding. |
| `whatsapp_export::{parse, try_parse, WhatsAppExportOptions, DateOrder}` | Stable | `try_parse` refuses an unknown `time_zone`; `parse` warns and falls back to `utc_offset_minutes`. `WhatsAppExportOptions` is `#[non_exhaustive]`: build it from `Default` with the `with_*` setters or by assigning fields; new options are not breaking changes. |
| `tzdata_version()` | Stable | The bundled IANA time-zone database release, also reported by the `version` operation. |
| `facebook::{parse, parse_value, FacebookExportOptions}` | Stable | `FacebookExportOptions` is `#[non_exhaustive]` (see above). |
| `device::{normalize, DeviceBatch, DeviceOptions, RawSms, RawCall}` | Stable | `DeviceOptions` is `#[non_exhaustive]` (see above). |
| `webhook::{normalize_whatsapp, normalize_messenger, verify_signature}` | Stable | |
| `store::{EventStore, MemoryStore, write_ndjson}` | Stable | New methods added to `EventStore` will have default implementations. |
| `webhook::sign` | Unstable | A helper for tests and tools; it is not an operation. |
| `facebook::fix_encoding` | Unstable | Implementation detail of the Facebook importer. |
| `phone::normalize_phone` | Unstable | Deliberately minimal; the rules may widen. |
| `id::derive_id` | Unstable | The function may change; the ids it produces are stable (see above). |
| `error::describe_json_error` | Unstable | Internal helper that keeps error text free of input. |
| `store::{EraseStore, RetentionStore, EraseOptions, address_key, erase_key, involves}` | Unstable until a release has used them |
| `store::conformance` | Unstable | A test harness for `EventStore` implementers; its checks may grow. |

`cargo-semver-checks` runs in CI against the tagged prototype (`baseline-prototype`) until the first release exists, then against the previous release.

## Rust (`msgkit-store-sqlite`)

| Item | Level |
|---|---|
| `SqliteStore::{open, open_in_memory, prune_acknowledged}` and its `EventStore` implementation | Stable |
| `SqliteStore::{open_with, open_in_memory_with, checkpoint, compact}`, `StoreOptions`, and the `EraseStore` and `RetentionStore` implementations | Unstable until a release has used them |
| `DEFAULT_RETENTION` | Stable |
| Database file layout | Unstable: do not read or write the file yourself. |

## Rust (`msgkit-redact`)

New and optional (see [REDACTION.md](REDACTION.md)). Everything in it is **Unstable** until it has been used by a release: no binding, no HTTP operation and no JSON schema depends on it, and it is outside the `cargo-semver-checks` gate until the baseline moves to the first release.

| Item | Level |
|---|---|
| `Redactor::{new, detect, redact_text, redact_event, redact_event_counted, redact_report}`, `RedactConfig` and its `with_*` setters, `Kind`, `Mask`, `Confidence`, `IdPolicy`, `Finding`, `Redacted`, `Summary`, `ConfigError` | Unstable |
| Placeholder text (`[PHONE]` and the like), the detection rules and the numbering-plan data | Unstable: detectors are refined over time; a text that was not masked may be, and the reverse |

## C ABI (`msgkit-ffi`) and WebAssembly ABI (`msgkit-wasm`)

Stable: the exported function names and signatures documented in [API.md](API.md) (`msgkit_call`, `msgkit_free`, `msgkit_version`; for WebAssembly `msgkit_alloc`, `msgkit_dealloc`, `msgkit_call`, `msgkit_last_len`, `msgkit_version`).

The `internal_error` envelope is produced only at this native boundary: by the library itself when a panic is caught, and by the Python and Node.js wrappers when the library returns no usable result.

## Python (`msgkit-core` on PyPI, import name `msgkit`)

| Item | Level |
|---|---|
| `version`, `parse_whatsapp_export`, `parse_facebook_export`, `normalize_device`, `normalize_whatsapp_webhook`, `normalize_messenger_webhook`, `verify_signature` | Stable |
| `MsgkitError` (`code`, `message`) | Stable |
| `msgkit.types` (the `TypedDict`s `Event`, `Message`, `Call`, `ImportReport`, `DeviceBatch`, `VersionInfo`, and so on) | Stable shapes; the way they are declared (for example the private `_*Required` base classes) is unstable |
| `dispatch` | Stable, low-level: returns the `data` of the response or raises `MsgkitError` |
| `native_version` | Unstable: a diagnostic for the bundled library |
| `__version__` | Stable |
| Names starting with `_` (`_native`, ...) | Unstable |

## Node.js (`@pamoda2004/msgkit`)

| Item | Level |
|---|---|
| `version`, `parseWhatsAppExport`, `parseFacebookExport`, `normalizeDevice`, `normalizeWhatsAppWebhook`, `normalizeMessengerWebhook`, `verifySignature` | Stable |
| `init`, `initSync` | Stable (loading the WebAssembly module) |
| `dispatch` | Stable, low-level |
| `MsgkitError`, `ErrorCode` | Stable; `code` is typed as an open union, so unknown codes are expected |
| Types in `index.d.ts` | Stable shapes |
| Files other than `src/index.js` and `src/index.d.ts` (the loaders and the `.wasm` file name) | Unstable |

## Kotlin and Android (`io.github.pamoda2004:msgkit`, `msgkit-android`)

| Item | Level |
|---|---|
| `Msgkit.version`, `parseWhatsAppExport`, `parseFacebookExport`, `normalizeDevice`, `normalizeWhatsAppWebhook`, `normalizeMessengerWebhook`, `verifySignature` | Stable |
| `Msgkit.dispatchRaw` | Stable, low-level: returns the raw response envelope |
| `MsgkitException` (`code`) | Stable |
| The data classes and enums in `Models.kt`, and `Event.instant` | Stable shapes |
| Everything in the `internal` package | Unstable (not part of the API even though the JVM can see it) |
| The packaged native libraries (`.so` files) and how they are loaded | Unstable |

## HTTP service (`msgkit-service`)

| Item | Level |
|---|---|
| Endpoints, methods, status codes and bodies in [`openapi.yaml`](../openapi.yaml) | Stable |
| Environment variables in [API.md](API.md) | Stable |
| The SQLite file written when `MSGKIT_DB_PATH` is set | Unstable (see above) |
| The Rust library API of the crate (`Config`, `AppState`, `router`, `prune_database`) | Unstable: the crate is an application, not a library |
| Log output | Unstable |

## Command line (`msgkit-cli`)

| Item | Level |
|---|---|
| Subcommands and flags, the NDJSON on stdout, the exit status | Stable |
| Text on stderr (warnings, errors) | Unstable |

## Error codes

Every binding reports the same `code` for the same situation, and the HTTP service adds a `code` field to its error bodies. The set is stable and grows only by addition; callers must tolerate codes they do not know. The full list, with the meaning of each code, where it is returned and where it is tested, is in [ERRORS.md](ERRORS.md).
