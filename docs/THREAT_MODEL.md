# Threat model

This is a STRIDE-style threat model for msgkit 0.2.x. It covers four surfaces: the **parsers** (the Rust core), the **FFI and WebAssembly boundary** (C ABI, WASM, and the Python, Node.js and Kotlin wrappers on top of them), the **HTTP service**, and **storage** (the SQLite store). It records what is protected today, what is not, and which planned change closes each gap.

It is written from the code, not from intentions. Each mitigation says where it lives. A row marked **Gap** is a known weakness, not a claim that it is fine. Nothing here was penetration-tested; the fuzz targets and tests named below are the evidence that exists.

## What we protect

| Asset | Why it matters |
|---|---|
| Message and call content (bodies, names, numbers, attachment names) | Personal data of the user and of third parties who never agreed to anything. The main asset. |
| Event ids | Derived from content ([IDS.md](IDS.md)); a leaked id lets someone confirm a guessed message. Personal data of the same kind. |
| Webhook app secrets, verify tokens, the API bearer token | Whoever holds them can forge deliveries or read and drain the event queue. |
| The host process and machine | A parser or boundary bug must not give an attacker code execution, memory disclosure or a way to take the host down. |
| Availability of the integrator's service | A hostile file or request must not exhaust memory or CPU. |

Non-negotiable rules that act as security controls: the core makes no network calls, reads no files, environment or clock ([purity test](../crates/msgkit-core/tests/purity.rs)); error text never repeats input; only synthetic data is committed (hygiene check in CI).

## Actors and trust

* **The integrator** (your application) calls the library or runs the service. Trusted, but may make mistakes (exposing the port, forgetting a token).
* **A hostile or careless data source.** Export files, device batches and webhook bodies are **untrusted input**. A user may import a file they did not write; a webhook may come from anyone who finds the URL.
* **Another local user or process** on the same machine: can read files and call loopback ports.
* **A network attacker** when the service listens beyond loopback.
* **A malicious dependency or build input** (supply chain): handled in the release work, see "Supply chain" below.

Out of scope: a fully compromised host or root attacker; the security of Meta's and Android's own systems; what an integrator does with data after it leaves msgkit; side channels in the host.

## Trust boundaries

```
 export file / device batch / webhook body            (untrusted bytes)
            |
   [ HTTP service ]  or  [ CLI ]  or  [ Python / Node / Kotlin wrapper ]
            |                                  |
            |                      [ C ABI | WASM ABI ]   <- memory-safety boundary
            +--------------+-------------------+
                           |
                    [ msgkit-core ]  pure: bytes in, events out, no I/O
                           |
              [ EventStore ]  MemoryStore | SqliteStore  -> disk   <- confidentiality boundary
```

## 1. Parsers (msgkit-core)

Input: WhatsApp text exports, Messenger JSON, device batches, webhook JSON. Safe Rust only in the parsers; no `unsafe` in the core's parsing code.

| STRIDE | Threat | Today | Status |
|---|---|---|---|
| **S**poofing | A line pasted into a chat that looks like a message or a system notice creates a fake participant or event (the audit's F03: a notice containing `: `). | Header-shaped lines are matched against known notice forms; a notice with a colon is no longer split into a fake sender. Conformance cases pin it. Real authorship cannot be proven from a text file: an exported chat can always contain a line someone typed. | Partly mitigated; inherent limit; to be added to [SUPPORTED_SOURCES.md](SUPPORTED_SOURCES.md) |
| **T**ampering | A crafted file changes how other lines are read (date order, offset). | Event ids and dates are derived per line; id stability is specified ([IDS.md](IDS.md)). A forged export is indistinguishable from a real one by design. | Accepted risk |
| **R**epudiation | Not applicable to the pure parsers: nothing is logged and nothing is stored. | | n/a |
| **I**nformation disclosure | Error text or warnings quote message content, and hosts log exceptions (F01). | Errors never repeat input; a guard test enforces it for every binding and the service (`describe_json_error`, `error_taxonomy.rs`, conformance `*_is_not_echoed` cases). Warnings count and name nothing (for example the time-zone warning quotes nothing). | Mitigated |
| **D**enial of service | Huge file, very long line, many messages, deeply nested JSON: memory and CPU blow-up. Measured in the audit: about 8 to 37 times the input size in memory; 116 s for 50 MB in Node.js. | Only the HTTP body size limits (64 MiB imports, 2 MiB webhooks). The core has **no limits of its own**. serde_json's own recursion limit (128) bounds JSON nesting. Fuzzing (6 targets, 60 s each in CI) runs with a 2 GiB RSS cap and a 10 s timeout and has found no crash to date. | **Gap (F02)** → Phase 4 PR (b) |
| **E**levation of privilege | Parser bug leads to code execution. | Parsers are safe Rust; fuzz targets and property tests exercise them; 242 of 242 viable mutants are caught by tests. | Low residual risk |

## 2. FFI and WebAssembly boundary

Surfaces: `msgkit_call` / `msgkit_free` (C ABI, used by Python ctypes and Kotlin JNA) and `msgkit_alloc` / `msgkit_call` / `msgkit_dealloc` / `msgkit_last_len` (WASM, used by Node.js).

| STRIDE | Threat | Today | Status |
|---|---|---|---|
| **S** | A wrong library is loaded in place of msgkit's (path hijack). | The Python and Kotlin wrappers first honour the `MSGKIT_LIB` environment variable, then the library bundled in the package; Python then falls back to a `target/release` or `target/debug` directory found relative to its own install path (meant for development checkouts, used only when no bundled library exists). Whoever controls the process environment already controls the process, so this is low risk, but the fallback is a gap worth removing from release builds. Integrity of the package itself is a supply-chain matter. | Low; **Gap** (fallback) → PR (g) |
| **T** | A caller breaks memory rules (double free, wrong pointer). | C ABI: pointers are borrowed for the call, results are freed with `msgkit_free` exactly once; `catch_unwind` turns panics into an `internal_error` envelope. The wrappers hide the pointers from application code. | Mitigated for correct wrappers; a hand-written C caller carries its own risk (documented under "Memory rules") |
| **R** | n/a | | n/a |
| **I** | A freed buffer is read, or returned memory contains earlier content. Results are caller-owned strings; WASM linear memory is never shrunk, so freed pages keep old content until overwritten (F11). | The WASM instance belongs to one process. Content sits in the host's own memory either way. | **Gap (F11)** (memory growth) → PR (b) |
| **D** | A hostile input exhausts the 4 GiB WASM memory or aborts the instance. WASM has no unwinding: a panic traps and the instance is lost (the Node host recreates it). | Panics are contained on the C ABI. On WASM the host recreates the instance. No size limit at the boundary. | **Gap (F02)** → PR (b) |
| **E** | `msgkit_dealloc(ptr, len)` rebuilds a `Box<[u8]>` from the **caller-supplied length**; a wrong length is undefined behaviour (F11). | The shipped Node wrapper always passes the length it received. Another host that passes a wrong length can corrupt memory *inside the WASM sandbox* (it cannot escape it). | **Gap (F11)** → PR (b) |

Unsafe surface is small: two C ABI functions, four WASM functions, all in `msgkit-ffi` and `msgkit-wasm` (`#![deny(unsafe_op_in_unsafe_fn)]`, every block has a `SAFETY` note). No Miri or sanitizer run covers the unsafe code yet (audit F12; fuzzing covers the parsers only) → PR (h).

## 3. HTTP service (msgkit-service)

Endpoints: `/healthz` (no auth); `/v1/*` imports, events, ack, version (bearer token when `MSGKIT_API_TOKEN` is set); `/webhooks/{whatsapp,messenger}` (GET handshake with a verify token, POST with an HMAC-SHA256 signature).

| STRIDE | Threat | Today | Status |
|---|---|---|---|
| **S** | A forged webhook. | A POST is refused unless an app secret is configured and `X-Hub-Signature-256` verifies (HMAC-SHA256, compared in constant time by the `hmac` crate). The handshake compares the verify token without leaking a matching prefix. An unconfigured webhook answers 503 and processes nothing. | Mitigated |
| **S** | Anyone who can reach the port calls `/v1/*`. | Binds to `127.0.0.1` by default. With a non-loopback bind and no token the service starts but logs a warning; **it does not refuse to start**. One shared bearer token, no scopes. | Partly mitigated; **Gap** (F15, Phase 5) |
| **S** | A web page the user visits attacks the loopback service (DNS rebinding or a local cross-origin request) when no token is set. | The `Host` header is not checked and no CORS policy is set; browsers will not hand a cross-origin page the response without CORS headers, but DNS rebinding makes the origin look same-site. Setting `MSGKIT_API_TOKEN` stops it. | **Gap**: recommend a token even on loopback; Host check proposed for Phase 5 |
| **T** | A replayed webhook delivery is processed again, or a body is altered in transit. | HMAC covers the body, so alteration is detected. Replay is rejected only by event-id de-duplication; after the 30-day prune an old delivery is accepted again (F14). No transport security is built in: serve behind TLS. | **Gap (F14)** → PR (f) |
| **R** | "Who read or drained the queue?" | No audit log of API calls; logs carry counts and ids only, never content (by rule). Acknowledge is not attributed to a caller. | Accepted for now; scoped tokens in Phase 5 |
| **I** | Content in logs, errors or health output. | Error bodies are fixed text with a stable `code`; `AppJson` and `AppQuery` replace axum's rejections, which quote the value; storage failures log the cause but clients get "storage error". The log filter defaults to `info`; a `debug` or `trace` filter set by the operator could log more from dependencies, though msgkit itself does not log content. | Mitigated; operators should keep the default `info` level |
| **I** | `GET /v1/events` returns every pending event to anyone with the token; `drain=true` on a GET deletes content. | A state change on a GET is a protocol wart rather than a vulnerability (the token is a header, not a cookie), but it is surprising. | Accepted; document |
| **D** | Large or many concurrent imports; slow clients; unbounded in-memory store. | Body caps (64 MiB, 2 MiB) with a JSON 413 `limit_exceeded`; parsing runs on the blocking pool so the async runtime is not stalled. **No** rate limit, request timeout, concurrency cap or `/readyz`; each import can use many times its body size in memory. The in-memory store has no cap. | **Gap (F02, F15)** → PR (b) limits; Phase 5 service hardening |
| **E** | Request smuggling or path tricks reaching the store. | axum routes are fixed; there is no file path from a request. The store path comes only from the operator's environment. | Low residual risk |

## 4. Storage (SQLite store)

| STRIDE | Threat | Today | Status |
|---|---|---|---|
| **S** | n/a (a local file) | | n/a |
| **T** | Another process or user edits the database; corrupt rows. | The file is created owner-only on Unix (`0600`); **on Windows no ACL is set** (the function is a no-op). WAL mode, `synchronous = FULL`, atomic batches. A corrupt row produces `storage_error` without quoting content. No integrity check beyond SQLite's own. | Partly mitigated; **Gap** on Windows permissions |
| **R** | n/a | | n/a |
| **I** | Content is stored as **plain JSON**; WAL and shared-memory files sit beside it; backups copy it (F13). Acknowledge sets `json = NULL`, but **`secure_delete` is not enabled and the file is never vacuumed**, so freed pages and the write-ahead log can still hold the old text until they are overwritten or checkpointed. `raw` payloads duplicate content. | Documented as "not encrypted" with advice to use disk encryption. | **Gap (F13)** → PR (d) deletion and retention (including `secure_delete` and checkpointing) and PR (e) encryption at rest (your choice) |
| **I** | Event ids kept for 30 days after acknowledge reveal that a message existed and allow guess-and-confirm. | Only the id and timestamps are kept; the retention is a fixed constant. | **Gap (F13)** → PR (d) makes it configurable |
| **D** | Disk full or a huge queue. | Errors become `storage_error` (500), and Meta retries. No cap on stored events. | Accepted; limits in PR (b)/(d) |
| **E** | SQL injection. | All statements are parameterized; no SQL is built from input. | Mitigated |

There is currently **no way to delete everything about one person** and no configurable retention (F13) → PR (d).

## Supply chain

| Threat | Today | Planned |
|---|---|---|
| A malicious or vulnerable dependency | Exact pins for `jiff`/`jiff-tzdb`; Cargo.lock committed; `cargo deny` and `cargo audit` required in CI ("Advisories and licences"); GitHub Actions pinned by commit SHA; Dependabot. | PR (g) SBOM; also enable Dependabot security updates (your call) |
| A tampered release artifact | Nothing is published yet. | PR (g): signed artifacts (Sigstore), build provenance, CycloneDX SBOM |
| Unreported vulnerabilities | **Private vulnerability reporting is not enabled** on the repository (checked 2026-10-20: the GitHub API reports it disabled; it needs your action in Settings or an explicit go-ahead), Dependabot security updates are disabled (version updates run), secret scanning and push protection are enabled; no SECURITY.md yet. | PR (g) |

## Summary of gaps and where they are closed

| Gap | Finding | Planned in |
|---|---|---|
| No limits on file size, line length, message count, nesting depth | F02 | Phase 4 (b) |
| WASM `msgkit_dealloc` trusts the caller's length; memory never shrinks | F11 | Phase 4 (b) |
| No redaction helper for integrators who must mask identifiers | | Phase 4 (c) |
| No retention setting or "delete person X"; deleted text may linger in free pages and the WAL | F13 | Phase 4 (d) |
| Plain-text store | F13 | Phase 4 (e), on your choice |
| Replay accepted after the 30-day prune | F14 | Phase 4 (f), opt-in |
| No SBOM, signatures, provenance, SECURITY.md | | Phase 4 (g) |
| No Miri or sanitizer run over the unsafe code | F12 | Phase 4 (h) |
| One shared token, no rate limit or timeouts, no Host check, no `/readyz`; Windows file permissions | F15 | Phase 5 |

## Keeping this document true

Change this file in the same pull request as any change to a boundary: a new endpoint, a new `unsafe` block, a new place that writes data, a new way to read configuration. A row marked **Gap** is removed (or moved to "Mitigated" with the code and test that prove it) in the pull request that closes it.
