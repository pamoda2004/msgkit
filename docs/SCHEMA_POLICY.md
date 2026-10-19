# Schema policy

The JSON data contract is the one thing every binding shares, so it has its own version and its own rules. This page says how that version works, what may change within a major version, and what readers must do. Which items of each binding are stable is in [API_SURFACE.md](API_SURFACE.md).

## What is versioned

* **`SCHEMA_VERSION`** (`major.minor.patch`, currently `1.0.0`) is a constant in `crates/msgkit-core/src/lib.rs`. It is separate from the package versions.
* It is reported at run time by the `version` operation as `schema_version`.
* It is written into every schema file in [`schema/`](../schema/README.md) as the top-level key `x-msgkit-schema-version`. The files are generated from the Rust types and a test fails if a file disagrees with the code, so the marker always matches the constant.
* The contract consists of the three schema files (`event`, `import-report`, `options`), and with them the meaning of every field, enum value and event id. [`openapi.yaml`](../openapi.yaml) refers to the schema files and must stay in step with them.

## Rules within one major version (1.x)

Within `1.x` the contract changes **only additively**.

| Change | Counts as | Version bump |
|---|---|---|
| New optional property, new definition, new enum value, new `oneOf` variant, new optional input option, new operation, new error code | Additive | **minor** (`1.0.0` to `1.1.0`) |
| Wording of descriptions, examples, documentation; no change to shape or meaning | Neutral | patch, or none |
| Removing or renaming a property, definition, enum value or variant | Breaking | **major** |
| Changing a property's type | Breaking | **major** |
| Output: a required property becoming optional | Breaking | **major** |
| Input: a new required property, or an optional one becoming required | Breaking | **major** |
| Changing what a field, value, id or default **means**, even when its shape is unchanged (for example how event ids are derived, or what an option does when absent) | Breaking | **major** |

Removals happen only in a new major version, and only after the item has been marked deprecated (`"deprecated": true` in the schema and a CHANGELOG entry) for at least one minor release.

The last row cannot be detected by a tool: it is a review responsibility, and the [conformance suite](../conformance/README.md) is what pins ids and outputs byte for byte.

## Rules for readers

A program that reads msgkit's JSON (or sends it options) must:

1. **Accept older minor versions.** A reader written for `1.N` accepts any `1.M` with `M <= N`: everything it relies on is still there, with the same meaning.
2. **Tolerate newer minor versions.** Ignore unknown properties. Treat an unknown enum value (a new channel, call outcome or attachment kind) and an unknown error `code` as "other". Do not fail because a field you do not know appeared.
3. **Check the major version.** Compare the major part of `schema_version` (or of `x-msgkit-schema-version`) with the one you were built for. A different major version is a different contract: refuse it or ask the user to upgrade, do not guess.
4. **Not depend on text for people.** Error messages and warning strings are not part of the contract.

msgkit itself follows the matching writer rules: the output of any `1.x` build validates against the `1.0` required properties, new output properties are optional, and a new input option is optional with a default that keeps the old behaviour (so existing callers and existing event ids do not change).

## How the rules are enforced

* **Drift test** (`cargo test -p msgkit-core --test schema`): the schema files must equal what the Rust types generate, including the version marker.
* **`scripts/schema_compat.py`**, run by the CI job "API and schema compatibility" against the pull request's base, classifies every difference as additive or breaking and also requires that
  * a breaking change comes with a higher **major** `SCHEMA_VERSION`;
  * an additive change comes with at least a higher **minor** `SCHEMA_VERSION`;
  * the version never goes backwards;
  * each schema file's marker equals `SCHEMA_VERSION`.
* **`cargo-semver-checks`** guards the Rust API in the same job.
* **Conformance suite**: pins the meaning of the contract, including ids and error codes, across Rust, Python, Node.js, Kotlin and HTTP.

## Making a schema change

1. Change the Rust types.
2. Regenerate: `UPDATE_SCHEMA=1 cargo test -p msgkit-core --test schema`.
3. Bump `SCHEMA_VERSION` as the table above says.
4. Add or update conformance cases, and the CHANGELOG entry (mark breaking changes clearly).
5. Make sure the packages' own versions move at least as far (a schema major bump needs a package major bump; a schema minor bump needs at least a package minor bump).

A breaking schema change needs the project owner's explicit approval before it is merged.

## Before 1.0 of the packages

The schema already follows these rules from `1.0.0`. The packages are still at `0.1.0` and nothing has been published, so no released consumer exists yet; the rules matter from the first publication on, and the project applies them from now so that the first release does not have to.
