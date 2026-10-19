# msgkit for Python

Import **SMS, WhatsApp and Messenger** messages and calls into one normalized model.

```python
import msgkit

report = msgkit.parse_whatsapp_export(
    open("_chat.txt", encoding="utf-8-sig").read(),
    self_name="Alice",
    utc_offset_minutes=330,
)
for warning in report["warnings"]:
    print("warning:", warning)
for event in report["events"]:
    print(event["timestamp"], event["type"], event.get("body"))
```

No dependencies. The wheel bundles the native msgkit library (loaded with `ctypes`), tagged `py3-none-<platform>`: one wheel per OS/architecture, valid for every Python 3.9+.

## API

| Function | Purpose |
|---|---|
| `parse_whatsapp_export(text, *, self_name, date_order, utc_offset_minutes, conversation_id, default_country_code, time_zone)` | WhatsApp "Export chat" `.txt` |
| `parse_facebook_export(thread, *, self_name, conversation_id)` | one Messenger `message_N.json` (object or text) |
| `normalize_device(batch, *, own_number, default_country_code)` | Android SMS / call-log rows |
| `normalize_whatsapp_webhook(payload)` | WhatsApp Business Cloud API webhook |
| `normalize_messenger_webhook(payload)` | Messenger Platform webhook |
| `verify_signature(*, secret, body, signature)` | `X-Hub-Signature-256` check on the **raw** body |
| `version()` | native library and JSON schema versions |
| `dispatch(op, payload)` | low-level access to any operation |

Every function returns plain `dict`/`list` data (`msgkit.types` has `TypedDict` definitions for editors and type checkers; the package ships `py.typed`). Errors raise `msgkit.MsgkitError` with a stable `.code` (`invalid_input`, `parse_error`, `unknown_op`, `internal_error`). The library is thread-safe.

> The `normalize_*_webhook` functions do **not** verify signatures. Call `verify_signature` on the raw request body first.

What each source can and cannot provide is documented in [docs/SUPPORTED_SOURCES.md](../../docs/SUPPORTED_SOURCES.md); the data shapes in [schema/](../../schema/README.md).

## Development

```bash
cargo build -p msgkit-ffi                    # the tests find target/debug automatically
python -m unittest discover -s packages/python/tests -v   # with PYTHONPATH=packages/python/src

python packages/python/scripts/build_wheel.py   # native library + platform wheel in packages/python/dist
```

`MSGKIT_LIB=/path/to/library` overrides library discovery.
