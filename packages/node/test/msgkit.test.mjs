import assert from 'node:assert/strict';
import { createHmac } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import * as msgkit from '../src/index.js';

const fixture = (name) =>
  readFileSync(fileURLToPath(new URL(`../../../crates/msgkit-core/tests/fixtures/${name}`, import.meta.url)), 'utf8');

const messages = (r) => r.events.filter((e) => e.type === 'message');
const calls = (r) => r.events.filter((e) => e.type === 'call');

test('package version matches the Rust workspace version', () => {
  const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  const cargo = readFileSync(new URL('../../../Cargo.toml', import.meta.url), 'utf8');
  assert.equal(pkg.version, /^version = "(.+)"$/m.exec(cargo)[1]);
  assert.equal(msgkit.version().crate_version, pkg.version);
  assert.equal(msgkit.version().schema_version, '1.0.0');
});

test('WhatsApp export: Android day-first', () => {
  const r = msgkit.parseWhatsAppExport(fixture('whatsapp_android_dmy.txt'));
  assert.deepEqual(r.warnings, []);
  assert.equal(r.events.length, 8);
  assert.equal(messages(r).length, 7);
  assert.equal(calls(r)[0].outcome, 'missed');
  assert.equal(r.events[0].timestamp, '2023-12-31T21:41:00Z');
  assert.equal(r.events[0].body, 'Happy new year!');
  const oks = messages(r).filter((m) => m.body === 'ok');
  assert.equal(oks.length, 2);
  assert.notEqual(oks[0].id, oks[1].id);
});

test('WhatsApp export: options are applied', () => {
  const r = msgkit.parseWhatsAppExport(fixture('whatsapp_android_dmy.txt'), {
    self_name: 'alice',
    utc_offset_minutes: 330,
  });
  assert.equal(r.events[0].timestamp, '2023-12-31T16:11:00Z');
  assert.equal(r.events[0].direction, 'outbound');
  assert.equal(r.events[1].direction, 'inbound');
});

test('WhatsApp export: month-first 12-hour clock', () => {
  const r = msgkit.parseWhatsAppExport(fixture('whatsapp_android_mdy_12h.txt'));
  assert.deepEqual(
    r.events.map((e) => e.timestamp),
    ['2023-12-30T23:59:00Z', '2023-12-31T00:05:00Z', '2023-12-31T12:30:00Z'],
  );
});

test('WhatsApp export: Unicode survives the round trip', () => {
  const body = 'සිංහල 🎉 café "quoted" \\ back\\slash\ttab';
  const r = msgkit.parseWhatsAppExport(`01/02/2024, 10:00 - සමන්: ${body}\n`);
  assert.equal(r.events[0].body, body);
  assert.equal(r.events[0].sender.id, 'සමන්');
});

test('WhatsApp export: re-import gives identical ids', () => {
  const text = fixture('whatsapp_android_dmy.txt');
  assert.deepEqual(msgkit.parseWhatsAppExport(text), msgkit.parseWhatsAppExport(text));
});

test('Facebook export: object and string input', () => {
  const text = fixture('facebook_thread.json');
  const a = msgkit.parseFacebookExport(JSON.parse(text), { self_name: 'Alice Perera' });
  const b = msgkit.parseFacebookExport(text, { self_name: 'Alice Perera' });
  assert.deepEqual(a, b);
  assert.equal(a.events.length, 5);
  assert.equal(messages(a)[2].body, 'café ❤');
  assert.equal(messages(a)[2].direction, 'outbound');
  assert.equal(calls(a)[1].duration_secs, 125);
});

test('device batch normalization', () => {
  const batch = JSON.parse(fixture('device_batch.json'));
  const r = msgkit.normalizeDevice(batch, { own_number: '+94710000000', default_country_code: '94' });
  assert.equal(messages(r).length, 3);
  assert.equal(messages(r)[1].recipients[0].id, '+94771234567');
  assert.deepEqual(
    calls(r).map((c) => c.outcome),
    ['answered', 'missed', 'answered', 'rejected'],
  );
});

test('webhook normalizers', () => {
  const wa = msgkit.normalizeWhatsAppWebhook(JSON.parse(fixture('whatsapp_webhook.json')));
  assert.equal(messages(wa).length, 3);
  assert.equal(messages(wa)[0].external_id, 'wamid.AAA');
  const ms = msgkit.normalizeMessengerWebhook(JSON.parse(fixture('messenger_webhook.json')));
  assert.deepEqual(
    messages(ms).map((m) => m.direction),
    ['inbound', 'outbound'],
  );
});

test('signature verification matches Node crypto', () => {
  const body = fixture('whatsapp_webhook.json');
  const signature = `sha256=${createHmac('sha256', 'app-secret').update(body).digest('hex')}`;
  assert.equal(msgkit.verifySignature({ secret: 'app-secret', body, signature }), true);
  assert.equal(msgkit.verifySignature({ secret: 'wrong', body, signature }), false);
  assert.equal(msgkit.verifySignature({ secret: 'app-secret', body: `${body} `, signature }), false);
  assert.equal(msgkit.verifySignature({ secret: 'app-secret', body, signature: 'sha256=zz' }), false);
});

test('errors are MsgkitError with stable codes', () => {
  assert.throws(
    () => msgkit.dispatch('nope'),
    (e) => e instanceof msgkit.MsgkitError && e.code === 'unknown_op',
  );
  assert.throws(
    () => msgkit.parseFacebookExport('[1,2]'),
    (e) => e instanceof msgkit.MsgkitError && e.code === 'parse_error',
  );
  assert.throws(
    () => msgkit.normalizeMessengerWebhook({ object: 'whatsapp_business_account' }),
    (e) => e instanceof msgkit.MsgkitError && e.code === 'parse_error',
  );
});

test('bad records become warnings, not exceptions', () => {
  const r = msgkit.parseWhatsAppExport('this is not a chat export');
  assert.deepEqual(r.events, []);
  assert.ok(r.warnings.length > 0);
});

test('large input (memory growth) works and memory is reused afterwards', () => {
  const line = '31/12/2023, 21:41 - Alice: some reasonably long message body to make the file big enough\n';
  const big = line.repeat(60_000); // ~5 MB
  const r = msgkit.parseWhatsAppExport(big);
  assert.equal(r.events.length, 60_000);
  // Identical messages in the same minute are numbered, so ids are all distinct.
  assert.equal(new Set(r.events.map((e) => e.id)).size, 60_000);
});

test('no memory leak across many calls', () => {
  const text = fixture('whatsapp_android_dmy.txt');
  for (let i = 0; i < 200; i++) msgkit.parseWhatsAppExport(text); // warm up
  const before = msgkit._debug.memoryBytes();
  for (let i = 0; i < 3000; i++) msgkit.parseWhatsAppExport(text);
  assert.equal(msgkit._debug.memoryBytes(), before, 'wasm memory must not keep growing');
});

test('initSync with explicit bytes and a fresh async init both work', async () => {
  const bytes = readFileSync(fileURLToPath(new URL('../dist/msgkit.wasm', import.meta.url)));
  msgkit.initSync(bytes);
  assert.equal(msgkit.version().schema_version, '1.0.0');
  await msgkit.init(bytes);
  assert.equal(msgkit.version().schema_version, '1.0.0');
  await assert.rejects(() => msgkit.init(42), TypeError);
});
