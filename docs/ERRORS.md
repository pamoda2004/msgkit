# Error codes

One list of stable error codes serves every front end: the Rust core, the Python, Node.js and Kotlin packages, the C and WebAssembly ABIs, and the HTTP service. The code is the contract; the text next to it is for people and may be reworded in any release (it never repeats message content).

## Where the code appears

| Front end | Shape |
|---|---|
| Rust core, C and WebAssembly ABIs | the response envelope `{"ok": false, "error": {"code": "<code>", "message": "<text>"}}`; in Rust, `Error::code()` and the constants in `msgkit_core::error::codes` |
| Python | `MsgkitError.code` |
| Node.js | `MsgkitError.code` |
| Kotlin | `MsgkitException.code` |
| HTTP service | the JSON body `{"error": "<text>", "code": "<code>"}` together with the HTTP status. (The HTTP service does not use the envelope: its `error` field is a plain string, as it always was; `code` is the addition.) |

## The codes

The set only grows. A code is never renamed, and never reused for a different situation; a code that is added does not change what an existing input returns. Callers should treat a code they do not know as a generic failure.

<!-- codes:begin -->
| Code | Meaning | Returned by | HTTP status | Covered by |
|---|---|---|---|---|
| `invalid_input` | The request could not be used: wrong JSON shape or types, a missing field, a bad option value (including an unknown `time_zone`, or a limit of `0`), an unusable query string or content type. | all operations; HTTP | 400, 415, 422 | `conformance/cases/invalid_input_missing_field`, `crates/msgkit-service/tests/error_codes.rs::http_error_responses_carry_stable_codes` |
| `parse_error` | The input is well-formed but is not a supported export or webhook (for example a file that is not a Facebook messages export, or a webhook for the wrong platform). | the Facebook export and webhook operations; HTTP | 400 | `conformance/cases/facebook_not_an_export`, `crates/msgkit-service/tests/error_codes.rs::http_error_responses_carry_stable_codes` |
| `unknown_op` | The operation name is not one of the supported operations (`version().operations`). | the operation dispatcher | none: unknown HTTP routes answer 404 with no body | `conformance/cases/unknown_operation` |
| `storage_error` | A store backend failed: disk full, an unusable or corrupt database. The text carries no stored content. | store backends such as `SqliteStore`; HTTP | 500 | `crates/msgkit-store-sqlite/src/lib.rs::rejects_files_that_are_not_databases` |
| `internal_error` | An unexpected failure inside msgkit or at the native boundary: a panic caught at the C or WebAssembly boundary, or no usable result. A bug if it ever happens. | the C and WebAssembly boundary; the Python and Node.js wrappers; HTTP | 500 | `crates/msgkit-ffi/src/lib.rs::fixed_envelopes_use_taxonomy_codes` |
| `limit_exceeded` | The input is over a configured limit: larger than `max_input_bytes`, a line longer than `max_line_bytes`, more than `max_messages` items, or JSON nested deeper than `max_json_depth` (see [API.md](API.md#input-limits)); over HTTP also a body over the service's cap (2 MiB for webhooks and acknowledgements). Nothing is imported. The text names the limit and, for a line, its number, never content. | the imports and webhook normalizers; HTTP | 413 | `conformance/cases/limit_whatsapp_too_many_messages`, `crates/msgkit-core/tests/limits.rs::whatsapp_input_size_line_length_and_message_count`, `crates/msgkit-service/tests/limits.rs::the_other_limits_are_refused_with_the_same_json_error` |
| `unauthorized` | The bearer token is missing or wrong. | HTTP only | 401 | `crates/msgkit-service/tests/error_codes.rs::http_error_responses_carry_stable_codes` |
| `signature_invalid` | A webhook delivery whose `X-Hub-Signature-256` does not match the body. (The `verify_signature` operation reports a mismatch as `{"valid": false}`, not as an error.) | HTTP only | 401 | `crates/msgkit-service/tests/error_codes.rs::http_error_responses_carry_stable_codes` |
| `verification_failed` | The webhook subscription handshake failed (wrong mode or verify token, or no challenge). | HTTP only | 403 | `crates/msgkit-service/tests/error_codes.rs::http_error_responses_carry_stable_codes` |
| `not_configured` | The webhook is not configured on this service (no app secret). | HTTP only | 503 | `crates/msgkit-service/tests/error_codes.rs::http_error_responses_carry_stable_codes` |
<!-- codes:end -->

"Covered by" names a conformance case (which every library binding must reproduce byte for byte) or a test. `crates/msgkit-core/tests/error_taxonomy.rs` checks that this table, `msgkit_core::error::codes::ALL` and the `code` description in `openapi.yaml` list the same codes, and that every test or case named here exists.

## Why `verify_signature` returns `{"valid": false}` and not an error

A wrong signature is an ordinary answer to the question "does this signature match?", not a failure of the operation. `verify_signature` therefore always succeeds with `{"valid": true}` or `{"valid": false}`, and returns an error only when it could not answer at all (for example a missing field, which is `invalid_input`). This keeps one distinction clear for callers: an error means the call itself was wrong or broke, `valid: false` means the call worked and the delivery should be rejected. The HTTP service, which acts on the answer, turns a mismatch into `401` with the code `signature_invalid`. Decided in Phase 3 (audit finding F10); the shape will not change within 1.x.

## Conformance

* The library codes `invalid_input`, `parse_error`, `unknown_op` and `limit_exceeded` have conformance cases that the Rust, Python, Node.js, Kotlin runners reproduce byte for byte; the HTTP conformance runner also sends the import cases that fail (`facebook_not_an_export`, `facebook_wrong_type_is_not_echoed`, `device_wrong_type_is_not_echoed`) through the service and requires the same `code`.
* `storage_error` and `internal_error` cannot be produced by the dispatcher on purpose, so they are covered by the tests named above.
* The HTTP-only codes are covered by `error_codes.rs`, which pins the status, the code and the text of every error response.

## Compatibility rules

* Existing codes keep their exact meaning, and the same input keeps returning the same code.
* New codes are additive (a minor `SCHEMA_VERSION` is not needed: codes are not part of the JSON Schemas, but the CHANGELOG lists each new code).
* A rename or re-mapping of an existing situation is a breaking change and needs the project owner's approval first.
* A situation that today answers `parse_error` (an input that is not a supported format) keeps answering `parse_error`.

## Open question before 1.0

* **Review a possible `parse_error` re-mapping.** Today every "this input is not a supported format" situation (a file that is not a Facebook messages export, a webhook for the wrong platform) answers `parse_error`, and that must not change without the project owner's approval. Before 1.0, decide whether such inputs should get a dedicated code (for example `unsupported_format`), keeping `parse_error` for malformed content. That would be a re-mapping of existing inputs, so it must be listed old to new, marked breaking in the CHANGELOG, and approved first.
