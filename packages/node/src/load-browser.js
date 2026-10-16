// Browsers, Deno, Bun and edge runtimes have no synchronous file access, so the
// module must be passed to init() explicitly:
//
//   import { init } from '@msgkit/core';
//   await init(fetch(new URL('@msgkit/core/msgkit.wasm', import.meta.url)));
export function readDefaultWasm() {
  throw new Error(
    "@msgkit/core: no default WebAssembly source in this environment; call `await init(source)` first",
  );
}
