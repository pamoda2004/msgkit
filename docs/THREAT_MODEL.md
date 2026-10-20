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
| **D**enial of service | Huge file, very long line, many messages, deeply nested JSON: memory and CPU blow-up. Measured in the audit: about 8 to 37 times the input size in memory; 116 s for 50 MB in Node.js. | Every import checks `Limits` before parsing (input size 32 MiB, line 256 KiB, 1,000,000 messages, JSON depth 32 by default; `limit_exceeded`, nothing imported): `crates/msgkit-core/tests/limits.rs` and the `limit_*` conformance cases, which every binding reproduces. serde_json's own recursion limit (128) still bounds parsing. At the 32 MiB default an import can still use about 1.2 GB in Python or Node.js (the audit's 37 times factor) and, in Node.js, take roughly a minute (extrapolated from the audit's 5 s at 10 MB and 116 s at 50 MB; not measured at 32 MiB). Fuzzing (6 targets, 60 s each in CI) runs with a 2 GiB RSS cap and a 10 s timeout and has found no crash to date. | Mitigated; the cost at the limit is still high, so integrators can lower it. Streaming is Phase 7 |
| **E**levation of privilege | Parser bug leads to code execution. | Parsers are safe Rust; fuzz targets and property tests exercise them; 242 of 242 viable mutants are caught by tests. | Low residual risk |

## 2. FFI and WebAssembly boundary

Surfaces: `msgkit_call` / `msgkit_free` (C ABI, used by Python ctypes and Kotlin JNA) and `msgkit_alloc` / `msgkit_call` / `msgkit_dealloc` / `msgkit_last_len` (WASM, used by Node.js).

| STRIDE | Threat | Today | Status |
|---|---|---|---|
| **S** | A wrong library is loaded in place of msgkit's (path hijack). | The Python and Kotlin wrappers first honour the `MSGKIT_LIB` environment variable, then the library bundled in the package; Python then falls back to a `target/release` or `target/debug` directory found relative to its own install path (meant for development checkouts, used only when no bundled library exists). Whoever controls the process environment already controls the process, so this is low risk, but the fallback is a gap worth removing from release builds. Integrity of the package itself is a supply-chain matter. | Low; **Gap** (fallback) → PR (g) |
| **T** | A caller breaks memory rules (double free, wrong pointer). | C ABI: pointers are borrowed for the call, results are freed with `msgkit_free` exactly once; `catch_unwind` turns panics into an `internal_error` envelope. The wrappers hide the pointers from application code. | Mitigated for correct wrappers; a hand-written C caller carries its own risk (documented under "Memory rules") |
| **R** | n/a | | n/a |
| **I** | A freed buffer is read, or returned memory contains earlier content. Results are caller-owned strings; WASM linear memory is never shrunk, so freed pages keep old content until overwritten. | The WASM instance belongs to one process. Content sits in the host's own memory either way. A double free or a stale pointer can no longer free a live buffer (see the next rows). | Accepted: the host owns the memory |
| **D** | A hostile input exhausts the 4 GiB WASM memory or aborts the instance. WASM has no unwinding: a panic traps and the instance is lost (the Node host recreates it). | Panics are contained on the C ABI. On WASM the host recreates the instance. The input limits above apply, and `msgkit_alloc` refuses a single buffer over 512 MiB (the Node wrapper reports `limit_exceeded`). WebAssembly memory still never shrinks inside one instance. | Mitigated; residual memory growth is Phase 7 |
| **E** | `msgkit_dealloc(ptr, len)` rebuilt a `Box<[u8]>` from the **caller-supplied length**; a wrong length was undefined behaviour (F11). | The module records every buffer it hands out. `msgkit_dealloc` ignores the length and any pointer that is not the start of a live buffer; `msgkit_call` reads only ranges inside live buffers (tests in `crates/msgkit-wasm/src/lib.rs`: wrong length, double free, foreign and interior pointers, ranges past the end). The exported signatures are unchanged. | Mitigated |

Unsafe surface is small: two C ABI functions, four WASM functions, all in `msgkit-ffi` and `msgkit-wasm` (`#![deny(unsafe_op_in_unsafe_fn)]`, every block has a `SAFETY` note). No Miri or sanitizer run covers the unsafe code yet (audit F12; fuzzing covers the parsers only) → PR (h).

## 3. HTTP service (msgkit-service)

Endpoints: `/healthz` (no auth); `/v1/*` imports, events, ack, version (bearer token when `MSGKIT_API_TOKEN` is set); `/webhooks/{whatsapp,messenger}` (GET handshake with a verify token, POST with an HMAC-SHA256 signature).

| STRIDE | Threat | Today | Status |
|---|---|---|---|
| **S** | A forged webhook. | A POST is refused unless an app secret is configured and `X-Hub-Signature-256` verifies (HMAC-SHA256, compared in constant time by the `hmac` crate). The handshake compares the verify token without leaking a matching prefix. An unconfigured webhook answers 503 and processes nothing. | Mitigated |
| **S** | Anyone who can reach the port calls `/v1/*`. | Binds to `127.0.0.1` by default. A non-loopback `MSGKIT_BIND` without `MSGKIT_API_TOKEN` is refused at startup, with no override (`crates/msgkit-service/tests/host_check.rs::a_non_loopback_bind_needs_a_token` and `the_binary_refuses_to_start_on_a_non_loopback_address_without_a_token`). One shared bearer token, no scopes. | Mitigated for exposure by misconfiguration; **Gap** (F15): one shared token, no scopes or rate limit, Phase 5 |
| **S** | A web page the user visits attacks the loopback service (DNS rebinding or a local cross-origin request) when no token is set. | Without a token, `/v1/*` refuses any `Host` that is not `localhost`, `*.localhost`, a loopback address or in `MSGKIT_ALLOWED_HOSTS` (403 `host_not_allowed`; `host_check.rs`). Requests without a `Host` header are let through (a browser always sends one). No CORS headers are set, so browsers will not hand a cross-origin page the response. With a token set the host is not checked: a page cannot know the token. | Mitigated |
| **T** | A replayed webhook delivery is processed again, or a body is altered in transit. | HMAC covers the body, so alteration is detected. Replay is rejected only by event-id de-duplication; after the 30-day prune an old delivery is accepted again (F14). No transport security is built in: serve behind TLS. | **Gap (F14)** → PR (f) |
| **R** | "Who read or drained the queue?" | No audit log of API calls; logs carry counts and ids only, never content (by rule). Acknowledge is not attributed to a caller. | Accepted for now; scoped tokens in Phase 5 |
| **I** | Content in logs, errors or health output. | Error bodies are fixed text with a stable `code`; `AppJson` and `AppQuery` replace axum's rejections, which quote the value; storage failures log the cause but clients get "storage error". The log filter defaults to `info`; a `debug` or `trace` filter set by the operator could log more from dependencies, though msgkit itself does not log content. | Mitigated; operators should keep the default `info` level |
| **I** | `GET /v1/events` returns every pending event to anyone with the token; `drain=true` on a GET deletes content. | A state change on a GET is a protocol wart rather than a vulnerability (the token is a header, not a cookie), but it is surprising. | Accepted; document |
| **D** | Large or many concurrent imports; slow clients; unbounded in-memory store. | Body cap of 32 MiB by default (2 MiB for webhooks), set by the operator only through `MSGKIT_MAX_*` and never by a request, plus the other input limits, answered with a JSON 413 `limit_exceeded` (`crates/msgkit-service/tests/limits.rs`); parsing runs on the blocking pool so the async runtime is not stalled. **No** rate limit, request timeout, concurrency cap or `/readyz`; each import can use many times its body size in memory. The in-memory store has no cap. | Partly mitigated: limits done in (b); **Gap** (F15) rate limit, timeouts, concurrency cap → Phase 5 |
| **E** | Request smuggling or path tricks reaching the store. | axum routes are fixed; there is no file path from a request. The store path comes only from the operator's environment. | Low residual risk |

## 4. Storage (SQLite store)

| STRIDE | Threat | Today | Status |
|---|---|---|---|
| **S** | n/a (a local file) | | n/a |
| **T** | Another process or user edits the database; corrupt rows. | The file is created owner-only on Unix (`0600`); **on Windows no ACL is set** (the function is a no-op). WAL mode, `synchronous = FULL`, atomic batches. A corrupt row produces `storage_error` without quoting content. No integrity check beyond SQLite's own. | Partly mitigated; **Gap** on Windows permissions |
| **R** | n/a | | n/a |
| **I** | Content is stored as **plain JSON**; WAL and shared-memory files sit beside it; backups copy it (F13). Acknowledge sets `json = NULL`. `raw` payloads duplicate content. | `secure_delete` is **on by default**, so deleted content is overwritten with zeros; erase, expire and prune end with `wal_checkpoint(TRUNCATE)` (acknowledge and drain do not, to stay cheap; the service's maintenance pass, at start-up and every six hours, truncates the log whether or not it deleted anything, so acknowledged text stays in the log for about six hours at most; a library user calls `checkpoint()`), and `compact()` adds `VACUUM`. Tests read the database file and its `-wal` file and find none of the erased text (`erased_text_is_gone_from_the_file_and_the_log`, `acknowledged_text_is_gone_from_the_file_too`). Documented as "not encrypted" with advice to use disk encryption, and that the database must not live in a cloud-synced folder. Erasure cannot reach backups, snapshots, sync-client copies or flash wear levelling. A second connection without `secure_delete` (an older msgkit) does not overwrite. | Mitigated for the live file. Confidentiality at rest: **optional SQLCipher** (`sqlcipher` feature; PR e), off by default, otherwise disk or volume encryption (see [ENCRYPTION.md](ENCRYPTION.md)) |
| **I** | A stolen, copied, backed-up or synced database file is read. | With the `sqlcipher` feature and a key (`MSGKIT_DB_KEY_FILE`, or the less safe `MSGKIT_DB_KEY`) the whole file, its write-ahead log and free pages are AES-256 encrypted (a test reads the bytes: no text, sender or id). **Fail closed:** a key with a plain file or a build without SQLCipher, and an encrypted file without a key, stop the service; the key is never logged and `Debug` shows `<redacted>`. `msgkit store encrypt` converts an existing database once (new file, verified, then the plain file is overwritten and deleted). A startup warning names a cloud-synced folder. **Losing the key loses the data; the key must be backed up separately.** The key is only as safe as where it is kept (the process environment, a file); the process memory and the plain backups made earlier are outside this protection. Tested in CI on Linux only. | Mitigated when enabled; **default builds are unencrypted**; no key rotation yet |
| **I** | Event ids kept for 30 days after acknowledge reveal that a message existed and allow guess-and-confirm. | Only the id and timestamps are kept. An id is derived from sender and text, so it is **pseudonymous personal data**. It is forgotten 30 days after the event was acknowledged, erased or expired (`DEFAULT_RETENTION`), by `prune_acknowledged`, which the service runs at start-up and every six hours; the window is configurable (`MSGKIT_ID_RETENTION_DAYS`). `erase_person` can drop the ids of the events it erases (`drop_ids`); the ids of events acknowledged earlier cannot be tied to a person and leave only by age. | Mitigated; the window is a trade-off against replay (F14) |
| **I** | Data about one person has to be deleted on request (F13). | `EraseStore::erase_person` (library) and `msgkit store erase` (command line) erase every held event in which the person is sender, recipient or call participant, matching any spelling of a phone number with the import's `default_country_code`; results are counts. `RetentionStore::expire_pending` / `MSGKIT_PENDING_MAX_AGE_DAYS` erase events nobody collected. Tests: `crates/msgkit-core/src/store.rs` (`run_erase`) and `crates/msgkit-store-sqlite/src/lib.rs`. | Mitigated |
| **I/E** | The erase endpoint becomes a way to delete (or probe for) someone's data, or a place where addresses leak. | `POST /v1/store/erase` needs the bearer token and is off (503 `not_configured`) without one, even on loopback. The address is only in the JSON body (4 KiB cap, 256 characters), never in the URL; unknown fields, including a country code, are refused; the answer and the one log line carry counts, a random request id and a UTC time, never the address (`crates/msgkit-service/tests/erase.rs`, `erase_log.rs` read the output). Errors do not echo the body. A caller with the token can still erase anyone: the token is the authority, and the request id in the log is the audit trail. The endpoint answers `erased: 0` for an unknown person, so it tells a token holder whether a person had held events. | Mitigated; no rate limit yet (Phase 5) |
| **D** | Disk full or a huge queue. | Errors become `storage_error` (500), and Meta retries. No cap on stored events. | Accepted; limits in PR (b)/(d) |
| **E** | SQL injection. | All statements are parameterized; no SQL is built from input. | Mitigated |

Deleting one person and configurable retention exist since PR (d). They cover the live database only (see above).

## Supply chain

| Threat | Today | Planned |
|---|---|---|
| A malicious or vulnerable dependency | Exact pins for `jiff`/`jiff-tzdb`; Cargo.lock committed; `cargo deny` and `cargo audit` required in CI ("Advisories and licences"); GitHub Actions pinned by commit SHA; Dependabot. | PR (g) SBOM; also enable Dependabot security updates (your call) |
| A tampered release artifact | Nothing is published yet. | PR (g): signed artifacts (Sigstore), build provenance, CycloneDX SBOM |
| Unreported vulnerabilities | **Private vulnerability reporting is not enabled** on the repository (checked 2026-10-20: the GitHub API reports it disabled; it needs your action in Settings or an explicit go-ahead), Dependabot security updates are disabled (version updates run), secret scanning and push protection are enabled; no SECURITY.md yet. | PR (g) |

## Summary of gaps and where they are closed

| Gap | Finding | Planned in |
|---|---|---|
| ~~No limits on file size, line length, message count, nesting depth~~ closed in Phase 4 (b) | F02 | done |
| ~~WASM `msgkit_dealloc` trusts the caller's length~~ closed in Phase 4 (b); memory still never shrinks inside one instance | F11 | Phase 7 (streaming) |
| No redaction helper for integrators who must mask identifiers | | Phase 4 (c) |
| ~~No retention setting or "delete person X"; deleted text may linger in free pages and the WAL~~ closed in Phase 4 (d); backups and sync copies are outside msgkit | F13 | done |
| ~~Plain-text store~~ optional SQLCipher in Phase 4 (e); default builds stay plain; no key rotation, no OS secret store | F13 | backlog |
| Replay accepted after the 30-day prune | F14 | Phase 4 (f), opt-in |
| No SBOM, signatures, provenance, SECURITY.md | | Phase 4 (g) |
| No Miri or sanitizer run over the unsafe code | F12 | Phase 4 (h) |
| One shared token (no scopes), no rate limit or timeouts, no `/readyz`; Windows file permissions | F15 | Phase 5 |

## Keeping this document true

Change this file in the same pull request as any change to a boundary: a new endpoint, a new `unsafe` block, a new place that writes data, a new way to read configuration. A row marked **Gap** is removed (or moved to "Mitigated" with the code and test that prove it) in the pull request that closes it.
