# Contributing

## Setup

Install a stable Rust toolchain (<https://rustup.rs>). On Windows you also need a linker: Visual Studio Build Tools (MSVC) or MinGW-w64 with the `x86_64-pc-windows-gnu` toolchain.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

All three must pass; CI runs them on Linux, macOS and Windows.

## Quality gates

Every change must pass the same gates locally and in CI (`.github/workflows/ci.yml`). Warnings are errors.

| Area | Command |
|---|---|
| Repository hygiene (no personal data) | `python scripts/check_repo_hygiene.py` and `python -m unittest discover -s scripts/tests` |
| Rust format, lint (clippy pedantic), tests | `cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace` |
| Rust supply chain | `cargo deny check` · `cargo audit` |
| Rust public API / JSON Schema compatibility | `cargo semver-checks --workspace --baseline-rev baseline-prototype` · `python scripts/schema_compat.py --base-ref <ref>` |
| Coverage (msgkit-core, minimum 90% lines) | `cargo llvm-cov -p msgkit-core` (needs Linux/macOS or an MSVC Windows toolchain) |
| Python | `pip install -r requirements-dev.txt` then `ruff check .` · `ruff format --check .` · `mypy` · `python -m unittest discover -s packages/python/tests` |
| Node.js | in `packages/node`: `npm ci` · `npm run lint` · `npm run typecheck` · `npm test` · `npm audit` |
| Kotlin | in `packages/kotlin`: `./gradlew ktlintCheck detekt :msgkit:test` (`./gradlew ktlintFormat` fixes style) |

Notes:

* The Rust toolchain is pinned in `rust-toolchain.toml`; bump it deliberately, together with `RUST_VERSION` in the workflows. A weekly canary job tries the newest stable release.
* **Never commit real exports or personal data.** The hygiene check fails the build on anything that looks like an export, a database, a personal e-mail address, a home-directory path or a realistic phone number outside the fixture directories. Use synthetic data; phone numbers in tests must look obviously invented (for example `+94771234567`, `+12025550123`).
* Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, `chore:`, `ci:`, `test:`, `refactor:`).
* **`main` is protected** by the `protect-main` ruleset: changes go in through a pull request (no approvals are required yet), force pushes and deletion are blocked, and these checks must pass before a merge: `Rust lint (fmt, clippy pedantic)`, `Rust tests (ubuntu-latest)`, `Rust tests (macos-latest)`, `Rust tests (windows-latest)`, `Rust MSRV (1.88)`, `Coverage (msgkit-core)`, `Python lint (ruff, mypy)`, `API and schema compatibility`, `Repository hygiene (no personal data)` and `Advisories and licences`.
* **Required checks are matched by name.** A pull request that renames or removes one of those jobs (for example the MSRV bump, which changes `Rust MSRV (1.88)`, or pinning `ubuntu-24.04` in a matrix) must update the ruleset in the same change (Settings > Rules > Rulesets > `protect-main`, or `gh api`), and say so in its description; otherwise it waits forever for a check that no longer exists. If CI breaks in a way that blocks every merge, tell the maintainer; do not add bypass actors.
* Commit with GitHub's no-reply e-mail address, not a personal one: `git config user.email "<id>+<username>@users.noreply.github.com"`.

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
