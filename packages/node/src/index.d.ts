// Types mirror the JSON Schemas in `schema/` of the msgkit repository
// (event.schema.json, import-report.schema.json, options.schema.json).

export type Channel = 'sms' | 'cellular' | 'whatsapp' | 'messenger';
export type Direction = 'inbound' | 'outbound' | 'unknown';
export type CallOutcome = 'answered' | 'missed' | 'rejected' | 'blocked' | 'voicemail' | 'unknown';
export type AttachmentKind = 'image' | 'video' | 'audio' | 'file' | 'other';

export interface Participant {
  /** E.164 phone number when known, otherwise a platform id or display name. */
  id: string;
  display_name?: string;
}

export interface Attachment {
  kind: AttachmentKind;
  uri?: string;
  mime_type?: string;
  file_name?: string;
  /** The platform's own media id, used to download the bytes via its API. */
  source_id?: string;
}

export interface Message {
  /** Deterministic: importing the same data twice yields the same id. */
  id: string;
  channel: Channel;
  external_id?: string;
  conversation_id?: string;
  direction: Direction;
  /** ISO 8601, always UTC (e.g. `2024-01-01T00:00:00Z`). */
  timestamp: string;
  sender: Participant;
  recipients: Participant[];
  body?: string;
  attachments: Attachment[];
  /** The untouched source payload (webhook events only). */
  raw?: unknown;
}

export interface Call {
  id: string;
  channel: Channel;
  external_id?: string;
  conversation_id?: string;
  direction: Direction;
  timestamp: string;
  /** Talk time in seconds; 0 for calls that were never answered. */
  duration_secs: number;
  outcome: CallOutcome;
  video: boolean;
  participants: Participant[];
  raw?: unknown;
}

export type MessageEvent = Message & { type: 'message' };
export type CallEvent = Call & { type: 'call' };
export type Event = MessageEvent | CallEvent;

export interface ImportReport {
  events: Event[];
  /** Non-fatal problems, e.g. lines that could not be parsed. */
  warnings: string[];
}

export interface WhatsAppExportOptions {
  /** The exporting user's name as shown in the chat; sets message direction. */
  self_name?: string | null;
  /** Default `auto`: inferred from the data, falling back to day-first with a warning. */
  date_order?: 'auto' | 'dmy' | 'mdy';
  /** Phone's UTC offset in minutes, e.g. 330 for UTC+05:30. Default 0. */
  utc_offset_minutes?: number;
  conversation_id?: string | null;
  /** Calling code (digits, e.g. "94") for national-format phone numbers. */
  default_country_code?: string | null;
}

export interface FacebookExportOptions {
  self_name?: string | null;
  conversation_id?: string | null;
}

/** A row of Android's `Telephony.Sms`. */
export interface RawSms {
  id?: string | null;
  address: string;
  body: string;
  /** Milliseconds since the Unix epoch. */
  date_ms: number;
  /** 1 = inbox, 2 = sent. */
  type: number;
  thread_id?: string | null;
}

/** A row of Android's `CallLog.Calls`. */
export interface RawCall {
  id?: string | null;
  number: string;
  name?: string | null;
  date_ms: number;
  duration_secs?: number;
  /** 1 incoming, 2 outgoing, 3 missed, 4 voicemail, 5 rejected, 6 blocked. */
  type: number;
}

export interface DeviceBatch {
  sms?: RawSms[];
  calls?: RawCall[];
}

export interface DeviceOptions {
  own_number?: string | null;
  default_country_code?: string | null;
}

export interface VersionInfo {
  crate_version: string;
  schema_version: string;
  operations: string[];
}

export type ErrorCode = 'invalid_input' | 'parse_error' | 'unknown_op' | 'storage_error' | 'internal_error';

export class MsgkitError extends Error {
  constructor(code: ErrorCode | (string & {}), message: string);
  readonly name: 'MsgkitError';
  readonly code: ErrorCode | (string & {});
}

/**
 * Loads the WebAssembly module. Optional in Node.js (the bundled module is
 * loaded on first use); required in browsers and other runtimes.
 */
export function init(source?: WebAssembly.Module | Response | Promise<Response> | BufferSource): Promise<void>;
/** Synchronous variant of {@link init}. Node.js only unless `bytes` is given. */
export function initSync(bytes?: BufferSource): void;

/** Low-level entry point: returns the `data` of the response envelope or throws {@link MsgkitError}. */
export function dispatch(op: string, payload?: unknown): any;

export function version(): VersionInfo;
export function parseWhatsAppExport(text: string, options?: WhatsAppExportOptions): ImportReport;
export function parseFacebookExport(thread: object | string, options?: FacebookExportOptions): ImportReport;
export function normalizeDevice(batch: DeviceBatch, options?: DeviceOptions): ImportReport;
/** Does NOT verify the signature: call {@link verifySignature} on the raw body first. */
export function normalizeWhatsAppWebhook(payload: unknown): ImportReport;
/** Does NOT verify the signature: call {@link verifySignature} on the raw body first. */
export function normalizeMessengerWebhook(payload: unknown): ImportReport;
export function verifySignature(args: { secret: string; body: string; signature: string }): boolean;
