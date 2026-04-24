<div align="center">

<img width="100%" alt="kreuzberg.dev banner" src="https://github.com/user-attachments/assets/1b6c6ad7-3b6d-4171-b1c9-f2026cc9deb8" />

<div style="display: flex; gap: 8px; justify-content: center; flex-wrap: wrap; margin-top: 16px;">

<a href="https://crates.io/crates/alef-cli">
  <img src="https://img.shields.io/crates/v/alef-cli?color=007ec6" alt="crates.io">
</a>
<a href="https://github.com/kreuzberg-dev/alef/actions/workflows/ci.yml">
  <img src="https://img.shields.io/github/actions/workflow/status/kreuzberg-dev/alef/ci.yml?label=CI&color=007ec6" alt="CI">
</a>
<a href="https://github.com/kreuzberg-dev/alef/blob/main/LICENSE">
  <img src="https://img.shields.io/badge/License-MIT-007ec6" alt="License">
</a>

</div>

<br>

<a href="https://discord.gg/xt9WY3GnKR">
  <img height="22" src="https://img.shields.io/badge/Discord-Join%20our%20community-7289da?logo=discord&logoColor=white" alt="Discord">
</a>

</div>

# Alef

Generate fully-typed, lint-clean language bindings for Rust libraries across 11 languages. Alef handles the entire pipeline -- API extraction, code generation, type stubs, package scaffolding, build orchestration, version sync, and e2e test generation -- from a single TOML config file.

## Key Features

- **API extraction** -- Parses your Rust crate's public API via `syn` into a language-agnostic intermediate representation. Handles `pub use` re-exports across workspace crates, `#[cfg(feature)]` gating, serde `rename_all` metadata, doc comments, `Default` detection, and transparent newtype resolution.
- **11 language backends** -- Each backend generates idiomatic, lint-clean binding code using the target language's native framework. See the [supported languages](#supported-languages) table below.
- **Configurable DTO styles** -- Choose how types are represented in each language: Python `dataclass` vs `TypedDict` vs `pydantic` vs `msgspec`, TypeScript `interface` vs `zod`, Ruby `Struct` vs `dry-struct` vs `Data`, and more. Input and output types can use different styles.
- **Type stubs** -- Generates `.pyi` (Python), `.rbs` (Ruby), and `.d.ts` (TypeScript) type definition files for editor support and static analysis in consuming projects.
- **Package scaffolding** -- Generates complete package manifests for each language: `pyproject.toml`, `package.json`, `.gemspec`, `composer.json`, `mix.exs`, `go.mod`, `pom.xml`, `.csproj`, `DESCRIPTION` (R), and `Cargo.toml` for binding crates.
- **E2E test generation** -- Write test fixtures as JSON, get complete runnable test suites for all configured languages. Supports 40+ assertion types, field path aliases, per-language overrides, and skip conditions.
- **Adapter patterns** -- Define custom FFI bridging patterns in config: `sync_function`, `async_method`, `callback_bridge`, `streaming`, and `server_lifecycle`. Alef generates the glue code for each backend.
- **Version sync** -- Propagates the version from `Cargo.toml` to all package manifests, C headers, pkg-config files, and any custom file via regex text replacements.
- **Build orchestration** -- Wraps `maturin`, `napi`, `wasm-pack`, and `cargo`+`cbindgen` with post-processing steps (e.g., patching `.d.ts` files for `verbatimModuleSyntax` compatibility).
- **Visitor FFI** -- Generates a 40-method visitor callback interface via C FFI, enabling the visitor pattern in Go, Java, C#, and other FFI-based languages.
- **Caching** -- blake3-based content hashing skips regeneration when source and config haven't changed. `alef verify` checks staleness in CI.

## Supported Languages

| Language | Framework | Package Format | Test Framework | DTO Styles |
|----------|-----------|----------------|----------------|------------|
| Python | [PyO3](https://pyo3.rs) | PyPI (.whl) | pytest | `dataclass`, `typed-dict`, `pydantic`, `msgspec` |
| TypeScript/Node.js | [NAPI-RS](https://napi.rs) | npm | vitest | `interface`, `zod` |
| WebAssembly | [wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/) | npm | vitest | -- |
| Ruby | [Magnus](https://github.com/matsadler/magnus) | RubyGems (.gem) | RSpec | `struct`, `dry-struct`, `data` |
| PHP | [ext-php-rs](https://github.com/davidcole1340/ext-php-rs) | Composer | PHPUnit | `readonly-class`, `array` |
| Go | cgo + C FFI | Go modules | go test | `struct` |
| Java | [Panama FFM](https://openjdk.org/jeps/454) | Maven (.jar) | JUnit | `record` |
| C# | P/Invoke | NuGet (.nupkg) | xUnit | `record` |
| Elixir | [Rustler](https://github.com/rusterlium/rustler) | Hex | ExUnit | `struct`, `typed-struct` |
| R | [extendr](https://extendr.github.io/extendr/) | CRAN | testthat | `list`, `r6` |
| C | [cbindgen](https://github.com/mozilla/cbindgen) | Header (.h) | -- | -- |

## Quick Start

### Install

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

### Initialize

```bash
cd your-rust-crate
alef init --lang python,node,ruby,go
```

This creates `alef.toml` with your crate's configuration.

### Generate Bindings

```bash
alef generate              # Generate all configured languages
alef generate --lang node  # Generate for specific language
alef generate --clean      # Regenerate everything (ignore cache)
```

### Build

```bash
alef build                 # Build all languages
alef build --lang node     # Build Node.js (runs napi build + patches .d.ts)
alef build --release       # Release profile
```

### Test

```bash
alef test                  # Run all language tests
alef test --e2e            # Include e2e tests
alef test --lang python,go # Specific languages
```

## Commands

| Command | Description |
|---------|-------------|
| `alef init` | Initialize `alef.toml` for your crate |
| `alef extract` | Extract API surface from Rust source into IR JSON |
| `alef generate` | Generate language bindings from IR |
| `alef stubs` | Generate type stubs (`.pyi`, `.rbs`, `.d.ts`) |
| `alef scaffold` | Generate package manifests (`pyproject.toml`, `package.json`, etc.) |
| `alef readme` | Generate per-language README files |
| `alef build` | Build bindings with native tools (`maturin`, `napi`, `wasm-pack`, etc.) |
| `alef test` | Run per-language test suites (`--e2e` for e2e tests, `--coverage` for coverage) |
| `alef lint` | Run configured linters on generated output |
| `alef fmt` | Run only the format phase of lint |
| `alef update` | Update dependencies per language (`--latest` for aggressive upgrades) |
| `alef setup` | Install dependencies per language |
| `alef clean` | Clean build artifacts per language |
| `alef sync-versions` | Sync version from `Cargo.toml` to all manifests (use `--set <version>` to override) |
| `alef verify` | Check if bindings are up-to-date (CI mode with `--exit-code`) |
| `alef diff` | Show what would change without writing |
| `alef all` | Run full pipeline: generate + stubs + scaffold + readme + e2e + docs + sync |
| `alef e2e` | Generate e2e test projects from JSON fixtures |
| `alef cache` | Manage build cache |

## Configuration-Driven Defaults

All operational commands (`lint`, `fmt`, `update`, `setup`, `clean`, `build`, `test`) ship with built-in per-language defaults. No configuration is required to use them — run `alef lint` or `alef update` out of the box. Override any default by adding the corresponding section to `alef.toml`:

```toml
# Override Node lint commands
[lint.node]
format = "oxfmt packages/node/src/"
check = "oxlint packages/node/src/"

# Add coverage support for Python tests
[test.python]
command = "pytest packages/python/tests/"
coverage = "pytest --cov packages/python/tests/ --cov-report=xml"

# Override dependency update commands
[update.python]
update = "uv sync"
upgrade = "uv sync -U"

# Override setup/clean
[setup.node]
install = "pnpm install --frozen-lockfile"

[clean.node]
clean = "rm -rf node_modules dist"

# Override build commands
[build_commands.python]
build = "maturin develop"
build_release = "maturin build --release"
```

All command fields accept either a single string or an array of strings.

Node and WASM scaffolding uses the [Oxc](https://oxc.rs) toolchain: `oxfmt` for formatting and `oxlint` for linting. Biome is no longer included in generated Node/WASM projects.

## Configuration Reference

Alef is configured via `alef.toml` in your project root. Run `alef init` to generate a starter config.

### Minimal Example

```toml
[crate]
name = "my-library"
sources = ["src/lib.rs", "src/types.rs"]

languages = ["python", "node", "go", "java"]

[output]
python = "crates/my-library-py/src/"
node = "crates/my-library-node/src/"
ffi = "crates/my-library-ffi/src/"

[python]
module_name = "_my_library"

[node]
package_name = "@myorg/my-library"

[dto]
python = "dataclass"
node = "interface"
```

### `[crate]` -- Source Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | *required* | Rust crate name |
| `sources` | string[] | *required* | Rust source files to extract |
| `version_from` | string | `"Cargo.toml"` | File to read version from (supports workspace Cargo.toml) |
| `core_import` | string | `{name}` with `-` replaced by `_` | Import path for the core crate in generated bindings |
| `workspace_root` | string | -- | Workspace root for resolving `pub use` re-exports from sibling crates |
| `skip_core_import` | bool | `false` | Skip adding `use {core_import};` to generated bindings |
| `features` | string[] | `[]` | Cargo features treated as always-present (`#[cfg(feature)]` fields are included) |
| `path_mappings` | map | `{}` | Rewrite extracted Rust path prefixes (e.g., `{ "spikard" = "spikard_http" }`) |

### `languages` -- Target Languages

Top-level array of languages to generate bindings for:

```toml
languages = ["python", "node", "ruby", "php", "elixir", "wasm", "ffi", "go", "java", "csharp", "r"]
```

The `ffi` language generates the C FFI layer required by `go`, `java`, and `csharp`. If you enable any of those three, `ffi` is implicitly included.

### `[exclude]` / `[include]` -- Filtering

```toml
[exclude]
types = ["InternalHelper"]
functions = ["deprecated_fn"]
methods = ["MyType.internal_method"]   # dot-notation: "TypeName.method_name"

[include]
types = ["PublicApi", "Config"]        # whitelist only these types
functions = ["extract", "parse"]       # whitelist only these functions
```

### `[output]` -- Output Directories

Per-language output directories for generated Rust binding code:

```toml
[output]
python = "crates/{name}-py/src/"
node = "crates/{name}-node/src/"
ruby = "crates/{name}-rb/src/"
php = "crates/{name}-php/src/"
elixir = "crates/{name}-rustler/src/"
wasm = "crates/{name}-wasm/src/"
ffi = "crates/{name}-ffi/src/"
go = "packages/go/"
java = "packages/java/src/main/java/"
csharp = "packages/csharp/"
r = "crates/{name}-extendr/src/"
```

The `{name}` placeholder is replaced with the crate name.

### Language-Specific Sections

<details>
<summary><strong>[python]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module_name` | string | `_{name}` | Python module name (the native extension name) |
| `async_runtime` | string | -- | Async runtime spec for `pyo3_async_runtimes` |
| `stubs.output` | string | -- | Output directory for `.pyi` stub files |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[node]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `package_name` | string | `{name}` | npm package name |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[ruby]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `gem_name` | string | `{name}` with `_` | Ruby gem name |
| `stubs.output` | string | -- | Output directory for `.rbs` type stubs |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[php]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `extension_name` | string | `{name}` with `_` | PHP extension name |
| `feature_gate` | string | `"extension-module"` | Feature gate wrapping all generated code |
| `stubs.output` | string | -- | Output directory for PHP facades/stubs |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[elixir]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `app_name` | string | `{name}` with `_` | Elixir application name |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[wasm]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `exclude_functions` | string[] | `[]` | Functions to exclude from WASM bindings |
| `exclude_types` | string[] | `[]` | Types to exclude from WASM bindings |
| `type_overrides` | map | `{}` | Override types (e.g., `{ "DOMNode" = "JsValue" }`) |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[ffi]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `prefix` | string | `{name}` with `_` | C symbol prefix for all exported functions |
| `error_style` | string | `"last_error"` | Error reporting convention |
| `header_name` | string | `{prefix}.h` | Generated C header filename |
| `lib_name` | string | `{prefix}_ffi` | Native library name (for Go/Java/C# linking) |
| `visitor_callbacks` | bool | `false` | Generate visitor/callback FFI support |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[go]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `module` | string | `github.com/kreuzberg-dev/{name}` | Go module path |
| `package_name` | string | derived from module path | Go package name |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[java]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `package` | string | `dev.kreuzberg` | Java package name |
| `ffi_style` | string | `"panama"` | FFI binding style (Panama Foreign Function & Memory API) |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[csharp]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `namespace` | string | PascalCase of `{name}` | C# namespace |
| `target_framework` | string | -- | Target framework version |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

<details>
<summary><strong>[r]</strong></summary>

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `package_name` | string | `{name}` | R package name |
| `features` | string[] | inherits `[crate] features` | Per-language Cargo feature override |

</details>

### `[dto]` -- Type Generation Styles

Controls how Rust structs are represented in each language's public API:

```toml
[dto]
python = "dataclass"         # dataclass | typed-dict | pydantic | msgspec
python_output = "typed-dict"  # separate style for return types (optional)
node = "interface"           # interface | zod
ruby = "struct"              # struct | dry-struct | data
php = "readonly-class"       # readonly-class | array
elixir = "struct"            # struct | typed-struct
go = "struct"                # struct
java = "record"              # record
csharp = "record"            # record
r = "list"                   # list | r6
```

### `[scaffold]` -- Package Metadata

Metadata used when generating package manifests:

```toml
[scaffold]
description = "My library for doing things"
license = "MIT"
repository = "https://github.com/org/repo"
homepage = "https://docs.example.com"
authors = ["Your Name"]
keywords = ["parsing", "extraction"]
```

### `[[adapters]]` -- Custom FFI Adapters

Define custom binding patterns that alef can't extract automatically:

```toml
[[adapters]]
name = "convert"
pattern = "sync_function"
core_path = "my_crate::convert"
params = [
  { name = "input", type = "String" },
  { name = "options", type = "Options", optional = true },
]
returns = "Result"
error_type = "ConvertError"
gil_release = true    # Python: release GIL during call
```

Supported patterns: `sync_function`, `async_method`, `callback_bridge`, `streaming`, `server_lifecycle`.

### `[generate]` / `[generate_overrides.<lang>]` -- Generation Control

Toggle individual generation passes (all default to `true`):

```toml
[generate]
bindings = true          # struct wrappers, From impls, module init
errors = true            # error type hierarchies from thiserror enums
configs = true           # config builder constructors from Default types
async_wrappers = true    # async/sync function pairs with runtime management
type_conversions = true  # recursive type marshaling helpers
package_metadata = true  # package manifests (pyproject.toml, package.json, etc.)
public_api = true        # idiomatic public API wrappers

# Override per language:
[generate_overrides.wasm]
async_wrappers = false
```

### `[sync]` -- Version Synchronization

Configure the `alef sync-versions` command (use `--set <version>` to override the version read from `Cargo.toml`):

```toml
[sync]
extra_paths = ["packages/go/go.mod"]

[[sync.text_replacements]]
path = "crates/*/cbindgen.toml"
search = 'header = ".*"'
replace = 'header = "/* v{version} */"'
```

### `[test.<lang>]` / `[lint.<lang>]` / `[update.<lang>]` / `[setup.<lang>]` / `[clean.<lang>]` / `[build_commands.<lang>]`

Override per-language commands for any operational task. All fields accept a string or an array of strings.

```toml
[test.python]
command = "pytest packages/python/tests/"
e2e = "cd e2e/python && pytest"
coverage = "pytest --cov packages/python/tests/"

[test.node]
command = "npx vitest run"

[lint.python]
format = "ruff format packages/python/"
check = "ruff check packages/python/"
typecheck = "mypy packages/python/"

[update.node]
update = "pnpm up"
upgrade = "pnpm up --latest"

[setup.python]
install = "uv sync"

[clean.rust]
clean = "cargo clean"

[build_commands.python]
build = "maturin develop"
build_release = "maturin build --release"
```

### `[e2e]` -- E2E Test Generation

Configure fixture-driven test generation:

```toml
[e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["python", "node", "rust", "go"]

[e2e.call]
function = "extract"
module = "my_library"
async = true
args = [
  { name = "path", field = "input.path", type = "string" },
]
```

### `[opaque_types]` -- External Type Declarations

Declare types from external crates that alef can't extract:

```toml
[opaque_types]
Tree = "tree_sitter_language_pack::Tree"
```

These get opaque wrapper structs in all backends with handle-based FFI access.

### `[custom_modules]` / `[custom_registrations]` -- Hand-Written Code

Declare hand-written modules that alef should include in `mod` declarations and module init:

```toml
[custom_modules]
python = ["custom_handler"]

[custom_registrations.python]
classes = ["CustomHandler"]
functions = ["custom_extract"]
init_calls = ["register_custom_types(m)?;"]
```

## Architecture

```text
Rust Source Files
       |
  [alef extract]
       |
  Intermediate Representation (IR)
  ApiSurface { types, functions, enums, errors }
       |
  [alef generate]
       |
  +----+----+----+----+----+----+----+----+----+----+----+
  |    |    |    |    |    |    |    |    |    |    |    |
 PyO3 NAPI WASM FFI  Magnus PHP Rustler extendr Go Java C#
  |    |    |    |
  |    |    |    +-- cbindgen --> C header
  |    |    +-- wasm-pack --> npm
  |    +-- napi build --> npm + .d.ts
  +-- maturin --> PyPI wheel + .pyi
```

### Crate Structure

| Crate | Role |
|-------|------|
| [`alef-core`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-core) | IR types, config schema, `Backend` trait |
| [`alef-extract`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-extract) | Rust source to IR extraction via `syn` |
| [`alef-codegen`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-codegen) | Shared code generation (type mappers, converters, builders) |
| [`alef-adapters`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-adapters) | Adapter pattern code generators |
| [`alef-cli`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-cli) | CLI binary with all commands |
| [`alef-docs`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-docs) | API reference documentation generator |
| [`alef-e2e`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-e2e) | Fixture-driven e2e test generator |
| [`alef-readme`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-readme) | Per-language README generator |
| [`alef-scaffold`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-scaffold) | Package manifest generator |
| [`alef-backend-pyo3`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-pyo3) | Python backend |
| [`alef-backend-napi`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-napi) | TypeScript/Node.js backend |
| [`alef-backend-wasm`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-wasm) | WebAssembly backend |
| [`alef-backend-ffi`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-ffi) | C FFI backend (used by Go, Java, C#) |
| [`alef-backend-magnus`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-magnus) | Ruby backend |
| [`alef-backend-php`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-php) | PHP backend |
| [`alef-backend-rustler`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-rustler) | Elixir backend |
| [`alef-backend-extendr`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-extendr) | R backend |
| [`alef-backend-go`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-go) | Go backend |
| [`alef-backend-java`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-java) | Java backend |
| [`alef-backend-csharp`](https://github.com/kreuzberg-dev/alef/tree/main/crates/alef-backend-csharp) | C# backend |

## Pre-commit Hooks

Alef ships pre-commit hooks that consumer projects can use to keep generated output up to date.

### Verify mode (CI-friendly, check-only)

Fails if any generated file is stale -- does not modify files:

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/kreuzberg-dev/alef
    rev: v0.4.0
    hooks:
      - id: alef-verify
```

### Generate mode (auto-regenerate)

Regenerates all output (bindings, stubs, docs, readme, scaffold) when source files change:

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/kreuzberg-dev/alef
    rev: v0.4.0
    hooks:
      - id: alef-generate
```

Both hooks trigger on `.rs` and `.toml` file changes. They require `alef` to be installed and available on `PATH`.

## Contributing

Contributions welcome! Please open an issue or PR on [GitHub](https://github.com/kreuzberg-dev/alef).

## License

[MIT](LICENSE) -- Copyright (c) 2025-2026 Kreuzberg, Inc.
