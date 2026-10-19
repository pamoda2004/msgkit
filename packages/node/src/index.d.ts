// Types mirror the JSON Schemas in `schema/` of the msgkit repository
// (event.schema.json, import-report.schema.json, options.schema.json).

// <generated types: do not edit; run `python scripts/generate_types.py` after changing schema/>
export type Channel = 'sms' | 'cellular' | 'whatsapp' | 'messenger';
export type Direction = 'inbound' | 'outbound' | 'unknown';
export type CallOutcome = 'answered' | 'missed' | 'rejected' | 'blocked' | 'voicemail' | 'unknown';
export type AttachmentKind = 'image' | 'video' | 'audio' | 'file' | 'other';

/** A person or address taking part in a conversation. */
export interface Participant {
  /**
   * Stable identifier. An E.164 phone number (`+94700000000`) when one is known, otherwise the
   * platform's own id or the display name.
   */
  id: string;
  display_name?: string;
}

/** A media item or file belonging to a message. msgkit records references, never the bytes. */
export interface Attachment {
  kind: AttachmentKind;
  /** Location of the media, if the source provided one (URL or export path). */
  uri?: string;
  mime_type?: string;
  file_name?: string;
  /** The platform's own media id, used to download the bytes via its API. */
  source_id?: string;
}

/** A text or media message. */
export interface Message {
  /** Deterministic id: importing the same data twice yields the same id. */
  id: string;
  channel: Channel;
  /** The id the source platform assigned, when it has one. */
  external_id?: string;
  conversation_id?: string;
  direction: Direction;
  /** ISO 8601, always UTC (for example `2024-01-01T00:00:00Z`). */
  timestamp: string;
  sender: Participant;
  recipients: Participant[];
  body?: string;
  attachments: Attachment[];
  /** The untouched source payload (webhook events only), kept so records can be re-parsed later. */
  raw?: unknown;
}

/** A voice or video call. */
export interface Call {
  id: string;
  channel: Channel;
  external_id?: string;
  conversation_id?: string;
  direction: Direction;
  /** ISO 8601, always UTC (for example `2024-01-01T00:00:00Z`). */
  timestamp: string;
  /** Talk time in seconds; `0` for calls that were never answered. */
  duration_secs: number;
  outcome: CallOutcome;
  video: boolean;
  participants: Participant[];
  raw?: unknown;
}

export type MessageEvent = Message & { type: 'message' };
export type CallEvent = Call & { type: 'call' };
export type Event = MessageEvent | CallEvent;

/** The result of an import: the events plus anything worth telling the user. */
export interface ImportReport {
  events: Event[];
  /** Non-fatal problems, e.g. lines that could not be parsed. */
  warnings: string[];
}

export type DateOrder = 'auto' | 'dmy' | 'mdy';

export interface WhatsAppExportOptions {
  /**
   * The exporting user's name as it appears in the chat. Used to decide message direction; without it
   * direction is `unknown`.
   */
  self_name?: string | null;
  /** Default `auto`: inferred from the data, falling back to day-first with a warning. */
  date_order?: DateOrder;
  /**
   * Offset of the phone's local time zone from UTC, in minutes (e.g. `330` for UTC+05:30). Exports
   * carry no time zone. Defaults to `0`. Ignored when `time_zone` is set.
   */
  utc_offset_minutes?: number;
  conversation_id?: string | null;
  /** Calling code (digits, e.g. `"94"`) used to normalize sender names that are national phone numbers. */
  default_country_code?: string | null;
  /**
   * IANA time-zone name of the phone (exact spelling, e.g. `"Asia/Colombo"`). When set it replaces
   * `utc_offset_minutes` and follows the zone's daylight-saving rules; event ids do not depend on it.
   * A time in a repeated hour uses the first occurrence and a time in a skipped hour moves forward by
   * the gap (reported in one warning). The zone database is bundled with msgkit, so results are the
   * same everywhere.
   */
  time_zone?: string | null;
}

export interface FacebookExportOptions {
  /** The exporting user's name, used to decide message direction. */
  self_name?: string | null;
  /** Overrides the conversation id (defaults to the export's `thread_path`). */
  conversation_id?: string | null;
}

/** A row from `Telephony.Sms`. */
export interface RawSms {
  /** `_id` column, if available. Only stable on one device. */
  id?: string | null;
  /** Remote party: a phone number or an alphanumeric sender id. */
  address: string;
  body: string;
  /** Milliseconds since the Unix epoch. */
  date_ms: number;
  /** `type` column: `1` inbox, `2` sent. Other values map to `unknown`. */
  type: number;
  thread_id?: string | null;
}

/** A row from `CallLog.Calls`. */
export interface RawCall {
  id?: string | null;
  number: string;
  /** Cached contact name, if any. */
  name?: string | null;
  date_ms: number;
  duration_secs?: number;
  /** `type` column: `1` incoming, `2` outgoing, `3` missed, `4` voicemail, `5` rejected, `6` blocked. */
  type: number;
}

export interface DeviceBatch {
  sms?: RawSms[];
  calls?: RawCall[];
}

export interface DeviceOptions {
  /** The device owner's own number, recorded as the other party of each message. Optional. */
  own_number?: string | null;
  /** Calling code (digits) for national numbers such as `0700000000`. */
  default_country_code?: string | null;
}

// </generated types>

export interface VersionInfo {
  crate_version: string;
  schema_version: string;
  operations: string[];
  /** Version of the bundled IANA time-zone database (for example `2026e`). */
  tzdata_version: string;
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
