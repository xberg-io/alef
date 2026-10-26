---
type: Decision
title: Centralized Naming
paths:
  - "src/backends/**"
  - "src/codegen/**"
  - "src/e2e/**"
  - "src/extract/**"
x-ai-rulez:
  kind: rule
  id: centralized-naming
  metadata:
    priority: high
---

- Public host-language identifiers come from `src/codegen/naming.rs` or a backend helper that delegates to it.
- No backend-local generic casing or serde helpers (`apply_rename_all`, `apply_serde_rename`, `wire_variant_value`, `snake_to_camel`, private `to_snake_case`).
- `serde_rename`/`serde_rename_all` define wire/JSON names only, never host identifier casing; `serde_rename` wins over `serde_rename_all`.
- `rename_fields` sets per-language public field names; every backend that emits public DTO fields honors it or documents why it has none.
- Public host identifiers, wire names, internal Rust names and ABI/native symbols are separate name surfaces; C ABI, JNI, FFI and internal symbols use explicit ABI/internal helpers.
- A naming or serde change adds table-driven unit tests plus backend snapshot/e2e coverage.
