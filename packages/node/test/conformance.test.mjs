// Runs the language-independent conformance suite (conformance/) through the WebAssembly binding.
// The expected files hold the canonical response envelope; every binding must reproduce them byte for byte.
// See conformance/README.md for the case format and the canonical JSON rules.
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import * as msgkit from '../src/index.js';

const CASES = fileURLToPath(new URL('../../../conformance/cases/', import.meta.url));

const SHORT = new Map([
  ['"', '\\"'],
  ['\\', '\\\\'],
  ['\b', '\\b'],
  ['\f', '\\f'],
  ['\n', '\\n'],
  ['\r', '\\r'],
  ['\t', '\\t'],
]);

function canonicalString(value) {
  let out = '"';
  for (const char of value) {
    const code = char.codePointAt(0);
    if (SHORT.has(char)) out += SHORT.get(char);
    else if (code < 0x20) out += `\\u${code.toString(16).padStart(4, '0')}`;
    else out += char;
  }
  return `${out}"`;
}

// Order by Unicode code point (not by UTF-16 code unit, which is JavaScript's default).
function compareCodePoints(a, b) {
  const x = Array.from(a, (c) => c.codePointAt(0));
  const y = Array.from(b, (c) => c.codePointAt(0));
  for (let i = 0; i < Math.min(x.length, y.length); i++) {
    if (x[i] !== y[i]) return x[i] - y[i];
  }
  return x.length - y.length;
}

/** Compact JSON with keys sorted by code point and minimal string escapes (integers only). */
function canonical(value) {
  if (value === null) return 'null';
  if (typeof value === 'boolean') return value ? 'true' : 'false';
  if (typeof value === 'number') {
    assert.ok(Number.isInteger(value), `conformance output must not contain floats: ${value}`);
    return String(value);
  }
  if (typeof value === 'string') return canonicalString(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  const keys = Object.keys(value).sort(compareCodePoints);
  return `{${keys.map((k) => `${canonicalString(k)}:${canonical(value[k])}`).join(',')}}`;
}

function loadInput(dir, spec) {
  const payload = { ...spec.input };
  for (const [key, name] of Object.entries(spec.files ?? {})) payload[key] = readFileSync(`${dir}/${name}`, 'utf8');
  for (const [key, name] of Object.entries(spec.json_files ?? {})) {
    payload[key] = JSON.parse(readFileSync(`${dir}/${name}`, 'utf8'));
  }
  return payload;
}

function envelope(op, payload) {
  try {
    return { ok: true, data: msgkit.dispatch(op, payload) };
  } catch (e) {
    if (e instanceof msgkit.MsgkitError) return { ok: false, error: { code: e.code, message: e.message } };
    throw e;
  }
}

test('every conformance case is reproduced byte for byte', () => {
  const names = readdirSync(CASES)
    .filter((n) => statSync(`${CASES}${n}`).isDirectory())
    .sort();
  assert.ok(names.length >= 40, `expected the full suite, found ${names.length} cases`);
  for (const name of names) {
    const dir = `${CASES}${name}`;
    const spec = JSON.parse(readFileSync(`${dir}/case.json`, 'utf8'));
    const actual = `${canonical(envelope(spec.op, loadInput(dir, spec)))}\n`;
    const expected = readFileSync(`${dir}/expected.json`, 'utf8');
    assert.equal(actual, expected, `case ${name}`);
  }
});
