---
type: Concept
title: Architecture
x-ai-rulez:
  kind: context
  id: architecture
  metadata:
    priority: high
---

# Alef Architecture

Pipeline: `extract → core → codegen → backends::<lang>`, in one root-flat crate `alef` (binary and library). Modules under `src/`:

- `extract/` — parses Rust source into IR (`ApiSurface`) with `syn`
- `core/` — IR types, config schema (`AlefConfig`), `Backend` trait, `AlefError`
- `codegen/` — shared generation utilities: type mapping, naming, generators, Jinja templates
- `backends/<lang>/` — one module per target language, each implements `Backend`
- `bin_cli/` — the binary's command dispatch: `args.rs`, `dispatch.rs::run(cli: Cli)`, per-command handlers
- `cli/` — library support: build cache, crate-filter resolution (`dispatch.rs::select_crates`), git helpers, breaking-change detection, version pinning
- `extensions/` (dylib/template loading), `adapters/` (framework adapters), `docs/`, `readme/`, `scaffold/`, `snippets/`, `e2e/`, `publish/`

- `src/cli/dispatch.rs` only resolves `--crate`; the real command dispatch is `src/bin_cli/dispatch.rs`.
- `src/codegen/naming.rs` is the naming-policy module (host identifiers, wire names, internal Rust names, ABI symbols).
- `generated_header: true` files are overwritten by `alef build`; `generated_header: false` files are written once by `alef scaffold` and user-owned after.
