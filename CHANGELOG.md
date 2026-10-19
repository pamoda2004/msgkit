# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/) and the project uses [Semantic Versioning](https://semver.org/). The JSON data contract is versioned separately (`SCHEMA_VERSION`).

## [Unreleased]

### Changed (breaking, before any release)
- **Kotlin/Android package renamed from `dev.msgkit` to `io.github.pamoda2004.msgkit`** (and `dev.msgkit.android` to `io.github.pamoda2004.msgkit.android`), so the source package matches the Maven group `io.github.pamoda2004`. Update imports; ProGuard/R8 users get the new keep rule from the AAR automatically. Nothing had been published, so no released version is affected.
- **Event ids for WhatsApp exports and device (SMS/call-log) records are derived differently** (decision D1). They no longer depend on `utc_offset_minutes`, `date_order` or `default_country_code`: WhatsApp ids use the local wall-clock time exactly as written in the export, the sender as written and the body, plus an occurrence index so two identical messages in the same minute (for example two "ok") keep separate ids; device ids use the address or number as written. Re-importing the same chat with different settings, or from overlapping exports, now yields the same ids. Every id produced by earlier builds changes once; nothing had been published and no stored data exists, so no migration is provided. Facebook and webhook ids are unchanged.

### Added (API and schema stability, Phase 3)
- **Weekly mutation-testing workflow** (`.github/workflows/mutation.yml`, also runnable on demand): `cargo mutants` on `msgkit-core` with a pinned `cargo-mutants`, about 35 minutes. It is not a required check; it fails when a mutant survives and uploads the results as an artifact. The last full run caught all 216 viable mutants.
- **Public API surface and stability levels** (`docs/API_SURFACE.md`): lists what the Rust crates, the C and WebAssembly ABIs, and the Python, Node.js, Kotlin, HTTP and command-line interfaces expose, marks each item stable or unstable, and states the compatibility rules for each level. Documentation only; no API changed. `docs/API.md` now lists `storage_error` among the error codes (it was missing).
- **Schema policy** (`docs/SCHEMA_POLICY.md`): the schema has its own version (`SCHEMA_VERSION`); within 1.x it changes only additively, removals need a new major version (after a deprecation period of at least one minor release), and readers must accept older minor versions and tolerate newer ones. Every schema file now carries `x-msgkit-schema-version` (a metadata key that readers ignore; the data contract is unchanged and `SCHEMA_VERSION` stays `1.0.0`). `scripts/schema_compat.py` now also fails CI when an additive schema change lacks a minor `SCHEMA_VERSION` bump, when the version goes backwards, or when a file's marker disagrees with `SCHEMA_VERSION`.
- **Data types for TypeScript, Python and Kotlin are now generated from `schema/`** (`scripts/generate_types.py`, standard library only) instead of being written by hand. The generated code sits between `<generated types ...>` markers in `packages/node/src/index.d.ts`, `packages/python/src/msgkit/types.py` and `Models.kt`; a new required CI job, "Generated types match the schema", fails when they disagree with the schema. Names, field order, optionality and defaults are unchanged, and runtime behaviour is unchanged. The differences, all additive or widening:
  - TypeScript: new exported type `DateOrder` (`'auto' | 'dmy' | 'mdy'`); `WhatsAppExportOptions.date_order` now uses it (structurally identical to the inline union before).
  - Python: new `TypedDict`s `WhatsAppExportOptions`, `FacebookExportOptions` and `DeviceOptions` in `msgkit.types`.
  - Python: four input fields widened from `str` to `Optional[str]` to align the types with the schema, which allows `null` there: `RawSms.id`, `RawSms.thread_id`, `RawCall.id` and `RawCall.name`. Callers that pass strings are unaffected; only code that reads one of these fields and assumes `str` needs a `None` check under mypy.
  - Kotlin: none (`VersionInfo` moved to the end of the file).
  - Doc comments now come from the Rust doc comments; five hand-written comments were moved into the Rust docs, and the example phone numbers in them are now obviously fictional (`+94700000000`, `0700000000`).

### Changed (tests and fixtures only, Phase 3)
- **Example phone numbers are now obviously fictional** (audit finding F22). The Sri Lankan mobile number that tests, fixtures, conformance cases, the fuzz corpus and docs used (in its international, national, bare and spaced or hyphenated forms) is replaced everywhere by `+94700000000` / `0700000000`. Production code is unchanged. Because event ids are derived from the sender text, the expected ids of 10 events in 3 conformance cases changed (`device_batch`, `ids_phone_senders_country_94`, `ids_phone_senders_no_country_code`); a fourth case (`whatsapp_webhook`) changed only in text. Nothing else in any expected output differs.

### Changed (HTTP service, Phase 3)
- **A 413 (request body too large) on the text import and webhook POST endpoints is now a JSON error.** The status is unchanged. Before, the body was axum's plain text (`Content-Type: text/plain; charset=utf-8`):
  ```
  Failed to buffer the request body: length limit exceeded
  ```
  Now it is the same JSON as every other error (`Content-Type: application/json`):
  ```json
  {"error": "the request body is too large", "code": "limit_exceeded"}
  ```
  Affected: `POST /v1/import/whatsapp-export` (over 64 MiB) and `POST /webhooks/whatsapp` and `/webhooks/messenger` (over 2 MiB). The JSON endpoints already answered 413 with JSON and keep their text (`the request body could not be read (too large?)`), now with `"code": "limit_exceeded"` added. Clients that read the 413 body as text and matched the old sentence need to read `code` instead.

### Added (IANA time zones, Phase 3, decision D7)
- **Optional `time_zone` for WhatsApp exports** (for example `Asia/Colombo`), next to `utc_offset_minutes`: an option of `parse_whatsapp_export` in every binding (Python keyword `time_zone`, Node and Kotlin through the generated option types), a query parameter of `POST /v1/import/whatsapp-export` and the `--time-zone` flag of the command line. It follows the zone's daylight-saving rules and history. **Additive:** without it nothing changes (all 54 earlier conformance cases are byte-identical), `time_zone` wins when both are given, and **event ids never depend on it**. Names are exact and case-sensitive IANA names of the bundled database (aliases such as `Asia/Calcutta` work; `asia/colombo`, `+05:30`, `IST` and the empty string do not); anything else is `invalid_input` and the name is not repeated. A time in a repeated period (clocks back) uses its first occurrence, a time in a skipped period (clocks forward) moves forward by the gap, and one warning counts both and quotes nothing.
- **The time-zone database is bundled in the binary** (`jiff =0.2.38` and `jiff-tzdb =0.1.9`, default features off, `tzdb-bundle-always` on; IANA tzdata 2026e), so a name gives the same UTC instant in every binding on every platform. `msgkit-core` still reads no files, no environment and no clock: `crates/msgkit-core/tests/purity.rs` fails if source code touches them or if any OS time-zone feature of `jiff` is enabled, and a decoy `TZDIR` shows that OS files are ignored.
- **Tzdata update policy** (`CONTRIBUTING.md`, "Time-zone data"): a `jiff` or `jiff-tzdb` bump is reviewed as a behaviour change; its CHANGELOG entry names the old and new tzdata versions and the zones whose rules changed (found with `cargo run --release -p msgkit-core --example tz_fingerprint`); Dependabot opens these bumps as separate pull requests; `crates/msgkit-core/tests/tzdata_pin.rs` fails until the reviewed version is recorded.
- **`version` now also reports `tzdata_version`** (the bundled tzdata release; new field in `VersionInfo` of the Node, Python and Kotlin types). New Rust items: `whatsapp_export::try_parse` (like `parse`, but an unknown zone is `Error::InvalidInput`; `parse` warns and keeps `utc_offset_minutes`) and `msgkit_core::tzdata_version()`.
- **`SCHEMA_VERSION` is now 1.1.0** (a new optional property, `WhatsAppExportOptions.time_zone`; additive). The generated TypeScript, Python and Kotlin types and `openapi.yaml` follow.
- 12 conformance cases (`conformance/cases/tz_*`: Asia/Colombo including its 1996 and 2006 changes, New York and London gaps and folds, Lord Howe's 30-minute change, an alias, the zone replacing the offset, and four rejected names), in all five runners; the fuzz target and corpus replay now vary the zone; property tests for ids and for never panicking or repeating a zone name.
- CI prints package sizes (Python wheel per platform, npm package and WASM, Android libraries and AAR) in the job logs and summaries, so dependency changes can be measured.

### Changed (Rust API, Phase 3)
- **`WhatsAppExportOptions` gained a public field, `time_zone`.** Rust code that builds the struct field by field without `..Default::default()` must add it. JSON, the other bindings and code that uses `..Default::default()` are unaffected.

### Added (error taxonomy, Phase 3)
- **One list of stable error codes** (`docs/ERRORS.md`, `msgkit_core::error::codes`) shared by the Rust core, the Python, Node.js and Kotlin packages, the C and WebAssembly ABIs and the HTTP service. **Every existing code keeps its meaning and is returned for exactly the same inputs as before** (`invalid_input`, `parse_error`, `unknown_op`, `storage_error`, `internal_error`): all 54 conformance cases pass unchanged in the Rust, Python, Node.js, Kotlin and HTTP runners, and no expected file was edited.
- **HTTP error responses now carry a `code`** next to the unchanged `error` text and unchanged HTTP status: `{"error": "...", "code": "..."}`. Five codes are new and are returned only by the HTTP service, for situations that previously had only a status and a text: `limit_exceeded` (413), `unauthorized` (401, bearer token), `signature_invalid` (401, webhook signature), `verification_failed` (403, webhook handshake) and `not_configured` (503). The library operations return no new code; `verify_signature` still answers `{"valid": false}`.
- `openapi.yaml`: the `Error` schema gains `code`; POST operations document the 413 response.
- The HTTP conformance runner now also sends the failing import cases through the service and requires the same `code` as the library; `crates/msgkit-service/tests/error_codes.rs` pins the status, code and text of every HTTP error response; `crates/msgkit-core/tests/error_taxonomy.rs` keeps the code, the documentation and `openapi.yaml` in step and checks that every code's test or case exists.

### Added (testing depth, Phase 2)
- **Conformance suite** (`conformance/`): 54 language-independent cases (input files plus the expected canonical JSON response) that the Rust core, the Python wheel, the Node.js WebAssembly package, the Kotlin/JNA package and the HTTP service must reproduce byte for byte; CI fails on any difference. Covers day/month ambiguity, 12/24-hour clocks, multi-line messages, media and system notices, deleted/edited messages, group chats, Facebook mojibake repair, emoji, Sinhala, Tamil and right-to-left text, very long messages, empty/truncated files, CRLF/LF/BOM variants, the id rules, signatures and error envelopes. See `conformance/README.md`.
- **Fuzz targets** (`fuzz/`, cargo-fuzz): the WhatsApp and Facebook parsers, the device normalizer, both webhook normalizers, the signature verifier and the JSON entry point, with a seed corpus taken from the conformance cases. CI fuzzes each target for 60 seconds on Linux; `crates/msgkit-core/tests/fuzz_corpus.rs` replays the corpus on every platform.
- **Property-based tests** (`proptest`, `crates/msgkit-core/tests/properties.rs`): no input panics a parser, the JSON entry point or the signature check; importing twice gives identical reports and a store de-duplicates; ids are unique and ignore the UTC offset and the default country code; timestamps are always UTC; error text never repeats the input; signatures round-trip and any change to body or signature is rejected.

### Fixed
- **System notices that contain a colon no longer become fake participants** (audit finding F03): `Alex changed the subject from "Team: A" to "B"` used to be parsed as a message from a sender called `Alex changed the subject from "Team`. Lines whose sender part is a known notice phrase or contains a quotation mark are notices.
- **Notices are no longer dropped silently** (F09, first part): a line without a sender that is neither a missed call nor a recognized English notice is counted in one warning (`N line(s) without a sender were ignored ...`), so exports in other languages show up instead of losing data unnoticed. The warning never quotes the line. Modelling deleted/edited messages as events stays with the lifecycle work in Phase 6.
- **Error messages no longer repeat the input** (audit finding F01). `serde_json` messages quoted the offending value (for example ``unknown variant `...` ``), and the HTTP service returned axum's default rejection text, which quoted request bodies. Core errors now keep only the category, a missing field's name and the position; the service answers malformed bodies and query strings with fixed texts. Guarded by `tests/error_redaction.rs`, an HTTP test and three conformance cases.

### Changed
- Python: requests to the native library are serialized as compact JSON, so error messages that mention a column are identical in every binding.

### Added (quality gates, Phase 1)
- CI matrix: Rust on Linux, macOS and Windows; Python 3.9 to 3.13 on all three; Node.js 20 to 24; Kotlin on JDK 17 and 21 (plus Windows and macOS); Android emulator tests; weekly advisory run and a newest-Rust canary. All actions are pinned to commit SHAs.
- Gates: `rustfmt`, `clippy::pedantic` as errors, `ruff` and `mypy` (strict) for Python, `eslint` and `tsc --noEmit` (strict) for Node.js, `ktlint` and `detekt` for Kotlin, `cargo audit`, `cargo deny`, `npm audit`, `pip-audit`, a 90% line-coverage minimum for `msgkit-core`, `cargo-semver-checks`, and a JSON Schema compatibility check (`scripts/schema_compat.py`).
- Repository hygiene guard (`scripts/check_repo_hygiene.py`) that rejects real exports, databases, secrets, personal e-mail addresses, home paths and realistic phone numbers outside the fixture directories.
- Dependabot for Cargo, npm, pip, Gradle and GitHub Actions; `deny.toml`; `requirements-dev.txt`; `.gitattributes` (LF line endings; CRLF for Windows batch files, LF for `gradlew` and shell scripts).

### Changed
- **Minimum supported Rust version raised from 1.85 to 1.88**: `sha2` 0.11 requires it. `hmac` 0.13 and `sha2` 0.11 were updated together (they share `digest` 0.11); signature verification still compares in constant time (`digest` 0.11 uses `ctutils::CtEq`), and known-answer tests (RFC 4231) and pinned id tests guard the behaviour.
- Distribution names: npm `@pamoda2004/msgkit`, PyPI `msgkit-core` (import name `msgkit`; the name `msgkit` is taken on PyPI by an unrelated project), Maven group `io.github.pamoda2004`. Nothing has been published yet.
- The Rust toolchain is pinned to 1.99.0; the Node.js package now requires Node 20 or later; internal crates are declared once in `[workspace.dependencies]` with versions, as crates.io requires.
- Licence holder is now "Pamoda Jayathilaka and msgkit contributors"; `SECURITY.md` names GitHub private vulnerability reporting.
- Kotlin: JSON-parse failures no longer quote the parser's message, which can contain a snippet of the input.
- Test fixtures use obviously fictional phone numbers (NANP 555-01xx range).

### Added
- **Language packages** over the same core, with identical results in every language:
  - `@pamoda2004/msgkit` (`packages/node`): WebAssembly build (`msgkit-wasm`), no native addon, TypeScript types, runs on Node 20+, browsers, Deno and Bun.
  - `msgkit-core` on PyPI, imported as `msgkit` (`packages/python`): typed, dependency-free, platform wheel (`py3-none-<platform>`) bundling the native library; `scripts/test_wheel.py` tests the *installed* wheel.
  - `io.github.pamoda2004:msgkit` and `io.github.pamoda2004:msgkit-android` (`packages/kotlin`): Kotlin API with serializable models over JNA; the Android artifact packages `libmsgkit_ffi.so` for arm64-v8a, armeabi-v7a, x86_64 and x86 (64-bit libraries are 16 KiB page-aligned).
  - CI jobs for all three and a tag-triggered release workflow (not yet exercised).
- `msgkit-wasm` crate: import-free WebAssembly module with a tiny C-like ABI (documented in docs/API.md).

### Changed
- `chrono`'s `clock` feature is no longer enabled in `msgkit-core` (nothing used the system clock), which keeps the core buildable for `wasm32-unknown-unknown` without JavaScript glue.
- The standalone `examples/python` demo was replaced by the `msgkit` Python package.

### Added (earlier, unreleased)
- `msgkit-store-sqlite`: durable SQLite `EventStore` (WAL, `synchronous = FULL`, atomic batches, versioned on-disk layout, owner-only file on Unix, multi-process safe). Acknowledged events have their content erased; ids are kept for de-duplication and pruned after a retention window.
- `msgkit-service`: `MSGKIT_DB_PATH` makes webhook events survive restarts; `POST /v1/events/ack` for at-least-once consumption; periodic pruning.
- `msgkit_core::store::conformance`: a reusable test suite every `EventStore` backend must pass.
- `Event::channel()` and `Event::kind()`, `Error::Storage` (`storage_error`).

### Changed (breaking, Rust API only)
- `EventStore` methods are now fallible (`Result`), and the trait gained `insert_all`, `list`, `drain` and `acknowledge`. `MemoryStore::events()`/`drain()` inherent methods were replaced by the trait methods. The JSON contract (`SCHEMA_VERSION`) is unchanged.
- `AppState::new` returns a `Result` (the database may fail to open).

## [0.1.0]

Initial release. Schema `1.0.0`.

### Added
- Normalized `Event` model (`message`, `call`) with deterministic ids and generated JSON Schemas.
- Parsers: WhatsApp chat export (Android/iOS, 12/24h, day/month-first), Facebook Messenger JSON export (with encoding repair).
- Normalizers: WhatsApp Business Cloud API and Messenger Platform webhooks, with HMAC-SHA256 signature verification.
- Normalizer for Android SMS and call-log rows.
- `api::dispatch`: single JSON-in/JSON-out entry point shared by all bindings.
- `msgkit` CLI, `msgkit-service` HTTP sidecar (OpenAPI), `msgkit-ffi` C ABI, Python example.
