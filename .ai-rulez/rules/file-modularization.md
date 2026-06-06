---
priority: high
---

Backend, codegen, e2e generator, and test source files must stay at or below 1,000 lines of
code, including tests. Files approaching 800 lines should be split before more behavior is
added. Split by concern, not by arbitrary line count.

Existing files over 1,000 lines are remediation targets and must not grow except in commits whose
purpose is splitting them. When touching an over-limit file, either split the touched concern
into a smaller module/test file or explicitly keep the change no-growth and preparatory.

Standard module structure for `src/backends/<lang>/`:

- `mod.rs` — module entry, backend struct, `Backend` trait impl
- `gen_bindings/` — type and function binding generation, one file per concern (`types.rs`, `methods.rs`, `functions.rs`, `enums.rs`, `errors.rs`, `helpers.rs`)
- `trait_bridge.rs` or `trait_bridge/` — trait vtable/bridge generation
- `gen_visitor.rs` or `gen_visitor/` — visitor pattern generation
- `template_env.rs` — minijinja environment setup and template registration

Functions exceeding 50 lines should be extracted into named helpers. Deeply nested conditional
blocks (>3 levels) should be extracted. When a file handles multiple distinct concepts, split it
at the concept boundary — not by line count alone. The 1,000-line cap applies to `src/**/*.rs`,
`src/**/*.jinja`, and `tests/**/*.rs`; generated snapshots are excluded.
