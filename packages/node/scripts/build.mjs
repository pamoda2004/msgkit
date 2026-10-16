// Builds the WebAssembly module and copies it to dist/msgkit.wasm.
//
//   node scripts/build.mjs            release build (default)
//
// Requires a Rust toolchain with the wasm32-unknown-unknown target:
//   rustup target add wasm32-unknown-unknown
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '..', '..', '..');
const cargo = process.env.CARGO ?? 'cargo';

const result = spawnSync(
  cargo,
  ['build', '-p', 'msgkit-wasm', '--target', 'wasm32-unknown-unknown', '--release', '--locked'],
  { cwd: repo, stdio: 'inherit' },
);
if (result.status !== 0) {
  console.error('cargo build failed (is the wasm32-unknown-unknown target installed?)');
  process.exit(result.status ?? 1);
}

const built = join(repo, 'target', 'wasm32-unknown-unknown', 'release', 'msgkit_wasm.wasm');
const dist = resolve(here, '..', 'dist');
mkdirSync(dist, { recursive: true });
copyFileSync(built, join(dist, 'msgkit.wasm'));
copyFileSync(join(repo, 'LICENSE'), resolve(here, '..', 'LICENSE'));
console.log(`wrote dist/msgkit.wasm (${(statSync(join(dist, 'msgkit.wasm')).size / 1024).toFixed(0)} KiB)`);
