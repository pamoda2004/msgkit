// Compile-time checks for the public typings (run with `npm run typecheck`). Nothing here executes.
import {
  MsgkitError,
  normalizeDevice,
  normalizeMessengerWebhook,
  normalizeWhatsAppWebhook,
  parseFacebookExport,
  parseWhatsAppExport,
  verifySignature,
  version,
  type Event,
  type ImportReport,
  type Message,
} from '../src/index.js';

const report: ImportReport = parseWhatsAppExport('', { self_name: 'A', utc_offset_minutes: 330, date_order: 'mdy' });
const first: Event | undefined = report.events[0];
if (first?.type === 'message') {
  const m: Message = first;
  const body: string | undefined = m.body;
  const direction: 'inbound' | 'outbound' | 'unknown' = m.direction;
  void [body, direction];
} else if (first?.type === 'call') {
  const seconds: number = first.duration_secs;
  void seconds;
}

const fb: ImportReport = parseFacebookExport('{}', { self_name: null });
const dev: ImportReport = normalizeDevice({ sms: [{ address: 'x', body: 'y', date_ms: 1, type: 1 }] });
const wa: ImportReport = normalizeWhatsAppWebhook({});
const ms: ImportReport = normalizeMessengerWebhook({});
const valid: boolean = verifySignature({ secret: 's', body: 'b', signature: 'sha256=00' });
const schema: string = version().schema_version;
void [fb, dev, wa, ms, valid, schema];

try {
  parseWhatsAppExport('');
} catch (e) {
  if (e instanceof MsgkitError) {
    const code: string = e.code;
    void code;
  }
}

// @ts-expect-error: date_order only accepts the documented values
parseWhatsAppExport('', { date_order: 'sideways' });
// @ts-expect-error: verifySignature requires all three fields
verifySignature({ secret: 's' });
