// A tiny application that uses msgkit as an *installed dependency* (not from source).
// Prints chat statistics; the same program exists for Python, Kotlin and Rust and prints the same output.
import { createHmac } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { parseWhatsAppExport, verifySignature } from '@msgkit/core';

const file =
  process.argv[2] ?? fileURLToPath(new URL('../../../crates/msgkit-core/tests/fixtures/whatsapp_android_dmy.txt', import.meta.url));
const report = parseWhatsAppExport(readFileSync(file, 'utf8'), { self_name: 'Alice', utc_offset_minutes: 330 });

const messages = report.events.filter((e) => e.type === 'message');
const calls = report.events.filter((e) => e.type === 'call');
const perSender = new Map();
for (const m of messages) perSender.set(m.sender.id, (perSender.get(m.sender.id) ?? 0) + 1);

console.log(`events: ${report.events.length}`);
console.log(`messages: ${messages.length}, calls: ${calls.length}`);
console.log('per sender:');
for (const [name, n] of [...perSender].sort()) console.log(`  ${name}: ${n}`);
console.log(`attachments: ${messages.reduce((n, m) => n + m.attachments.length, 0)}`);
console.log(`missed calls: ${calls.filter((c) => c.outcome === 'missed').length}`);
console.log(`first: ${report.events[0].timestamp}  last: ${report.events.at(-1).timestamp}`);

// Webhook signature check with a signature computed independently by the language's own crypto library.
const body = '{"object":"page","entry":[]}';
const good = `sha256=${createHmac('sha256', 'app-secret').update(body).digest('hex')}`;
console.log(`signature valid: ${verifySignature({ secret: 'app-secret', body, signature: good })}`);
console.log(`tampered body valid: ${verifySignature({ secret: 'app-secret', body: `${body} `, signature: good })}`);
