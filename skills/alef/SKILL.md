---
name: alef
description: >-
  Generate fully-typed polyglot language bindings for Rust libraries using Alef.
  Use when configuring alef.toml, running alef CLI commands, writing e2e test
  fixtures, debugging binding generation, or setting up CI/CD for multi-language
  Rust libraries. Covers 16 language backends (Python, TypeScript, WASM, Ruby,
  PHP, Go, Java, C#, Kotlin, Elixir, Gleam, R, Swift, Dart, Zig, C), DTO
  styles, trait bridges, adapter patterns, version sync, and pre-commit hooks.
license: MIT
metadata:
  author: kreuzberg-dev
  version: "1.0"
  repository: https://github.com/kreuzberg-dev/alef
---

# Alef Polyglot Binding Generator

Alef generates fully-typed, lint-clean language bindings for Rust libraries across 16 languages from a single TOML config file. It handles the entire pipeline: API extraction, code generation, type stubs, package scaffolding, build orchestration, version sync, and e2e test generation. Trait bridges (foreign-language objects implementing Rust traits) are emitted for all 16 backends.

Use this skill when:

- Configuring `alef.toml` for a new or existing Rust library
- Running alef CLI commands (generate, build, test, verify, e2e)
- Writing or debugging e2e test fixtures (JSON fixtures -> multi-language test suites)
- Adding a new language backend to a project
- Setting up CI/CD pipelines for polyglot Rust libraries
- Debugging binding generation issues (stale bindings, type mismatches, missing types)
- Configuring DTO styles, adapter patterns, or custom FFI bridges
- Deciding what to include/exclude in bindings for a Rust library

## Installation

```bash
# Pre-built binary (fastest)
cargo binstall alef-cli

# From crates.io
cargo install alef-cli

# Via Homebrew
brew install kreuzberg-dev/tap/alef

# From source
git clone https://github.com/kreuzberg-dev/alef.git
cd alef && cargo install --path crates/alef-cli
```

## Quick Start

### 1. Initialize

```bash
cd your-rust-crate
alef init --lang python,node,ruby,go
```

This creates `alef.toml` with your crate's configuration.

### 2. Generate Bindings

```bash
alef generate              # Generate all configured languages (all crates)
alef generate --lang node  # Generate for specific language
alef generate --crate my-library  # Restrict to a single crate
alef generate --clean      # Regenerate everything (ignore cache)
```

When your workspace has multiple crates, use `--crate <name>` (repeatable) to restrict operations to a subset. If omitted, all crates are processed.

### 3. Build

```bash
alef build                 # Build all languages
alef build --lang python   # Build specific (runs maturin)
alef build --release       # Release profile
```

### 4. Test

```bash
alef test                  # Run all language tests
alef test --e2e            # Include e2e tests
alef test --lang python,go # Specific languages
```

### 5. Verify (CI)

```bash
alef verify --exit-code    # Fails if any binding, stub, scaffold, doc, or README is stale
alef diff                  # Show what would change
```

### 6. Publish (release)

```bash
alef publish prepare --target x86_64-unknown-linux-gnu
alef publish build --target x86_64-unknown-linux-gnu --use-cross
alef publish package --output dist
alef publish validate
```

## Minimal Configuration

Alef now uses a multi-crate schema. A `[workspace]` section defines shared defaults; each `[[crates]]` entry describes one independently published binding package:

```toml
[workspace]
languages = ["python", "node", "go", "java"]

[[crates]]
name = "my-library"
sources = ["src/lib.rs", "src/types.rs"]

[crates.output]
python = "crates/my-library-py/src/"
node = "crates/my-library-node/src/"
ffi = "crates/my-library-ffi/src/"

[workspace.tools]
python_package_manager = "uv"      # uv | pip | poetry  (default: uv)
node_package_manager = "pnpm"      # pnpm | npm | yarn  (default: pnpm)

[crates.python]
module_name = "_my_library"

[crates.node]
package_name = "@myorg/my-library"

[workspace.dto]
python = "dataclass"
node = "interface"
```

### Legacy `alef.toml` Migration

If you have an existing `alef.toml` in the old single-crate schema (with top-level `[crate]`, `languages`, etc.), run this migration command:

```bash
alef migrate --write
```

This rewrites your config to the new `[workspace]` + `[[crates]]` layout atomically. The migration handles naming, path updates, and other structural changes automatically. Review the output and fix any path issues if needed, then continue as normal.

`alef.toml` is validated at load time. Custom `[lint|test|build_commands|setup|update|clean].<lang>` tables that override a main command must declare a `precondition`; redundant fields (value identical to the built-in default) emit a `tracing::warn!` so the file stays minimal.

## Supported Languages

| Language           | Framework    | DTO Styles                                       |
| ------------------ | ------------ | ------------------------------------------------ |
| Python             | PyO3         | `dataclass`, `typed-dict`, `pydantic`, `msgspec` |
| TypeScript/Node.js | NAPI-RS      | `interface`, `zod`                               |
| WebAssembly        | wasm-bindgen | --                                               |
| Ruby               | Magnus       | `struct`, `dry-struct`, `data`                   |
| PHP                | ext-php-rs   | `readonly-class`, `array`                        |
| Go                 | cgo + C FFI  | `struct`                                         |
| Java               | Panama FFM   | `record`                                         |
| C#                 | P/Invoke     | `record`                                         |
| Elixir             | Rustler      | `struct`, `typed-struct`                         |
| R                  | extendr      | `list`, `r6`                                     |
| C                  | cbindgen     | --                                               |

## Common Workflows

### Add a New Language

1. Add the language to `languages` array in `alef.toml`
2. Add output directory in `[output]`
3. Add language-specific config section (e.g., `[python]`)
4. Run `alef generate && alef scaffold`

### Update After Changing Rust API

```bash
alef all                   # Full pipeline: generate + stubs + scaffold + readme + docs + e2e (when configured)
alef verify --exit-code    # Or just check what changed
```

### Run E2E Tests

```bash
alef e2e generate          # Generate test suites from fixtures
alef test --e2e            # Run all tests including e2e
```

### Version Bump

```bash
alef sync-versions --bump patch   # Bump patch and sync everywhere
alef sync-versions --set 1.2.3    # Set specific version and sync
alef sync-versions                # Just sync current version
```

## Pre-commit Hooks

Alef provides pre-commit hooks for consumer repos:

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/kreuzberg-dev/alef
    rev: v1.1.11
    hooks:
      - id: alef-verify # Check-only: fails if any output (incl. README) is stale
      # OR
      - id: alef-generate # Auto-regenerate on .rs/.toml change
      # OR
      - id: alef-readme # README-only refresh (template / snippets changes)
```

## Local-install Workflow

When iterating on alef's code generation logic (changes to `alef-codegen`, `alef-backend-*`, or `alef-cli`), install the local binary instead of using the published crates.io version:

```bash
cargo install --path crates/alef-cli --force
```

The resulting `~/.cargo/bin/alef` shadows any crates.io install until replaced. After every meaningful change to the codegen pipeline, reinstall with the same command so the binary picks up your latest changes. Only reinstall from crates.io (via `cargo binstall alef-cli` or `cargo install alef-cli`) after a published release.

## Caching

Alef uses blake3-based content hashing to skip regeneration when inputs haven't changed. The cache lives in `.alef/` (gitignored).

```bash
alef cache status   # Show cache state
alef cache clear    # Force full regeneration next run
```

## Verify (per-file source+output, idempotent across alef versions)

`alef verify` is a **pure read+strip+rehash+compare**. The `alef:hash:<hex>` line in every generated file is a per-file fingerprint of the rust sources and the on-disk byte content:

```text
alef:hash:<hex> = blake3( sources_hash || file_content_without_hash_line )
sources_hash    = blake3( sorted(rust_source_files) )
```

`alef generate` writes whitespace-normalised codegen output and finalises the hash _after_ the optional formatter pass (`--format`) has run, so the on-disk hash always describes the on-disk byte content. `alef verify` reads each alef-headered file, strips the `alef:hash:` line, recomputes the same hash, and compares — no regeneration, no writes. Without `--format`, `alef generate` does not invoke any formatter; if you keep formatters in pre-commit hooks, run `alef fmt` (or `alef generate --format`) before committing so the hash matches the formatted bytes.

The hash deliberately does **not** include the alef CLI version or `alef.toml`. Bumping the alef CLI on a tagged repo does not by itself flag any file as stale; verify only goes red when (a) a crate's `sources` rust file changed, (b) an alef-generated file was edited or mutated by something post-format, or (c) `alef generate` would now produce a different file body. The IR cache (`.alef/<crate>/ir.json`) keys on `sources_hash` alone — pass `--clean` to bust it when the alef extractor itself has changed.

`--lang`, `--compile`, `--lint` flags on verify are accepted for backwards compatibility but ignored — verify is a per-file hash compare. Use `alef build` / `alef lint` / `alef test` for the per-language checks those flags used to imply.

See `references/cli-reference.md#alef-verify` for the full mental model.

## Common Pitfalls

1. **Missing `ffi` language**: Go, Java, and C# require the C FFI layer. Add `ffi` to `languages` or it's implicitly included.
2. **Stale bindings after Rust changes**: Run `alef generate` or `alef all` after modifying your Rust source files.
3. **Wrong DTO style**: Check `[workspace.dto]` or `[crates.dto]`. Python `typed-dict` is read-only, `dataclass` is mutable. Choose based on usage.
4. **Types not appearing**: Check `[exclude]`/`[include]` filters. Use `alef extract -o /dev/stdout | jq` to inspect the IR.
5. **Version mismatch**: Always use `alef sync-versions` instead of manually editing package manifests.
6. **Opaque vs transparent types**: Types with private fields or complex generics need `[crates.opaque_types]` config.

## Additional References

- [Configuration Reference](references/configuration.md) -- Complete `alef.toml` documentation
- [CLI Reference](references/cli-reference.md) -- All commands with flags and examples
- [E2E Testing](references/e2e-testing.md) -- Fixture schema, assertion types, generation
- [Language Backends](references/backends.md) -- Per-language details, DTO styles, limitations
- [Adapter Patterns](references/adapters.md) -- Custom FFI bridging patterns
- [Designing alef.toml](references/designing-alef-toml.md) -- Practical guide for configuring alef.toml with real-world patterns
- [Troubleshooting](references/troubleshooting.md) -- Common errors and fixes
- For cutting and publishing a release, use the `release-procedure` skill — do not improvise.
