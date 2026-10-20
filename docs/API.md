# API reference

All integrations expose the same operations with the same JSON.

## Operations

Every operation takes a JSON object and returns an envelope:

```json
{"ok": true,  "data": { ... }}
{"ok": false, "error": {"code": "invalid_input", "message": "..."}}
```

Every error has a stable `code`; the full list, with the meaning of each code, where it is returned and how it is tested, is in [ERRORS.md](ERRORS.md). The HTTP service reports it as `{"error": "<text>", "code": "<code>"}`. Which parts of each binding are stable and which are not is listed in [API_SURFACE.md](API_SURFACE.md).

Import operations return `data = {"events": [Event...], "warnings": [string...]}`. The `Event` shape is defined by [`schema/event.schema.json`](../schema/event.schema.json).

| Operation | Input | Output `data` |
|---|---|---|
| `version` | `{}` | `{crate_version, schema_version, operations, tzdata_version}` |
| `parse_whatsapp_export` | `{"text": "<file contents>", "options": WhatsAppExportOptions}` | import report |
| `parse_facebook_export` | `{"thread": <parsed message_N.json>, "options": FacebookExportOptions}` | import report |
| `normalize_device` | `{"batch": DeviceBatch, "options": DeviceOptions}` | import report |
| `normalize_whatsapp_webhook` | `{"payload": <webhook JSON>}` | import report |
| `normalize_messenger_webhook` | `{"payload": <webhook JSON>}` | import report |
| `verify_signature` | `{"secret", "body", "signature"}` | `{"valid": bool}` |

Option schemas are in [`schema/options.schema.json`](../schema/options.schema.json). Omitted options use defaults.

## Input limits

Every import checks its input against a `limits` object before it does the expensive work, and fails with the error code `limit_exceeded` when a limit is passed. Nothing is truncated: a partial import would look like a complete one. `limits` is an option of `parse_whatsapp_export`, `parse_facebook_export` and `normalize_device`, and a top-level field next to `payload` for the two webhook operations.

| Limit | Default | What it bounds |
|---|---|---|
| `max_input_bytes` | 33554432 (32 MiB) | the exported text (WhatsApp), or the whole JSON document (the other imports; through the JSON entry point that is the whole request) |
| `max_line_bytes` | 262144 (256 KiB) | the longest line of a WhatsApp export |
| `max_messages` | 1000000 | messages, calls and notices in one import (SMS rows plus call rows for a device batch) |
| `max_json_depth` | 32 | nesting of JSON arrays and objects in the document handed to msgkit (the request wrapper counts through the JSON entry point; over HTTP the body is the document) |

An absent limit (or `null`) has its default; name only the limits you want to change, each as a number of **at least 1**. **`0` is refused with `invalid_input`**: it reads as "allow nothing", so it must never switch protection off. The one explicit way to switch every limit off is `"unlimited": true` (Rust `Limits::unlimited()`, Python `limits={"unlimited": True}`, Node.js `{ unlimited: true }`, Kotlin `Limits(unlimited = true)`, command line `--no-limits`, service `MSGKIT_NO_LIMITS=true`); to merely raise one limit, give a larger number. Real exports are far below the defaults (WhatsApp itself stops an export at 40,000 messages without media, about 4 MB). Raise a limit or switch them off only for input you trust.

Where the limit is checked: the JSON entry point parses the request first (a cheap pass), then checks size and depth before the importer runs, so the 8 to 37 times amplification of parsing is not paid for an oversized input. A caller who must cap memory before parsing should cap the buffer it hands over; the HTTP service and the WebAssembly module do (below). Rust callers can use the `Limits` type (`msgkit_core::limits`) and the `with_limits` setters; the infallible `whatsapp_export::parse` and `device::normalize` return no events and one warning when over a limit, and `try_parse` and `try_normalize` return the error.

The HTTP service takes its limits **from the operator only** (`MSGKIT_MAX_*`, `MSGKIT_NO_LIMITS`); a request cannot change them: a `limits` query parameter is refused with `invalid_input`, and any `limits` or `options` inside a JSON body, loose `max_*` query parameters and headers are ignored. A bad value (0, text) stops the service at startup, naming the variable. The command line has the flags `--max-input-bytes`, `--max-line-bytes`, `--max-messages`, `--max-json-depth` (each at least 1, applied per file) and `--no-limits`. If your application forwards options from its own users into msgkit, strip `limits` first: the JSON entry point trusts its caller.

> `verify_signature` answers `{"valid": false}` for a mismatch instead of returning an error; see [ERRORS.md](ERRORS.md#why-verify_signature-returns-valid-false-and-not-an-error).

> `normalize_*_webhook` does **not** verify the signature. Call `verify_signature` over the raw request body first, or use the HTTP sidecar, which always does.

## C ABI

Header: [`crates/msgkit-ffi/include/msgkit.h`](../crates/msgkit-ffi/include/msgkit.h). Build with `cargo build -p msgkit-ffi --release`; artifacts are in `target/release/` (`msgkit_ffi.dll` / `libmsgkit_ffi.so` / `libmsgkit_ffi.dylib`, plus a static library).

```c
char       *msgkit_call(const char *op, const char *input_json);
void        msgkit_free(char *s);          /* free every msgkit_call result exactly once */
const char *msgkit_version(void);          /* static, do not free */
```

* Strings are NUL-terminated UTF-8.
* Results are owned by the caller; inputs are only borrowed.
* Panics never cross the boundary.
* The library is thread-safe (no shared state).

Reference wrappers over this ABI: [`packages/python/src/msgkit/_native.py`](../packages/python/src/msgkit/_native.py) (ctypes) and [`packages/kotlin/.../NativeLib.kt`](../packages/kotlin/msgkit/src/main/kotlin/io/github/pamoda2004/msgkit/internal/NativeLib.kt) (JNA). When binding from another language:

* Keep the result as a **raw pointer** (not an auto-converted string) so it can be passed to `msgkit_free`; with ctypes that means `restype = c_void_p`.
* Pass strings as explicit **UTF-8 bytes**. Auto-marshalled strings use the platform charset in some runtimes (JNA on Windows, for example) and corrupt non-ASCII text.

## Language packages

The Node, Python and Kotlin packages expose the operations above as idiomatic functions over the same core, so results are identical everywhere:

| Package | Backend | Docs |
|---|---|---|
| `@pamoda2004/msgkit` (npm) | `msgkit-wasm` (WebAssembly, no native addon) | [packages/node](../packages/node/README.md) |
| `msgkit-core` (PyPI; import name `msgkit`) | `msgkit-ffi` via ctypes, bundled in a platform wheel | [packages/python](../packages/python/README.md) |
| `io.github.pamoda2004:msgkit`, `io.github.pamoda2004:msgkit-android` (Maven) | `msgkit-ffi` via JNA; `.so` per ABI on Android | [packages/kotlin](../packages/kotlin/README.md) |

### WebAssembly ABI

For hosts that want to drive the WebAssembly module directly (the Node package is the reference): the module has **no imports** and exports `memory`, `msgkit_alloc(len)`, `msgkit_dealloc(ptr, len)`, `msgkit_call(op_ptr, op_len, in_ptr, in_len) -> ptr`, `msgkit_last_len()` and `msgkit_version()`. Copy UTF-8 inputs into memory obtained from `msgkit_alloc`, call, read `msgkit_last_len()` bytes at the returned pointer, then free the result and both inputs. The module is single-threaded. It tracks every buffer it hands out: `msgkit_dealloc` ignores its length argument and any pointer that is not a live buffer (so a wrong length or a repeated free cannot corrupt memory), `msgkit_call` answers `invalid_input` for a range that is not inside a live buffer, and `msgkit_alloc` returns 0 for a request over 512 MiB. See [`crates/msgkit-wasm/src/lib.rs`](../crates/msgkit-wasm/src/lib.rs).

## HTTP sidecar

Full description: [`openapi.yaml`](../openapi.yaml).

| Method and path | Purpose |
|---|---|
| `GET /healthz` | liveness (no auth) |
| `GET /v1/version` | versions |
| `POST /v1/import/whatsapp-export` | body: raw `.txt`; options as query params |
| `POST /v1/import/facebook-export` | body: one `message_N.json`; options as query params |
| `POST /v1/import/device` | body: `DeviceBatch` JSON; options as query params |
| `GET /v1/events[?drain=true]` | webhook events not yet acknowledged, oldest arrival first |
| `POST /v1/events/ack` | body `{"ids": [...]}`; marks events as consumed |
| `GET/POST /webhooks/whatsapp` | Meta subscription handshake / signed deliveries |
| `GET/POST /webhooks/messenger` | same for Messenger |

Configuration (environment):

| Variable | Meaning | Default |
|---|---|---|
| `MSGKIT_BIND` | listen address | `127.0.0.1:8080` |
| `MSGKIT_API_TOKEN` | require `Authorization: Bearer <token>` on `/v1/*` | unset (open; keep it on loopback) |
| `MSGKIT_WHATSAPP_APP_SECRET` | Meta app secret for signature checks | unset → webhook returns 503 |
| `MSGKIT_WHATSAPP_VERIFY_TOKEN` | handshake token | unset → handshake rejected |
| `MSGKIT_MESSENGER_APP_SECRET` / `_VERIFY_TOKEN` | same for Messenger | unset |
| `MSGKIT_DB_PATH` | SQLite file for webhook events | unset → in memory, **lost on restart** |
| `MSGKIT_MAX_INPUT_BYTES` | largest request body for the imports (webhooks: the lower of this and 2 MiB); a whole number of at least 1 | 33554432 (32 MiB) |
| `MSGKIT_MAX_LINE_BYTES` | longest WhatsApp export line; at least 1 | 262144 |
| `MSGKIT_MAX_MESSAGES` | most messages, calls and notices per import; at least 1 | 1000000 |
| `MSGKIT_MAX_JSON_DEPTH` | deepest JSON nesting; at least 1 | 32 |
| `MSGKIT_NO_LIMITS` | `true` switches every input limit off (a warning is logged at startup) | unset (limits on) |
| `RUST_LOG` | log level | `info` |

### Webhook event storage

Webhook events are de-duplicated by id (Meta retries failed deliveries) and held until your application consumes them.

* **Durable (recommended):** set `MSGKIT_DB_PATH=/var/lib/msgkit/events.db`. Each webhook delivery is written in one transaction and fsynced *before* the `200` is returned, so an accepted event survives a crash or restart. If the write fails the service answers `500` and stores nothing, so Meta's retry redelivers the whole payload.
* **In memory:** the default, for development. A startup warning says so.

Consume events reliably with **list, process, acknowledge**:

```bash
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8080/v1/events              # list pending
curl -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
     -d '{"ids":["<id1>","<id2>"]}' http://127.0.0.1:8080/v1/events/ack            # after you stored them
```

Events you do not acknowledge are listed again, so a crash in your consumer loses nothing. (`?drain=true` lists and acknowledges in one step; it is at-most-once, so use it only when losing an event on a crash is acceptable.)

When an event is acknowledged its **content is erased** from the database and only its id is kept, so the service never becomes a permanent archive of messages. The id is remembered to reject redelivered webhooks, and is forgotten after 30 days (a pruning job runs every 6 hours; Meta retries for a few days at most).

The database file is **not encrypted**. Put it on an encrypted volume and restrict access (it is created owner-only on Unix). Embedding `msgkit-core` instead? Use `msgkit-store-sqlite` directly, or implement the `EventStore` trait for your own backend.

Behind a reverse proxy, terminate TLS there (Meta requires HTTPS) and forward the original body and `X-Hub-Signature-256` header unchanged.
