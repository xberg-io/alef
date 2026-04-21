# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.5] - 2026-04-21

### Added

- Codegen: visitor bridge generation for all backends (Go, Java, C#, Python, Ruby, Elixir, etc.)
- Codegen: `gen_visitor.rs` for Go, Java, C# backends
- E2E: visitor e2e test codegen for 8 languages

### Fixed

- Codegen: NAPI trait_bridge for napi-rs v3 compatibility
- Codegen: PHP trait_bridge for ext-php-rs 0.15 compatibility
- E2E: NodeConfig test fixes

## [0.4.4] - 2026-04-21

### Changed

- CLI: parallelize code generation, build, lint, and test loops with rayon
- CLI: parallelize file writing and diff checks across languages
- CLI: build commands within independent and FFI-dependent groups run concurrently
- CLI: switch rustfmt to stdin-based piping with explicit `--config-path` (thread-safe, respects `rustfmt.toml` from any CWD)
- CLI: lazy-compile fixed regex patterns in version sync with `LazyLock`
- Docs: replace `push_str(&format!())` with `write!()` macro to eliminate temporary string allocations
- Docs: pre-allocate `String::with_capacity(8192)` for markdown output buffers
- PyO3: use `ahash::AHashSet<&str>` instead of `std::collections::HashSet<String>` in code generation hot paths

### Added

- Docs: Rust language API reference generation
- Docs: tuple type detection and per-language rendering (`tuple[A, B]` for Python, `[A, B]` for TypeScript, `(A, B)` for Rust/Go/C#, `Tuple<A, B>` for Java)

### Fixed

- CLI: remove redundant double rustfmt pass on generated Rust files
- CLI: fix post-build file patch race condition by running patches sequentially after parallel builds
- CLI: fix lint/test output truncation — all language output is now printed before reporting errors
- E2E: resolve `field='input'` correctly in TypeScript codegen
- E2E: Tree method mapping, bytes arg type, features, json_object ref
- E2E: Option return handling, `default-features`, `is_none`/`is_some`
- Java: escape `*/` in Javadoc comments; TypeScript codegen updates
- PHP: per-fixture call routing and `json_encode` arg passing in PHP e2e
- PHP: add let-binding support for `Named` ref and `Vec<String>` ref params
- Python: resolve call config per fixture for correct function dispatch
- C#: fix `CS0029` type shadowing, `CS1503` nullable unwrap, `Vec<String>` serde conversion
- C#: add `using System`, fix `CS8910` copy constructor clash for newtype variants
- Rustler: use `_rustler` crate name in `native.ex` (not `_nif`)
- Adapters: use `_core` locals in streaming adapter body (not `.into()` on moved param)
- Codegen: fix codegen for opaque types, ref params, and `Vec<&str>`
- FFI: unnecessary clone removal; NAPI/Rustler/scaffold improvements
- Sync: don't blanket-replace versions in `pyproject.toml` `extra_paths`

### Removed

- Codegen: remove unused `create_env()` and dead `templates` module
- Codegen: remove unused `minijinja` dependency from alef-codegen

## [0.4.3] - 2026-04-20

### Added

- Codegen: trait bridge generation for NAPI, Magnus, Rustler, WASM, and PHP backends
- Codegen: `gen_error_class` for structured Python error hierarchies
- E2E: C codegen backend for e2e test generation
- E2E: Python e2e improvements (fixture handling, codegen updates)

### Fixed

- E2E: various codegen fixes across C and Python backends
- PHP: binding function generation fixes

## [0.4.2] - 2026-04-20

### Added

- Codegen: trait bridge code generation — auto-generate FFI bridges for Rust traits (PyO3 backend)
- Codegen: shared `TraitBridgeGenerator` trait and `TraitBridgeSpec` for cross-backend bridge generation
- Config: `[[trait_bridges]]` table for configuring trait bridge generation per trait
- IR: `super_traits` field on `TypeDef` for trait inheritance tracking
- IR: `has_default_impl` field on `MethodDef` for default method detection
- IR: `CoreWrapper::ArcMutex` variant for `Arc<Mutex<T>>` wrapping
- E2E: standalone mock server binary generation for cross-language e2e tests
- E2E: WASM e2e spawns standalone mock server via `globalSetup`
- E2E: `client_factory` override for WASM instance-method calls
- E2E: `count_equals` and `is_true` assertion types across all language targets
- Verify: blake3 output content hashing for idempotent verify

### Fixed

- PyO3: resolve `Named` types via `rust_path` lookup instead of constructing paths
- PyO3: use `py.detach()` instead of removed `allow_threads` (PyO3 0.28 compat)
- PyO3: fix 5 public API codegen bugs (python_safe_name, string escaping, param ordering, import filtering)
- PyO3: re-export return types from native module, not `options.py`
- NAPI: export enums as types for `verbatimModuleSyntax` compatibility
- C#: rename `record` param to `Value` when it clashes with variant or type name
- C#: add `using System` to `NativeMethods.cs` for `IntPtr`
- Go: marshal non-opaque receivers to JSON instead of using `r.ptr`
- Ruby: normalize dashes to underscores in ext dir paths, add extra `../` for native nesting
- Rustler: use `_rustler` suffix instead of `_nif` for crate name and path
- Codegen: don't apply Vec conversion on non-Vec enum variant fields
- E2E: WASM `globalSetup` paths, `BigInt()` wrapping for u64/i64, `new` constructor usage
- E2E: PHP async function naming, phpunit bump, namespace fixes
- E2E: resolve `input` field correctly for `options_type` import detection
- Scaffold: remove readme from scaffold output
- Sync: version regex matches pre-release suffix to prevent double-append
- Verify: write stubs manifest so output hashes cover stubs

## [0.4.1] - 2026-04-19

### Added

- E2E: `MockResponse` struct on `Fixture` for mock HTTP server support
- E2E: Rust mock server codegen — generates Axum-based mock servers from fixture `MockResponse` data
- E2E: always add `[workspace]` to generated Cargo.toml (prevents workspace inheritance issues)

### Fixed

- Codegen: upstream codegen fixes for liter-llm migration (multiple backends)
- Codegen: additional prek compliance fixes across all backends
- Codegen: add `clippy::allow` attrs to PyO3 and FFI backend generated code
- Codegen: Rustler NIF parameter type conversions and param handling refinement
- Codegen: NAPI — wrap boxed enum variant fields with `Box::new` in binding-to-core conversion
- Codegen: Python `options.py`/`api.py` docstrings on all public classes (D101)
- Codegen: Python D101 docstrings, mypy `dict` type args, exception docstrings
- Codegen: Java tagged union newtype variants use `@JsonUnwrapped`
- Codegen: PHP facade void return
- Codegen: WASM — prefix unused parameters with `_` in unimplemented stubs
- Codegen: handle `Vec<Optional<Json>>` field conversion (`Value` → `String`)
- Codegen: async unimplemented returns `PyNotImplementedError`
- E2E: add `cargo-machete` ignore for `serde_json` in Rust e2e Cargo.toml
- Scaffold: phpstan.neon generation for PHP
- Scaffold: safe `Cargo.toml` handling in `extra_paths` (prevents dep version corruption)
- Scaffold: WASM `package.json` + `tsconfig`, Duration lossy conversion fix
- Scaffold: remove `wasm-pack` `.gitignore` from `pkg/` after build
- Scaffold: don't gitignore `pkg/` — npm needs it for WASM publish
- Version sync: scan workspace excludes correctly
- Overwrite READMEs, e2e, and docs when `--clean` or standalone commands

## [0.4.0] - 2026-04-19

### Added

- Codegen: `Option<Option<T>>` flattening — binding structs use `Option<T>` with `.map(Some)` / `.flatten()` in From impls
- Codegen: RefMut functional pattern — `&mut self` methods on non-opaque types generate clone-mutate-return (`&self → Self`)
- Codegen: auto-derive `Default`, `Serialize`, `Deserialize` on all binding types and enums
- Codegen: builder-pattern methods returning `Self` now auto-delegate instead of emitting `compile_error!`
- Codegen: `can_generate_default_impl()` validation — skip Default derivation for structs with non-Default fields
- Config: configurable `type_prefix` for WASM and NAPI backends, default WASM to `Wasm`
- E2E: Brew/Homebrew CLI test generator (13th e2e language target)
- IR: `returns_cow` field on `FunctionDef` for Cow return type handling in free functions
- IR: `is_mut` field on `ParamDef` for `&mut T` parameter handling
- NAPI: conditional serde derives gated on `has_serde` for structs, enums, and tagged enum structs
- NAPI: tagged enum From impls distinguish binding-struct fields from serde-flattened String fields
- Scaffold: generate CMake config for FFI, add `build:ts` to Node scaffold
- Scaffold: add serde dependency to NAPI and Rustler scaffold templates
- WASM: `exclude_reexports` config for public API generation
- WASM: TypeScript re-export support for custom modules

### Fixed

- Codegen: `TypeRef::Char` in `gen_lossy_binding_to_core_fields` — use `.chars().next().unwrap_or('*')` instead of `.clone()`
- Codegen: optionalized Duration field conversion — handle `has_default` types where Duration is stored as `Option<u64>`
- Codegen: enum match arm field access — use destructured field name instead of `val.{name}`
- Codegen: Default derive conflicts in extendr/magnus backends
- Codegen: convert optional Named params with `.map(Into::into).as_ref()` for non-opaque ref params
- Codegen: pass owned `Vec<u8>` when `is_ref=false` for Bytes params
- Codegen: add explicit type annotations in let bindings to resolve E0283 ambiguity
- Codegen: skip `apply_core_wrapper_from_core` for sanitized fields (fixes Mutex clone)
- Codegen: replace `compile_error!` with `Default::default()` for Named returns without error variant
- Codegen: generate `Vec<Named>` let bindings for non-optional `is_ref=true` params
- Codegen: fix `Option<&T>` double-reference — don't add extra `&` when let binding already produces `Option<&T>`
- Codegen: correct float literal defaults (`0.0f32`/`0.0f64`) in unimplemented body for float return types
- Codegen: handle `&mut T` parameters via `is_mut` IR field — emit `&mut` refs instead of `&`
- Codegen: parse `TypeRef::Json` parameters with `serde_json::from_str()` instead of passing raw String
- Codegen: skip auto-delegation for trait-source methods on opaque types (prevents invalid Arc deref calls)
- Codegen: convert `Vec<BindingType>` to `Vec<CoreType>` via let bindings before passing to core functions
- Codegen: handle `Vec<Optional<Json>>` field conversion (`Value` → `String`)
- Codegen: async unimplemented returns `PyNotImplementedError` instead of `py.None()`
- Codegen: handle `Optional(Vec(Named))` in sync function return wrapping
- Codegen: `#[default]` on first enum variant for `derive(Default)` to work
- Extract: skip `#[cfg(...)]`-gated free functions during extraction
- Extract: prune non-re-exported items from private modules
- FFI: use `std::mem::drop()` instead of `.drop()` for owned receiver methods
- FFI: clone `&mut String` returns before CString conversion
- FFI: deserialize `Optional(Vec/Map)` JSON params and pass with `.as_deref()`
- FFI: handle `Option<&Path>` / `Option<PathBuf>` conversion from `Option<String>`
- FFI: handle `&Value` params by deserializing into owned Value then passing reference
- FFI: add explicit `Vec<_>` type annotations for serde deserialization of ref/mut params
- FFI: handle `returns_cow` — emit `.into_owned()` for `Cow<'_, T>` returns before boxing
- FFI: unwrap `Arc<T>` fields in accessors via `core_wrapper` check
- FFI: respect `is_ref` for `Path` and `String`/`Char` parameters
- FFI: correct codegen for `Option<Option<primitive>>` field getters
- FFI: collapse nested match for `Option<Option<T>>` getters
- NAPI: `Optional(Vec(Named))` return wrapping for free functions
- NAPI: U32 in tagged enum core→binding stays as u32, only U64/Usize/Isize cast to i64
- NAPI: detect and import `serde_json` when serde-based parameter conversion is needed
- NAPI: enum variant field casts use `.map()` for Optional wrapping
- PHP: float literals in `gen_php_unimplemented_body` (`0.0f32`/`0.0`)
- PHP: BTreeMap conversion via `.into_iter().collect()` in binding→core From
- PHP: `Map(_, Json)` sanitized fields use `Default::default()`
- PHP: enum returns use serde conversion instead of `.into()`
- PHP: exclude `PooledString` (ext-php-rs incompatible methods)
- PHP: filter excluded types from class registrations in `get_module`
- PHP: flatten nested `Option<Option<T>>` to single nullable type in stubs
- PHP: optionalized Duration in lossy binding→core conversion
- Scaffold: skip existing files, align Java `pom.xml` with kreuzberg
- Scaffold: remove `compilers` directive for Rustler 0.34+
- Scaffold: remove unused license format arg from C# template
- Scaffold: use `PackageLicenseFile` for C# (NuGet rejects non-OSI licenses)
- WASM: don't add Rust `pub mod` for custom modules
- Docs: add trailing newlines and wrap bare URLs (MD034)
- Docs: shift heading levels down for frontmatter compatibility

### Documentation

- Skills: add `--registry` flag, `[readme]` config, `[e2e.registry]` block, `custom_files` key, Brew e2e target
- Skills: update `alef all` description to include e2e + docs generation
- Skills: add `--set` flag to `sync-versions` documentation
- Skills/READMEs: update pre-commit rev tags to v0.3.5
- README: add `init` and `all` to alef-cli command list

## [0.3.5] - 2026-04-16

### Fixed

- Codegen: deserialize sanitized fields in binding-to-core enum variant conversion
- Codegen: skip trait types in all backend type generation loops
- Codegen: replace glob imports with explicit named imports in PyO3 and FFI backends
- Codegen: generate `From` impls for function return types and nested params
- FFI: generate `from_json`/`to_json` for types with sanitized fields
- FFI: remove unused explicit core imports from FFI backend
- FFI: normalize dashes to underscores in IR `rust_path` for code generation
- CLI: format generated Rust content before diffing in verify/diff commands
- E2E: use configured `class::function` for PHP calls regardless of `result_is_simple`
- Extract: check `lib.rs` as parent module for reexport shortening
- Resolve trait sources in post-processing to handle module ordering
- Normalize dashes to underscores in rust path comparison for `From` impl generation
- Scaffold: add `compilers` directive to Elixir `mix.exs` template
- Bump cbindgen scaffold version from 0.28 to 0.29

## [0.3.4] - 2026-04-15

### Added

- `alef sync-versions --set <version>` — set an explicit version (supports pre-release like `0.1.0-rc.1`)
- `alef verify` now checks version consistency across all package manifests
- `alef-sync-versions` pre-commit hook for automatic version propagation on Cargo.toml changes
- PEP 440 pre-release conversion for Python (`0.1.0-rc.1` → `0.1.0rc1`)
- `reverse_conversions` config flag to gate binding-to-core `From` impls
- E2E registry mode for test_apps generation across all 12 languages
- `alef-docs` fallback description generator for struct fields and enum variants
- `alef all` now includes e2e and docs generation; scaffold reads workspace version
- Elixir `ex_doc` support in scaffold
- Java scaffold: switch to `central-publishing-maven-plugin`; Node scaffold: `optionalDependencies`
- Enriched PHP/Python/Java scaffolds for CI and publishing
- PHP composer.json scaffold: `scripts` section with `phpstan`, `format`, `format:check`, `test`, `lint`, and `lint:fix` commands

### Fixed

- Docs generator: critical type, default value, enum, and error generation fixes across Go, C#, Rust
- Go doc signatures with empty return type; C# double `Async` suffix removed
- FFI codegen cast corrections; README version template rendering
- PHP scaffold autoload and backslash escaping in `composer.json` PSR-4 namespace
- PHP stubs: generate public property declarations on classes (ext-php-rs exposes fields as properties, PHPStan needs them declared)
- PHP stubs: camelCase naming; remove hardcoded `createEngineFromJson` from facade and stubs
- PHP codegen: remove needless borrow in `serde_json::to_value` calls (clippy fix)
- Python stubs: add `# noqa: A002` to constructor parameters that shadow Python builtins (e.g. `id`)
- Python stubs: place `noqa` comment after comma in multi-line `__init__` params
- Python scaffold: removed `[tool.ruff]` section — linter config belongs in root `pyproject.toml`
- Python duration stubs: correct mypy annotations
- Ruby: replace `serde_magnus` with `serde_json`, handle sanitized fields in `From` impls
- Ruby gemspec version sync: match single-quote `spec.version = '...'` (was only matching double quotes)
- Java: `checkLastError` throws `Throwable`; correct Jackson version to 2.19.0
- WASM: `option_duration` handling; added missing `wasm-bindgen-futures` dependency
- WASM codegen: remove unused `HashMap` import
- Node/FFI scaffolds: removed unused `serde` dependency
- Node scaffold: removed unused `serde_json` dependency
- Rustler backend: output path uses `_nif` suffix instead of `_rustler`
- Version sync: recursive `.csproj` scanning, WASM/root `composer.json`, FFI crate name
- Only generate binding-to-core `From` impls for input types, not output-only types
- `path_mismatch` false positive on re-exported types (same crate root + name)
- Add `Language::Rust` to all match arms across codebase
- Rust e2e: `assert!(bool)` for clippy, `is_empty` for len comparisons, unqualified imports
- E2e: add `setuptools packages=[]` to Python registry `pyproject.toml`
- Clippy `field_reassign_with_default` suppressed for Duration builder pattern
- Scaffold Cargo.toml templates: removed unused deps — `pyo3-async-runtimes` (Python), `serde_json` (Node), `tokio` (PHP, FFI), `wasm-bindgen-futures` (WASM), `serde`+`tokio` (Elixir/Rustler) — only include what generated binding code actually uses

## [0.3.3] - 2026-04-14

### Added

- Distributable Claude Code skill for alef consumers

### Fixed

- PHP 100% coverage — `createEngineFromJson`, JSON config e2e support
- PHP snake_case properties, enum scalars, serde casing, camelCase facade
- PHP class registration, property access, per-field attributes
- PHP setters documented, enum serde casing via `serde_json`
- PHP tests updated for `Api` class pattern (no more standalone `#[php_function]`)
- Correct DTO style naming (`typed-dict` not `typeddict`), add `serde_rename_all` to config docs
- Add Credo to Elixir scaffold, PHPStan + PHP-CS-Fixer to PHP scaffold
- Drop unused `tokio` from Python scaffold, add e2e license, split assertions
- Remove unused `serde` dep from Node.js and FFI scaffolds
- PHP and Rustler miscellaneous fixes

## [0.3.2] - 2026-04-13

### Fixed

- Disable `jsonschema` default features (`resolve-http`, `tls-aws-lc-rs`) to remove `reqwest`/`aws-lc-sys` from dependency tree, fixing GoReleaser cross-compilation

## [0.3.1] - 2026-04-13

### Fixed

- Elixir resource registration and enum conversion fixes
- C# opaque handle wrapping, error handling, `Json` to `object` type mapping, `WhenWritingDefault` serialization
- Dropped `x86_64-apple-darwin` from GoReleaser targets (`aws-lc-sys` cross-compile failure)

## [0.3.0] - 2026-04-13

### Added

- Pre-commit hook support for consumer repos (`alef-verify` and `alef-generate` hooks via `.pre-commit-hooks.yaml`)
- Blake3-based caching for stubs, docs, readme, scaffold, and e2e generation — all commands now skip work when inputs are unchanged
- `cargo binstall alef-cli` support via `[package.metadata.binstall]` metadata
- `alef-docs` and `alef-e2e` added to crates.io publish workflow

### Changed

- All workspace dependencies consolidated to root `Cargo.toml` — every crate now uses `{ workspace = true }` for both internal and shared external deps
- Bumped all crates from 0.2.0 to 0.3.0

### Fixed

- Replaced `.unwrap()` with `.extend()` in `to_snake_case` (`alef-adapters`)
- Replaced runtime `todo!()`/`panic!()` with `compile_error!()` in generated code for unimplemented adapter patterns
- Made `PrimitiveType` matches exhaustive, removing `unreachable!()` (`alef-codegen`)
- Added inline SAFETY comments to FFI pointer dereferences (`alef-backend-ffi`)
- Clamped negative Duration values before `u64` cast in NAPI bindings
- Fixed rustler public API test assertions to match Elixir conventions (no parens on zero-arg defs, keyword defstruct)

## [0.2.0] - 2026-04-13

### Added

- `alef e2e` CLI command with `generate`, `list`, `validate`, `scaffold`, `init` subcommands
- E2E test generators for 12 languages (Rust, Python, TypeScript, Go, Java, C#, PHP, Ruby, Elixir, R, WASM, C)
- `alef-e2e` crate with fixture loading, JSON Schema validation, and `FieldResolver` for nested field paths
- `options_type`, `enum_module`, `enum_fields`, `options_via` config support for flexible argument passing

### Fixed

- crates.io version specifier fixes for path dependencies
- Backend-specific e2e test generation fixes across multiple languages

## [0.1.0] - 2026-04-09

### Added

- Initial release with 20 crates in a Cargo workspace
- Full CLI: `extract`, `generate`, `stubs`, `scaffold`, `readme`, `docs`, `sync-versions`, `build`, `lint`, `test`, `verify`, `diff`, `all`, `init`
- 11 language backends: Python (PyO3), TypeScript (NAPI-RS), Ruby (Magnus), Go (cgo/FFI), Java (Panama FFM), C# (P/Invoke), PHP (ext-php-rs), Elixir (Rustler), R (extendr), WebAssembly (wasm-bindgen), C (FFI)
- Async bridging and adapter patterns: streaming, callback bridge, sync function, async method
- Method delegation with opaque type wrapping via `Arc<T>`
- Error type generation from `thiserror` enums with cross-language exception mapping
- Type alias and trait extraction from Rust source
- Blake3-based caching for `extract` and `generate` commands
- CI pipeline: cargo fmt, clippy, deny, machete, sort, taplo
- GoReleaser-based publish workflow with cross-platform binaries and Homebrew tap
