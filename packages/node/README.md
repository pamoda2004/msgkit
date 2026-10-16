# @pamoda2004/msgkit for Node.js and browsers

Import **SMS, WhatsApp and Messenger** messages and calls into one normalized model.

```js
import { parseWhatsAppExport } from '@pamoda2004/msgkit';
import { readFileSync } from 'node:fs';

const report = parseWhatsAppExport(readFileSync('_chat.txt', 'utf8'), {
  self_name: 'Alice',
  utc_offset_minutes: 330,
});
for (const w of report.warnings) console.warn('warning:', w);
for (const e of report.events) console.log(e.timestamp, e.type, e.body);
```

It is a **WebAssembly** build of the msgkit core: one small file (about 1.4 MB), no native addon, no `node-gyp`, no per-platform binaries. It runs unchanged on every OS and architecture, and in browsers, Deno, Bun and edge runtimes. TypeScript types are included.

## API

| Function | Purpose |
|---|---|
| `parseWhatsAppExport(text, options?)` | WhatsApp "Export chat" `.txt` |
| `parseFacebookExport(thread, options?)` | one Messenger `message_N.json` (object or text) |
| `normalizeDevice(batch, options?)` | Android SMS / call-log rows |
| `normalizeWhatsAppWebhook(payload)` | WhatsApp Business Cloud API webhook |
| `normalizeMessengerWebhook(payload)` | Messenger Platform webhook |
| `verifySignature({ secret, body, signature })` | `X-Hub-Signature-256` check on the **raw** body |
| `version()` | library and JSON schema versions |
| `dispatch(op, payload)` | low-level access to any operation |

Options and results use the same `snake_case` JSON as every other msgkit binding (see the [JSON Schemas](../../schema/README.md)), so documentation and data move freely between languages. Failures throw `MsgkitError` with a stable `.code`; bad records inside valid input become `warnings`, not exceptions.

> `normalize*Webhook` does **not** verify signatures. Call `verifySignature` on the raw request body first.

```js
import { verifySignature, normalizeWhatsAppWebhook } from '@pamoda2004/msgkit';

app.post('/webhooks/whatsapp', express.raw({ type: 'application/json' }), (req, res) => {
  const body = req.body.toString('utf8'); // the raw, unparsed body
  if (!verifySignature({ secret: process.env.APP_SECRET, body, signature: req.get('x-hub-signature-256') ?? '' })) {
    return res.sendStatus(401);
  }
  const { events } = normalizeWhatsAppWebhook(JSON.parse(body));
  // ...store events, de-duplicating on event.id
  res.sendStatus(200);
});
```

## Browsers, Deno, Bun, edge

There is no synchronous file access, so load the module explicitly once:

```js
import { init, parseWhatsAppExport } from '@pamoda2004/msgkit';
await init(fetch(new URL('@pamoda2004/msgkit/msgkit.wasm', import.meta.url)));
```

(Bundlers such as Vite and webpack resolve that URL; otherwise host `msgkit.wasm` yourself and pass its URL.) `init()` also accepts a `WebAssembly.Module`, an `ArrayBuffer` or a typed array.

## Notes

* Calls run synchronously on the calling thread. For very large exports (tens of MB) run them in a `Worker`/worker thread.
* A WebAssembly trap (which the core is built to avoid) surfaces as `MsgkitError('internal_error')` and the instance is recreated on the next call.
* Memory is released after every call; the module does not leak across calls (covered by a test).

## Development

```bash
rustup target add wasm32-unknown-unknown
npm run build   # cargo build for wasm32 and copy to dist/msgkit.wasm
npm test        # node:test; no dependencies
```
