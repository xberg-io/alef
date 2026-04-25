---
priority: high
---

# Alef Architecture

## Code Generation Pipeline

`alef-extract` → `alef-core` → `alef-codegen` → `alef-backend-*` → `alef-cli`

- `alef-extract` — parses Rust source into IR (`ApiSurface`); uses `syn` for AST traversal
- `alef-core` — IR types (`ApiSurface`), config schema (`AlefConfig`), `Backend` trait, `AlefError`
- `alef-codegen` — shared generation utilities: type mapping, naming, struct/enum/function generators, Jinja templates
- `alef-backend-*` — one crate per target language; each implements `Backend` trait
- `alef-cli` — entry point: `alef build`, `alef scaffold`, `alef readme`
- `alef-adapters` — framework-specific adapters (e.g., PyO3 async, NAPI async)
- `alef-docs` — generates language-native doc comments from Rust rustdoc
- `alef-e2e` — end-to-end integration tests

## Adding a New Target Language

1. Create `crates/alef-backend-{lang}` crate
2. Add to workspace `Cargo.toml` members and `[workspace.dependencies]`
3. Implement `Backend` trait; use `alef-codegen` for shared helpers
4. Set `depends_on_ffi: true` in `BuildConfig` if binding via C FFI (Go, Java, C#)
5. Register in `alef-cli`'s backend dispatch table

## Pipeline Hooks

All command configs (`LintConfig`, `TestConfig`, `SetupConfig`, `UpdateConfig`, `BuildCommandConfig`, `CleanConfig`) support:

- `precondition: Option<String>` — shell command that must exit 0; skip with warning on failure
- `before: Option<StringOrVec>` — commands run before the main command; abort on failure

Execution order per language: precondition → before → main command(s).

Rust is a first-class language in all pipelines. In `build()`, Rust is handled via configurable `[build_commands.rust]` (not the backend registry, which panics for `Language::Rust`).

## Generated vs User-Maintained Boundary

- `generated_header: true` — prepended with `// DO NOT EDIT`; overwritten by `alef build`
- `generated_header: false` — written once by `alef scaffold`; user-owned after that
- Binding glue code and type stubs are generated; package manifests are scaffolded once
