# Architecture decision records

alef's decision log is [`adrs/`](../adrs/), in MADR 4 as adrlint generates it; ADR 0000 records the convention.
Organization-wide decisions live in the
[org decision log](https://github.com/xberg-io/agent-conventions/blob/main/docs/adr-index.md) and are cited as
`org ADR-NNNN` instead of restated. This index lives outside `adrs/` because adrlint lints every Markdown file in the
log; add a row in the same change as a new record.

| ADR | Title | Status | Date |
|-----|-------|--------|------|
| [0000](../adrs/0000-record-architecture-decisions.md) | Record architecture decisions | accepted | 2026-10-30 |
| [0001](../adrs/0001-extract-the-rust-api-into-an-ir-with-syn.md) | Extract the Rust API into an IR with syn | accepted | 2026-04-01 |
| [0002](../adrs/0002-structured-values-cross-the-c-ffi-as-serde-json.md) | Structured values cross the C FFI as serde JSON | accepted | 2026-04-01 |
| [0003](../adrs/0003-file-ownership-by-an-embedded-alef-marker.md) | File ownership by an embedded alef marker | accepted | 2026-04-26 |
| [0004](../adrs/0004-generated-code-goes-through-minijinja-templates.md) | Generated code goes through minijinja templates | accepted | 2026-05-08 |
| [0005](../adrs/0005-one-root-crate-for-the-generator.md) | One root crate for the generator | accepted | 2026-05-23 |
| [0006](../adrs/0006-a-project-agnostic-generator.md) | A project-agnostic generator | accepted | 2026-05-27 |
| [0007](../adrs/0007-generational-handles-across-the-c-abi.md) | Generational handles across the C ABI | accepted | 2026-08-13 |
