// Browsers, Deno, Bun and edge runtimes have no synchronous file access, so the
// module must be passed to init() explicitly:
//
//   import { init } from '@pamoda2004/msgkit';
//   await init(fetch(new URL('@pamoda2004/msgkit/msgkit.wasm', import.meta.url)));
export function readDefaultWasm() {
  throw new Error(
    "@pamoda2004/msgkit: no default WebAssembly source in this environment; call `await init(source)` first",
  );
}
