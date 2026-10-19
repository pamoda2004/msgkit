# JSON Schemas

The language-neutral data contract. **Generated from the Rust types: do not edit by hand.**

| File | Describes |
|---|---|
| `event.schema.json` | `Event` (a `message` or a `call`), the normalized output |
| `import-report.schema.json` | `{events, warnings}`, what every import returns |
| `options.schema.json` | option objects for each operation and the `DeviceBatch` input |

Regenerate after an intentional model change (CI fails if they drift):

```bash
UPDATE_SCHEMA=1 cargo test -p msgkit-core --test schema
```

## Types in the msgkit packages

The TypeScript (`packages/node/src/index.d.ts`), Python (`packages/python/src/msgkit/types.py`) and Kotlin (`Models.kt`) data types are **generated from these schemas** by `python scripts/generate_types.py` (Python standard library only). Never edit the code between the `<generated types ...>` markers by hand: change the Rust types, run `UPDATE_SCHEMA=1 cargo test -p msgkit-core --test schema`, then `python scripts/generate_types.py`. CI runs `python scripts/generate_types.py --check` and fails when a generated region and the schema disagree. The doc comments in the generated types come from the Rust doc comments.

## Generating types for another language

Any JSON Schema code generator works, for example:

```bash
npx quicktype --src-lang schema --lang typescript --src schema/event.schema.json -o event.ts
npx quicktype --src-lang schema --lang kotlin     --src schema/event.schema.json -o Event.kt
npx quicktype --src-lang schema --lang swift      --src schema/event.schema.json -o Event.swift
pip install datamodel-code-generator && datamodel-codegen --input schema/event.schema.json --output event.py
```

Consumers should **ignore unknown fields** so additive (minor) schema changes do not break them.

## Versioning

Every file carries the version of the contract it was generated for in the top-level key `x-msgkit-schema-version` (the same value the `version` operation reports as `schema_version`). Within one major version the schemas change only additively; removals need a new major version. The full rules, for msgkit and for readers, are in [docs/SCHEMA_POLICY.md](../docs/SCHEMA_POLICY.md).
