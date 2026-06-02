---
priority: high
---

# Alef Architecture

## Code Generation Pipeline

`extract → core → codegen → backends::<lang>`

The entire pipeline lives in a single root-flat crate named `alef` (binary `alef`, library `alef`). Modules under `src/`:

- `extract/` — parses Rust source into IR (`ApiSurface`); uses `syn` for AST traversal
- `core/` — IR types (`ApiSurface`), config schema (`AlefConfig`), `Backend` trait, `AlefError`
- `codegen/` — shared generation utilities: type mapping, naming, struct/enum/function generators, Jinja templates
- `backends/<lang>/` — one module per target language; each implements `Backend` trait
- `cli/` — command dispatch for `alef build`, `alef scaffold`, `alef readme`
- `adapters/` — framework-specific adapters (e.g., PyO3 async, NAPI async)
- `docs/` — generates language-native doc comments from Rust rustdoc
- `e2e/` — end-to-end fixture/test generation
- `readme/` — README generation
- `scaffold/` — project scaffolding
- `snippets/` — doc snippet extraction and validation
- `publish/` — release/publish orchestration

`src/main.rs` is the binary entry point; `src/lib.rs` re-exports library surface.

`src/codegen/naming.rs` is the canonical naming-policy module for public host-language identifiers,
serde/wire names, internal generated Rust names, and ABI/native symbols.

## Adding a New Target Language

1. Create `src/backends/<lang>/` module (one file per concern: `mod.rs`, `gen_bindings.rs`, etc.)
2. Add the module to `src/backends/mod.rs`
3. Implement `Backend` trait; use `crate::codegen` for shared helpers
4. Set `depends_on_ffi: true` in `BuildConfig` if binding via C FFI (Go, Java, C#)
5. Register in the CLI's backend dispatch table (`src/cli/`)

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
