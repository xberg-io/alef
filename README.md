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

Generate fully-typed, lint-clean language bindings for Rust libraries across 16 languages. Alef handles the entire pipeline -- API extraction, code generation, type stubs, package scaffolding, build orchestration, version sync, and e2e test generation -- from a single TOML config file.

## Key Features

- **API extraction** -- Parses your Rust crate's public API via `syn` into a language-agnostic intermediate representation. Handles `pub use` re-exports across workspace crates, `#[cfg(feature)]` gating, serde `rename_all` metadata, doc comments, `Default` detection, and transparent newtype resolution.
- **16 language backends** -- Each backend generates idiomatic, lint-clean binding code using the target language's native framework. See the [supported languages](#supported-languages) table below.
- **Configurable DTO styles** -- Choose how types are represented in each language: Python `dataclass` vs `TypedDict` vs `pydantic` vs `msgspec`, TypeScript `interface` vs `zod`, Ruby `Struct` vs `dry-struct` vs `Data`, and more. Input and output types can use different styles.
- **Type stubs** -- Generates `.pyi` (Python), `.rbs` (Ruby), and `.d.ts` (TypeScript) type definition files for editor support and static analysis in consuming projects.
- **Package scaffolding** -- Generates complete package manifests for each language: `pyproject.toml`, `package.json`, `.gemspec`, `composer.json`, `mix.exs`, `go.mod`, `pom.xml`, `.csproj`, `DESCRIPTION` (R), and `Cargo.toml` for binding crates.
- **E2E test generation** -- Write test fixtures as JSON, get complete runnable test suites for all configured languages. Supports 40+ assertion types, field path aliases, per-language overrides, and skip conditions.
- **Adapter patterns** -- Define custom FFI bridging patterns in config: `sync_function`, `async_method`, `callback_bridge`, `streaming`, and `server_lifecycle`. Alef generates the glue code for each backend.
- **Trait bridges** -- Generate FFI bridges so foreign-language objects can implement Rust traits (plugin pattern). Configured via `[[trait_bridges]]`; supported across all 16 backends including Kotlin, Swift, Dart, Gleam, and Zig.
- **Version sync** -- Propagates the version from `Cargo.toml` to all package manifests, C headers, pkg-config files, and any custom file via regex text replacements.
- **Build orchestration** -- Wraps `maturin`, `napi`, `wasm-pack`, and `cargo`+`cbindgen` with post-processing steps (e.g., patching `.d.ts` files for `verbatimModuleSyntax` compatibility).
- **Pipeline commands with smart defaults** -- `alef lint`, `test`, `build`, `setup`, `update`, `clean` ship with built-in per-language defaults that dispatch on the configured package manager (`uv`/`pip`/`poetry` for Python; `pnpm`/`npm`/`yarn` for Node) via the top-level `[tools]` section. Every default carries a POSIX `command -v` precondition so missing tools warn-and-skip instead of failing the run.
- **Override knobs that absorb common customisations** -- per-language `run_wrapper`, `extra_lint_paths`, and `project_file` (Java/C#) reduce override boilerplate so consumers rarely need to redefine entire `[lint.<lang>]` / `[build_commands.<lang>]` tables.
- **Publish pipeline** -- `alef publish prepare|build|package|validate` vendors the core crate, cross-compiles release artifacts (cargo / cross / maturin / napi / wasm-pack), and packages distributables (C FFI tarball with pkg-config/CMake, PHP PIE archive, Go FFI tarball).
- **Visitor FFI** -- Generates a 40-method visitor callback interface via C FFI, enabling the visitor pattern in Go, Java, C#, and other FFI-based languages.
- **Config validation** -- `alef.toml` is validated at load time. Custom command tables that override a main field must declare a `precondition` so warn-and-skip behavior is preserved on user systems. Fields whose value matches the built-in default emit a `tracing::warn!` so consumer configs stay minimal.
- **Caching** -- blake3-based content hashing skips regeneration when source and config haven't changed.
- **Version-idempotent verify** -- `alef verify` is a pure read+strip+rehash+compare. The hash baked into every generated file is `blake3(sources_hash || file_content_without_hash_line)` -- per-file, derived from the rust sources and the on-disk byte content; deliberately *no* alef CLI version dimension. Upgrading the alef CLI does not by itself invalidate verify on a tagged repo. `alef generate` finalises the embedded hash after the optional formatter pass (`--format`) runs, so the on-disk hash always matches the on-disk content. See [Verify model](#verify-model) below for details.
- **Opt-in formatting + live output** -- `alef generate` writes whitespace-normalised output without invoking external formatters by default; pass `--format` to also run `cargo fmt`, `ruff format`, `oxfmt`, etc. Long-running commands (`alef setup`, `alef update`, `alef lint`, `alef test`) stream stdout/stderr live with `[<lang>]` prefixes — no more multi-minute blackouts during `pnpm install` / `bundle install` / `cargo update`. Global flags `--verbose` / `--quiet` / `--no-color` and `--version` give standard CLI ergonomics.

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
| Kotlin/JVM | [Panama FFM](https://openjdk.org/jeps/454) (shared with Java) | Maven (.jar) | JUnit | `data-class`, `sealed-class` |
| Gleam | [Rustler](https://github.com/rusterlium/rustler) NIF + `@external` | Hex | gleeunit | record types |
| Zig | C FFI via `@cImport` | tarball | `std.testing` | structs |
| C | [cbindgen](https://github.com/mozilla/cbindgen) | Header (.h) | -- | -- |
| Swift | [swift-bridge](https://github.com/chinedufn/swift-bridge) | SwiftPM (RustBridge + RustBridgeC targets) | XCTest | structs with `Codable` |
| Dart | [flutter_rust_bridge](https://cjycode.com/flutter_rust_bridge/) v2 | pub.dev | `package:test` | classes |

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
| `alef verify` | Recompute the per-file `blake3(sources_hash \|\| file_content)` hash on every alef-headered file and compare against the embedded value. Idempotent across alef CLI versions. `--exit-code` for CI. |
| `alef diff` | Show what would change without writing |
| `alef all` | Run full pipeline: generate + stubs + scaffold + readme + e2e + docs + sync |
| `alef e2e` | Generate e2e test projects from JSON fixtures (subcommands: `generate`, `init`, `scaffold`, `list`, `validate`) |
| `alef publish` | Vendor, cross-compile, and package release artifacts (subcommands: `prepare`, `build`, `package`, `validate`) |
| `alef cache` | Manage build cache (subcommands: `clear`, `status`) |

## Verify model

`alef verify` is the canonical "are the generated files in sync with the source?" check. It is **idempotent across alef CLI versions**: a green verify on a tagged repo continues to pass after upgrading the alef CLI as long as the rust sources and the on-disk file contents are unchanged.

### How the hash is computed

Every alef-generated file carries a comment-style header:

```text
# This file is auto-generated by alef — DO NOT EDIT.
# alef:hash:9c5b8…  ← this line
# To regenerate: alef generate
# To verify freshness: alef verify --exit-code
# Issues & docs: https://github.com/kreuzberg-dev/alef
```

The `alef:hash:<hex>` value is a per-file `blake3(sources_hash || file_content_without_hash_line)`, where:

- `sources_hash = blake3(sorted(rust_source_files))` — both each file's path *and* its content are mixed in, sorted by path so the order in `[crate].sources` doesn't matter.
- `file_content_without_hash_line` is the actual on-disk byte content of the file with the existing `alef:hash:` line stripped (so the function is symmetric: calling it on content that already carries a hash line is the same as calling it on the bare content).

The hash deliberately does **not** include the alef CLI version or `alef.toml`: any input change that affects the generated bytes is already reflected by hashing the file content itself, and excluding the alef version is what makes verify idempotent across alef upgrades. `alef generate` finalises the embedded hash *after* the optional formatter pass — formatters run only when invoked with `--format`, in which case rustfmt, rubocop, dotnet format, spotless, oxfmt, oxlint, mix format, php-cs-fixer, ruff, taplo, etc. mutate the file before the hash is stamped. Without `--format`, the hash describes the raw whitespace-normalised codegen output. Either way, the on-disk hash always describes the actual on-disk byte content.

### How verify works

`alef verify` does, in pseudocode:

```text
sources_hash = blake3(sorted(rust_sources))

for each file under <repo> not in (.git, target, node_modules, _build, parsers, dist, vendor, .alef, …):
    if file extension is in (rs, py, ts, rb, php, go, java, cs, ex, R, toml, json, md, h, c, …):
        read the file
        if "alef:hash:<hex>" line present:
            expected = blake3(sources_hash || strip_hash_line(content))
            if hex != expected:
                report stale
```

That is the entire algorithm. Verify never runs codegen, never invokes a formatter, never writes anything. It exits 0 if every alef-headered file's embedded hash matches the recomputed per-file hash; with `--exit-code`, it exits 1 on the first mismatch (after listing all stale files).

### Properties that fall out of this design

- **Idempotent across alef versions.** Upgrading the alef CLI doesn't invalidate verify on a repo whose rust sources and generated bytes haven't changed. The version is not a hash dimension.
- **Symmetric generate / verify.** The same `compute_file_hash(sources_hash, content)` runs on both sides; verify just recomputes what generate finalised on disk.
- **Scaffold-once files are not under verify.** Templates like `Cargo.toml` (for binding crates), `composer.json`, `gemspec`, `package.json` for the npm shim, and lockfiles don't carry the alef header marker. `alef:hash:` lookup returns `None` for them and verify skips them silently — alef never claims ownership of files it didn't tag.

### When verify reports stale

The embedded hash mismatches the recomputed hash if and only if **at least one of**:

- A `[crate].sources` Rust file was edited, added, removed, or renamed.
- An alef-generated file was edited by hand or by a tool that mutated content after `alef generate` ran.
- `alef.toml` (or alef itself) changed in a way that produced different generated bytes.

In any of those cases the fix is `alef generate` (and `alef e2e generate` if the repo uses `[e2e]`), which finalises the new per-file hash in every alef-headered file.

### Backwards compatibility

This is a behavior change from v0.9.0–v0.10.0, where the embedded hash was a single repo-wide input fingerprint that incorporated the alef CLI version. After upgrading to v0.10.1+, every existing alef-generated file carries the old uniform hash; one `alef generate` rewrites all of them with the new per-file scheme. The IR / generated content is unchanged — only the hash header value differs.

The `alef verify --lang`, `--compile`, `--lint` flags are still accepted but have no effect. Use `alef build`, `alef lint`, `alef test` for the per-language checks those flags used to imply.

## Configuration-Driven Defaults

All operational commands (`lint`, `fmt`, `update`, `setup`, `clean`, `build`, `test`) ship with built-in per-language defaults. No configuration is required to use them — run `alef lint` or `alef update` out of the box. Defaults dispatch on the top-level `[tools]` section (package-manager selection) and per-language knobs (`run_wrapper`, `extra_lint_paths`, `project_file`) — both described below — so most consumer projects can avoid redefining whole command tables.

Every built-in default declares a POSIX `command -v <tool>` **precondition**: when the underlying tool is missing on the user's system, the language is skipped with a warning rather than failing the run. When you override a main command field (`format`, `command`, `update`, `install`, `clean`, `build`, …), you **must** declare a `precondition` of your own so the warn-and-skip semantics survive on consumer systems — alef rejects override tables that omit it at config load.

Override any default by adding the corresponding section to `alef.toml`:

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

### `[tools]` -- Package Manager Selection

The top-level `[tools]` section selects which package-manager / dev-tool variants every default pipeline command targets. All fields are optional; defaults match what most projects use.

```toml
[tools]
python_package_manager = "uv"      # uv | pip | poetry  (default: uv)
node_package_manager = "pnpm"      # pnpm | npm | yarn  (default: pnpm)
rust_dev_tools = [                 # tools `alef setup rust` installs via `cargo install`
  "cargo-edit",
  "cargo-sort",
  "cargo-machete",
  "cargo-deny",
  "cargo-llvm-cov",
]
```

The Python and Node selections drive the default `lint`, `test`, `setup`, `update`, and `clean` commands — switching to `pip` swaps `uv run pytest` for plain `pytest`, switching to `yarn` swaps `pnpm up` for `yarn upgrade`, etc. Set `rust_dev_tools = []` to skip dev-tool installation entirely.

### Per-language override knobs

Three optional per-language fields cover the most common reasons consumers used to redefine entire command tables. They feed into every relevant default command (`format`, `check`, `typecheck`, `command`, `coverage`, `update`, `upgrade`, `install`, `clean`, `build`, `build_release`).

```toml
# Wrap every default tool invocation. Common for projects that need to
# inherit the package-manager environment without a `before` hook.
[python]
run_wrapper = "uv run --no-sync"  # → "uv run --no-sync ruff format packages/python"

# Append extra paths to default lint commands.
[python]
extra_lint_paths = ["scripts"]    # → "ruff format packages/python scripts"

# For Maven / .NET, point default lint/build/test commands at a project
# descriptor instead of the package directory.
[java]
project_file = "pom.xml"

[csharp]
project_file = "MySolution.slnx"
```

`run_wrapper`, `extra_lint_paths`, and `project_file` are accepted on every binding language section (`[python]`, `[node]`, `[ruby]`, `[php]`, `[elixir]`, `[wasm]`, `[go]`, `[java]`, `[csharp]`, `[r]`). `project_file` only takes effect for `[java]` and `[csharp]`.

## Configuration Reference

Alef is configured via `alef.toml` in your project root. Run `alef init` to generate a starter config.

### Minimal Example

```toml
languages = ["python", "node", "go", "java"]

[crate]
name = "my-library"
sources = ["src/lib.rs", "src/types.rs"]

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

`languages` is a top-level array, so it must appear **before** the first `[table]` header — otherwise TOML scopes it as `[crate].languages` and parsing fails.

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
| `path_mappings` | map | `{}` | Rewrite extracted Rust path prefixes (e.g., `{ "mylib" = "mylib_http" }`) |

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

Every command config section supports two hook fields:

- **`precondition`** -- A shell command that must exit 0 for the main command to run. If it fails, the language is skipped with a warning (not an error). Useful for languages that need a shared library or tool to be present.
- **`before`** -- Command(s) to run before the main command. Unlike `precondition`, failure aborts the command for that language. Accepts a single string or an array. Use this to build prerequisites (e.g., FFI libraries, maturin develop).

```toml
# Go lint needs the FFI shared library built first
[lint.go]
precondition = "test -f target/release/libmy_lib_ffi.dylib"
format = "gofmt -w packages/go/"
check = "cd packages/go && golangci-lint run ./..."

# Python test needs maturin develop first
[test.python]
before = "cd packages/python && maturin develop --release"
command = "cd packages/python && uv run pytest tests/ -v"
coverage = "cd packages/python && uv run pytest --cov=. --cov-report=lcov"

# Node test needs the NAPI binding built first
[test.node]
before = "napi build --platform --manifest-path crates/my-lib-node/Cargo.toml"
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

Rust is a first-class language in all pipelines -- add `"rust"` to your `languages` array to include Rust in `alef build`, `alef test`, `alef lint`, etc. alongside your binding languages.

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

Fails if any generated file is stale (including per-language READMEs) -- does not modify files:

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/kreuzberg-dev/alef
    rev: v0.7.9
    hooks:
      - id: alef-verify
```

### Generate mode (auto-regenerate)

Regenerates all output (bindings, stubs, docs, readme, scaffold) when source files change:

```yaml
# .pre-commit-config.yaml
repos:
  - repo: https://github.com/kreuzberg-dev/alef
    rev: v0.7.9
    hooks:
      - id: alef-generate
```

### README-only refresh

Regenerate per-language README files without re-running the full pipeline. Useful when only the README templates or snippets have changed:

```yaml
repos:
  - repo: https://github.com/kreuzberg-dev/alef
    rev: v0.7.9
    hooks:
      - id: alef-readme
```

All hooks trigger on `.rs` and `.toml` file changes (`alef-readme` additionally watches the `[readme].template_dir` and `snippets_dir`). They require `alef` to be installed and available on `PATH`. `alef verify` walks the repo for alef-headered files (binding glue, stubs, scaffolds, docs, READMEs, e2e tests), strips the `alef:hash:` line from each, and recomputes `blake3(sources_hash || stripped_content)` to compare against the embedded value — see [Verify model](#verify-model). The check is idempotent across alef CLI upgrades: bumping `rev:` does not by itself flag any file as stale.

## Contributing

Contributions welcome! Please open an issue or PR on [GitHub](https://github.com/kreuzberg-dev/alef).

## License

[MIT](LICENSE) -- Copyright (c) 2025-2026 Kreuzberg, Inc.
