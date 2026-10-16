// Imports a WhatsApp chat export with the msgkit Node package and prints a summary.
//
//   node examples/node/import_chat.mjs "C:\chats\chat.txt" --self-name "Alice" --utc-offset 330
//
// Only counts and ids are printed, never message text, so the output is safe to share.
// Runs straight from a repository checkout; build the wasm module once with:
//   cd packages/node && node scripts/build.mjs
import { readFileSync } from 'node:fs';
import { parseArgs } from 'node:util';

import { parseWhatsAppExport, version } from '../../packages/node/src/index.js';

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    'self-name': { type: 'string' },
    'utc-offset': { type: 'string', default: '0' },
    'date-order': { type: 'string', default: 'auto' },
  },
});

if (positionals.length !== 1) {
  console.error('usage: node import_chat.mjs <chat.txt> [--self-name NAME] [--utc-offset MINUTES] [--date-order auto|dmy|mdy]');
  process.exit(2);
}

// utf8 decoding keeps a leading BOM as U+FEFF; msgkit ignores it.
const text = readFileSync(positionals[0], 'utf8');
const report = parseWhatsAppExport(text, {
  self_name: values['self-name'] ?? null,
  utc_offset_minutes: Number(values['utc-offset']),
  date_order: values['date-order'],
});

const { events, warnings } = report;
const v = version();
console.log(`msgkit ${v.crate_version} (schema ${v.schema_version})`);
console.log(`events:   ${events.length}`);
console.log(`warnings: ${warnings.length}`);
for (const w of warnings) console.log('  -', w);

const counts = new Map();
for (const e of events) {
  const key = `${e.type.padEnd(8)} ${e.direction.padEnd(9)}`;
  counts.set(key, (counts.get(key) ?? 0) + 1);
}
for (const [key, n] of [...counts].sort()) console.log(`  ${key} ${n}`);
if (events.length) {
  console.log('first:', events[0].timestamp, events[0].id);
  console.log('last: ', events.at(-1).timestamp, events.at(-1).id);
}
