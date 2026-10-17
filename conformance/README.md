# Conformance suite

Language-independent test cases that every msgkit binding must pass. The parsing logic exists once (in Rust); this
suite proves that the Python, Node.js (WebAssembly), Kotlin (JNA) and HTTP front ends hand back **exactly the same
bytes** as the Rust core for the same input. CI fails on any difference.

## Layout

```
conformance/cases/<name>/
  case.json       the operation and its input
  <input files>   optional; large or byte-sensitive inputs (CRLF, byte order marks) live in files
  expected.json   the canonical response envelope followed by one LF
```

`case.json`:

```json
{
  "op": "parse_whatsapp_export",
  "input": { "options": { "date_order": "dmy" } },
  "files": { "text": "chat.txt" },
  "json_files": {},
  "note": "what this case shows"
}
```

* `op` is a core operation (see `docs/API.md`).
* `input` is the JSON input of the operation.
* `files` maps an input key to a file whose UTF-8 text becomes that string value (a byte order mark is kept).
* `json_files` maps an input key to a file whose parsed JSON becomes that value.

The case directories are marked `-text` in `.gitattributes`, so git never rewrites their line endings.

## Canonical JSON

The expected file holds the response envelope `{"ok":true,"data":...}` or `{"ok":false,"error":{"code":...,"message":...}}`
in this form, which every runner implements itself so no JSON library's formatting choices leak in:

* compact: no whitespace;
* object keys sorted by Unicode code point;
* strings escape only `"`, `\`, and control characters (`\b \f \n \r \t`, otherwise `\u00xx` in lower case);
  everything else, including non-ASCII text, is written as is, in UTF-8;
* numbers are integers (a float in the output is a test failure);
* one trailing LF.

## Runners

| Binding | Runner | What it exercises |
|---|---|---|
| Rust (reference) | `crates/msgkit-core/tests/conformance.rs` | `api::dispatch` |
| Python | `packages/python/tests/test_conformance.py` | ctypes wheel / native library |
| Node.js | `packages/node/test/conformance.test.mjs` | WebAssembly module |
| Kotlin | `packages/kotlin/msgkit/src/test/kotlin/.../ConformanceTest.kt` | JNA and the native library |
| HTTP | `crates/msgkit-service/tests/conformance.rs` | the three import endpoints (the others have no HTTP endpoint) |

## Changing an expected file

Rust is the reference. After an intentional behaviour change:

```bash
UPDATE_CONFORMANCE=1 cargo test -p msgkit-core --test conformance
git diff conformance/
```

Read the diff before committing it: a changed expected file is a changed contract (ids, timestamps, warnings).
Never generate expected files from a binding other than Rust.

## Adding a case

Create `conformance/cases/<name>/case.json` (and input files), run the update command above, review `expected.json`,
and commit. Cases must use synthetic data only (fictional names, `example.com`, obviously invented phone numbers);
the repository hygiene check rejects anything that looks real.
