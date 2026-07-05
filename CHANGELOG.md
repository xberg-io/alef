# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.32.5] - 2026-07-05

### Changed

- **java**: scaffolded Maven packages no longer wire the Spotless Maven plugin
  or emit `eclipse-formatter.xml`; Java formatting is delegated to `poly` while
  Checkstyle remains focused on correctness checks.

### Fixed

- **rustler**: plugin trait registration stubs now include the
  `implemented_methods` parameter, matching the native Rust NIF signature and
  avoiding load-time arity failures.
- **kotlin-android**: generated JNI dispatchers are public so public native
  registration methods do not expose an internal parameter type or trigger JVM
  symbol name mangling.
- **swift-e2e**: `count_min` assertions over opaque scalar method-call fields
  now convert `RustString` values to Swift `String` before checking `.count`.
- **zig-e2e**: generated tests convert returned C string pointers with
  `std.mem.span()` before JSON parsing, formatting, or byte-length assertions.

## [0.32.4] - 2026-07-05

### Added

- **php**: `package_entry_filenames` now resolves the PHP public facade class
  file (`<ExtensionNamePascal>.php`, emitted in the public-API pass) so an
  extension's `public_api_additions` attaches to it, matching the existing
  Python/Ruby wiring. Go/Dart/Node emit their entry file in a different pass and
  remain a documented no-op.

### Fixed

- **trait-bridge**: sync infallible bridge methods no longer swallow host
  failures silently. A raised/thrown host callback is logged with the wrapper
  and method name before the default value is substituted (value-returning
  methods) or the call is discarded (unit methods), so a fabricated default —
  e.g. a zero token count that reads as "fits any budget" — is no longer
  indistinguishable from a real result. Covers pyo3, napi, magnus, php, wasm
  (console.error), jni (including the host-error envelope text the dispatcher
  already marshals), rustler, extendr, the csharp primitive-return adapter,
  and the ffi null-slot/null-result edge defaults.
- **go**: generated cgo trampolines recover host panics instead of crashing
  the process, logging to stderr and returning the zero value (fallible slots
  marshal the panic text through `outError`). The invalid-handle paths —
  including the four plugin lifecycle slots — log and marshal `outError`
  instead of fabricating `1` as a return value.
- **dart**: the block_on shim logs and returns the default when an infallible
  host callback panics, instead of aborting the calling thread via `expect`.
- **java**: sync infallible trait methods now match the vtable slot signature
  exactly. Primitive/unit returns use the direct-value convention (the previous
  JSON-convention upcall stubs mismatched the C slot — a wild pointer write
  plus the status code read back as the return value — breaking such methods on
  every call); infallible `Char`/`Path` slots no longer declare a phantom
  `outError`, and infallible `Optional<non-primitive>`/`Bytes` slots declare no
  out-pointers at all, mirroring `c_return_convention`.

## [0.32.2] - 2026-07-04

### Fixed

- **swift**: first-class DTO method wrappers (`{type}_{method}_from_json`) now
  honor owned and optional parameters. Optional params are declared as
  `Option<T>` in the wrapper signature (mirroring the extern block's
  `!needs_json_bridge` guard) instead of a bare `T`, and `String`/`Named` call
  args are borrowed only when the core parameter is a reference (`is_ref`).
  Methods taking owned `String` or `Option<T>` params (e.g.
  `Response::set_cookie` / `set_header`) previously failed to compile (E0308).

## [0.32.1] - 2026-07-04

### Fixed

- **napi**: async JS handlers are now awaited in the generated handler bridge.
  The threadsafe-function return type is `Either<Promise<HandlerReturn>,
  HandlerReturn>`, so a handler that returns a thenable routes to the `Promise`
  arm (awaited on the Rust side) and a plain object routes to the value arm —
  supporting both sync and async handlers. Previously a Promise return
  serialized to `{}` and dispatch failed with a missing-field error. Adds a
  `HandlerReturn` newtype implementing `ValidateNapiValue`/`TypeName`, because
  `serde_json::Value` cannot satisfy the `Either`/`Promise` bounds directly.
- **jni**: the generated handler-bridge struct and trait-object storage now use
  `jni::refs::Global<jni::objects::JObject<'static>>` instead of the
  `jni::objects::GlobalRef` alias. In jni 0.22.4 `GlobalRef` and the whole
  `jni::objects::*` reference-type re-export are `#[deprecated]`, so the old
  emission tripped deprecation errors under `-D warnings` in generated bindings.
- **swift**: DTO Unit-returning method wrappers no longer bind `let __value =
  ...?` when the ok type is `()`, which tripped `clippy::let_unit_value` in
  generated bindings under `-D warnings`.
- **pyo3** (#174): `.pyi` stub field annotations that shadow a builtin (e.g. a
  field named `bytes`) are now qualified as `builtins.bytes` for both the field
  and `__init__` signatures, and `gen_stubs` auto-imports `builtins` — fixing a
  `mypy --strict` `valid-type` error. Salvages the #173 regression test onto
  main (the `binding_fields` converter filter has been present since 0.31.0).

## [0.32.0] - 2026-07-04

### Added

- **pipeline**: `transform_scaffold_files` extension hook, letting extensions
  post-process generated scaffold files before they are written.

### Fixed

- **jni**: trait-bridge registration now dispatches. The kotlin-android bridge
  object wraps the host in a generated `<Trait>JniDispatcher` (suspend
  interface methods are bridged via `runBlocking`), and the generated Rust
  bridge routes every trait method through its JSON `dispatch` entry point —
  previously registration discarded the object and no plugin call ever reached
  the host. Rust-defaulted methods and the `Plugin` lifecycle hooks get the
  same presence-guarded forwarding as the other dynamic backends (#170).
- **swift**: first-class DTO instance methods now emit real dispatch instead of
  being excluded/crashing. The Swift side serializes `self`, calls a generated
  Rust wrapper extern, and decodes the JSON result; the Rust wrapper
  deserializes into the **core** type (not the serde-less swift-bridge wrapper
  newtype), converts `Path` params to `PathBuf`/`&Path`, and uses swift-bridge's
  unlabeled arguments + `RustString` return. Both the extern block and the Rust
  wrapper are emitted for non-opaque types (previously nested in the `is_opaque`
  branch, so the Swift calls referenced Rust wrappers that were never generated).
  Also fixes `Renderer` trait-bridge dispatch.
- **zig**: complex trait-vtable return types are serialized to JSON and handed
  back as a caller-owned, NUL-terminated C string via `out_result`, replacing a
  placeholder that silently wrote null. Uses the Zig 0.16 `std.json.fmt` API.
- **csharp**: `Register{Trait}(impl)` now delegates to `Register`, which calls
  the native `Register{Trait}` — previously it stored the bridge but never
  registered it natively (a silent no-op).
- **rustler**: opaque resources are stored behind `Arc<RwLock<T>>` so `&mut self`
  methods (e.g. `Registry::extend_from_dir`) mutate the held value in place
  through a write lock instead of returning `Not implemented` (or, worse,
  mutating a throwaway clone). Reads take a read lock; all lock acquisitions
  recover from poison (`unwrap_or_else(|e| e.into_inner())`) to avoid crashing
  the BEAM.
- **napi**: TypeScript service wrappers call the `native{UpperCamel}` methods the
  Rust `#[napi]` glue actually exposes (`nativeRun`/`nativeIntoRouter`), not the
  bare `run`/`intoRouter` which do not exist on the native class.

### Removed

- **pyo3**: dropped the never-rendered `trait_bridge/bridge_function.jinja`
  placeholder template and its registration.

## [0.31.2] - 2026-07-04

### Fixed

- **pyo3**: field-less `_from_native_*` options converters (types whose fields
  are all binding-excluded, e.g. `App`, `GraphQLRouteConfig`) now name their
  parameter `_native` and emit a bare `return X()`, so the unused parameter no
  longer trips ruff `ARG001` in the generated `options.py`.
- **pyo3**: the visitor `Protocol` stub's "Optional methods…" note is now gated
  on `emit_docstrings`, so the default no longer emits a docstring into the
  generated `.pyi` (ruff `PYI021`/`PYI013`).

## [0.31.1] - 2026-07-04

### Fixed

- **jni**: complete the `needless_borrows_for_generic_args` fix from 0.31.0.
  The 0.31.0 change only touched the inline Optional-JSON marshaller; the
  `string_to_jstring(env, &s)` warnings in generated shims actually originate
  in the return templates. Pass the owned `String` by value there too
  (`return_optional_string`, `return_json`, `streaming_shims`).

## [0.31.0] - 2026-07-04

### Added

- **config**: `[workspace.poly.pyrefly-sub-configs]` — a glob → error-code map
  emitted as extra `[[tool.pyrefly.sub-config]]` blocks in the generated
  `pyproject.toml` (alongside the built-in `api.py` block), so extensions can
  suppress type-checker errors on generated modules whose runtime-reconciled
  pyo3 boundaries a static checker cannot follow.

### Fixed

- **pyo3**: `_from_native_*` options converters now reference only the fields
  the `@dataclass` declares (via `binding_fields`), no longer passing
  binding-excluded fields (`methods_joined_cache`, `headers_joined_cache`,
  `lifecycle_hooks`, `di_container`, …) as keyword arguments — which raised
  `unexpected-keyword` at type-check time and `TypeError` at runtime.
- **codegen**: extra clippy allows (`[workspace] extra_clippy_allows`) are now
  filtered against the backend's default allow block emitted above them, so a
  lint that is already allowed is not re-emitted — clearing clippy's
  `duplicated_attributes` lint under `-D warnings`.
- **codegen**: `clippy::redundant_field_names` is now in the crate-level allow
  block of the php, pyo3, napi, wasm, and dart backends, silencing pre-existing
  warnings in generated binding crates under clippy 1.95.
- **jni**: the `Optional` return marshaller no longer borrows the owned
  serialized `String` when calling `string_to_jstring` (`&s` → `s`), clearing a
  `clippy::needless_borrows_for_generic_args` warning in every generated JNI
  shim.
- **ffi**: the generated `build.rs` capsule header fixup now emits direct
  `header.replace(...)` statements instead of a `for` loop over an array
  literal, clearing a `clippy::single_element_loop` warning when a crate
  exposes a single capsule pointee type.
- **pyo3**: `options.py` now imports `Any` whenever `_from_native_*` converters
  are emitted (their `native: Any` parameter), not only when a `TypeRef::Json`
  field is present, fixing an `unknown-name` type-check error.

## [0.30.19] - 2026-07-04

### Fixed

- **swift**: `Vec<opaque-handle>` getters on an opaque parent type now bridge as
  a real `Vec<T>` (e.g. `ExtractionResult.results()` yields
  `RustVec<ExtractedDocument>`) instead of `Vec<String>`, so opaque-element
  accessors such as `.mimeType()`/`.content()` resolve. JSON degradation of a
  `Vec<Named>` getter to `Vec<String>` is now gated on the containing type being
  a first-class Codable struct rather than on the element type, keeping the two
  code paths (`gen_bindings` DTO classification and `gen_rust_crate` extern/getter
  emission) in lockstep via a shared `compute_first_class_dto_names` helper.
- **trait-bridge**: dynamic-backend bridges (pyo3, magnus, php, napi, wasm,
  rustler, extendr) now forward Rust-defaulted trait methods to the host
  object when it implements them, falling back to the genuine Rust default
  body otherwise. Previously a host implementation of a defaulted method
  (e.g. `supports_table_detection`, `process_document`) was silently ignored
  and the Rust default always won (#167).
- **trait-bridge**: generated host surfaces (Python `Protocol`, Ruby `.rbs`,
  PHP `interface`, Elixir behaviour, Node `.d.ts`) now match the runtime
  contract: Rust-defaulted methods are no longer required members (documented
  as optional instead), Elixir behaviours gain `@optional_callbacks` plus the
  lifecycle callbacks, and Node plugin interfaces declare the optional
  lifecycle hooks. Bridges treat a missing `initialize`/`shutdown` as a no-op
  instead of failing registration. On magnus the bridge no longer invokes
  `initialize` — which is the Ruby constructor — on host objects (#166).
- **pyo3**: plugin `Protocol` config parameters are now typed as the public
  options dataclass the package exports, and the bridge passes that type to
  the host, so an implementer typed against the public API conforms to the
  Protocol (#165).
- **rustler**: behaviour `@callback` specs now declare natively-marshalled
  struct params as `map()` instead of the stale JSON `String.t()` (#168).

## [0.30.18] - 2026-07-03

### Added

- **extension**: `Extension::public_api_additions` is now honored for **Ruby**,
  not just Python. `package_init_filename` is generalized to
  `package_entry_filenames(language, &ResolvedCrateConfig)`, which resolves each
  language's package entry file — including dynamic conventions like Ruby's
  `lib/<gem_name_snake>.rb` — so an extension can wire its public API into the
  gem entry. Additions remain append-only with exact-line de-dup and still do
  not feed the generation-inputs hash (`alef verify` unaffected). Languages
  whose entry file is produced outside the public-API pass continue to be a
  silent no-op.
- **hooks**: `alef all`, `alef scaffold`, and `alef init` now run `poly hooks
  install` after scaffolding, wiring poly's pre-commit + commit-msg git hooks
  (polylint, polyfmt, file_safety, the `cargo` builtin — clippy / cargo-sort /
  machete / deny — and the conventional-commit hook) from the generated
  `poly.toml`. Best-effort and idempotent: a no-op when `poly` is absent or the
  target is not a git repository.

### Changed

- **format**: generated code is now formatted by the `poly` (polylint) CLI as a
  single system dependency — one `poly fmt --fix` pass replaces the previous ~19
  per-language formatter shell-outs (ruff, oxfmt, rubocop, php-cs-fixer, gofmt,
  google-java-format, ktfmt, swift-format, dart, gleam, zig, shfmt, …). poly is
  invoked as a subprocess rather than compiled in, keeping alef's build lean and
  its dependency tree unchanged; a missing `poly` binary is a best-effort no-op.
  The scaffolded `poly.toml` drives lint, format, cargo interop
  (clippy/sort/machete/deny), and the pre-commit + commit-msg hooks. A residual
  `cargo sort` still runs at generation time for workspace-excluded binding
  crates so `alef verify` stays hash-stable.

## [0.30.17] - 2026-07-03

### Fixed

- **swift**: getters returning `Vec<T>` or `Option<Vec<T>>` where `T` is a
  serde-serializable struct now JSON-decode each bridged element. The Rust
  bridge serializes such collections to `Vec<String>` (per-element JSON) or a
  single JSON `String`, but the generated swift wrapper previously emitted
  `.map { try T($0) }`, which only compiles for scalar `RustVec<RustString>`
  getters and left the binding uncompilable. It now decodes with `JSONDecoder`
  (per-element for `Vec<T>`, whole-array for `Option<Vec<T>>`). Fixes generated
  bindings for core types such as `CellChange`, `PageRange`, `PageSignals`,
  `LayoutDetection`, and `PageInfo`.

## [0.30.16] - 2026-07-03

### Added

- **extension**: new `Extension::public_api_additions(api, cfg, language)`
  hook. Extensions can now contribute raw lines to a package's public-API
  init file (e.g. Python's `__init__.py`) during public-API generation, once
  per resolved language. Returned lines are appended verbatim with exact-line
  de-duplication so re-runs are idempotent; the extension owns all language
  semantics (imports, `__all__` merges). The default implementation returns an
  empty list. The appended content does not feed the generation-inputs hash,
  so `alef verify` is unaffected.

## [0.30.15] - 2026-07-03

### Fixed

- **config**: scaffold language-specific tests (`test_scaffold_python`,
  `test_scaffold_node`, and 12 others) no longer fail after
  `feat(scaffold): emit canonical rustfmt.toml`. `rustfmt.toml` is a
  repo-level file like `poly.toml`; the `language_files` test helper now
  filters it out so file-count assertions in language-specific tests remain
  stable. The `crates/alpha/Cargo.toml` fixture in the
  `sync_versions_patches_dep_tables_on_version_change` test now includes a
  minimal `src/lib.rs` stub so `cargo update --workspace --offline` no longer
  prints a "no targets specified in the manifest" error to the test output.

- **cli**: `alef sync-versions` no longer regenerates test_apps/ and scaffold
  files by default, which was causing ~20min hangs on large repos. The command
  now only updates version fields in manifests and alef.toml; regeneration is
  the responsibility of explicit `alef generate`, `alef all`, or `task
  alef:generate` invocations. Use `--regen` flag to opt into the old behavior
  (expensive, not recommended for routine version syncs).

### Added

- **poly**: `[workspace.poly.typos]` in `alef.toml` now feeds typos
  spell-checker allowlists into the generated `poly.toml`. Declare
  `[workspace.poly.typos.extend-words]` and
  `[workspace.poly.typos.extend-identifiers]` (each a `word = "word"` table)
  to preserve repo-specific allowlists across every `alef all` regeneration.
  Previously, `alef generate` clobbered hand-edited `[lint.typos.*]` sections
  in `poly.toml`; those customisations must now live in `alef.toml` under
  `[workspace.poly.typos]` (fixes #66, enables #67).

- **config**: resolve `[[crates.source_crates]]` from the cargo registry via
  `from_registry = true`. When set, each `sources` entry is treated as relative
  to the crate's published source root (resolved through `cargo metadata`)
  instead of a workspace-relative sibling path, making regeneration hermetic in
  worktrees, CI, and fresh clones. Default (`false`) behavior is unchanged.

## [0.30.14] - 2026-07-03

### Fixed

- **swift**: fix the `ExtractedDocument.tables()` opaque-`Vec` marshaling SIGSEGV
  (called out as still-open in 0.30.13). A `Vec<Named struct>` getter on a serde
  type was emitted as an opaque `RustVec<Table>`, which swift-bridge cannot
  marshal safely — dereferencing it (e.g. `.tables().count`) crashed at runtime
  with SIGSEGV. Such getters are now bridged as a JSON `Vec<String>` (mirroring
  the existing `Vec<Named enum>` handling), yielding a countable, safely
  marshaled swift collection.

### Added

- **scaffold**: honor per-target core-dependency overrides in the scripting
  bindings (#164).

### Changed

- **style**: apply canonical poly formatting (rustfmt `max_width = 120`, taplo,
  oxc) across the jni/kotlin emitters, `deny.toml`, `renovate.json`, `.mcp.json`,
  and the e2e fixture schema.

## [0.30.13] - 2026-07-02

### Fixed

- **swift**: revert the broken Option-wrapping of non-optional JSON-bridged
  `Vec<T>` extern-block return types (introduced in 0.30.10). The wrapper
  declared `Option<String>` while the impl returned bare `String`, producing an
  E0308 type mismatch that failed every consuming swift binding's compile. The
  swift codegen now emits consistent `String`/`String`. (Does not address the
  separate `ExtractedDocument.tables()` opaque-`Vec` marshaling SIGSEGV.)

## [0.30.12] - 2026-07-02

### Added

- **scaffold**: the poly scaffold now also emits a canonical repo-root `rustfmt.toml`
  (`max_width = 120`, alef-managed). poly's Rust formatter defers to rustfmt's own
  config discovery (matching `cargo fmt`), so this pins the width both tools use;
  without it rustfmt falls back to its 100 default. Every alef-managed repo
  standardizes on 120 to match poly's global `line_length` default.

## [0.30.11] - 2026-07-02

### Added

- **config**: `[workspace] extra_clippy_allows` — a string list of additional clippy lints
  to allow in every generated Rust binding file. Entries may be bare lint names
  (`"single_match"`) or `clippy::`-prefixed (`"clippy::single_match"`); both forms are
  accepted and normalised internally. The configured lints are merged (union,
  de-duplicated; defaults first, extras appended) with each backend's built-in default
  allow-list, and a single extra `#![allow(...)]` attribute is emitted after the defaults.
  When the list is absent or empty the generated output is byte-identical to the previous
  behaviour. Affected backends: pyo3, napi, magnus, php, rustler, extendr, wasm, dart,
  swift.

  Example:

  ```toml
  [workspace]
  extra_clippy_allows = ["single_match", "collapsible_match"]
  ```

## [0.30.10] - 2026-07-02

### Fixed

- **pyo3**: exclude capsule types from `_rust`-qualified return annotations. Capsule types (both raw
  round-trip and `ConstructFrom`) resolve to a host type imported from another package (e.g.
  `tree_sitter.Parser`), not a native pyclass. Qualifying them with `_rust.` in a free function's
  return annotation produced an attribute (`_rust.Parser`) that no longer exists, raising
  `AttributeError` at import on Pythons with eager annotations (<3.14). They are now excluded from
  `return_type_names`, consistent with how they are special-cased elsewhere in api.py generation.
- **swift**: nil-safe accessor for non-optional JSON-bridged `Vec<T>` fields. Wrapping such a field in
  `Option<>` makes swift-bridge emit the nil-checked accessor, matching sibling accessors, so a null
  bridged pointer degrades gracefully instead of segfaulting. Defensive fix; the underlying
  null-pointer root cause is not yet confirmed.

### Changed

- **chore**: consolidate the typos allowlist into `poly.toml` and drop dead configs.

## [0.30.9] - 2026-07-02

### Fixed

- **codegen/ffi**: complete the service-owner forward-declaration fix from 0.30.8. The new
  `api.services` loop filtered by `exclude_types`, but a service owner is `binding_excluded` by
  construction and therefore always in that set — so the owner (`App`) was still dropped and the
  `typedef struct {PREFIX}App {PREFIX}App;` never emitted. Service owners are now forward-declared
  unconditionally (their `{PREFIX}{Service}Opaque.inner` pointer references them regardless of
  exclusion). Regression test tightened to mark the owner `binding_excluded`.

## [0.30.8] - 2026-07-02

### Fixed

- **codegen/ffi**: the C header no longer references an undeclared service-owner type. The cbindgen
  forward-declaration pass iterated `api.types`/`enums`/`errors` but not `api.services`, so a service
  owner (e.g. `App`) emitted as the opaque `inner` pointer of its `{PREFIX}{Service}Opaque` handle
  (`{PREFIX}App *inner`) had no `typedef struct {PREFIX}App {PREFIX}App;` — cbindgen then failed the
  downstream C/Go build with "unknown type name". Service owners are now forward-declared too
  (filtered by `exclude_types`). Declaring the owner in `[workspace.opaque_types]` is not required.
- **sync-versions**: three alef-emitted version sites were left at the prior version on every bump.
  - Root `Package.swift`: the `.binaryTarget` artifactbundle URL
    (`releases/download/vX.Y.Z/…`) was only updated via the `v__ALEF_SWIFT_VERSION__` placeholder,
    which is gone after the first sync — so subsequent bumps left the concrete tag stale (downstream
    `from: "X.Y.Z"` consumers fetched the wrong artifact). Now rewrites the concrete
    `releases/download/vX.Y.Z/` segment too, matching the shape `verify_versions` already checks.
  - C# `.csproj`: `<InformationalVersion>` was never rewritten (only `<Version>` was). Both are now
    bumped.
  - Ruby native (Magnus) crate `packages/ruby/ext/*/native/Cargo.toml`: the core-crate dependency
    pin (`<core> = { version = "X.Y.Z", path = "…" }`) drifted because this crate is not a workspace
    member and the workspace dep-pin pass never saw it. The pin now tracks the workspace version.

## [0.30.7] - 2026-07-02

### Fixed

- **codegen/pyo3**: `_to_rust_*` converters dropped all cfg-gated fields from the Rust constructor
  call (filter was `f.cfg.is_none()`). Feature-gated fields such as `UrlExtractionConfig.crawl`
  (gated on `any(feature = "url-ingestion", feature = "url-config-types")`) ARE compiled into the
  pyo3 `#[new]` constructor, so omitting them left them unset. Added `cfg_present_for_pyo3`
  (mirroring the `.pyi` stub's `cfg_present_for_pyo3_stub`): keep fields with no cfg or whose cfg
  resolves to present in the native pyo3 build (feature gates, `not(target_arch = "wasm32")`, or
  `any(...)` of those), while still dropping genuinely platform-specific fields.
- **maven**: pin jackson to `2.19.0`. jackson 2.20+ adopted a 2-component scheme (2.20/2.21/2.22)
  only partially on Maven Central (jackson-core/databind 2.22 and any x.y.0 return 404), breaking
  generated Java/Kotlin e2e dependency resolution. `2.19.0` is fully present across all five jackson
  artifacts.

## [0.30.6] - 2026-07-02

### Fixed

- `core_to_binding_convertible_types` false-negative: types whose only non-convertible binding
  fields are excluded from the backend surface (e.g. wasm `exclude_types`) were wrongly removed
  from the convertible set. The function now accepts `excluded_field_types: &[String]` and skips
  those fields in the predicate. All non-wasm backends pass `&[]`; the wasm backend passes its
  `exclude_types` list so structs with core-only omitted fields are correctly convertible.
- Wasm `gen_struct` emitted the delegating `impl Default` unconditionally for `has_default` types
  without checking convertibility, causing E0277 when `From<core::T>` was not generated.
  Non-convertible `has_default` wasm structs now correctly keep `#[derive(Default)]` instead.

## [0.30.5] - 2026-07-02

### Fixed

- **codegen/pyo3**: suppress delegating `Default` impl for types absent from `core_to_binding_convertible_types`. The struct generator emitted a delegating `impl Default` (calling `<core::T as Default>::default().into()`) for every `has_default` type, but `gen_from_core_to_binding` is only emitted when a type passes `can_generate_conversion`. A type with `has_default=true` whose fields include an unconvertible nested type received no `From<core::T>` impl, causing E0277 in the pyo3 backend (e.g. `ServerConfig`). Fixed by adding `emit_delegating_default_for_types: Option<&AHashSet<String>>` to `RustBindingConfig` and pre-computing the eligible set in the pyo3 backend before the type loop.
- **codegen/wasm**: apply `source_crate_remaps` inside `gen_delegating_default_impl`. When a `core_crate_override` remaps the leading crate segment (e.g. `spikard` → `spikard_http`), the delegating `Default` body used the raw `rust_path` verbatim, emitting `<spikard::ServerConfig as Default>::default().into()` instead of `<spikard_http::ServerConfig as Default>::default().into()`, causing E0433 in wasm. Fixed by calling `apply_crate_remaps` on the qualified path in `gen_delegating_default_impl` and threading `source_crate_remaps` through `RustBindingConfig`.

## [0.30.4] - 2026-07-02

### Fixed

- **defaults**: unwrap `Some(inner)` Rust defaults instead of collapsing them to `Empty`.
  `expr_to_default_value` had no `Some(...)` case in the `Expr::Call` arm, so `Option` fields with a
  `Some(literal)` default (e.g. `document_max_size: Some(50 * 1024 * 1024)`,
  `extraction_timeout_secs: Some(60)`) rendered as the type's zero value — Dart's `documentMaxSize`
  became `0`, truncating fetched documents to 0 bytes. The extractor now recurses into `Some(inner)`
  so the inner literal surfaces in synthesized default-config literals across every backend that
  emits them (dart/php/swift/…).
- **php**: map cfg-gated fields the binding keeps in the `From<binding>` conversion for core. The
  enum-tainted `From<binding>` generator unconditionally skipped every cfg-gated field, letting
  `..Default::default()` fill it. PHP keeps cfg-gated fields in the binding struct
  (`strip_cfg_fields_from_binding_struct = false`), so real values (`ExtractionConfig::keywords`,
  `UrlExtractionConfig::crawl`) were silently dropped on the PHP→core conversion. The skip is now
  gated on `strip_cfg_fields_from_binding_struct`, mirroring the standard `render.rs` path.
- **wasm**: infallible trait-bridge result conversion now returns `Option`. The `unwrap_or_default`
  branch chained `.and_then` on the `Option<String>` from `.as_string()` but the closure returned a
  `Result`, failing to compile (`E0308` expected `Option`, found `Result`; `E0425` unknown `e`). The
  closure now uses `.ok()`, fixing infallible trait methods that return enums/collections
  (`backend_type`, `processing_stage`, `supported_languages`, `dimensions`).
- **wasm**: add `--allow-multiple-definition` to the scaffolded `wasm32` rustflags.
  `wasm32-unknown-unknown` has no unified libc, so multiple C deps each ship functionally-equivalent
  libc stubs (tree-sitter's shim defines `__assert_fail`; a WASI-built Tesseract bundles
  wasi-libc `assert.o`/`atexit.o`) that `wasm-ld` rejects. The emitted `.cargo/config.toml` now
  passes first-def-wins linking, a no-op unless duplicates exist.
- **e2e/dart**: clear process-global plugin registries in `tearDownAll` to prevent a cross-isolate
  deadlock. Each Dart test file runs in its own isolate, but the Rust plugin registries are
  process-global; a file that registered a Dart-backed plugin left its `DartFnFuture` callback in the
  registry after its isolate died, and a later file's isolate deadlocked (30s timeout) invoking the
  dead callback via `block_on`. The generator now emits a `clear<Registry>()` call for each
  `register_*` backend fixture present in a file, taking the Dart e2e suite from 27 to 78 passing.

## [0.30.3] - 2026-07-01

### Changed

- **scaffold**: bump the generated e2e Java `jackson-databind` version (`JACKSON_E2E`) from
  2.18.2 to 2.22.0, matching the main jackson pin so regenerated e2e poms carry the security
  update instead of drifting from a manually-bumped dependency.
- **scaffold**: fold generated-test-code lint allowances into the emitter — `A001` and `N801`
  added to `TEST_IGNORES` (generated e2e tests take an `input` param shadowing the builtin;
  generated plugin trait-bridge stub classes aren't CapWords), and `I001` added to the
  `options.py` per-file-ignore. Consumer repos no longer need repo-specific `[workspace.poly]`
  overrides for these.

## [0.30.2] - 2026-07-01

### Added

- **config**: a `[workspace.poly]` section in `alef.toml` for repo-specific poly.toml overrides —
  extra `exclude` globs and cross-engine `per-file-ignores` that the scaffolder merges into the
  generated `poly.toml`, so repo-local lint suppressions survive regeneration.

### Changed

- **scaffold**: emit a single repo-root `poly.toml` that drives lint, format, git hooks, and
  commit-message policy, replacing `.pre-commit-config.yaml` and the per-tool config files
  (`[tool.ruff]`, `[tool.mypy]`, `phpstan.neon`, `.php-cs-fixer.dist.php`, `.lintr`, `.typos.toml`,
  `.rumdl.toml`). Python type-checking moves from mypy to pyrefly. The emitted config excludes
  Jinja templates from poly (reformatting them corrupts `{{ }}` placeholders) and carries
  generated-test-code lint allowances so regenerated e2e/test-app suites stay clean.

### Fixed

- **pyo3**: strip the Rust raw-identifier prefix in `.pyi` constructor params — PyO3 exposes a
  field declared `r#type` to Python as `type`, but the stub emitted `r#type` verbatim (invalid
  Python that ruff cannot parse). The `#[new]` signature keeps `r#` to compile.
- **pyo3**: drop the duplicate OptionsField trait-bridge parameter from the `.pyi __init__` stub.
  The field was emitted both as a regular param and as the dedicated bridge kwarg, producing a
  duplicate parameter; the stub now filters the bridge field out, mirroring `#[new]`.
- **pyo3**: drop the redundant closure when wrapping a zero-argument sync core call in
  `py.detach`. `py.detach(|| xberg::list_supported_formats())` tripped `clippy::redundant_closure`
  and failed `clippy -D warnings`; zero-arg calls now pass the function path directly
  (`py.detach(xberg::list_supported_formats)`). Calls that capture arguments keep the closure.
- **php**: generate the correct return type for `serde(default = "...")` helpers on fields whose
  core type is mirrored into a binding DTO. The helper returned the core type (e.g.
  `crawlberg::SsrfPolicy`) while the field is rendered as the crate-root mirror, so the generated
  php crate failed to compile (`expected SsrfPolicy, found crawlberg::SsrfPolicy`). The helper now
  returns the mirror and converts the core value via `.into()`.

## [0.30.1] - 2026-06-29

### Fixed

- **tests**: normalize docs-stage generated path assertions across Windows and Unix.
- **java**: always generate `ByteArraySerializer.java`. The generated ObjectMapper registers
  `new ByteArraySerializer()` unconditionally, but the class was only emitted when a record had a
  non-optional `Bytes` field — leaving a dangling reference that fails to compile for packages
  without one. It is now emitted unconditionally, matching `JsonUtil`.

## [0.30.0] - 2026-06-29

### Added

- **docs**: add a template-driven docs stage for API, CLI, MCP, `llms.txt`, agent skills, and
  snippet validation. Repos can configure generated reference output, required local templates for
  `llms.txt` and grouped skill files, static Clap/rmcp source extraction, and docs-specific snippet
  checks. Alef now warns on explicit skipped docs inputs such as missing configured sources or
  unavailable snippet toolchains while avoiding noisy warnings for unset optional docs layers.

- **snippets**: `typecheck` validation level. Ordered between `compile` and `run`, it statically
  type-checks a snippet without executing it, and for compiled languages without needing the native
  library. Each language runs its strict static checker: `python -m mypy`, `tsc --noEmit`,
  `cargo check`, `go vet`, `javac -Xlint:all -Werror`, `dotnet build -warnaserror`,
  `swiftc -typecheck -warnings-as-errors`, `kotlinc -Werror`, `dart analyze --fatal-infos`, and
  `cc -fsyntax-only -Wall -Werror`. This catches dual-representation mistakes (a config field typed
  against a flattened union alias that rejects the documented data-enum constructor) that
  `py_compile` and a lenient compile cannot see. A matching `snippet:typecheck-only` ceiling
  annotation sits alongside `syntax-only` and `compile-only`. mypy is optional: when it is not
  installed the Python snippet is reported as unavailable rather than failing.

### Fixed

- **napi**: give the generated streaming `WORKER_POOL` tokio runtime a 16 MB worker stack, so a
  deep consumer future does not overflow the default (~2 MB) worker stack and abort with `SIGBUS`.
- **pyo3**: provision an enlarged worker-thread stack on the generated module's async runtime.
  pyo3-async-runtimes' default multi-thread runtime gives workers a small (~2 MB) stack, which a
  deep consumer future (e.g. a multi-stage OCR pipeline) overflows — aborting the whole process
  with `SIGBUS`. The `#[pymodule]` init now installs a `tokio` runtime with a 16 MB
  `thread_stack_size` before the first `future_into_py`.
- **pyo3**: serialize `dict`/`list` values for JSON (`serde_json::Value`) config fields in the
  generated `api.py` converters. PyO3 cannot expose a settable `serde_json::Value` field, so the
  binding stores such fields as `str`, while the public dataclass and `.pyi` stub type them as
  `dict[str, Any]`. The converter forwarded the dict straight through, so the documented dict form
  raised `TypeError: 'dict' object is not an instance of 'str'` at runtime; it now `json.dumps`es a
  dict/list (passing `str`/`None` through unchanged).
- **pyo3**: re-point each re-exported exception's `__module__` at the public package in the
  generated `exceptions.py`. The classes are the native ones (`create_exception!` sets their
  module to the compiled `_native` extension), so tracebacks and `repr()` previously read
  `_native.DownloadError` instead of the public name, and the exceptions were not picklable under
  their public path. `exceptions.py` now reassigns `__module__` for every name in `__all__`
  (tree-sitter-language-pack issue #147).
- **codegen**: generate compiling binding→core conversions for core structs that have private
  (`pub(crate)`) fields. Such a struct cannot be built with struct-literal syntax from a foreign
  crate — neither by naming the private field nor by patching it with `..Default::default()` — so
  the conversion now seeds the core type's `Default` (which fills the private fields inside the
  defining crate) and assigns only the public fields onto it. The strategy is centralized in a
  shared helper used by the pyo3/napi/wasm/extendr/rustler/magnus generator, the Dart mirror crate
  generator, and the PHP enum-tainted conversion path; when the core type has private fields but no
  `Default`, a `compile_error!` guides the author to derive `Default`. A new `has_private_fields`
  flag on struct IR records the condition during extraction.
- **php**: marshal owned (by-value) native-struct callback parameters by value rather than
  dereferencing them as a borrow (`(*input)` does not type-check on an owned `core::T`), and stop
  emitting the native-object return fast-path — a PHP `#[php_class]` binding struct implements
  `FromZvalMut` (for `&mut T`) but not `FromZval` (for `T`), so the bridge keeps the JSON return
  path that is well-defined for PHP.
- **pyo3**: marshal owned (by-value) native-struct callback parameters into the host's native
  binding object via `From<core::T>`, the same way borrowed ones already were. A trait method that
  takes a serde struct by value (e.g. an extraction-input envelope) previously passed the raw
  `core::T` across the Python boundary, which has no `IntoPyObject` and failed to compile.
- **pyo3**: when a core `register_*` free function shares its name with a trait bridge's
  `register_fn`, emit only the bridge's duck-typed registration. The function loop no longer also
  emits the auto-wrapped core version, which collided (`E0428`) with the bridge definition and no
  longer type-checks against a registry that takes `Arc<dyn Trait>`.
- **pyo3**: the generated Python package now type-checks clean under `mypy`. Data-enum config fields
  are annotated against their public class (so `EmbeddingConfig(model=EmbeddingModelType.plugin(...))`
  is accepted) instead of a flattened union alias that shadowed the class; constructors accept the
  public dataclass/dict for factory parameters; data-enum `__init__` signatures match the runtime
  `#[new]`; `Json` maps to `dict[str, Any]`; and the duplicate `clear_*` registry stub is no longer
  emitted twice.
- **napi**: substitute binding-excluded types (e.g. `InternalDocument`) with `JsonValue` in the
  `.d.ts` host-interface signatures. Referencing a type that is never emitted produced an undefined
  TypeScript name; the runtime bridge marshals such values as JSON, so `JsonValue` is the faithful
  stand-in and `tsc --strict` is clean.
- **magnus**: apply the same excluded-type substitution (to `json_value`) in generated `.rbs`
  interfaces and skip re-declaring a bridge `clear_*` function that is already exposed as a registry
  function, so `rbs validate` no longer reports an undefined type or a duplicated method definition.

- **node/wasm**: require Node 22 or newer in generated npm package
  manifests, and keep Python package generation on Python 3.10 or newer.

- **e2e/dart**: resolve `config` JSON object helper types from compatible
  call overrides so generated tests use concrete helpers such as
  `createExtractionConfigFromJson`.

- **wasm**: filter cfg-gated struct fields with the WASM backend's active feature set so
  inactive fields are omitted and active fields are generated consistently across structs,
  constructors, accessors, and conversions.

- **r**: keep cfg-gated struct fields when the R backend's configured feature set enables
  them, and align R wrapper exports with the classes registered in `extendr_module!`.

- **scaffold**: let managed `.cargo/config.toml` render an explicit
  `rustc-wrapper`, and make the R Rust crate honor curated feature sets the
  same way as WASM by disabling core default features and declaring cfg
  passthrough features without enabling them by default.

- **r**: merge crate-level `extra_dependencies` into the generated R Rust
  crate so external DTO conversion impls can depend on sibling Rust crates
  such as `crawlberg`.

- **elixir**: render known generated public DTO fields in struct typespecs as
  their concrete module types instead of falling back to `map()`.

- **swift**: filter host Swift bindings with the same effective cfg feature set
  as the generated Rust bridge crate, including default cfg passthrough
  features.

- **swift**: wrap method-shim DTO returns for `Option<&T>` and `Vec<T>`, and
  pass `&Path` method parameters as borrowed paths instead of owned `PathBuf`s.

- **pyo3/magnus/wasm**: delegate generated binding defaults for defaultable
  DTOs to the core Rust `Default` impl so omitted nested config fields keep
  semantic core defaults.

- **extract**: support root-scoped external DTO source crates so host bindings
  can expand typed config graphs from sibling crates without exposing sibling
  functions or importing sibling language packages.

- **extract**: preserve explicit field `type_rust_path` values and reject
  same-name types from different crates, while keeping binding-excluded fields
  out of include-list expansion.

- **go/java**: avoid callback return local-name collisions in generated trait
  bridges when a method parameter is named `result`.

- **ffi**: keep cbindgen forward declarations for live binding DTOs when cfg-gated
  skipped duplicates leave older entries in Alef's excluded type-path map.

- **dart**: suppress ordinary trait-bridge lifecycle wrappers so FRB only sees the generated
  `{Trait}DartImpl` registration surface.

- **e2e**: emit typed single-call `json_object` inputs for Dart, Swift, and R so unified
  `extract(input, config)` fixtures pass their `ExtractInput` payload instead of defaulting it away.

- **pyo3**: include Pyo3-present cfg-gated fields in generated `.pyi` constructor stubs so native
  signatures and type stubs agree for typed nested configs such as `UrlExtractionConfig.crawl`.

- **dart**: normalize trailing whitespace in FRB-generated Dart files, including `*.freezed.dart`
  files that `dart format` leaves unchanged.

- **e2e**: prefer configured config DTO types when rendering Dart `config`
  JSON objects, preventing fallback helpers such as `createConfigFromJson`.

- **e2e**: include WASM nested DTO imports reached through `json_object`
  element types, such as per-input file configs nested under extract inputs.

- **elixir**: JSON-encode default-typed single DTO parameters before calling
  Rustler NIFs, matching the NIF boundary used for unified extract inputs.

## [0.29.4] - 2026-06-27

### Changed

- **tooling**: extend the `no-project-special-casing` pre-commit hook to reject the `xberg` and
  `crawlberg` downstream product names (case-insensitive, including camelCase and separator
  variants), and consolidate the brand allowlist so the `xberg-io` org namespace and the `xberg.io`
  domain stay permitted while `xberg-io/xberg` and bare `xberg` mentions are still caught. Neutralize
  the `xberg`-named Java/enum test fixtures to generic sample names.

### Fixed

- **e2e**: keep public Ruby and Elixir test calls on configured method names and
  resolve `$mock_url` placeholders inside typed JSON-array arguments across
  generated language e2e suites.

- **e2e**: resolve `$mock_url` placeholders for Ruby object arrays, Elixir typed
  object arguments, and Kotlin/PHP typed object setup while allowing Elixir e2e
  calls to target keyword-opts public facades.

- **e2e**: avoid Elixir typed-object variable collisions and align Kotlin typed
  object mock URL fallbacks with the generated mock-server harness.

- **node**: remove downstream internal DTO names from generated trait-bridge
  return-value comments.

- **ffi**: honor `[crates.ffi].exclude_types` when generating `cbindgen.toml`.
  Excluded Rust-only helper DTOs are now omitted from the header prelude forward
  declarations and emitted in `[export].exclude`, keeping C and cgo headers from
  leaking types that the FFI layer does not expose.

- **java/kotlin-android**: route configured trait-bridge lifecycle functions through the generated
  bridge APIs instead of also emitting ordinary FFI wrappers. This keeps raw Rust functions such as
  `register_document_extractor` from shadowing typed host interfaces (`IDocumentExtractor`,
  `IRenderer`) with dangling `DocumentExtractor`/`Renderer` parameter types or JSON-string JNI
  declarations.

## [0.29.3] - 2026-06-26

### Fixed

- **java/kotlin-android**: honor per-language `generate.async_wrappers = false` when emitting
  Java `CompletableFuture` helpers and Kotlin Android suspend convenience wrappers. This keeps
  bindings that want a single canonical method name from leaking extra `fooAsync` entrypoints while
  still preserving Rust functions that are themselves named `*_async`.

- **java (scaffold)**: derive the `maven-source-plugin` source include from the Maven group's first
  path segment instead of a hardcoded `dev/**`. After the `dev.kreuzberg` → `io.xberg` rebrand,
  generated sources moved to `io/<group>/…`, so the stale `dev/**` include matched nothing, the
  source jar came out empty, and Sonatype Central rejected the deployment with "Sources must be
  provided but not found in entries". The include now tracks the group (`io/**` for `io.xberg.*`).

## [0.29.2] - 2026-06-26

### Fixed

- **java**: read i32-returning FFM downcall results as `(int) (long)` instead of `(int)`. Since all
  integer FFM layouts are promoted to `JAVA_LONG` (for JBR Win64 Panama compatibility), the downcall
  handle returns `long`; casting the `invoke(...)` result straight to `(int)` forced an illegal
  `long → int` `asType` conversion that threw `WrongMethodTypeException` at the call boundary. This
  broke every byte-result method (e.g. `speech`, `fileContent`) and the trait-bridge
  register/unregister/clear lifecycle calls. The call sites now narrow via `(int) (long)`, matching
  the canonical pattern already used for `last_error_code`.

- **swift**: encode enum-typed struct field getters to match how the Swift side decodes each enum
  kind. Tagged enums (some variant carries data, e.g. `AssistantContent`) are serialized with
  `serde_json::to_string` of the source value and decoded via `JSONDecoder` — the discriminant-only
  bridge wrapper's `.to_string()` previously dropped the payload and returned an unquoted name (e.g.
  `Text`), which `JSONDecoder` rejected with "The given data was not valid JSON." Unit enums (all
  variants fieldless, e.g. `FinishReason`) keep returning their bare serde raw value via the wrapper's
  `.to_string()`, which Swift reconstructs with `Type(rawValue:)`; serializing those to JSON would
  emit a quoted string the rawValue init cannot parse.

- **elixir**: keep async NIF symbols suffixed internally while exposing async free functions under
  their original public names in the high-level Elixir facade. Generated modules now expose
  `extract/1` and `extract_batch/1` when the Rust API names are `extract` and `extract_batch`, while
  still delegating to `Native.extract_async/2` and `Native.extract_batch_async/2`.

- **magnus**: register suffixed async helper functions under their original public Ruby names. Ruby
  bindings now expose canonical methods such as `extract` and `extract_batch` even when the generated
  native helper functions are named `extract_async` and `extract_batch_async`; RBS stubs use the same
  public names.

### Removed

- **napi: stop generating the legacy `packages/typescript` wrapper package.** The napi backend no
  longer emits the `packages/typescript/src/index.ts` re-export barrel or its `bridges/*.ts` files;
  the native package (`crates/{lib}-node`, published with its own `index.d.ts`) is the canonical
  TypeScript surface, and `packages/node` is the modern package directory. `generate_public_api` for
  the napi backend now falls back to the default (no-op), and the existing orphan sweep removes any
  previously generated `packages/typescript/` tree on the next run. Version sync/checks and the e2e
  node package fallback now reference `packages/node` instead of the legacy `packages/typescript`.

### Added

- **e2e: support typed JSON-object arguments and `$mock_url` placeholders inside request DTOs.**
  Generated e2e tests now resolve non-array `json_object` argument types from per-argument metadata
  (`element_type`, and `go_type` for Go) before falling back to call-level `options_type`, so calls with
  separate request/config DTOs can be generated correctly. Structured JSON args can also embed
  `$mock_url`, which is replaced at test runtime with the fixture's mock-server URL.

- **e2e: accept fixture-level args, config, and route mocks in validation.**
  The embedded fixture schema now matches Alef's fixture model for per-fixture argument overrides,
  top-level `config`, `mock_response`, `setup`, `env`, and HTTP fixtures. Fixture loading mirrors
  top-level `config` into `input.config` before generation, and semantic missing-field validation now
  respects fixture-level `args`.

## [0.29.0] - 2026-06-26

### Fixed

- **pyo3 (Python): qualify builtin containers shadowed by a data-enum variant factory name.**
  A data enum with a `List` variant emits a `def list(...)` `@staticmethod` factory, which shadows the
  builtin `list` within the class body — so a sibling factory annotated `entries: list[MetadataEntry]`
  resolves to the factory and mypy rejects the `.pyi`
  (`Function ... is not valid as a type [valid-type]`). Factory annotations now qualify a shadowed
  builtin container (`list`/`dict`/`set`/`tuple`/`frozenset`/`type`) as `builtins.<name>[...]`, and the
  stub emits `import builtins` when referenced.

- **java: promote all integer FFM `FunctionDescriptor` layouts to `JAVA_LONG` for JBR Win64 Panama
  compat.** JetBrains Runtime's Panama linker casts every descriptor layout to `OfLong` internally, so
  any sub-64-bit integer layout (`JAVA_BYTE`/`JAVA_SHORT`/`JAVA_INT`) threw
  `ClassCastException: OfIntImpl cannot be cast to OfLong` at `NativeLib` class load and corrupted
  `TreeCursor` FFM calls. `java_ffi_type`, `service_api`, the enum-discriminant layout, the
  `LAST_ERROR_CODE` descriptor, and the visitor/trait-bridge/registration callback descriptors now
  emit `JAVA_LONG` for bool, 8/16/32-bit ints, and enum discriminants. `java_ffi_return_cast` emits
  compound narrowing casts (`(int)(long)`, `(short)(long)`, `(byte)(long)`) and the primitive-result
  templates no longer double-wrap them in parens. Generated `FunctionDescriptor`s now contain zero
  sub-64-bit integer layouts.

- **swift: add a runtime rpath to the generated `Package.swift` so the FFI dylib loads at runtime.**
  The `RustBridge` target emitted only `-L` (compile-time search). Because the FFI dylib's
  install_name is `@rpath/lib…dylib`, the consumer (and any test bundle linking the target) needs an
  `LC_RPATH` or `swift test` aborts with `dlopen … Library not loaded: @rpath/libhtml_to_markdown_ffi.dylib`.
  The manifest now derives the Cargo target dir absolutely from `#filePath` (CWD-independent, like the
  Zig/C e2e generators) and adds the rpath for both the release and debug profiles via the
  swiftc-native `-Xlinker -rpath -Xlinker <dir>` spelling (swiftc rejects `-Wl,-rpath,<dir>`). The e2e
  Swift package inherits the rpath transitively through this target.

- **extendr (R): skip per-variant factory constructors whose fields cannot cross the extendr input boundary.**
  A tagged data enum (e.g. `NodeContent`) generates a `_factory_<variant>` `#[extendr]` constructor per
  struct variant. When a variant field was a Named DTO (`grid: TableGrid`) or `Vec<DTO>`
  (`entries: Vec<MetadataEntry>`), the constructor took it _by value_, which the `#[extendr]` proc-macro
  cannot accept (`error[E0277]: T: TryFrom<&Robj> not satisfied`) — extendr derives `TryFrom<&Robj>` only
  for `&T`, never owned `T`, and has no R-list conversion for `Vec<DTO>`. `gen_extendr_enum_variant_constructors`
  and `extendr_enum_variant_constructor_registrations` now skip such variants (predicate
  `extendr_factory_param_is_constructible`); those variants remain constructible via the enum's `from_json`
  factory.

- **extendr (R): exclude methods with R-incompatible `Vec`/`Option<Vec>` params from `#[extendr]` impls.**
  Method filtering only dropped methods with bare-enum or bare owned-struct params; it missed
  `Vec<struct>`, `Vec<enum>`, `Vec<Vec<_>>`, and `Option<Vec<_>>` params. extendr generates no
  `TryFrom<&Robj>` for those, so the proc-macro failed downstream with
  `error[E0277]: T: TryFrom<&Robj> not satisfied` (e.g. `Vec<MetadataEntry>`). The two method-filter
  sites in `gen_bindings/mod.rs` now also apply the existing `is_extendr_native_incompatible` param
  check (already used for free functions), so such methods are omitted from the impl block.

- **php: per-variant constructor boxes `Box<T>` fields.** The flat-data-enum factory
  (`gen_flat_data_enum_variant_constructors`) emitted `field: field.clone().into()` for a variant
  field whose core type is `Box<T>`/`Option<Box<T>>` (Named `T`), which fails to compile (no
  `From<Binding> for Box<Core>`). It now wraps the converted value in `Box::new(...)` (or
  `.map(Box::new)` when optional), using the `VariantConstructor::boxed` flags — mirroring
  `flat_enum_binding_to_core_field_expr` and the shared `variant_field_init`.

- **magnus: per-variant constructors no longer collide with tagged-enum modules.** Tagged data enums
  are represented on the Ruby side as a `module <Name>` interface with per-variant `Data.define`
  classes, but the per-variant-constructor feature also emitted a Rust `module.define_class("<Name>")`
  with singleton factories. At load the `.so` defined the class first, so the pure-Ruby `module <Name>`
  raised `TypeError: <Name> is not a module` and the extension failed to load. Tagged data enums now
  skip the Rust factory class entirely — the class/singleton registration (`module_init`), the Rust
  `_factory_*` methods (avoids unused-method `-D warnings`), and the `.rbs` singleton stubs are all
  gated on `serde_tag.is_none()`. Construction for tagged enums goes through the variant `Data` classes
  (`<Name>Basic.new(...)`) and `from_hash`; non-tagged data enums keep their factory constructors.

### Added

- **Exception handling architecture guide and cross-language pattern documentation.** Added comprehensive
  `EXCEPTION_HANDLING.md` documenting exception/error handling patterns across all 15 language bindings
  (Python, Node.js, Ruby, PHP, Go, Java, C#, Elixir, WebAssembly, Dart, Swift, Kotlin Android, R, Zig, C FFI).
  Covers issue #147 (Python exception class identity), type identity preservation, error code standardization
  (1000+), and implementation checklists for new bindings. Ensures consistency across polyglot bindings.

- **CI resource optimization guide.** Added `CI_RESOURCE_OPTIMIZATION.md` documenting optimization strategies
  for large polyglot codebases (300+ grammars) on resource-constrained GitHub-hosted runners. Covers concurrency
  tuning (CLONE_CONCURRENCY=8, GENERATE_CONCURRENCY=2), sharding across parallel jobs, memory monitoring,
  and troubleshooting. Resolves exit-code 143 (SIGTERM) resource exhaustion issues.

- **PyO3 exception handling pattern documentation.** Enhanced `src/backends/pyo3/gen_bindings/errors.rs` with
  detailed cross-language exception handling patterns and core principle that exception class/type identity
  raised by native code must match the type exposed by public API. Reference for all polyglot backends.

### Trait-callback host returns accept the native binding object across the dynamic backends

  (pyo3, magnus, php, extendr).** Host-implementable trait callbacks already received native
  arguments (#142/#143), but the return value was still marshalled through a mapping/JSON path that
  rejected the binding's native result object even though the generated host interface advertised
  that type. Each dynamic backend's return path now tries the native object first
  (`extract::<Binding>()` / `TryConvert` / `FromZval` / `ExternalPtr` unwrap) and converts via
  `From<Binding> for Core`, falling back to the existing dict/array/hash/JSON path. The native path
  is gated on the binding→core conversion actually being generated (`convertible_types`), and extendr
  additionally gates on extendr-representability so non-representable rich types keep the JSON path. A
  shared `native_marshalled_struct_returns` classifier mirrors the param-side allowlist. On pyo3 the
  Protocol method also changes from `async def` to `def`, matching the `spawn_blocking` bridge that
  never awaited it. Resolves #153.

### Fixed

- **Per-variant constructors now box `Box<T>` fields.** When a data enum's struct variant has a
  field whose core type is `Box<T>`/`Option<Box<T>>` for a Named `T` (e.g. `CrawlEvent::Page {
  result: Box<CrawlPageResult> }`), the generated `_factory_<variant>` constructor emitted
  `result.into()`, which fails to compile because there is no `From<Binding> for Box<Core>`. The
  factory path now wraps the converted value (`Box::new(result.into())`, or
  `result.map(Into::into).map(Box::new)` for the optional case), mirroring the existing
  `From`/`Into` impl path (`conversions::binding_to_core::render`). The `is_boxed` flag is carried
  on `VariantConstructor` (parallel to `params`) and threaded into `variant_field_init`, so the
  pyo3, magnus, and extendr per-variant factories all box correctly.
- **pyo3 (Python): type stubs declare per-variant data-enum constructors.** The `.pyi` stub for a
  tagged data enum now emits a `@staticmethod` per data-carrying variant — `def circle(radius: float)
  -> Shape: ...` — between the tag attribute and the `__str__`/`__repr__` dunders, so type-checkers and
  IDE autocomplete see the `Shape.circle(...)` factories the runtime binding already exposed. The
  declared name is the public host name (`#[pyo3(name = "<snake>")]`), each param maps through the
  stub's `python_type` mapper, and the return type is the enum. Optional params — naturally optional
  fields and those promoted because they follow an optional one — render as `T | None = None`, matching
  the runtime constructor signature. Variant selection is shared with the runtime binding via
  `collect_variant_constructors`, so unit / tuple / `binding_excluded` / sanitized-field variants and
  hand-written method collisions are skipped identically.
- **magnus (Ruby): RBS stubs declare per-variant data-enum constructors.** The `.rbs` stub for a
  tagged data enum was an empty `class Shape ... end`; it now declares a singleton method per
  data-carrying variant — `def self.circle: (Float radius) -> Shape` — so RBS sees the
  `Shape.circle(...)` factories the runtime binding registers via `define_singleton_method`. The
  declared name is the bare snake_case host name, each param maps through the stub's `rbs_type`
  mapper, and the return type is the enum. Optional params — naturally optional fields and those
  promoted because they follow an optional one — render as the nilable `?T name` form, matching the
  runtime constructor signature. Variant selection is shared with the runtime binding via
  `collect_variant_constructors`, so unit / tuple / `binding_excluded` / sanitized-field variants and
  hand-written method collisions are skipped identically.
- **php: type stubs declare per-variant data-enum constructors.** The IDE/PHPStan stub for a tagged
  data enum (lowered to a flat PHP class) was an empty `final class Shape {}`; it now declares a
  static factory per data-carrying variant — `public static function circle(float $radius): Shape` —
  so PHPStan and IDEs see the `Shape::circle(...)` constructors the flat class exposes at runtime. The
  declared name is the camelCase host name (`to_php_name`), each param maps through the stub's
  `php_type` mapper (optional fields become `?T $x = null`), and the return type is the enum class.
  Variant selection is shared with the runtime binding via `collect_variant_constructors`, so unit /
  tuple / `binding_excluded` / sanitized-field variants and hand-written method collisions are skipped
  identically.
- **pyo3 (Python): enum-variant payloads accept the public dataclass/dict.** A data-enum
  per-variant constructor (e.g. `EmbeddingModelType.llm(...)`) now coerces a config-DTO payload the
  same way struct fields are coerced, so passing the public `LlmConfig` dataclass — or a plain
  `dict` — builds the variant instead of raising `TypeError: 'LlmConfig' object is not an instance
  of 'LlmConfig'`. Previously the generated factory demanded the compiled `#[pyclass]` instance
  while the package re-exported the pure-Python `@dataclass` for the same name, so the two never
  matched. A payload field whose type is a dataclass-backed config DTO — directly, or as a
  `list`/`dict`/`Optional` of one — is now generated as `&Bound<PyAny>` and routed through the
  module-level `__alef_coerce_dto` helpers (dataclass via `dataclasses.asdict` / dict / JSON-native
  → serde into the core type). Renamed fields round-trip with full fidelity: a per-DTO
  `__ALEF_WIRE_*` schema rewrites dataclass field names to serde wire names, honoring both
  `#[serde(rename)]` and `#[serde(rename_all)]` and recursing through nested DTOs, sequences, maps,
  and optionals — wire names are sourced from the same centralized naming transform the Python
  `_to_rust_*` converters use. Native re-exported return types stay compiled and are left untouched;
  the config-vs-native-return classification is shared with `__init__.py` import routing as a single
  source of truth (xberg #1165).

## [0.1.0 – 0.28.1] - 2026-04-09 – 2026-06-25

Early development history (592 releases through 0.28.1) has been trimmed to keep
this file small. The full per-version changelog is preserved in the git tags and
GitHub releases: <https://github.com/xberg-io/alef/releases>
