---
priority: high
---

Backend source files must be kept small and split by concern. A file approaching 300 lines should be split; files over 500 lines must be split.

Standard module structure for `alef-backend-*` crates:

- `src/gen_bindings/` — type and function binding generation, one file per concern (types.rs, methods.rs, functions.rs, enums.rs, errors.rs, helpers.rs)
- `src/trait_bridge.rs` or `src/trait_bridge/` — trait vtable/bridge generation
- `src/gen_visitor.rs` or `src/gen_visitor/` — visitor pattern generation
- `src/template_env.rs` — minijinja environment setup and template registration

Functions exceeding 50 lines should be extracted into named helpers. Deeply nested conditional blocks (>3 levels) should be extracted. When a file handles multiple distinct concepts, split it at the concept boundary — not by line count alone.
