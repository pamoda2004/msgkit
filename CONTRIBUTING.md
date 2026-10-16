# Contributing

## Setup

Install a stable Rust toolchain (<https://rustup.rs>). On Windows you also need a linker: Visual Studio Build Tools (MSVC) or MinGW-w64 with the `x86_64-pc-windows-gnu` toolchain.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

All three must pass; CI runs them on Linux, macOS and Windows.

## Language packages

`packages/node`, `packages/python` and `packages/kotlin` are thin wrappers (see each README for build and test commands). Rules of thumb:

* **No logic in wrappers.** Parsing and normalization belong in `msgkit-core`; a wrapper only builds the JSON request, calls `dispatch`, and decodes the result. Fix bugs once, in Rust.
* **Same fixtures everywhere.** Every package tests against `crates/msgkit-core/tests/fixtures/`. Add a fixture once and assert it in all three.
* **Keep versions in lock-step.** Each package's version is checked against the workspace `version` in `Cargo.toml` by its tests; bump them together.
* **When you add an operation** to `api::dispatch`, add one function to each package (and its types/docs).

## Changing the data model

The Rust types in `crates/msgkit-core/src/model.rs` are the contract.

1. Prefer **additive** changes: new optional fields (`#[serde(default)]`) or new enum variants.
2. Regenerate schemas: `UPDATE_SCHEMA=1 cargo test -p msgkit-core --test schema`, and commit the result. Then update the hand-written types in `packages/node/src/index.d.ts`, `packages/python/src/msgkit/types.py` and `packages/kotlin/.../Models.kt`.
3. Bump `SCHEMA_VERSION` (minor for additive, major for breaking) and note it in `CHANGELOG.md`.

## Adding or improving a parser

* Add a **fixture** under `crates/msgkit-core/tests/fixtures/` that reproduces the real-world format (anonymized: never commit real conversations or real phone numbers).
* Add tests for the happy path *and* malformed input. Malformed records must produce warnings, never panics.
* Keep ids deterministic and stable: changing how an id is derived invalidates every consumer's de-duplication, so treat it as a breaking change.
* Document limits in `docs/SUPPORTED_SOURCES.md`.

## Pull requests

* Small, focused, with tests.
* Don't add dependencies lightly; the core is deliberately lean and portable (no I/O, no platform code).
* Out of scope: reading data without the account owner's consent, or using unofficial/ToS-violating access to WhatsApp, Facebook or other platforms.
