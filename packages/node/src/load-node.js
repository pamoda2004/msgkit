// Node.js loader for the bundled WebAssembly module (selected through the
// "#load" entry in package.json "imports").
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export function readDefaultWasm() {
  return readFileSync(fileURLToPath(new URL('../dist/msgkit.wasm', import.meta.url)));
}
