# alef.toml Configuration Reference

Alef is configured via `alef.toml` in your project root. Run `alef init` to generate a starter config.

## Validation Behavior

`alef.toml` is validated when alef loads it. Two checks emit diagnostics:

1. **Required preconditions on overrides.** Any `[lint|test|build_commands|setup|update|clean].<lang>` table that overrides a main command field (`format`, `check`, `typecheck`, `command`, `coverage`, `e2e`, `update`, `upgrade`, `install`, `clean`, `build`, `build_release`) must declare a `precondition`. This guarantees the warn-and-skip semantics that built-in defaults provide carry over to user overrides — without it, a missing tool on a consumer's system would hard-fail the run instead of being skipped. Missing preconditions are a load-time error, not a warning.
2. **Redundant-default warnings.** Any field whose value matches the built-in default verbatim emits a `tracing::warn!` naming the section and field. Pure-noise overrides clutter consumer configs; alef calls them out so they can be removed.

Both behaviors are intentional. They keep consumer `alef.toml` files minimal and honest about the actual surface that's being customised.

## Minimal Example

The new multi-crate schema groups workspace defaults under `[workspace]` and per-crate config under `[[crates]]`:

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

[crates.python]
module_name = "_my_library"

[crates.node]
package_name = "@myorg/my-library"

[workspace.dto]
python = "dataclass"
node = "interface"
```

### Migration from Legacy Schema

If you have an existing single-crate `alef.toml` with top-level `[crate]`, `languages`, etc., run:

```bash
alef migrate --write [PATH]
```

This rewrites the entire config to the new layout automatically and atomically. The old schema is rejected on load with a clear migration hint.

---

## `[workspace]` -- Workspace Defaults

The `[workspace]` section defines defaults that apply to all `[[crates]]` entries unless overridden. Most fields here can also appear on individual crates to take precedence.

```toml
[workspace]
alef_version = "0.13.0"
languages = ["python", "node"]
# ... other shared defaults ...
```

---

## `[workspace.tools]` -- Package Manager Selection

Section that selects which package-manager / dev-tool variants the default per-language pipeline commands target. Every field is optional; defaults match the most common project setup.

```toml
[workspace.tools]
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

| Field                    | Type     | Default   | Description                                                                                                                             |
| ------------------------ | -------- | --------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `python_package_manager` | string   | `"uv"`    | One of `uv`, `pip`, `poetry`. Drives default Python `lint`, `test`, `setup`, `update`, `clean` commands.                                |
| `node_package_manager`   | string   | `"pnpm"`  | One of `pnpm`, `npm`, `yarn`. Drives default Node and Wasm pipeline commands.                                                           |
| `rust_dev_tools`         | string[] | see right | Defaults to `["cargo-edit", "cargo-sort", "cargo-machete", "cargo-deny", "cargo-llvm-cov"]`. Set to `[]` to skip dev-tool installation. |

The selection feeds every default that calls a package-manager-specific tool — switching to `pip` swaps `uv run pytest` for `pytest`, switching to `yarn` swaps `pnpm up` for `yarn upgrade`, etc.

---

## `[workspace.client_constructors]` -- Custom Opaque Handle Constructors

Per-type custom constructors emitted by backends that support opaque handles (FFI, Go, Zig, C#, JNI/Kotlin, PyO3, NAPI, WASM, PHP, Rustler, Dart). Each entry under `[workspace.client_constructors.<TypeName>]` defines constructor parameters and a body template with `{type_name}` and `{source_path}` substitution.

```toml
[workspace.client_constructors.DefaultClient]
body = "{source_path}::new().map_err(|e| e.to_string())"
error_type = "String"

[[workspace.client_constructors.DefaultClient.params]]
name = "api_key"
type = "*const std::ffi::c_char"

[[workspace.client_constructors.DefaultClient.params]]
name = "endpoint"
type = "*const std::ffi::c_char"
```

| Field         | Type                      | Default    | Description                                                           |
| ------------- | ------------------------- | ---------- | --------------------------------------------------------------------- |
| `params`      | array of `ConstructorParam` | `[]`     | Ordered list of constructor parameters; each has `name` and `type`   |
| `body`        | string                    | _required_ | Template for constructor body; use `{type_name}` and `{source_path}` |
| `error_type`  | string                    | `"String"` | Error type returned by the constructor (`Result<Self, ErrType>`)    |

---

## `[[crates]]` -- Per-Crate Configuration

Each `[[crates]]` entry defines one independently published binding package. Every field except `name` is optional and inherits from `[workspace]` defaults during resolution.

| Field                | Type     | Default                           | Description                                                                                                              |
| -------------------- | -------- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `name`               | string   | _required_                        | Crate name; must be unique within the workspace                                                                          |
| `sources`            | string[] | _required_ (or `source_crates`)   | Rust source files to extract                                                                                             |
| `languages`          | string[] | inherits `[workspace].languages`  | Target languages for this crate only                                                                                     |
| `version_from`       | string   | `"Cargo.toml"`                    | File to read version from (supports workspace Cargo.toml)                                                                |
| `core_import`        | string   | `{name}` with `-` replaced by `_` | Import path for the core crate in generated bindings                                                                     |
| `workspace_root`     | string   | --                                | Workspace root for resolving `pub use` re-exports from sibling crates                                                    |
| `skip_core_import`   | bool     | `false`                           | Skip adding `use {core_import};` to generated bindings                                                                   |
| `features`           | string[] | `[]`                              | Cargo features treated as always-present (`#[cfg(feature)]` fields are included)                                         |
| `path_mappings`      | map      | `{}`                              | Rewrite extracted Rust path prefixes (e.g., `{ "mylib" = "mylib_http" }`)                                                |
| `extra_dependencies` | map      | `{}`                              | Additional Cargo dependencies added to all binding crate Cargo.tomls (crate name to TOML dep spec)                       |
| `auto_path_mappings` | bool     | `true`                            | Auto-derive path_mappings from source file locations (`crates/{name}/src/` to `core_import`)                             |
| `source_crates`      | array    | `[]`                              | Multi-crate source groups for workspaces (overrides `sources` when non-empty)                                            |
| `error_type`         | string   | `"Error"`                         | Crate error type name (e.g. `"KreuzbergError"`)                                                                          |
| `error_constructor`  | string   | --                                | Pattern for constructing error values from a String in trait bridges. `{msg}` is replaced with `format!(...)` expression |

---

## `[[crates.source_crates]]` -- Multi-Crate Extraction (within a single binding package)

For workspaces where types are spread across multiple Rust crates, `source_crates` lets you extract from each crate separately while preserving the actual defining crate in `rust_path`. This is per-binding-package, so use it within a single `[[crates]]` entry.

| Field     | Type     | Description                                                        |
| --------- | -------- | ------------------------------------------------------------------ |
| `name`    | string   | Rust crate name (hyphens converted to underscores for `rust_path`) |
| `sources` | string[] | Source files belonging to this crate                               |

```toml
[[crates]]
name = "my-library"

[[crates.source_crates]]
name = "tree-sitter-language-pack"
sources = ["crates/ts-pack-core/src/lib.rs"]

[[crates.source_crates]]
name = "tree-sitter-types"
sources = ["crates/ts-types/src/lib.rs"]
```

When `source_crates` is non-empty, the per-crate `sources` field is ignored.

---

## `languages` -- Target Languages

Top-level array of languages to generate bindings for:

```toml
languages = ["python", "node", "ruby", "php", "elixir", "wasm", "ffi", "go", "java", "csharp", "r"]
```

The `ffi` language generates the C FFI layer required by `go`, `java`, and `csharp`. If you enable any of those three, `ffi` is implicitly included.

---

## `[exclude]` / `[include]` -- Filtering

Control which types and functions are included in or excluded from generated output.

```toml
[exclude]
types = ["InternalHelper"]
functions = ["deprecated_fn"]
methods = ["MyType.internal_method"]   # dot-notation: "TypeName.method_name"

[include]
types = ["PublicApi", "Config"]        # whitelist only these types
functions = ["extract", "parse"]       # whitelist only these functions
```

When `[include]` is specified, only the listed items are included. `[exclude]` takes precedence when both are present for the same item.

---

## `[crates.output]` -- Per-Crate Output Directories

Per-language output directories for generated Rust binding code. Specified under each `[[crates]]` entry. The `{name}` placeholder is replaced with that crate's name.

```toml
[[crates]]
name = "my-library"

[crates.output]
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

---

## Language-Specific Sections

Language sections appear under each `[[crates]]` entry as `[crates.python]`, `[crates.node]`, etc.

### Shared per-language fields

The fields below are accepted on every language section listed in this document (`[crates.python]`, `[crates.node]`, `[crates.ruby]`, `[crates.php]`, `[crates.elixir]`, `[crates.wasm]`, `[crates.ffi]`, `[crates.go]`, `[crates.java]`, `[crates.csharp]`, `[crates.r]`). They aren't duplicated in each table; the language-specific tables only list fields unique to that language. These can also be set at the workspace level (`[workspace.python]`, etc.) to apply to all crates.

| Field                | Type     | Default                                                                                                | Description                                                                                                                                                                                                                                                                                                           |
| -------------------- | -------- | ------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `features`           | string[] | inherits `[crate].features`                                                                            | Per-language Cargo feature override                                                                                                                                                                                                                                                                                   |
| `serde_rename_all`   | string   | language-idiomatic (`snake_case` for Python/Ruby/PHP/Elixir/Go/FFI; `camelCase` for Node/Wasm/Java/C#) | Override JSON field naming strategy                                                                                                                                                                                                                                                                                   |
| `extra_dependencies` | map      | `{}`                                                                                                   | Additional Cargo deps for this language's binding crate (`{ "tokio" = "1" }` or full TOML spec). Not present on `[go]`, `[java]`, `[csharp]`, `[r]`.                                                                                                                                                                  |
| `scaffold_output`    | path     | derived from `[output].<lang>`                                                                         | Override where this language's package files (`pyproject.toml`, `package.json`, …) are scaffolded. Not present on `[ffi]`, `[go]`, `[java]`, `[csharp]`, `[r]`.                                                                                                                                                       |
| `rename_fields`      | map      | `{}`                                                                                                   | Per-field name remapping. Key is `TypeName.field_name` (e.g. `"LayoutDetection.class"`), value is the desired binding field name. Applied after automatic keyword escaping.                                                                                                                                           |
| `exclude_functions`  | string[] | `[]`                                                                                                   | Functions to exclude from this language's bindings. Currently honoured by `[python]`, `[node]`, `[ruby]`, `[php]`, `[elixir]`, `[wasm]`, `[ffi]`, `[go]`.                                                                                                                                                             |
| `exclude_types`      | string[] | `[]`                                                                                                   | Types to exclude from this language's bindings. Same backends as `exclude_functions`.                                                                                                                                                                                                                                 |
| `run_wrapper`        | string   | `None`                                                                                                 | Prefix every default tool invocation with this string. Example: `[python] run_wrapper = "uv run --no-sync"` turns the default `ruff format packages/python` into `uv run --no-sync ruff format packages/python`. Applies across `lint`, `test`, `setup`, `update`, `clean`, `build` defaults. Not present on `[ffi]`. |
| `extra_lint_paths`   | string[] | `[]`                                                                                                   | Append paths (space-separated) to default `format`, `check`, `typecheck` commands. Example: `[python] extra_lint_paths = ["scripts"]` makes `ruff format packages/python scripts`. Ignored on Java/C# when `project_file` is set. Not present on `[ffi]`.                                                             |
| `project_file`       | string   | `None`                                                                                                 | Java/C# only. When set, default lint/build/test commands target this file (`pom.xml`, `MyProject.csproj`, `MySolution.slnx`) instead of the package directory.                                                                                                                                                        |

### `[crates.python]`

| Field                    | Type     | Default                       | Description                                                                 |
| ------------------------ | -------- | ----------------------------- | --------------------------------------------------------------------------- |
| `module_name`            | string   | `_{name}`                     | Python module name (the native extension name)                              |
| `async_runtime`          | string   | --                            | Async runtime spec for `pyo3_async_runtimes`                                |
| `stubs.output`           | string   | --                            | Output directory for `.pyi` stub files                                      |
| `stubs.emit_docstrings`  | bool     | `false`                       | Emit Rust doc comments as stub docstrings (ruff PYI021 prohibits by default) |
| `features`               | string[] | inherits per-crate `features` | Per-language Cargo feature override                                         |
| `serde_rename_all`       | string   | `"snake_case"`                | Override JSON field naming strategy for this language                       |

### `[crates.node]`

| Field              | Type     | Default                       | Description                                           |
| ------------------ | -------- | ----------------------------- | ----------------------------------------------------- |
| `package_name`     | string   | `{name}`                      | npm package name                                      |
| `features`         | string[] | inherits per-crate `features` | Per-language Cargo feature override                   |
| `serde_rename_all` | string   | `"camelCase"`                 | Override JSON field naming strategy for this language |

### `[crates.ruby]`

| Field                    | Type     | Default                       | Description                                           |
| ------------------------ | -------- | ----------------------------- | ----------------------------------------------------- |
| `gem_name`               | string   | `{name}` with `_`             | Ruby gem name                                         |
| `stubs.output`           | string   | --                            | Output directory for `.rbs` type stubs                |
| `stubs.emit_docstrings`  | bool     | `false`                       | Emit Rust doc comments as stub docstrings             |
| `features`               | string[] | inherits per-crate `features` | Per-language Cargo feature override                   |
| `serde_rename_all`       | string   | `"snake_case"`                | Override JSON field naming strategy for this language |

### `[crates.php]`

| Field              | Type     | Default                       | Description                                           |
| ------------------ | -------- | ----------------------------- | ----------------------------------------------------- |
| `extension_name`   | string   | `{name}` with `_`             | PHP extension name                                    |
| `feature_gate`     | string   | `"extension-module"`          | Feature gate wrapping all generated code              |
| `stubs.output`     | string   | --                            | Output directory for PHP facades/stubs                |
| `features`         | string[] | inherits per-crate `features` | Per-language Cargo feature override                   |
| `serde_rename_all` | string   | `"snake_case"`                | Override JSON field naming strategy for this language |

### `[crates.elixir]`

| Field              | Type     | Default                       | Description                                           |
| ------------------ | -------- | ----------------------------- | ----------------------------------------------------- |
| `app_name`         | string   | `{name}` with `_`             | Elixir application name                               |
| `features`         | string[] | inherits per-crate `features` | Per-language Cargo feature override                   |
| `serde_rename_all` | string   | `"snake_case"`                | Override JSON field naming strategy for this language |

### `[crates.wasm]`

| Field               | Type     | Default                       | Description                                           |
| ------------------- | -------- | ----------------------------- | ----------------------------------------------------- |
| `exclude_functions` | string[] | `[]`                          | Functions to exclude from WASM bindings               |
| `exclude_types`     | string[] | `[]`                          | Types to exclude from WASM bindings                   |
| `type_overrides`    | map      | `{}`                          | Override types (e.g., `{ "DOMNode" = "JsValue" }`)    |
| `features`          | string[] | inherits per-crate `features` | Per-language Cargo feature override                   |
| `serde_rename_all`  | string   | `"camelCase"`                 | Override JSON field naming strategy for this language |

### `[crates.ffi]`

| Field               | Type     | Default                       | Description                                           |
| ------------------- | -------- | ----------------------------- | ----------------------------------------------------- |
| `prefix`            | string   | `{name}` with `_`             | C symbol prefix for all exported functions            |
| `error_style`       | string   | `"last_error"`                | Error reporting convention                            |
| `header_name`       | string   | `{prefix}.h`                  | Generated C header filename                           |
| `lib_name`          | string   | `{prefix}_ffi`                | Native library name (for Go/Java/C# linking)          |
| `visitor_callbacks` | bool     | `false`                       | Generate visitor/callback FFI support                 |
| `features`          | string[] | inherits per-crate `features` | Per-language Cargo feature override                   |
| `serde_rename_all`  | string   | `"snake_case"`                | Override JSON field naming strategy for this language |

### `[crates.go]`

| Field              | Type     | Default                           | Description                                           |
| ------------------ | -------- | --------------------------------- | ----------------------------------------------------- |
| `module`           | string   | `github.com/kreuzberg-dev/{name}` | Go module path                                        |
| `package_name`     | string   | derived from module path          | Go package name                                       |
| `features`         | string[] | inherits per-crate `features`     | Per-language Cargo feature override                   |
| `serde_rename_all` | string   | `"snake_case"`                    | Override JSON field naming strategy for this language |

### `[crates.java]`

| Field              | Type     | Default                       | Description                                              |
| ------------------ | -------- | ----------------------------- | -------------------------------------------------------- |
| `package`          | string   | `dev.kreuzberg`               | Java package name                                        |
| `ffi_style`        | string   | `"panama"`                    | FFI binding style (Panama Foreign Function & Memory API) |
| `features`         | string[] | inherits per-crate `features` | Per-language Cargo feature override                      |
| `serde_rename_all` | string   | `"camelCase"`                 | Override JSON field naming strategy for this language    |

### `[crates.csharp]`

| Field              | Type     | Default                       | Description                                           |
| ------------------ | -------- | ----------------------------- | ----------------------------------------------------- |
| `namespace`        | string   | PascalCase of `{name}`        | C# namespace                                          |
| `target_framework` | string   | --                            | Target framework version                              |
| `features`         | string[] | inherits per-crate `features` | Per-language Cargo feature override                   |
| `serde_rename_all` | string   | `"camelCase"`                 | Override JSON field naming strategy for this language |

### `[crates.r]`

| Field              | Type     | Default                       | Description                                           |
| ------------------ | -------- | ----------------------------- | ----------------------------------------------------- |
| `package_name`     | string   | `{name}`                      | R package name                                        |
| `features`         | string[] | inherits per-crate `features` | Per-language Cargo feature override                   |
| `serde_rename_all` | string   | `"snake_case"`                | Override JSON field naming strategy for this language |

---

## `[workspace.dto]` and `[crates.dto]` -- Type Generation Styles

Controls how Rust structs are represented in each language's public API. Set at the workspace level (`[workspace.dto]`) to apply to all crates, or override per-crate with `[crates.dto]`. An optional `_output` variant allows using a different style for return types.

```toml
[workspace.dto]
python = "dataclass"         # dataclass | typed-dict | pydantic | msgspec
python_output = "typed-dict" # separate style for return types (optional)
node = "interface"           # interface | zod
ruby = "struct"              # struct | dry-struct | data
php = "readonly-class"       # readonly-class | array
elixir = "struct"            # struct | typed-struct
go = "struct"                # struct
java = "record"              # record
csharp = "record"            # record
r = "list"                   # list | r6

# Override for a specific crate:
[[crates]]
name = "my-special-lib"
# ... rest of config ...

[crates.dto]
python = "pydantic"  # use pydantic for this crate only
```

| Language        | Available Styles                                 |
| --------------- | ------------------------------------------------ |
| Python          | `dataclass`, `typed-dict`, `pydantic`, `msgspec` |
| Node/TypeScript | `interface`, `zod`                               |
| Ruby            | `struct`, `dry-struct`, `data`                   |
| PHP             | `readonly-class`, `array`                        |
| Elixir          | `struct`, `typed-struct`                         |
| Go              | `struct`                                         |
| Java            | `record`                                         |
| C#              | `record`                                         |
| R               | `list`, `r6`                                     |

---

## `[[crates.readme]]` and `[crates.readme]` -- README Generation

Configuration for per-language README generation. Specified per-crate under each `[[crates]]` entry.

```toml
[[crates]]
name = "my-library"

[crates.readme]
template_dir = "readme-templates"
snippets_dir = "readme-snippets"
output_pattern = "{lang}/README.md"
discord_url = "https://discord.gg/yourserver"
banner_url = "https://example.com/banner.png"

[crates.readme.languages.python]
key = "value"

[crates.readme.languages.node]
key = "value"
```

| Field              | Type   | Default | Description                                                          |
| ------------------ | ------ | ------- | -------------------------------------------------------------------- |
| `template_dir`     | string | --      | Directory containing minijinja README templates by language          |
| `snippets_dir`     | string | --      | Directory containing reusable code snippets for embedding in READMEs |
| `config`           | map    | `{}`    | Global context variables for minijinja template rendering            |
| `output_pattern`   | string | --      | Output path pattern using `{lang}` placeholder                       |
| `discord_url`      | string | --      | Discord community URL to include in generated READMEs                |
| `banner_url`       | string | --      | Banner image URL to include in generated READMEs                     |
| `languages.<lang>` | map    | `{}`    | Per-language key/value context variables for minijinja templates     |

---

## `[workspace.package_metadata]` / `[crates.package_metadata]` -- Package Metadata

Central metadata used when generating package manifests (`pyproject.toml`, `package.json`, `.gemspec`,
`composer.json`, `mix.exs`, `go.mod`, `pom.xml`, `.csproj`, `DESCRIPTION`). Workspace values apply to every crate;
per-crate values override them field-by-field.

```toml
[workspace.package_metadata]
description = "My library for doing things"
license = "MIT"
repository = "https://github.com/org/repo"
homepage = "https://docs.example.com"
documentation = "https://docs.example.com"
issues = "https://github.com/org/repo/issues"
authors = ["Your Name <you@example.com>"]
keywords = ["parsing", "extraction"]
categories = ["text-processing"]

[[crates]]
name = "my-library"

[crates.package_metadata]
description = "Language bindings for my-library"
```

| Field                     | Type     | Default | Description                                                     |
| ------------------------- | -------- | ------- | --------------------------------------------------------------- |
| `description`             | string   | --      | Package description used in all manifests                       |
| `license`                 | string   | --      | SPDX license identifier                                         |
| `repository`              | string   | --      | Source code repository URL                                      |
| `homepage`                | string   | --      | Project homepage URL                                            |
| `documentation`           | string   | --      | Documentation URL where the package registry supports one       |
| `issues`                  | string   | --      | Issue tracker URL where the package registry supports one       |
| `funding`                 | string   | --      | Funding/support URL where the package registry supports one     |
| `authors`                 | string[] | --      | List of package authors                                         |
| `keywords`                | string[] | `[]`    | Keywords/tags for package registries                            |
| `categories`              | string[] | `[]`    | Registry categories/labels where supported                      |
| `truncate_registry_lists` | bool     | `false` | Truncate registry-limited lists instead of failing validation   |

crates.io supports at most five keywords/categories. Alef validates that limit by default; set
`truncate_registry_lists = true` only when deterministic truncation is intended.

Legacy `[crates.scaffold]` metadata is still accepted as a compatibility fallback, but new configs should use
`package_metadata`.

---

## `[[crates.adapters]]` -- Custom FFI Adapters

Define custom binding patterns that alef cannot extract automatically. Each `[[crates.adapters]]` entry defines a single adapter for that crate.

```toml
[[crates]]
name = "my-library"

[[crates.adapters]]
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

| Field         | Type   | Default    | Description                                                             |
| ------------- | ------ | ---------- | ----------------------------------------------------------------------- |
| `name`        | string | _required_ | Adapter function/method name                                            |
| `pattern`     | string | _required_ | Adapter pattern type (see below)                                        |
| `core_path`   | string | _required_ | Fully-qualified Rust path to the core function                          |
| `params`      | array  | `[]`       | Parameter definitions with `name`, `type`, and optional `optional` flag |
| `returns`     | string | --         | Return type (`Result`, `Option`, concrete type, or omitted for void)    |
| `error_type`  | string | --         | Error type name for `Result` returns                                    |
| `gil_release` | bool   | `false`    | Python: release the GIL during this call                                |

Supported patterns:

- `sync_function` -- synchronous standalone function
- `async_method` -- async method on a type
- `callback_bridge` -- callback-based FFI bridge
- `streaming` -- streaming/iterator pattern
- `server_lifecycle` -- server start/stop lifecycle management

---

## `[[crates.trait_bridges]]` -- Foreign Trait Implementations

Generate FFI bridges so a foreign-language object can implement a Rust trait (the plugin pattern). Supported across all backends; each `[[crates.trait_bridges]]` entry produces a wrapper struct, the trait `impl`, and an optional registration function for that trait.

```toml
[[crates]]
name = "my-library"

[[crates.trait_bridges]]
trait_name = "OcrBackend"
super_trait = "Plugin"
registry_getter = "kreuzberg::plugins::registry::get_ocr_backend_registry"
register_fn = "register_ocr_backend"
type_alias = "VisitorHandle"
exclude_languages = ["wasm"]
```

| Field                 | Type     | Default    | Description                                                                                                                                                                |
| --------------------- | -------- | ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `trait_name`          | string   | _required_ | Name of the Rust trait to bridge (e.g. `"OcrBackend"`)                                                                                                                     |
| `super_trait`         | string   | --         | Super-trait that requires forwarding. When set, the bridge generates an `impl SuperTrait for Wrapper` block.                                                               |
| `registry_getter`     | string   | --         | Rust path to the registry getter function. When set, the registration function inserts the bridge into a registry.                                                         |
| `register_fn`         | string   | --         | Name of the registration function to generate. When absent, only the wrapper struct and trait impl are emitted (per-call bridge pattern).                                  |
| `type_alias`          | string   | --         | Named type alias in the IR that maps to this bridge (e.g. `"VisitorHandle"`). When a parameter has a `TypeRef::Named` matching this alias, the bridge type is substituted. |
| `param_name`          | string   | --         | Parameter-name override for cases where the extractor sanitised the type (e.g. `VisitorHandle` becomes `String` because it is a type alias over `Rc<RefCell<dyn Trait>>`). |
| `register_extra_args` | string   | --         | Extra arguments appended to `registry.register(arc, …)`. E.g. `"0"` produces `registry.register(arc, 0)`.                                                                  |
| `exclude_languages`   | string[] | `[]`       | Backend names that should not generate this trait bridge. Backend names match `Backend::name()` (e.g. `["elixir", "wasm"]`).                                               |

---

## `[workspace.generate]` / `[crates.generate]` / `[crates.generate_overrides.<lang>]` -- Generation Control

Toggle individual generation passes. Set at workspace level to apply globally, or override per-crate. All default to `true`.

```toml
[workspace.generate]
bindings = true          # struct wrappers, From impls, module init
errors = true            # error type hierarchies from thiserror enums
configs = true           # config builder constructors from Default types
async_wrappers = true    # async/sync function pairs with runtime management
type_conversions = true  # recursive type marshaling helpers
package_metadata = true  # package manifests (pyproject.toml, package.json, etc.)
public_api = true        # idiomatic public API wrappers

# Override per crate:
[[crates]]
name = "my-special-lib"
[crates.generate]
public_api = false
```

| Field              | Type | Default | Description                                                  |
| ------------------ | ---- | ------- | ------------------------------------------------------------ |
| `bindings`         | bool | `true`  | Generate struct wrappers, `From` impls, and module init code |
| `errors`           | bool | `true`  | Generate error type hierarchies from `thiserror` enums       |
| `configs`          | bool | `true`  | Generate config builder constructors from `Default` types    |
| `async_wrappers`   | bool | `true`  | Generate async/sync function pairs with runtime management   |
| `type_conversions` | bool | `true`  | Generate recursive type marshaling helpers                   |
| `package_metadata` | bool | `true`  | Generate package manifests                                   |
| `public_api`       | bool | `true`  | Generate idiomatic public API wrappers                       |

Override per language with `[crates.generate_overrides.<lang>]`:

```toml
[crates.generate_overrides.wasm]
async_wrappers = false
```

---

## `[crates.sync]` -- Per-Crate Version Synchronization

Configure version sync for each crate. Version is read from the file specified by that crate's `version_from` (default: `Cargo.toml`).

```toml
[[crates]]
name = "my-library"

[crates.sync]
extra_paths = ["packages/go/go.mod"]

[[crates.sync.text_replacements]]
path = "crates/*/cbindgen.toml"
search = 'header = ".*"'
replace = 'header = "/* v{version} */"'
```

| Field               | Type     | Default | Description                                                               |
| ------------------- | -------- | ------- | ------------------------------------------------------------------------- |
| `extra_paths`       | string[] | `[]`    | Additional files to update version in (beyond auto-detected manifests)    |
| `text_replacements` | array    | `[]`    | Regex-based text replacements with `path`, `search`, and `replace` fields |

The `{version}` placeholder in `replace` is substituted with the current version.

---

## `[crates.lint.<lang>]` -- Per-Crate Lint Commands

Define per-language shell commands for linting and formatting generated output. Built-in defaults are provided for all languages (ruff, rubocop, clippy, oxlint, etc.) so this section is optional unless you need to override a default or add extra steps. Can be set at workspace level or per-crate.

All fields accept either a single string or an array of strings (`StringOrVec`).

```toml
[[crates]]
name = "my-library"

[crates.lint.python]
format = "ruff format packages/python/"
check = "ruff check packages/python/"
typecheck = "mypy packages/python/"

[crates.lint.node]
format = "oxfmt packages/node/src/"
check = "oxlint packages/node/src/"

# Array syntax — run multiple commands for one phase
[crates.lint.go]
check = ["golangci-lint run ./...", "go vet ./..."]
```

| Field       | Type        | Description                                                             |
| ----------- | ----------- | ----------------------------------------------------------------------- |
| `format`    | StringOrVec | Command(s) to format generated code (run by `alef fmt` and `alef lint`) |
| `check`     | StringOrVec | Command(s) to run lint checks (run by `alef lint`)                      |
| `typecheck` | StringOrVec | Command(s) to run type checking (run by `alef lint`)                    |

The Node and WASM built-in defaults use the Oxc toolchain: `oxfmt` for formatting and `oxlint` for linting. Generated scaffolding uses Oxfmt and Oxlint.

---

## `[crates.test.<lang>]` -- Per-Crate Test Commands

Define per-language shell commands for running tests. Can be set at workspace level or per-crate.

All fields accept either a single string or an array of strings (`StringOrVec`).

```toml
[[crates]]
name = "my-library"

[crates.test.python]
command = "pytest packages/python/tests/"
e2e = "cd e2e/python && pytest"
coverage = "pytest --cov packages/python/tests/"

[crates.test.node]
command = "npx vitest run"
coverage = "npx vitest run --coverage"

# Array syntax
[crates.test.rust]
command = ["cargo test", "cargo test --doc"]
```

| Field      | Type        | Description                                                                        |
| ---------- | ----------- | ---------------------------------------------------------------------------------- |
| `command`  | StringOrVec | Command(s) to run unit tests (used by `alef test`)                                 |
| `e2e`      | StringOrVec | Command(s) to run e2e tests (used when `alef test --e2e` is passed)                |
| `coverage` | StringOrVec | Command(s) to run tests with coverage (used when `alef test --coverage` is passed) |

---

## `[crates.update.<lang>]` -- Per-Crate Dependency Update Commands

Define per-language shell commands for updating dependencies. Built-in defaults are provided for all languages so this section is optional unless you need to override. Can be set at workspace level or per-crate.

All fields accept either a single string or an array of strings (`StringOrVec`).

```toml
[[crates]]
name = "my-library"

[crates.update.python]
update = "uv sync --upgrade-package"
upgrade = "uv sync -U"

[crates.update.node]
update = "pnpm up"
upgrade = "pnpm up --latest"

# Array syntax
[crates.update.rust]
update = ["cargo update", "cargo deny check advisories"]
```

| Field     | Type        | Description                                                              |
| --------- | ----------- | ------------------------------------------------------------------------ |
| `update`  | StringOrVec | Command(s) for safe, compatible updates (run by `alef update`)           |
| `upgrade` | StringOrVec | Command(s) for aggressive/latest updates (run by `alef update --latest`) |

---

## `[crates.setup.<lang>]` -- Per-Crate Dependency Installation Commands

Define per-language shell commands for installing dependencies. Built-in defaults are provided for all languages. Can be set at workspace level or per-crate.

All fields accept either a single string or an array of strings (`StringOrVec`).

```toml
[[crates]]
name = "my-library"

[crates.setup.python]
install = "uv sync"

[crates.setup.node]
install = "pnpm install"

# Array syntax
[crates.setup.java]
install = ["mvn dependency:resolve", "mvn dependency:resolve-sources"]
```

| Field     | Type        | Description                                              |
| --------- | ----------- | -------------------------------------------------------- |
| `install` | StringOrVec | Command(s) to install dependencies (run by `alef setup`) |

---

## `[crates.clean.<lang>]` -- Per-Crate Clean Commands

Define per-language shell commands for cleaning build artifacts. Built-in defaults are provided for all languages. Can be set at workspace level or per-crate.

All fields accept either a single string or an array of strings (`StringOrVec`).

```toml
[[crates]]
name = "my-library"

[crates.clean.rust]
clean = "cargo clean"

[crates.clean.node]
clean = "rm -rf node_modules dist"

# Array syntax
[crates.clean.java]
clean = ["mvn clean", "rm -rf target"]
```

| Field   | Type        | Description                                               |
| ------- | ----------- | --------------------------------------------------------- |
| `clean` | StringOrVec | Command(s) to clean build artifacts (run by `alef clean`) |

---

## `[crates.build_commands.<lang>]` -- Per-Crate Build Command Overrides

Override the default build commands for a language. Built-in defaults call `maturin`, `napi build`, `wasm-pack`, or `cargo build` + `cbindgen` as appropriate; use this section only when your project requires non-standard tooling. Can be set at workspace level or per-crate.

All fields accept either a single string or an array of strings (`StringOrVec`).

```toml
[[crates]]
name = "my-library"

[crates.build_commands.python]
build = "maturin develop"
build_release = "maturin build --release"

[crates.build_commands.node]
build = "napi build --platform"
build_release = "napi build --platform --release"

# Array syntax
[crates.build_commands.wasm]
build_release = ["wasm-pack build --target web", "wasm-pack build --target nodejs"]
```

| Field           | Type        | Description                                                   |
| --------------- | ----------- | ------------------------------------------------------------- |
| `build`         | StringOrVec | Command(s) for dev/debug builds (run by `alef build`)         |
| `build_release` | StringOrVec | Command(s) for release builds (run by `alef build --release`) |

---

## `[crates.e2e]` -- Per-Crate E2E Test Generation

Configure fixture-driven test generation per crate. Requires a `[crates.e2e]` section to use `alef e2e` subcommands.

```toml
[[crates]]
name = "my-library"

[crates.e2e]
fixtures = "fixtures"
output = "e2e"
languages = ["python", "node", "rust", "go"]

[crates.e2e.call]
function = "extract"
module = "my_library"
async = true
args = [
  { name = "path", field = "input.path", type = "string" },
]

[crates.e2e.registry]
output = "e2e-registry"
packages = { python = "my-library", node = "@myorg/my-library" }
categories = ["smoke", "basic"]
github_repo = "org/repo"
```

| Field                  | Type     | Default    | Description                                                                   |
| ---------------------- | -------- | ---------- | ----------------------------------------------------------------------------- |
| `fixtures`             | string   | _required_ | Directory containing JSON fixture files                                       |
| `output`               | string   | _required_ | Output directory for generated e2e test projects                              |
| `languages`            | string[] | _required_ | Languages to generate e2e tests for                                           |
| `call.function`        | string   | --         | Function name to invoke in generated tests                                    |
| `call.module`          | string   | --         | Module/package to import in generated tests                                   |
| `call.async`           | bool     | `false`    | Whether the function is async                                                 |
| `call.args`            | array    | `[]`       | Argument mappings from fixture fields to function parameters                  |
| `registry.output`      | string   | --         | Output directory for registry-based test projects (when `--registry` is used) |
| `registry.packages`    | map      | `{}`       | Map of language to published package name/identifier                          |
| `registry.categories`  | string[] | `[]`       | Fixture categories to include in registry test generation                     |
| `registry.github_repo` | string   | --         | GitHub repository identifier for release artifacts                            |

---

## `[crates.publish]` -- Per-Crate Release Pipeline Configuration

Configures the `alef publish prepare|build|package|validate` subcommands per crate. Used to vendor the core crate, cross-compile platform-specific FFI artifacts, and assemble distributable archives.

```toml
[[crates]]
name = "my-library"

[crates.publish]
core_crate = "crates/my-lib"

[crates.publish.languages.ruby]
vendor_mode = "core-only"
precondition = "command -v cargo >/dev/null 2>&1"
before = ["cargo build --release -p my-lib-rb"]

[crates.publish.languages.r]
vendor_mode = "full"

[crates.publish.languages.elixir]
vendor_mode = "core-only"
nif_versions = ["2.16", "2.17"]

[crates.publish.languages.c_ffi]
pkg_config = true
cmake_config = true
archive_format = "tar.gz"

[crates.publish.languages.go]
build_command = "cross build --release --target {target}"
package_command = "custom-packager"
```

| Field              | Type   | Default                                   | Description                                  |
| ------------------ | ------ | ----------------------------------------- | -------------------------------------------- |
| `core_crate`       | string | auto-detected from that crate's `sources` | Path to the core Rust crate to vendor        |
| `languages.<lang>` | map    | `{}`                                      | Per-language publish overrides (table below) |

### `[crates.publish.languages.<lang>]`

| Field             | Type        | Default              | Description                                                                           |
| ----------------- | ----------- | -------------------- | ------------------------------------------------------------------------------------- |
| `precondition`    | string      | --                   | Shell command that must exit 0 for publish steps to run; skip with warning on failure |
| `before`          | StringOrVec | --                   | Command(s) to run before main publish commands; aborts on failure                     |
| `after`           | StringOrVec | --                   | Command(s) to run after main publish commands; aborts on failure                      |
| `vendor_mode`     | string      | `"none"`             | One of `"core-only"` (Ruby/Elixir), `"full"` (R), or `"none"`                         |
| `nif_versions`    | string[]    | --                   | Elixir NIF versions to build for (e.g. `["2.16", "2.17"]`)                            |
| `build_command`   | StringOrVec | per-language default | Override the cross-compilation build command                                          |
| `package_command` | StringOrVec | per-language default | Override the packaging command                                                        |
| `archive_format`  | string      | per-language default | `"tar.gz"` or `"zip"`                                                                 |
| `pkg_config`      | bool        | `false`              | C FFI: generate a pkg-config `.pc` file                                               |
| `cmake_config`    | bool        | `false`              | C FFI: generate a CMake find module                                                   |

Language keys recognised by `[crates.publish.languages]`: `python`, `node`, `ruby`, `php`, `elixir`, `wasm`, `go`, `java`, `csharp`, `r`, `c_ffi`. The `c_ffi` key configures the standalone C FFI distribution (header + shared library + pkg-config + CMake).

---

## `[crates.opaque_types]` -- Per-Crate External Type Declarations

Declare types from external crates that alef cannot extract from source. These get opaque wrapper structs in all backends with handle-based FFI access.

```toml
[[crates]]
name = "my-library"

[crates.opaque_types]
Tree = "tree_sitter_language_pack::Tree"
```

Keys are the type name used in bindings. Values are the fully-qualified Rust path.

---

## `[crates.custom_modules]` / `[crates.custom_registrations]` -- Per-Crate Hand-Written Code

Declare hand-written modules that alef should include in `mod` declarations and module init registration.

### `[crates.custom_modules]`

Per-language lists of module names to include in the generated `mod` declarations:

```toml
[[crates]]
name = "my-library"

[crates.custom_modules]
python = ["custom_handler"]
```

### `[crates.custom_registrations.<lang>]`

Per-language registration of hand-written classes, functions, and init calls:

```toml
[crates.custom_registrations.python]
classes = ["CustomHandler"]
functions = ["custom_extract"]
init_calls = ["register_custom_types(m)?;"]
```

| Field        | Type     | Description                                                 |
| ------------ | -------- | ----------------------------------------------------------- |
| `classes`    | string[] | Class/type names to register in the module init             |
| `functions`  | string[] | Function names to register in the module init               |
| `init_calls` | string[] | Raw Rust expressions to include in the module init function |

---

## `[crates.custom_files]` -- Per-Crate Custom File Inclusion

Per-crate map of output group names to lists of custom file paths to include in generation.

```toml
[[crates]]
name = "my-library"

[crates.custom_files]
python = ["src/custom_helpers.py"]
node = ["src/custom_bridge.ts"]
all = ["LICENSE", "CHANGELOG.md"]
```

| Field     | Type     | Description                                                                |
| --------- | -------- | -------------------------------------------------------------------------- |
| `<group>` | string[] | List of custom file paths to include (group can be language name or `all`) |
