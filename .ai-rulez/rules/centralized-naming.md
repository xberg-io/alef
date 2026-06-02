---
priority: critical
---

All casing and naming transformations must be centralized.

- Public host-language identifiers must be produced through `src/codegen/naming.rs` or a backend helper that delegates to it.
- Do not add backend-local generic casing or serde helpers such as `apply_rename_all`, `apply_serde_rename`, `wire_variant_value`, `snake_to_camel`, or private generic `to_snake_case`.
- `serde_rename` and `serde_rename_all` define wire/JSON names only. They must not be used as host-language public identifier casing rules.
- `serde_rename` always wins over `serde_rename_all`.
- `rename_fields` defines per-language public binding field/property names. Every backend that emits public DTO fields must honor it or document why the backend has no public field surface.
- Keep public host identifiers, wire names, internal generated Rust names, and ABI/native symbols as separate name surfaces.
- C ABI, JNI, FFI, and internal Rust symbols must use explicit ABI/internal helpers, not host-language public naming helpers.
- Any naming or serde behavior change must add table-driven unit tests and relevant backend snapshot/e2e coverage.
