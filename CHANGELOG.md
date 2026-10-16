# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/) and the project uses [Semantic Versioning](https://semver.org/). The JSON data contract is versioned separately (`SCHEMA_VERSION`).

## [Unreleased]

### Added
- **Language packages** over the same core, with identical results in every language:
  - `@msgkit/core` (`packages/node`): WebAssembly build (`msgkit-wasm`), no native addon, TypeScript types, runs on Node 18+, browsers, Deno and Bun.
  - `msgkit` (`packages/python`): typed, dependency-free, platform wheel (`py3-none-<platform>`) bundling the native library; `scripts/test_wheel.py` tests the *installed* wheel.
  - `dev.msgkit:msgkit` and `dev.msgkit:msgkit-android` (`packages/kotlin`): Kotlin API with serializable models over JNA; the Android artifact packages `libmsgkit_ffi.so` for arm64-v8a, armeabi-v7a, x86_64 and x86 (64-bit libraries are 16 KiB page-aligned).
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
