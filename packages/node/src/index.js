// @pamoda2004/msgkit: JavaScript host for the msgkit WebAssembly module.
//
// Every function takes and returns plain JSON-compatible objects whose shape
// is defined by the JSON Schemas in the msgkit repository (`schema/`), so the
// data is identical in every language binding.

import { readDefaultWasm } from '#load';

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

/** Error returned by msgkit. `code` is one of the stable codes in docs/API.md. */
export class MsgkitError extends Error {
  /**
   * @param {string} code
   * @param {string} message
   */
  constructor(code, message) {
    super(message);
    this.name = 'MsgkitError';
    this.code = code;
  }
}

/**
 * The exports of the msgkit WebAssembly module (see crates/msgkit-wasm).
 * @typedef {object} MsgkitExports
 * @property {WebAssembly.Memory} memory
 * @property {(len: number) => number} msgkit_alloc
 * @property {(ptr: number, len: number) => void} msgkit_dealloc
 * @property {(opPtr: number, opLen: number, inPtr: number, inLen: number) => number} msgkit_call
 * @property {() => number} msgkit_last_len
 * @property {() => number} msgkit_version
 */

/** @type {WebAssembly.Module | null} */
let compiled = null;
/** @type {MsgkitExports | null} */
let exportsCache = null;

/** @param {WebAssembly.Module} module */
function instantiateSync(module) {
  const instance = new WebAssembly.Instance(module, {});
  exportsCache = /** @type {MsgkitExports} */ (/** @type {unknown} */ (instance.exports));
}

/** @param {unknown} source */
async function toModule(source) {
  const resolved = await source;
  if (resolved instanceof WebAssembly.Module) return resolved;
  if (typeof Response !== 'undefined' && resolved instanceof Response) {
    if (typeof WebAssembly.compileStreaming === 'function') {
      try {
        return await WebAssembly.compileStreaming(resolved.clone());
      } catch {
        // Wrong MIME type or no streaming support: fall back to bytes.
      }
    }
    return WebAssembly.compile(await resolved.arrayBuffer());
  }
  if (resolved instanceof ArrayBuffer || ArrayBuffer.isView(resolved)) {
    return WebAssembly.compile(/** @type {BufferSource} */ (resolved));
  }
  throw new TypeError('init(): expected a WebAssembly.Module, Response, ArrayBuffer or typed array');
}

/**
 * Loads the WebAssembly module. Optional in Node.js (the bundled module is
 * loaded on first use); required in browsers and other runtimes.
 *
 * @param {WebAssembly.Module | Response | Promise<Response> | BufferSource} [source]
 */
export async function init(source) {
  compiled = await toModule(source ?? readDefaultWasm());
  instantiateSync(compiled);
}

/**
 * Synchronous variant of {@link init}. Node.js only unless `bytes` is given.
 *
 * @param {BufferSource} [bytes]
 */
export function initSync(bytes) {
  compiled = new WebAssembly.Module(bytes ?? readDefaultWasm());
  instantiateSync(compiled);
}

/** @returns {MsgkitExports} */
function wasm() {
  if (exportsCache === null) {
    if (compiled !== null) instantiateSync(compiled);
    else initSync();
  }
  if (exportsCache === null) throw new MsgkitError('internal_error', 'WebAssembly module is not initialised');
  return exportsCache;
}

/**
 * @param {MsgkitExports} x
 * @param {Uint8Array} bytes
 * @returns {number}
 */
function writeBytes(x, bytes) {
  const ptr = x.msgkit_alloc(bytes.length);
  // Re-read `memory.buffer` after every allocation: growing memory detaches old views.
  new Uint8Array(x.memory.buffer, ptr, bytes.length).set(bytes);
  return ptr;
}

/**
 * Low-level entry point: runs `op` with a JSON `payload` and returns the
 * `data` of the response envelope, or throws {@link MsgkitError}.
 *
 * @param {string} op
 * @param {unknown} [payload]
 * @returns {any}
 */
export function dispatch(op, payload = {}) {
  const x = wasm();
  const opBytes = encoder.encode(op);
  const inBytes = encoder.encode(JSON.stringify(payload));
  let opPtr = 0;
  let inPtr = 0;
  let outPtr = 0;
  let outLen = 0;
  let text;
  let trapped = false;
  try {
    opPtr = writeBytes(x, opBytes);
    inPtr = writeBytes(x, inBytes);
    outPtr = x.msgkit_call(opPtr, opBytes.length, inPtr, inBytes.length);
    outLen = x.msgkit_last_len();
    text = decoder.decode(new Uint8Array(x.memory.buffer, outPtr, outLen));
  } catch (e) {
    if (e instanceof WebAssembly.RuntimeError || e instanceof WebAssembly.CompileError) {
      // A trap leaves the module in an unknown state: drop the instance (its
      // memory goes with it) and start from a fresh one on the next call.
      trapped = true;
      exportsCache = null;
      throw new MsgkitError('internal_error', `WebAssembly trap: ${e.message}`);
    }
    throw e;
  } finally {
    if (!trapped) {
      if (opPtr) x.msgkit_dealloc(opPtr, opBytes.length);
      if (inPtr) x.msgkit_dealloc(inPtr, inBytes.length);
      if (outPtr) x.msgkit_dealloc(outPtr, outLen);
    }
  }

  const envelope = JSON.parse(text);
  if (!envelope.ok) throw new MsgkitError(envelope.error.code, envelope.error.message);
  return envelope.data;
}

/** Versions of the library and of the JSON data contract. */
export function version() {
  return dispatch('version');
}

/**
 * Parses a WhatsApp "Export chat" text file.
 * @param {string} text file contents (UTF-8)
 * @param {import('./index.js').WhatsAppExportOptions} [options]
 */
export function parseWhatsAppExport(text, options = {}) {
  return dispatch('parse_whatsapp_export', { text, options });
}

/**
 * Parses one Messenger `message_N.json` file ("Download Your Information").
 * @param {object | string} thread the parsed JSON, or its text
 * @param {import('./index.js').FacebookExportOptions} [options]
 */
export function parseFacebookExport(thread, options = {}) {
  return dispatch('parse_facebook_export', {
    thread: typeof thread === 'string' ? JSON.parse(thread) : thread,
    options,
  });
}

/**
 * Normalizes Android SMS and call-log rows collected on a device.
 * @param {import('./index.js').DeviceBatch} batch
 * @param {import('./index.js').DeviceOptions} [options]
 */
export function normalizeDevice(batch, options = {}) {
  return dispatch('normalize_device', { batch, options });
}

/**
 * Normalizes a WhatsApp Business Cloud API webhook payload. Does NOT verify
 * the signature: call {@link verifySignature} on the raw body first.
 * @param {object} payload
 */
export function normalizeWhatsAppWebhook(payload) {
  return dispatch('normalize_whatsapp_webhook', { payload });
}

/**
 * Normalizes a Messenger Platform webhook payload. Does NOT verify the
 * signature: call {@link verifySignature} on the raw body first.
 * @param {object} payload
 */
export function normalizeMessengerWebhook(payload) {
  return dispatch('normalize_messenger_webhook', { payload });
}

/**
 * Checks an `X-Hub-Signature-256` header against the raw, unparsed request
 * body (HMAC-SHA256 with the app secret, compared in constant time).
 * @param {{ secret: string, body: string, signature: string }} args
 * @returns {boolean}
 */
export function verifySignature({ secret, body, signature }) {
  return dispatch('verify_signature', { secret, body, signature }).valid;
}

/** Test-only helpers. Not part of the public API. */
export const _debug = {
  memoryBytes: () => wasm().memory.buffer.byteLength,
};
