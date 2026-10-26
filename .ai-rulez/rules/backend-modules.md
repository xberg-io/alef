---
type: Decision
title: Backend Modules
paths:
  - "src/backends/**"
x-ai-rulez:
  kind: rule
  id: backend-modules
  metadata:
    priority: high
---

Module layout for `src/backends/<lang>/`:

- `mod.rs` — backend struct and `Backend` trait impl
- `gen_bindings/` — one file per concern (`types.rs`, `methods.rs`, `functions.rs`, `enums.rs`, `errors.rs`, `helpers.rs`)
- `trait_bridge.rs` or `trait_bridge/` — trait vtable/bridge generation
- `gen_visitor.rs` or `gen_visitor/` — visitor pattern generation
- `template_env.rs` — minijinja environment and template registration

Adding a target language:

1. Create `src/backends/<lang>/` and add it to `src/backends/mod.rs`.
2. Implement `Backend`; use `crate::codegen` for shared helpers.
3. Set `depends_on_ffi: true` in `BuildConfig` when binding via C FFI (Go, Java, C#).
4. Register it in the CLI backend dispatch table.
