# msgkit

[![CI](https://github.com/pamoda2004/msgkit/actions/workflows/ci.yml/badge.svg)](https://github.com/pamoda2004/msgkit/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A portable toolkit for importing **SMS, WhatsApp and Messenger messages and calls** into one normalized model, usable from any language: backends, Android apps, scripts, desktop tools.

```
 user exports ─┐
 webhooks ─────┼─▶  msgkit-core (Rust)  ─▶  normalized Event stream  ─▶  your app / database
 device rows ──┘     parse · verify · normalize · dedupe
```

msgkit does **not** store anything or talk to the network on its own. It turns data you legitimately obtained into clean, de-duplicated events; what you do with them is up to your application.

## What it can import

| Source | How | Status |
|---|---|---|
| WhatsApp | User's "Export chat" `.txt` file | ✅ |
| WhatsApp Business | Cloud API webhooks (verified) | ✅ messages · ❌ calls |
| Messenger | "Download Your Information" JSON export | ✅ messages and calls |
| Messenger (Pages) | Messenger Platform webhooks (verified) | ✅ messages |
| Android SMS and call log | Rows from a native collector | ✅ normalizer · collector is yours (see [docs](docs/ANDROID_COLLECTOR.md)) |

Read [docs/SUPPORTED_SOURCES.md](docs/SUPPORTED_SOURCES.md) for exactly what each source can and cannot provide. In short: there is **no official way to read a personal WhatsApp or Facebook account's messages or calls programmatically**, and msgkit deliberately does not use unofficial ones (they violate the platforms' terms and risk account bans).

## Use it from your language

Pick the integration that fits; all of them run the same core and return the same JSON.

| You are writing... | Use | Docs |
|---|---|---|
| **Node.js, browsers, Deno, Bun** | `@pamoda2004/msgkit`: WebAssembly, no native addon, TypeScript types | [packages/node](packages/node/README.md) |
| **Python** (3.9+) | `msgkit-core` (`import msgkit`): platform wheel, no dependencies, typed | [packages/python](packages/python/README.md) |
| **Kotlin / Java**: JVM servers, desktop | `io.github.pamoda2004:msgkit` (natives bundled) | [packages/kotlin](packages/kotlin/README.md) |
| **Android** (API 26+) | `io.github.pamoda2004:msgkit-android` (AAR with all four ABIs) | [packages/kotlin](packages/kotlin/README.md) |
| Anything else, over the network | **HTTP sidecar** `msgkit-service` | [openapi.yaml](openapi.yaml) |
| C#, Go, Swift, PHP, Dart... in-process | **C ABI** `msgkit-ffi` (one function, JSON in/out) | [docs/API.md](docs/API.md) |
| Rust | the **`msgkit-core`** crate | `cargo doc -p msgkit-core --open` |
| Shell / data pipelines | the **`msgkit` CLI** (NDJSON out) | `msgkit --help` |

The three language packages are thin wrappers over the same core, so a given input produces **identical JSON in every language** (each package's tests run the same fixtures).

Data types for your language can be generated from the JSON Schemas in [`schema/`](schema/README.md).

### CLI

```bash
cargo run -p msgkit-cli -- whatsapp-export chat.txt --self-name "Alice" --utc-offset-minutes 330 > events.ndjson
cargo run -p msgkit-cli -- facebook-export messages/inbox/bob_abc123/message_*.json
cargo run -p msgkit-cli -- device batch.json --default-country-code 94
```

### HTTP sidecar

```bash
MSGKIT_API_TOKEN=change-me MSGKIT_DB_PATH=./events.db cargo run -p msgkit-service     # 127.0.0.1:8080
curl -H "Authorization: Bearer change-me" --data-binary @chat.txt \
     "http://127.0.0.1:8080/v1/import/whatsapp-export?self_name=Alice"
```

The service listens on loopback by default; any other address requires `MSGKIT_API_TOKEN` (generate one with `openssl rand -hex 32`) or it will not start, and without a token a `Host` header other than `localhost` is refused. See [Securing the service](docs/API.md#securing-the-service), including Docker.

Webhook endpoints (`/webhooks/whatsapp`, `/webhooks/messenger`) refuse all traffic until `MSGKIT_*_APP_SECRET` is set and the `X-Hub-Signature-256` header verifies. With `MSGKIT_DB_PATH` set, received events are stored in SQLite and **survive restarts**; read them with `GET /v1/events` and confirm with `POST /v1/events/ack`. See [docs/API.md](docs/API.md#http-sidecar).

### The same import in each language

```js
// Node.js / browsers (WebAssembly)
import { parseWhatsAppExport } from '@pamoda2004/msgkit';
const report = parseWhatsAppExport(text, { self_name: 'Alice', utc_offset_minutes: 330 });
```

```python
# Python
import msgkit

report = msgkit.parse_whatsapp_export(text, self_name="Alice", utc_offset_minutes=330)
```

```kotlin
// Kotlin (JVM and Android)
val report = Msgkit.parseWhatsAppExport(text, WhatsAppExportOptions(selfName = "Alice", utcOffsetMinutes = 330))
```

## The data model

Everything becomes an `Event`, either a `message` or a `call`, with a deterministic `id` (re-importing the same data yields the same ids, so de-duplication is trivial), UTC timestamps, direction, participants (E.164 numbers where known), attachments as references, and an optional raw source payload. See [`crates/msgkit-core/src/model.rs`](crates/msgkit-core/src/model.rs) and [`schema/event.schema.json`](schema/event.schema.json).

## Repository layout

```
crates/
  msgkit-core/      portable engine: model, parsers, webhook normalizers, JSON dispatcher
  msgkit-cli/       `msgkit` command line tool
  msgkit-store-sqlite/  durable SQLite `EventStore` (used by the service, usable on its own)
  msgkit-service/   HTTP sidecar (axum)
  msgkit-ffi/       C ABI (cdylib/staticlib) + include/msgkit.h
  msgkit-wasm/      WebAssembly build (no imports, tiny C-like ABI)
packages/
  node/             @pamoda2004/msgkit: JS host for the wasm module + TypeScript types
  python/           msgkit: ctypes wrapper, typed, platform wheel
  kotlin/           Gradle build: `msgkit` (JVM, JNA) and `msgkit-android` (AAR with .so per ABI)
schema/             generated JSON Schemas: the language-neutral contract
docs/               architecture, API, supported sources, privacy, Android collector
examples/           HTTP examples, import scripts, and consumers/ (4 separate projects that use msgkit as a dependency)
openapi.yaml        sidecar API description
```

Design details and the reasoning behind them: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Development

Requires a stable Rust toolchain (see `rust-toolchain.toml`). The minimum supported Rust version (MSRV) is **1.88**; CI builds the whole workspace with it.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
# after an intentional model change:
UPDATE_SCHEMA=1 cargo test -p msgkit-core --test schema
```

Each language package has its own build and test commands, described in its README ([Node](packages/node/README.md#development), [Python](packages/python/README.md#development), [Kotlin](packages/kotlin/README.md#development)).

See [CONTRIBUTING.md](CONTRIBUTING.md). Report vulnerabilities as described in [SECURITY.md](SECURITY.md).

## Roadmap

- Publish the packages (npm, PyPI, Maven Central); the release workflow is written but has not run yet
- Swift, .NET and Go packages over the same C ABI
- Language types generated from `schema/` instead of hand-written
- More `EventStore` backends (PostgreSQL) and optional encryption at rest
- WhatsApp Cloud API call webhooks
- Reference Android collector module

## License

MIT. See [LICENSE](LICENSE).
