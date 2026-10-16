# msgkit audit (Phase 0)

| | |
|---|---|
| **Date** | 2026-10-16 |
| **Version audited** | 0.1.0 (JSON schema 1.0.0) |
| **Scope** | The whole local working tree, which is **uncommitted** (see section 2) |
| **Method** | Read the sources, docs, workflows and tests; ran every test suite that could run on this machine; probed the parsers with crafted inputs; measured time and memory; inspected the dependency graph |
| **Code changes made** | None. This file is the only addition. |

Everything below is tagged with how it is known: **[ran]** a command produced the evidence, **[read]** found by reading the code, **[claimed]** stated in docs but not proven, **[not run]** could not be run here.

---

## 1. Executive summary

The prototype is **functionally healthy**: every suite that could run here passes, the C ABI, WebAssembly, Python, Node.js and Kotlin bindings return identical results on the shared fixtures, and the core is small and pure. It is **not yet production grade**. The most important gaps:

1. **Error messages leak message content** (F01) [ran]. A malformed Messenger export produced `invalid type: string "SECRET-MESSAGE-TEXT-123"` in the error returned by the CLI, the dispatcher and therefore every binding. Any host that logs exceptions would log personal data, which breaks non-negotiable constraint 4.
2. **No input limits, and large inputs are expensive** (F02) [ran]. A 50 MB export used 387 MB (CLI), 1.86 GB (Python) and 1.87 GB (Node.js/WebAssembly) and took 116 s in Node.js (superlinear). There is no `limit_exceeded` protection.
3. **Parser correctness bugs on realistic input** (F03, F09) [ran]. A group "changed the subject" notice containing `: ` becomes a fake message from a fake participant; unrecognised system notices are dropped silently with no warning; deleted messages are ordinary text.
4. **Ids are not stable across import options** (F05) [ran]. The same file imported with a different `utc_offset_minutes` produces different ids, and a short export and a longer export of the same chat can get different ids *and different dates* because date order is auto-detected per file. This defeats the headline de-duplication promise across re-exports.
5. **CI and release automation have never run** (F04) [read]. The repository has zero commits, and the GitHub repository is public and empty.
6. **Types are hand-written in three languages** (F06) [read], contrary to constraint 3, and nothing checks them against `schema/`.
7. **No guard against real personal data in the repository** (F07) [read], while the GitHub repository is **public**.

Strengths worth keeping: pure core (no I/O, no clock), deterministic ids within one import, signature verification in constant time, tiny `unsafe` surface, honest docs, schema drift test, MSRV verified, no `unwrap` on untrusted input.

---

## 2. Repository and environment state

| Item | Finding |
|---|---|
| Local git | Branch `main`, **0 commits**, no remote, all files untracked. Executable bit on `packages/kotlin/gradlew` is not set (matters for Linux CI). |
| GitHub `pamoda2004/msgkit` | **Exists, public, empty** (API returned 409 "empty repository" for commits; 0 workflow runs) [ran]. Nothing has been pushed, so the audit is of the local tree. |
| Commit identity | `git config` has a personal name and Gmail address. On a public repository these appear in every commit unless a no-reply address is used (F29). |
| Files | 123 non-generated files: 22 Rust, 11 Python, 7 Kotlin, 4 `.mjs`, 3 `.js`, 15 Markdown, 3 PDFs (940 KB, generated). |
| Code size | Rust src: core 1,548, service 368, sqlite 316, cli 154, wasm 118, ffi 103 lines. Rust integration tests: 583 lines (plus in-file unit tests). Node 334, Python 389, Kotlin 634 lines (src and tests). |
| Toolchain used | Windows 11, Rust 1.99.0 (GNU host with MinGW), Node 24.18, Python 3.12, JDK 25 (Android Studio JBR), Gradle 9.6 |
| Tools **not installed** | cargo-audit, cargo-deny, cargo-llvm-cov, cargo-fuzz, cargo-mutants, cargo-semver-checks, cargo-cyclonedx, gh, ruff, mypy, eslint, tsc, ktlint, detekt, pip-audit |

---

## 3. Architecture summary

```
consumers: Rust app | CLI | HTTP clients | C hosts | Node.js | Python | Kotlin/Android
wrappers : msgkit-cli  msgkit-service  msgkit-ffi  msgkit-wasm  ctypes  JNA
                 \________________ api::dispatch(op, json) -> json ________________/
core     : msgkit-core (model, ids, phone, whatsapp_export, facebook, webhook, device, store trait)
optional : msgkit-store-sqlite (durable EventStore)
```

* **Core is pure**: no network, no files, no clock (chrono `clock` feature removed). Its only I/O trait use is `write_ndjson<W: Write>` and the `EventStore` trait, both host-supplied. [read]
* **One entry point**: `api::dispatch` is the only surface the bindings use. Wrappers contain no parsing logic. [read]
* **Contract**: JSON Schemas under `schema/` are generated from the Rust types and guarded by a drift test. [ran]
* **Bindings**: WebAssembly for JS (no imports; memory plus 5 functions), C ABI for Python/Kotlin/Android (3 functions), HTTP service for everything else.
* **Storage**: `EventStore` trait (insert, atomic `insert_all`, list, drain, acknowledge) with `MemoryStore` and SQLite; a shared conformance suite pins behaviour.
* **Workspace**: 6 crates, resolver 2, edition 2021, MSRV 1.85 (verified with 1.85 [ran]).

---

## 4. Verified versus claimed

| Claim | Evidence | Status |
|---|---|---|
| Rust workspace passes tests, fmt, clippy `-D warnings` | `cargo test --workspace` → **77 passed, 0 failed**; `cargo fmt --check` exit 0; `cargo clippy --workspace --all-targets -- -D warnings` exit 0 | **Verified now** |
| Builds on Rust 1.85 | `cargo +1.85 check --workspace --all-targets --locked` | Verified earlier this project, not re-run in Phase 0 |
| Node package works | `node scripts/build.mjs` + `npm test` → **15/15**, wasm 1,421 KiB | **Verified now** |
| Python package works (dev checkout) | `unittest` → **16 passed** | **Verified now** |
| Python wheel works from a clean venv | `test_wheel.py` → exit 0, **16 passed** | **Verified now** |
| Kotlin JVM works | `gradlew :msgkit:cleanTest :msgkit:test --no-build-cache` → **17 passed**, 0 failures | **Verified now** |
| HTTP service (auth, signed webhooks, dedupe, SQLite restart, ack) | 15 service tests inside the Rust run | **Verified now** |
| Android works | 7/7 on an emulator and on a real ARM64 phone | Verified earlier this project; **not re-run** in Phase 0 (no device attached) |
| Identical results in every language | Same fixtures asserted per language; real-chat ids matched across CLI, HTTP, Python, Node.js | Verified by spot check; **no automated byte-for-byte comparison exists** (F18) |
| Linux/macOS support, CI green | Workflows exist; never executed | **Claimed only** |
| Release workflow produces wheels/JARs/AAR/tarball | Never executed | **Claimed only** |
| Messenger export works on real data | Fixture only | **Claimed only** |
| Android 32-bit ARM and x86 libraries | Built and inspected, never executed | **Claimed only** |
| Core never logs content | Core has no logging; service logs ids/counts only, **but error text can contain content** | **Partly false** (F01) |

---

## 5. Test inventory and gaps

| Suite | Count | Notes |
|---|---|---|
| msgkit-core unit | 16 | ids, phone, webhook, store conformance (memory), dispatcher |
| msgkit-core integration | 10 + 14 + 3 | providers, WhatsApp export, schema drift |
| msgkit-ffi | 4 | round trip, null args, version |
| msgkit-service | 15 | HTTP behaviour incl. restart persistence |
| msgkit-store-sqlite | 11 + 1 doc | conformance on memory and disk, corruption, versioning |
| msgkit-wasm | 3 | |
| Node.js | 15 | |
| Python | 16 | |
| Kotlin JVM | 17 | |
| Android (earlier) | 7 | |

**Gaps** (all [read] unless noted):

* No property-based tests, no fuzzing, no mutation testing, no coverage measurement (tools absent).
* `msgkit-cli` has 0 tests. `facebook` and `device` modules have fixture tests only; no negative or boundary tests for oversize, truncated, empty or mixed-encoding input beyond a few cases.
* No automated cross-language conformance runner; each language asserts selected fields, not complete output.
* No large-input, memory or timing tests in CI.
* No test that messages with emoji/Sinhala/Tamil/RTL text survive the Messenger and device paths (only WhatsApp is covered, per language).
* Webhook handling tests use one hand-written payload per platform; no malformed or hostile payloads, no batch with hundreds of entries.
* Android instrumented tests cover the happy path and one error; no memory-pressure or 32-bit run.

---

## 6. Unsafe and FFI surfaces

| Location | Unsafe sites | Notes |
|---|---|---|
| `msgkit-ffi` | 2 `unsafe extern "C"` functions (`msgkit_call`, `msgkit_free`) + 2 unsafe blocks in production code | `catch_unwind` wraps dispatch; null checked; strings must be NUL-terminated UTF-8 (C contract). Double free or a foreign pointer is undefined behaviour (inherent). |
| `msgkit-wasm` | 2 `unsafe extern "C"` functions (`msgkit_call`, `msgkit_dealloc`) + 2 blocks | `msgkit_dealloc(ptr, len)` rebuilds a `Box<[u8]>` from the **caller-supplied length**; a wrong length is undefined behaviour (F11). Panics abort (no unwinding on wasm32), so a trap loses the instance; the Node host recreates it. |
| Other Rust crates | 0 | Workspace lint `unsafe_code = "forbid"` applies; ffi and wasm opt out explicitly. |
| Dependencies | `libsqlite3-sys` (bundled C SQLite), `cc` | The only C code compiled into the Rust build. |
| Production `unwrap`/`expect` | `Regex::new(..).unwrap()` on two static patterns, `HMAC new_from_slice.expect`, `FixedOffset::east_opt(0).expect`, `unreachable!` in the CLI | All on constant inputs; none on untrusted data. |

Not run: Miri/ASan/sanitizers on the unsafe code (no nightly toolchain installed).

---

## 7. Dependency risks

[ran: `cargo tree`, `cargo metadata`]

* 136 unique crates in the workspace graph; `msgkit-core` has 46 transitive crates, `msgkit-wasm` 47.
* One duplicate: `syn` 2.x and 3.x (build-time only).
* Licences: all permissive (MIT, Apache-2.0, Unlicense, BSD-3-Clause, Zlib, Unicode-3.0). One crate offers `MIT OR Apache-2.0 OR LGPL-2.1-or-later` (dual-licensed, MIT chosen). No unlicensed crates. A `cargo deny` policy has not been run.
* `schemars` (JSON Schema derive) is a runtime dependency of `msgkit-core`, so every consumer and the 1.4 MB wasm carry it although it is only needed to generate schemas (F23).
* `rusqlite` with `bundled` compiles C SQLite from source (needs a C compiler on every build host; large attack surface that must be kept patched).
* Python package: zero runtime dependencies. Node package: zero runtime dependencies. Kotlin: `kotlin-stdlib`, `kotlinx-serialization-json 1.9.0`, `jna 5.17.0`.
* Not run: `cargo audit`, `cargo deny`, `npm audit`, `pip-audit`, Dependabot/Renovate (none configured).

---

## 8. Privacy risks

| ID | Risk | Evidence |
|---|---|---|
| F01 | **Error text echoes input content** (serde type errors include the offending value) | [ran] Malformed export → `parse error: not a Facebook messages export: invalid type: string "SECRET-MESSAGE-TEXT-123", expected i64`. Same text returned by the dispatcher to Python/Node/Kotlin/HTTP. |
| F13 | SQLite stores content as plain JSON until acknowledged; `raw` webhook payloads duplicate content; WAL/SHM files; no encryption; no delete-by-contact; retention is a fixed 30-day id window | [read] |
| F14 | Webhook replay protection is only id de-duplication; after the 30-day prune, a replayed delivery is accepted again | [read] |
| F07 | No check preventing real exports or personal data from being committed; repository is public | [read] |
| F29 | Commit author email would be public | [ran] `git config` |
| F22 | Fixture phone numbers (`+94771234567`, `0771234567`) look like plausible real numbers | [ran] repo scan: no real names or numbers from the project owner's data were found; the scan covered the participant names in the owner's private test chat, the owner's name, the Windows user name and local export paths (only two generic example path strings matched) |
| – | Other logging: service logs only counts, booleans, addresses and storage error text; the CLI prints warnings that contain counts and platform ids, not text | [read] |

---

## 9. Performance (measured) and unknowns

[ran] Synthetic WhatsApp export of identical 95-byte lines, release builds, one run each, Windows 11, includes process start-up.

| Input | CLI (NDJSON to null) | Python | Node.js (WebAssembly) | Events |
|---|---|---|---|---|
| 1 MB | 126 ms, 15 MB | not measured | not measured | ~10 k |
| 10 MB | 833 ms, 82 MB | 2,435 ms, 390 MB | 5,417 ms, 397 MB | 99,971 |
| 50 MB | 3,995 ms, 387 MB | 11,318 ms, 1,861 MB | **116,406 ms**, 1,871 MB | 495,660 |

Observations:

* The Rust parser is ~12 MB/s with ~8× memory amplification (every line is cleaned, copied, regex-matched and stored; the ordinal map copies each message body into its key).
* Python and Node.js hold input, the JSON request, the response string and the decoded objects at once, so peak memory is 37–40× the input.
* **Node.js degrades superlinearly** (10 MB → 5 s, 50 MB → 116 s) and approaches the 4 GB limit of 32-bit WebAssembly memory. WebAssembly memory never shrinks inside one instance.
* Unknowns: Kotlin/Android time and memory (not measured), Facebook and webhook parsers at scale, behaviour with very long single lines, and multi-threaded throughput.

---

## 10. Parser correctness probes

[ran] The CLI was run on small crafted inputs.

| ID | Input | Result | Verdict |
|---|---|---|---|
| F03 | `Alice changed the subject from "Plans: 2024" to "New"` (system notice) | One message, **sender `Alice changed the subject from "Plans`**, body `2024" to "New"` | **Bug**: fake participant. Cause: `split_once(": ")` on every line. |
| F09 | `...: This message was deleted` / `You deleted this message` | Ordinary text messages | Not modelled (lifecycle) |
| F09 | `01/03/2024, 11:00 - fake line pasted by user` (header-shaped, no colon) | **Silently dropped**, no warning | Unknown system notices vanish without a count |
| F08 | Localized media placeholder (a Sinhala string used as a stand-in; the real string is unknown) | Treated as text, not an attachment | Only English placeholders and "missed voice/video call" are recognised; real Sinhala/Tamil exports need real samples |
| F19 | `Dr: Perera: hello there` | sender `Dr`, body `Perera: hello there` | Inherently ambiguous; needs an optional participant list |
| – | CRLF line endings with a BOM | Parsed correctly | OK |
| F05 | Same file, `--utc-offset-minutes 0` vs `330` | **Different ids** | Id depends on the offset option |
| F05 | `03/04/2024` alone vs together with `04/25/2024` | Short export → 2024-04-03, long export → 2024-03-04, **different ids and dates** | Auto date order flips per file; the short export only warns |

---

## 11. CI, supply chain and repository hygiene

[read: `.github/workflows/ci.yml`, `release.yml`]

* **Never executed.** Local git has no commits. First-run failures are likely (Windows/macOS specifics, `gradlew` executable bit, Android emulator job, manylinux container steps).
* CI jobs present: lint, test ×3 OS, MSRV, Node 20/22/24, Python 3.9 and 3.13 ×3 OS, Kotlin JVM (JDK 21), Android emulator (API 34). **Missing**: JDK 17, Python 3.10–3.12, coverage, semver and schema-compatibility checks, ruff/mypy/eslint/tsc/ktlint/detekt, `cargo audit`/`deny`, `npm audit`, `pip-audit`, secret/PII scan, Dependabot/Renovate.
* Hygiene: no top-level `permissions:` block, no `concurrency`, actions pinned to tags (not commit SHAs), toolchain is the floating `stable` (`rust-toolchain.toml`), no Gradle caching.
* Release workflow: no SBOM, no signing/provenance (npm `--provenance` is set), no `cargo publish` job, crates lack `repository`/`homepage` metadata; Maven Central deliberately not automated.
* Governance files missing: `CODE_OF_CONDUCT.md`, `MAINTAINERS.md`, issue/PR templates, ADRs, `CODEOWNERS`. `SECURITY.md` names no concrete contact and GitHub private vulnerability reporting is not enabled.
* Three generated PDFs (940 KB) are committed under `docs/`; they contain status statements that will go stale.

---

## 12. Public API, schema and error stability

* The public Rust API is everything `pub` in six crates; nothing is marked unstable. `msgkit_core::store::conformance` is a public test helper in a non-test build (F23).
* Schema files carry no `$id` or version field, and there is no compatibility policy or check. `SCHEMA_VERSION` exists only as a constant.
* Error codes in use: `invalid_input`, `parse_error`, `unknown_op`, `storage_error`, `internal_error`. Requested but absent: `signature_invalid`, `unsupported_format`, `limit_exceeded`. `verify_signature` returns `{"valid": bool}`, not an error.
* Hand-written types in `index.d.ts`, `types.py` and `Models.kt` (F06).

---

## 13. Prioritised findings

**P1 = fix before any public release. P2 = needed for 1.0. P3 = improve when convenient.**

| ID | P | Area | Finding | Proposed phase |
|---|---|---|---|---|
| F01 | P1 | Privacy | Error messages echo input content | 3 (taxonomy) with a redacting error layer; guard test in 2 |
| F02 | P1 | Robustness | No input limits; 8–37× memory amplification; Node.js 116 s at 50 MB | 4 (limits), 7 (streaming) |
| F03 | P1 | Correctness | `: ` inside system notices creates fake participants | 2 |
| F04 | P1 | CI | Workflows never executed; repository has no commits | 1 |
| F05 | P1 | Design | Ids depend on offset/date-order/country-code options; cross-export dedupe fails | Decision needed, then 2/3 |
| F06 | P1 | Contract | Types hand-written; no drift check | 3 |
| F07 | P1 | Privacy | No real-data guard; repository is public | 1 |
| F08 | P2 | Correctness | Only English system strings; Sinhala/Tamil locale exports unsupported | 2 (needs real anonymised samples from you) |
| F09 | P2 | Correctness | Deleted/edited/system notices not modelled; unknown notices dropped silently | 2, 6 |
| F10 | P2 | API | Error taxonomy incomplete; signature check returns bool | 3 |
| F11 | P2 | FFI | wasm `msgkit_dealloc` trusts caller length; memory never shrinks | 4, 7 |
| F12 | P2 | Safety | No Miri/sanitizer/fuzz coverage of unsafe and parsers | 2, 4 |
| F13 | P2 | Privacy | SQLite plaintext, `raw` duplicates content, no delete-by-contact | 4 |
| F14 | P2 | Security | Replay protection limited to id dedupe, lost after 30-day prune | 4 |
| F15 | P2 | Service | No rate limit, timeouts, scoped tokens, `/readyz`; 64 MB bodies | 5 |
| F16 | P2 | Correctness | Fixed UTC offset only; DST/historic offsets wrong on long chats | 2/3 (decision) |
| F17 | P2 | CI | Missing gates: coverage, semver/schema checks, audit/deny, lint for all languages, pinning | 1 |
| F18 | P2 | Tests | No property/fuzz/mutation tests, CLI untested, no byte-identical conformance runner | 2 |
| F20 | P2 | Supply chain | No SBOM, signing, crate metadata; package names unverified | 4, 8 |
| F29 | P2 | Privacy | Public commits would expose a personal email; SECURITY contact undefined | 1 (decision) |
| F19 | P3 | Correctness | Sender names containing `: ` ambiguous | 2 (optional participant list) |
| F21 | P3 | Docs | Generated PDFs committed; status text will go stale | 8 |
| F22 | P3 | Hygiene | Fixture numbers look real | 2 |
| F23 | P3 | API | `schemars` and test helper in the core build | 3 |
| F24 | P3 | Quality | ~75 `clippy::pedantic` warnings (missing `# Errors` docs 17, backticks 14, `must_use` 17, others); no bugs | 1 |
| F25 | P3 | Correctness | Same-minute duplicate ordinals can differ across overlapping partial exports | 2 |
| F26 | P3 | Correctness | A message body line that looks like a header starts a new message | documented limit |
| F28 | P3 | Compat | Node `engines >=18` but only Node 24 verified locally; CI covers 20–24 | 1 |

---

## 14. Decisions I need from you

| # | Decision | Options | Recommendation |
|---|---|---|---|
| D1 | **Id stability (F05)** | (a) keep ids as they are; (b) derive ids from local wall-clock time + sender + body + ordinal and ignore offset/date-order/country code; (c) keep both (`id` stable, `id_v1` legacy) | (b), done **now**: there are no users or stored data yet. It is a one-time breaking change to ids only, not to schema fields. Needs your explicit approval (constraint 7). |
| D2 | **Commit identity** | Use GitHub's no-reply email, or keep the personal one | No-reply email; set before the first commit |
| D3 | **Licence holder** | Your legal name, an organisation, or "msgkit contributors" | Your name or organisation; MIT stays |
| D4 | **Package names/namespaces** | crates.io `msgkit-*`, PyPI `msgkit`, npm `@msgkit/core` (needs the npm org), Maven group id (needs a domain you own or `io.github.pamoda2004`) | Check availability; `io.github.pamoda2004` works for Maven without owning a domain |
| D5 | **Phase 0 tool installs** | Allow me to install cargo-audit/deny/llvm-cov/mutants/fuzz/semver-checks, ruff, mypy, eslint, typescript in Phase 1 | Yes (downloads; I will list each) |
| D6 | **Real localized samples (F08)** | Send me anonymised WhatsApp export lines containing Sinhala/Tamil system strings (media omitted, deleted, missed call) | Yes, with names and numbers replaced |
| D7 | **Time zones (F16)** | Fixed offset only, or IANA zone name option | Add an optional IANA zone (additive), keep the offset option |

---

## 15. Proposed phase plan (adjusted to the findings)

The structure you gave still holds. Changes I recommend:

* **Phase 1** gains a "Phase 1a" before anything else: first commit, no-reply identity, exec bit, `.gitattributes` check, PII guard (F07), enabling private vulnerability reporting, pinned actions. Then the full CI matrix. Installing the gate tools needs your answer to D5.
* **Pull forward** three correctness/privacy fixes because they affect every later phase: **F01** (redacted errors), **F03/F09** (system-notice handling and a warning count), **F05** (id design, after D1). They land in Phase 2 together with the conformance suite so each has a golden test.
* **Phase 3** also adds the id-derivation specification and the new error codes; error redaction becomes an invariant tested by a property test (no input substring appears in any error string).
* **Phase 4** now includes the memory/limit work (F02) and the wasm length-trust fix (F11) because both are safety issues, not only performance.
* **Phase 7** (streaming and budgets) needs a baseline: the numbers in section 9 become the first benchmark targets.
* **Phase 6** (lifecycle events, providers, threads, extraction, signals, SLA) adds personal-data processing and behaviour classification. I recommend an ADR and a privacy review before writing code, `msgkit-signals` as strictly opt-in with explainable reasons, and no default enabling in any host crate. All schema changes stay additive.
* **Constraints on tooling**: `cargo-fuzz` needs nightly and runs on Linux/macOS, so fuzz targets run in CI rather than on this Windows machine. `cargo-semver-checks` needs a published or tagged baseline, so Phase 1 tags the first commit as the baseline.

| Phase | Outcome |
|---|---|
| 1a | Clean first commit and push; PII guard; hygiene |
| 1 | CI green on all OSes with all gates |
| 2 | Conformance suite, golden/property/fuzz/mutation tests; F01 test guard, F03, F05, F08, F09 fixed |
| 3 | Generated types, schema policy and checks, full error taxonomy, changelog discipline |
| 4 | Threat model, limits, encryption at rest (option choice), retention/deletion, redaction crate, SBOM and signing |
| 5 | Docker, observability, hardening, PostgreSQL, push delivery, contract tests |
| 6 | Lifecycle and optional crates, each behind an ADR |
| 7 | Benchmarks, streaming, budgets |
| 8 | Docs, ADRs, governance, dry-run release, then your confirmation to publish |

---

## 16. What was not verified in Phase 0

* Anything on macOS, Linux or in GitHub Actions.
* The Android suites in this phase (no device attached); the 32-bit ARM and x86 libraries have never been executed.
* Real Facebook exports and real localized WhatsApp exports.
* `cargo audit`, `cargo deny`, coverage, fuzzing, mutation testing, semver checks, SBOM generation, Miri/sanitizers, ruff, mypy, eslint, `tsc`, ktlint, detekt, `pip-audit`, `npm audit` (tools not installed; none of these results are claimed).
* Kotlin/Android performance, Facebook and webhook parser performance, and multi-threaded throughput.
