# API reference

All integrations expose the same operations with the same JSON.

## Operations

Every operation takes a JSON object and returns an envelope:

```json
{"ok": true,  "data": { ... }}
{"ok": false, "error": {"code": "invalid_input", "message": "..."}}
```

Error codes: `invalid_input`, `parse_error`, `unknown_op`, `internal_error`.

Import operations return `data = {"events": [Event...], "warnings": [string...]}`. The `Event` shape is defined by [`schema/event.schema.json`](../schema/event.schema.json).

| Operation | Input | Output `data` |
|---|---|---|
| `version` | `{}` | `{crate_version, schema_version, operations}` |
| `parse_whatsapp_export` | `{"text": "<file contents>", "options": WhatsAppExportOptions}` | import report |
| `parse_facebook_export` | `{"thread": <parsed message_N.json>, "options": FacebookExportOptions}` | import report |
| `normalize_device` | `{"batch": DeviceBatch, "options": DeviceOptions}` | import report |
| `normalize_whatsapp_webhook` | `{"payload": <webhook JSON>}` | import report |
| `normalize_messenger_webhook` | `{"payload": <webhook JSON>}` | import report |
| `verify_signature` | `{"secret", "body", "signature"}` | `{"valid": bool}` |

Option schemas are in [`schema/options.schema.json`](../schema/options.schema.json). Omitted options use defaults.

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

Reference wrappers over this ABI: [`packages/python/src/msgkit/_native.py`](../packages/python/src/msgkit/_native.py) (ctypes) and [`packages/kotlin/.../NativeLib.kt`](../packages/kotlin/msgkit/src/main/kotlin/dev/msgkit/internal/NativeLib.kt) (JNA). When binding from another language:

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

For hosts that want to drive the WebAssembly module directly (the Node package is the reference): the module has **no imports** and exports `memory`, `msgkit_alloc(len)`, `msgkit_dealloc(ptr, len)`, `msgkit_call(op_ptr, op_len, in_ptr, in_len) -> ptr`, `msgkit_last_len()` and `msgkit_version()`. Copy UTF-8 inputs into memory obtained from `msgkit_alloc`, call, read `msgkit_last_len()` bytes at the returned pointer, then free the result and both inputs. The module is single-threaded. See [`crates/msgkit-wasm/src/lib.rs`](../crates/msgkit-wasm/src/lib.rs).

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
