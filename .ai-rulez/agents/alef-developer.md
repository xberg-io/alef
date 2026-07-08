---
name: alef-developer
description: Alef binding generator development and code generation
model: sonnet
---

When working on alef:

1. Alef is a polyglot binding generator -- it reads Rust source via `syn`, builds an IR (`ApiSurface`), and generates type-safe language bindings
2. Alef is a single root-flat crate named `alef` (binary `alef`, library `alef`). All source lives under `src/`
3. Key modules: `src/cli/` (CLI command dispatch), `src/codegen/` (code generation orchestration), `src/core/` (IR types, `Backend` trait, config schema), `src/extract/` (Rust source parsing via syn)
4. Language backends live under `src/backends/<lang>/` — one module per target: `pyo3/`, `napi/`, `magnus/`, `php/`, `wasm/`, `ffi/` (C FFI for Go/Java/C#/Dart/Swift/Kotlin/Zig), `go/`, `java/`, `jni/`, `kotlin/`, `kotlin_android/`, `csharp/`, `dart/`, `swift/`, `zig/`, `gleam/`, `rustler/`, `extendr/`
5. Supporting modules: `src/scaffold/` (project scaffolding), `src/docs/` (documentation generation), `src/readme/` (README generation), `src/adapters/` (adapter utilities), `src/e2e/` (e2e test generation), `src/snippets/` (doc snippet validation), `src/publish/` (publish orchestration)
6. Configuration is in `alef.toml` at the consumer project root -- parsed by `alef::core::config::AlefConfig`
7. When adding a new language backend: implement the `Backend` trait from `alef::core`, register in the CLI dispatch table, add e2e fixtures
8. Generated bindings should be thin wrappers -- all business logic stays in the consumer's Rust core
9. Templates use `minijinja` for code generation
10. Test with `task test`, build with `task build`
