# Architecture

## Goals

1. **Usable from any language and stack**: backend, Android, scripts.
2. **One behaviour everywhere.** Parsing logic is written once; bindings cannot drift.
3. **Safe with personal data.** No storage, no network, no secrets handled by the core.
4. **Honest about limits.** Unsupported sources fail loudly instead of being scraped.

## Layers

```
┌────────────────────────── consumers ──────────────────────────┐
│ Rust app │ CLI │ HTTP clients (any lang) │ FFI hosts (any lang) │
└────┬─────────┬──────────┬───────────────────────┬─────────────┘
     │         │          │                       │
     │   msgkit-cli  msgkit-service          msgkit-ffi
     │         │          │                       │
     └─────────┴──────────┴─────── api::dispatch ─┘   (JSON in / JSON out)
                              │
                         msgkit-core
        model · whatsapp_export · facebook · webhook · device · store
```

* **`msgkit-core`** is pure: no I/O, no globals, no platform code. It takes `&str`/`&[u8]`/`serde_json::Value` and returns an `ImportReport` (`events` + `warnings`). This is what makes it trivially portable and testable.
* **`api::dispatch(op, json) -> json`** is the single language-neutral surface. Every binding wraps this one function, so adding an operation is done once and appears in every language.
* **`msgkit-ffi`** exposes `dispatch` over a C ABI (3 functions). Anything with a C FFI can load it.
* **`msgkit-service`** exposes the same operations over HTTP for stacks where loading a native library is impractical (JVM backends, serverless, other containers) and hosts webhook receivers.
* **`msgkit-cli`** is for scripting and for trying things out.
* **`msgkit-redact`** is an optional crate on top of `msgkit-core` (it uses the core's model and phone normalizer, and nothing uses it): best-effort masking of identifiers in text and events, see [REDACTION.md](REDACTION.md). It has no binding yet.
* **`msgkit-wasm`** is the same `dispatch` compiled to WebAssembly with a five-function, import-free ABI. It needs no generated glue, runs unchanged in Node, browsers, Deno and Bun, and avoids per-platform native binaries for JavaScript users.
* **`packages/`** holds the idiomatic wrappers: `@pamoda2004/msgkit` (JS over the wasm module), `msgkit` (Python over `msgkit-ffi` with ctypes) and `msgkit` / `msgkit-android` (Kotlin over `msgkit-ffi` with JNA). They contain **no parsing logic**: they serialize options to JSON, call `dispatch`, and decode the JSON into language-native types. That is why every language gives byte-identical results, and why a new operation added to `api::dispatch` becomes available everywhere by adding one small function per package.

### Why a different backend per language

| Language | Backend | Reason |
|---|---|---|
| JavaScript | WebAssembly | one artifact for every OS/CPU and for browsers; no `node-gyp`, no install-time compile, no prebuilt-binary matrix |
| Python | native library (ctypes) | fast on large exports, dependency-free; wheels are `py3-none-<platform>` so one wheel covers all Python 3 versions |
| Kotlin/JVM, Android | native library (JNA) | the only practical way to run Rust on Android; JNA keeps the wrapper declarative and shares the exact `libmsgkit_ffi` used by Python |

Choices that matter at the language boundary: strings are always passed as explicit UTF-8 bytes; the native result pointer is never auto-converted to a string, so it can be freed exactly once; decoding ignores unknown fields so a newer native library does not break an older wrapper.

## Why this shape

* **Rust core.** Compiles to native libraries, WebAssembly and Android/iOS targets, with no GC or runtime to ship.
* **JSON at the boundary.** Slower than a typed ABI but trivially bindable, debuggable and versionable. Import workloads are I/O- and parse-bound, so the overhead is irrelevant. Typed bindings can be layered on top later (UniFFI/napi-rs) without changing behaviour.
* **JSON Schema generated from Rust types** (`schema/`). The Rust types are the source of truth; a test fails if the committed schema drifts. Other languages generate their types from the schema (e.g. with `quicktype`, `datamodel-code-generator`, `json-schema-to-typescript`).
* **Collectors are outside the core.** Reading Android's SMS provider needs Android APIs and runtime permissions. A collector is a thin native module that emits a `DeviceBatch`; the core does the normalization.

## Storage

The core never decides where data lives; it defines the `EventStore` trait (insert, atomic `insert_all`, `list`, `drain`, `acknowledge`) and ships `MemoryStore`. Backends are separate crates so the core stays free of I/O:

* **`msgkit-store-sqlite`**: durable, single-file, zero-ops. WAL with `synchronous = FULL`, so an event is on disk before the webhook is answered. Batches are one transaction. The on-disk layout is versioned (`PRAGMA user_version`) and a database written by a newer version is refused rather than corrupted.
* Any other backend implements the trait and must pass `msgkit_core::store::conformance::run`, which pins down the shared behaviour (dedupe, arrival order, atomic batches, acknowledge semantics).

**Delivery semantics.** The service answers a webhook only after the batch is stored, and answers `500` (storing nothing) if storage fails, so Meta's retries give *at-least-once* delivery into the store. Ids make that effectively *exactly-once*: redeliveries are recognised as duplicates. Consumers get at-least-once again via `list` → process → `acknowledge`.

**Data minimization.** Acknowledging an event erases its content and keeps only the id (for dedupe) until the retention window passes, so the store is a hand-off buffer, not an archive. Applications that need an archive keep their own copy.

## Data contract

* `Event` is a tagged union (`type: "message" | "call"`).
* **Ids are deterministic** (SHA-256 of channel, kind and identifying fields; for exports an ordinal disambiguates identical lines within the same minute). Re-importing yields the same ids, so stores de-duplicate with a unique key. How an id is built: `sha256("msgkit:v1" + 0x1f + part + ...)`, first 16 bytes as hex. For WhatsApp exports the parts are the channel, `msg` or `call`, the **local wall-clock time exactly as the file wrote it** (the two leading date numbers in file order, 24-hour clock, no offset), the sender **as written**, the body and an **occurrence index** for identical lines in the same minute. SMS and call-log rows use the epoch milliseconds and the address as written. Consequently an id never changes with `utc_offset_minutes`, `date_order` or `default_country_code`; only the source data can change it. Changing this derivation is a breaking change for every consumer's de-duplication.
* **Timestamps are UTC.** Sources without a zone (WhatsApp exports) take an explicit `time_zone` (IANA name, bundled database) or `utc_offset_minutes`.
* **Participants** use E.164 numbers when known, otherwise the platform id or display name.
* **Attachments are references**, never bytes.
* **`raw`** keeps the source payload for webhook events so records can be re-parsed after a parser improvement.

### Versioning

`SCHEMA_VERSION` (semver) versions the JSON contract independently of the crates. Adding optional fields or enum variants is a minor bump; renaming/removing is a major bump. Consumers should ignore unknown fields.

## Error handling

* Bad input to an operation returns an error (`invalid_input`, `parse_error`, `unknown_op`).
* Bad *records* inside otherwise valid input (an unparsable date, an unsupported message type) never abort the import; they become `warnings` and the rest is processed.
* Nothing in the core panics on untrusted input; the FFI layer additionally catches panics.

## Security notes

* Webhook signatures are verified with HMAC-SHA256 in constant time over the raw body before any parsing. The service refuses webhooks entirely if no secret is configured.
* The service binds to loopback by default; `/v1/*` can require a bearer token.
* Secrets are supplied by the host (environment variables in the service); the core never reads the environment.
* No code path logs message content.

## Extending

To add a source, write a module in `msgkit-core` with a `parse`/`normalize` function that returns `ImportReport`, add fixtures and tests, add an operation to `api::dispatch`, and (optionally) a CLI subcommand and service route. Update `docs/SUPPORTED_SOURCES.md` with what it can and cannot do.
