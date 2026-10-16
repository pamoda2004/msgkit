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

## Generating types for your language

Any JSON Schema code generator works, for example:

```bash
npx quicktype --src-lang schema --lang typescript --src schema/event.schema.json -o event.ts
npx quicktype --src-lang schema --lang kotlin     --src schema/event.schema.json -o Event.kt
npx quicktype --src-lang schema --lang swift      --src schema/event.schema.json -o Event.swift
pip install datamodel-code-generator && datamodel-codegen --input schema/event.schema.json --output event.py
```

Consumers should **ignore unknown fields** so additive (minor) schema changes do not break them.
