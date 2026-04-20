---
description: Alef binding generator development and code generation
model: haiku
name: alef-developer
---

When working on alef:

1. Alef is a polyglot binding generator -- it reads Rust source via `syn`, builds an IR (`ApiSurface`), and generates type-safe language bindings
2. Key crates: `alef-cli` (CLI entry point), `alef-codegen` (code generation orchestration), `alef-core` (IR types, `Backend` trait, config schema), `alef-extract` (Rust source parsing via syn)
3. Language backends: `alef-backend-pyo3`, `alef-backend-napi`, `alef-backend-magnus`, `alef-backend-php`, `alef-backend-wasm`, `alef-backend-ffi` (C FFI for Go/Java/C#), `alef-backend-go`, `alef-backend-java`, `alef-backend-csharp`, `alef-backend-rustler`, `alef-backend-extendr`
4. Supporting crates: `alef-scaffold` (project scaffolding), `alef-docs` (documentation generation), `alef-readme` (README generation), `alef-adapters` (adapter utilities), `alef-e2e` (e2e test generation)
5. Configuration is in `alef.toml` at the consumer project root -- parsed by `alef-core::config::AlefConfig`
6. When adding a new language backend: implement the `Backend` trait from `alef-core`, register in the CLI, add e2e fixtures
7. Generated bindings should be thin wrappers -- all business logic stays in the consumer's Rust core
8. Templates use `minijinja` for code generation
9. Test with `task test`, build with `task build`
