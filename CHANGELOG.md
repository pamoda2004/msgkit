# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/) and the project uses [Semantic Versioning](https://semver.org/). The JSON data contract is versioned separately (`SCHEMA_VERSION`).

## [Unreleased]

### Changed (breaking, before any release)
- **Kotlin/Android package renamed from `dev.msgkit` to `io.github.pamoda2004.msgkit`** (and `dev.msgkit.android` to `io.github.pamoda2004.msgkit.android`), so the source package matches the Maven group `io.github.pamoda2004`. Update imports; ProGuard/R8 users get the new keep rule from the AAR automatically. Nothing had been published, so no released version is affected.
- **Event ids for WhatsApp exports and device (SMS/call-log) records are derived differently** (decision D1). They no longer depend on `utc_offset_minutes`, `date_order` or `default_country_code`: WhatsApp ids use the local wall-clock time exactly as written in the export, the sender as written and the body, plus an occurrence index so two identical messages in the same minute (for example two "ok") keep separate ids; device ids use the address or number as written. Re-importing the same chat with different settings, or from overlapping exports, now yields the same ids. Every id produced by earlier builds changes once; nothing had been published and no stored data exists, so no migration is provided. Facebook and webhook ids are unchanged.

### Added (testing depth, Phase 2)
- **Conformance suite** (`conformance/`): 49 language-independent cases (input files plus the expected canonical JSON response) that the Rust core, the Python wheel, the Node.js WebAssembly package, the Kotlin/JNA package and the HTTP service must reproduce byte for byte; CI fails on any difference. Covers day/month ambiguity, 12/24-hour clocks, multi-line messages, media and system notices, deleted/edited messages, group chats, Facebook mojibake repair, emoji, Sinhala, Tamil and right-to-left text, very long messages, empty/truncated files, CRLF/LF/BOM variants, the id rules, signatures and error envelopes. See `conformance/README.md`.

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
