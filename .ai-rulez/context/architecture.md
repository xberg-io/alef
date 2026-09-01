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
- `bin_cli/` — the `alef` binary's actual command dispatch: arg parsing (`args.rs`), the
  top-level `run(cli: Cli)` entry point (`dispatch.rs`), and per-command handlers
  (`core_commands/`, `publish_commands.rs`, `release_commands.rs`, `verify_*`)
- `cli/` — library-side support the binary calls into: build cache (`cache.rs`,
  `cache_identity.rs`, `cache_outputs.rs`), crate-filter resolution (`dispatch.rs::select_crates`
  — not the top-level command dispatcher, despite the filename), git helpers, breaking-change
  detection, version pinning, and the `commands/`/`pipeline/` submodules
- `extensions/` — dylib/template extension loading (`dylib.rs`, `template.rs`)
- `adapters/` — framework-specific adapters (e.g., PyO3 async, NAPI async)
- `docs/` — generates language-native doc comments from Rust rustdoc
- `e2e/` — end-to-end fixture/test generation
- `readme/` — README generation
- `scaffold/` — project scaffolding
- `snippets/` — doc snippet extraction and validation
- `publish/` — release/publish orchestration

**`src/cli/dispatch.rs` and `src/bin_cli/dispatch.rs` are two different files with overlapping
names.** The actual CLI command dispatch (`alef build`, `alef scaffold`, `alef readme`, etc.) is
`bin_cli::dispatch::run`. `cli::dispatch` only resolves the `--crate` filter list.

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

Alef is opinionated: as of 0.82.0, `lint`/`setup`/`update`/`clean`/`build_commands` are no longer
configurable in `alef.toml` at all (`LintConfig`, `SetupConfig`, `UpdateConfig`, `BuildCommandConfig`,
`CleanConfig` are plain internal data carriers returned by `core::config::{lint,setup,update,build,clean}_defaults`,
with no `Deserialize`/schema surface). `TestConfig` is the sole survivor — `test.e2e` has no code
default in any language but Dart, so `[test.<lang>]` stays configurable in `[workspace]`/`[[crates]]`.

Every command config still supports the same two hook fields:

- `precondition: Option<String>` — shell command that must exit 0; skip with warning on failure
- `before: Option<StringOrVec>` — commands run before the main command; abort on failure

Execution order per language: precondition → before → main command(s).

Rust is a first-class language in all pipelines. In `build()`, Rust is always driven by
`build_defaults::default_build_config(Language::Rust, ..)` (not the backend registry, which panics
for `Language::Rust`, and not any `alef.toml` override — there is no `[build_commands.rust]`
anymore).

## Standard Backend Module Layout

Standard module structure for `src/backends/<lang>/` (see `file-modularization` rule for the
1,000-line cap this layout is meant to keep files under):

- `mod.rs` — module entry, backend struct, `Backend` trait impl
- `gen_bindings/` — type and function binding generation, one file per concern (`types.rs`, `methods.rs`, `functions.rs`, `enums.rs`, `errors.rs`, `helpers.rs`)
- `trait_bridge.rs` or `trait_bridge/` — trait vtable/bridge generation
- `gen_visitor.rs` or `gen_visitor/` — visitor pattern generation
- `template_env.rs` — minijinja environment setup and template registration

## Generated vs User-Maintained Boundary

- `generated_header: true` — prepended with `// DO NOT EDIT`; overwritten by `alef build`
- `generated_header: false` — written once by `alef scaffold`; user-owned after that
- Binding glue code and type stubs are generated; package manifests are scaffolded once
