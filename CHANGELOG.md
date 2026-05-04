# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- fix(pyo3): use `val.kind.into()` instead of `Default::default()` for data enum fields in `From` impls. Data enums (like `StructureKind`) were added to `opaque_names_set` which caused the conversion generator to treat their fields as opaque and emit `Default::default()`. A separate `conversion_opaque_set` now excludes data enum names so their fields convert correctly with `.into()`.

- fix(e2e-ruby): use per-item text check for array field `contains` assertions. Previously `result.structure.to_s` produced an object-repr string that never matched the expected value. Now generates `array.any? { |item| alef_e2e_item_texts(item).any? { |t| t.include?(val) } }` using a shared helper method.

- fix(e2e-elixir): use `Enum.any?` for array field `contains` assertions instead of `to_string` on list. `to_string/1` on an Elixir list raises `ArgumentError`; now generates a per-item traversal using a shared `alef_e2e_item_texts/1` helper.

- fix(e2e-go): skip nil check and pointer dereference for `result_is_array` slice results. Go slice types (`[]string`) are not pointers, so the generated `if result == nil` check and `value := *result` dereference were compile errors. Array results now use `value := result` directly.

- fix(magnus-backend): add `options_field` visitor bridge support. The Magnus Ruby backend now generates proper wrapper functions for trait bridges using `bind_via = "options_field"` binding style. Previously only `bind_via = "function_param"` was supported, causing e2e tests to fail when trying to pass a visitor as a secondary argument to functions like `convert(html, visitor)`.

## [0.14.11] - 2026-05-03

### Fixed

- fix(e2e-python): fix `_to_rust_*()` converter field access when `python_output = "typed-dict"`. Previously the converter used `value.get("field")` dict-style access even for dataclass input types, causing `AttributeError` when called with `ProcessConfig` instances. Converters now use `value.field` attribute access (matching the input style) rather than the output style.

- fix(e2e-go): propagate `call_config.result_is_simple` to Go e2e generator. The generator was only checking per-language overrides, so call-level `result_is_simple = true` was ignored, causing `*string` returns to be treated as `error` and printing pointer addresses as failure messages.

- fix(e2e-php): add `options_type` support for PHP. When a call override sets `options_type = "ClassName"`, PHP e2e tests now generate `ClassName::from_json(json_encode([...]))` to construct typed config objects. Removes hardcoded `\HtmlToMarkdown\ConversionOptions::builder()` placeholder.

- fix(e2e-elixir): fix 0-arity functions (e.g. `language_count`, `available_languages`) called with `%{}` or `nil` when `args = []` and fixture input is empty. Now correctly emits no-arg calls.

- fix(e2e-ruby): same empty-args fix for Ruby — 0-arity functions no longer receive `nil` or `{}` from empty fixtures.

- fix(csharp-backend): store delegate objects in field array to prevent GC collection. The trait bridge generated code was creating 41+ `UnmanagedFunctionPointer` delegates as local variables in `BuildVtable()`, then immediately discarding them. The GC could collect these delegates, leaving dangling function pointers in the vtable and causing `AccessViolationException` on native callbacks. Now delegates are stored in `private readonly object[] _delegates` initialized in the constructor and kept alive for the bridge's lifetime.

## [0.14.10] - 2026-05-03

### Added

- feat(e2e-go): `client_factory` support in Go e2e codegen. When `[e2e.call.overrides.go] client_factory = "CreateClient"` is set, the generated test creates a client via `pkg.CreateClient("test-key", baseURL)` and calls API methods as `client.Method(args)` instead of `pkg.Function(args)`. Also fixes `fixture_has_go_callable` to return `true` when `client_factory` is configured.

- feat(e2e-python): `client_factory` support in Python e2e codegen. When `[e2e.call.overrides.python] client_factory = "create_client"` is set, the generated test imports only the factory function, creates a client, and dispatches calls as `client.method(args)`.

- feat(e2e-csharp): `client_factory` support in C# e2e codegen. When `[e2e.call.overrides.csharp] client_factory = "createClient"` is set, the generated test calls `ClassName.CreateClient("test-key", baseUrl)` and dispatches as `client.Method(args)`.

- feat(e2e-ruby): `client_factory` support in Ruby e2e codegen. When `[e2e.call.overrides.ruby] client_factory = "create_client"` is set, the generated spec creates a client via `ModuleName.create_client('test-key', ENV.fetch('MOCK_SERVER_URL'))` and calls methods on it.

- feat(e2e-elixir): `client_factory` support in Elixir e2e codegen. When `[e2e.call.overrides.elixir] client_factory = "create_client"` is set, the generated test calls `{:ok, client} = ModuleName.create_client("test-key", base_url)` and passes `client` as the first argument to API functions.

### Fixed

- fix(e2e): include fixture id in mock server base URL for C#/Ruby/Elixir e2e backends. Previously `client_factory` generated code pointed clients at bare `MOCK_SERVER_URL`; now appends `/fixtures/{fixture_id}` so API calls route correctly through the mock server's prefix-based routing.

- fix(e2e-php): parametrize extension name in generated `run_tests.php`; previously hardcoded to `libhtml_to_markdown_php`, now derived from the crate's extension name so generated test runners work across projects.

- fix(e2e-wasm): inject `initSync` initialization block in generated test files for Node.js environments. Uses `fs.readFileSync` + `initSync` to load the WASM binary from the local `node_modules` path, replacing the broken async fetch-based approach that fails in vitest's Node.js runner.

- fix(e2e): remove language suffix from generated e2e package names. TypeScript packages now use `{pkg}-e2e` instead of `{pkg}-e2e-typescript`; Python packages use `{pkg}-e2e` instead of `{pkg}-e2e-tests`. Python `pkg_name` now defaults to the call module name (replacing a stale hard-coded project name).

- fix(e2e-wasm): remove `initWasm()` call from generated test files. The bundler target auto-initializes when imported, so explicit initialization is unnecessary and causes `TypeError: initWasm is not a function`.

- fix(backend-java): use `LINKER.defaultLookup()` for Panama FFM symbol resolution. The `SymbolLookup.loaderLookup()` API returns symbols from Java's class loader, not the native linker; `LINKER.defaultLookup()` correctly returns symbols from all loaded native libraries.

## [0.14.9] - 2026-05-03

### Fixed

- fix(backend-pyo3): `_coerce_enum` helper now returns `_E` (not `_E | None`) by separating
  the `isinstance` early-return from the `None` case (which now raises `ValueError`). This
  eliminates the `[return-value]` mypy error and removes now-unused `# type: ignore` comments
  that newer mypy versions flag as `[unused-ignore]`.

- fix(backend-napi): `find_options_field_binding` now unwraps Optional types when matching
  options parameters against `options_type` in trait bridge configs. When a function parameter
  is `Option<ConversionOptions>` and a trait bridge specifies `bind_via = "options_field"`,
  the NAPI backend now correctly recognizes the parameter and generates the visitor bridge code.
  Fixes 45 failing Node.js visitor e2e tests where `options.visitor` was being discarded by
  the generated From impl.

- fix(e2e-php): emit `ConversionOptions::from_json()` instead of direct property assignment,
  since ext-php-rs doesn't support writable #[php(prop)] fields; fixes "No setter available for
  this property" errors in 7 e2e tests with preprocessing/metadata options

- fix(backend-go): initialize string-alias enum zero values as `EnumType("")` instead of `EnumType{}` (composite literal syntax only applies to structs/slices/maps, not string aliases)

- fix(backend-go): option setter functions for slice and map fields now assign `v` directly instead of `&v`, matching the `[]T` field types introduced in v0.14.5

- fix(backend-pyo3): fix `_coerce_enum` return type from `_E` to `_E | None` to allow None values
  in optional enum fields; remove unnecessary type: ignore comments that mypy flags as unused

- fix(backend-java): use short annotation names (`@JsonDeserialize`, `@JsonPOJOBuilder`) instead of
  fully-qualified names in generated record and builder classes so that the accompanying import
  statements are actually used and checkstyle's UnusedImports rule passes.

- fix(codegen): `gen_lossy_binding_to_core_fields` and `gen_lossy_binding_to_core_fields_mut` now
  accept `cast_uints_to_i32` and `cast_large_ints_to_f64` flags. All callers updated (magnus,
  integration tests). Extendr backend passes `true` for both to correctly cast R's i32/f64 types
  back to core Rust int types in generated method bodies.

- fix(e2e-php): do not append `_async` suffix to function name when a language-specific override
  explicitly provides the function name. Previously, async PHP functions with overridden names
  were incorrectly suffixed with `_async`.

## [0.14.8] - 2026-05-03

### Fixed

- fix(backend-ffi): `type_ref_to_rust_type` now prefixes named types with `kreuzberg::` so
  `Vec<BatchFileItem>` turbofish annotations in FFI bodies are emitted as
  `Vec<kreuzberg::BatchFileItem>`. The v0.14.6 fix added turbofish annotations without the
  crate prefix, causing "cannot find type X in this scope" E0425 errors in the FFI crate for
  all `Vec<T>`-parameter functions whose element type is a core library struct.

## [0.14.7] - 2026-05-03

### Fixed

- fix(backend-extendr): add `skip_impl_constructor` flag to `RustBindingConfig` and set it
  `true` for the extendr backend. The in-class `impl T { fn new(...) }` constructor is now
  suppressed; callers must use the kwargs-style free-function constructor (already generated
  by `gen_extendr_kwargs_constructor`). This prevents "TryFrom<&Robj> not satisfied" compile
  errors when struct parameters cannot be auto-converted by extendr.
- fix(backend-extendr): methods whose parameters take owned non-opaque structs by value, or
  whose return type is an enum, are now excluded from the `#[extendr]` impl block. Extendr
  generates `TryFrom<&Robj>` for references (`&T`) but not for owned values (`T`), and has
  no `ToVectorValue` impl for enum types; including such methods caused E0277 and E0277
  compile errors in the R binding.
- fix(backend-extendr): methods whose parameters take `Vec<T>` where T is a non-opaque,
  non-enum struct are also excluded from the `#[extendr]` impl block. There is no automatic
  R-list→Vec<ExternalPtr<T>> conversion in extendr.
- fix(codegen/extendr-kwargs): struct-typed fields (non-opaque, non-enum named types) are
  now omitted from the kwargs constructor parameter list and body. Extendr generates
  `TryFrom<&Robj>` only for `&T` (reference), not for owned `T`; including them as kwargs
  parameters caused "T: TryFrom<&Robj> not satisfied" compile errors.
- fix(codegen/extendr-kwargs): fields whose type is already `Option<T>` are no longer
  double-wrapped — the parameter type is now the same as the field type rather than
  `Option<Option<T>>`.
- fix(codegen/core-to-binding): `Vec<u8/u16/u32/i8/i16>` fields with `cast_uints_to_i32`
  are now converted element-wise (`v.iter().map(|&x| x as i32).collect()`) so R receives
  integer vectors instead of raw byte arrays.
- fix(backend-napi/pyo3/php): set `skip_impl_constructor: false` explicitly in those
  backends' `RustBindingConfig` initializers (no behaviour change; required after the new
  field was added to the struct).

## [0.14.6] - 2026-05-03

### Fixed

- fix(backend-ffi): `Vec<T>` parameters now always emit a concrete turbofish type annotation
  (`serde_json::from_str::<Vec<String>>(...)`) in generated FFI functions. Previously the
  annotation was only emitted when `is_ref || is_mut`; without it, calls to Rust core functions
  with generic bounds (e.g. `fn f<T: AsRef<str>>(v: Vec<T>)`) failed to compile with E0283
  "type annotations needed".

## [0.14.5] - 2026-05-03

### Fixed

- fix(backend-extendr): add `cast_uints_to_i32` and `cast_large_ints_to_f64` flags to
  `ConversionConfig` so the R binding maps u8/u16/u32/i8/i16 → `i32` and u64/usize/isize → `f64`
  in both binding→core and core→binding From impls. Without these casts, primitive type
  mismatches caused ~100 E0308 errors in the generated R binding.
- fix(backend-extendr): generate `#[extendr]` on impl blocks (previously `method_block_attr: None`)
  so generated structs register as R classes and satisfy the `ToVectorValue` trait bound.
- fix(backend-extendr): import `std::collections::HashMap` and `extendr_api::Result` in the
  generated binding so HashMap fields compile and `Result<T>` is unambiguously the extendr alias.
- fix(codegen/extendr-kwargs): optional struct fields (where `field.optional=true`) in the
  kwargs constructor now assign `Some(v)` instead of `v` so `Option<T>` fields are set correctly.
- fix(codegen/extendr-kwargs): enum fields in kwargs constructors now accept `Option<String>` and
  parse via serde_json instead of `Option<EnumType>`, which extendr cannot convert from Robj.
- fix(backend-extendr): skip opaque types that use `Rc` internally (identified by having a cfg
  gate) from struct generation and from field inclusion in non-opaque structs. `VisitorHandle` is
  `Rc<RefCell<dyn HtmlVisitor>>` and cannot be wrapped in `Arc` — its binding struct would
  violate `Send + Sync` bounds. Affected fields (e.g. `visitor` on `ConversionOptions`) are
  excluded from struct generation and conversion impls.
- fix(trait_bridge/extendr): use `List::from_pairs(attrs_pairs).into()` instead of the invalid
  `collect::<List>()` pattern for building named R lists from attribute key-value pairs.

- fix(backend-napi): NAPI options_field_bridge now preserves visitor handle across
  options conversion. The From<JsConversionOptions> impl was unconditionally setting
  visitor to Default::default() due to opaque field handling. Fixed by extracting
  the visitor before conversion and re-assigning it after the Into call.
- fix(backend-pyo3): PyO3 convert wrapper now reads visitor from options.visitor
  when the separate `visitor=` kwarg is None. This allows Python callers to pass
  the visitor via ConversionOptions without requiring a separate keyword argument.
- fix(e2e/wasm): replace static package imports with dynamic imports + top-level
  await to defer WASM module loading until after vite plugins have processed it.
  Fixes "failed to initialize WebAssembly bundle" errors in vitest e2e tests.
- fix(e2e/php): when `result_is_simple = true`, generated assertions now access the
  `content` property of the ConversionResult object instead of passing the object
  directly to assertion methods. Fixes TypeError when trimming or comparing results.
- fix(backend-pyo3): plain data structs now emit `#[pyclass(frozen)]` instead of
  `#[pyclass(unsendable)]`. The previous code used the transitive-closure set
  (`opaque_names_set`) to decide between frozen and unsendable, causing any struct
  whose fields transitively referenced an opaque type to be marked unsendable. Data
  structs are `Send + Sync` and crossing thread boundaries in async Python code must
  not panic with "<TypeName> is unsendable". Only truly-opaque Arc/Rc-wrapped handles
  need the unsendable annotation.
- fix(backend-go): `Vec<T>` function return types now emit `[]T` (a value slice) instead
  of `*[]T` (a pointer to a slice). Pointer-to-slice is unidiomatic in Go, breaks
  `len(result)` calls, and is unnecessary since Go slices are already reference types.
- fix(e2e/csharp): generate `List<string>` parameter type for VisitTableRow method
  (was `IReadOnlyList<string>`), emit `new VisitResult.XXX()` instead of
  `VisitResult.XXX()` to correctly instantiate sealed record types, and call
  `ConvertWithVisitor` instead of `Convert` when a visitor is present in fixtures.

## [0.14.4] - 2026-05-03

### Fixed

- fix(backend-pyo3): Required parameters whose Rust type is a has-default struct
  (e.g. `ExtractionConfig`) are now treated as optional at the Python wrapper layer.
  Callers may pass `None` and the wrapper substitutes a Rust default-constructed
  instance. Previously the PyO3 binding panicked with `.expect("'param' is required")`.
- feat(backend-pyo3): emit pass-through wrappers for trait-bridge `unregister_*` and
  `clear_*` functions in `api.py` so callers can use the public package path.
- fix(backend-go): Rust newtype-tuple enums (e.g.
  `enum OutputFormat { Plain, Markdown, Custom(String) }`) are now rendered as Go
  string types with const variants and a Custom fallthrough, instead of empty
  structs. Fixes JSON unmarshaling of config fields like `output_format: "markdown"`.
- fix(e2e/go): callable detection no longer skips fixtures with reason "non-HTTP
  fixture: Go binding does not expose a callable for the configured `[e2e.call]`
  function" — Go does expose the canonical 27 fns. Real test bodies are now
  emitted from the e2e generator, mirroring the Rust backend pattern.
- fix(backend-ffi): emit `visitor_create`/`free`/`convert_with_visitor` when both
  `options_field` and `visitor_callbacks` are configured.
- fix(wasm-visitor): use marker-relative search for visitor replacement; use
  bounded `replace` within impl blocks to avoid cross-impl pollution.
- feat(backend-napi): add `gen_options_field_bridge_function` for visitor
  embedding in options.

### Fixed (carry-forward from unreleased v0.14.3 follow-ups)

- fix(e2e/elixir): inject visitor into the options map argument instead of passing it as a
  separate positional argument, so the Elixir facade's `convert/2` can properly extract it
  via `Map.pop(options, :visitor)` and dispatch to the NIF with visitor callbacks.
- fix(e2e/elixir): read `returns_result` from the language override when available instead of
  always using the base CallConfig value, enabling `returns_result = true` in Elixir overrides
  to generate proper `{:ok, result} =` pattern matches in assertions.
- fix(backend-napi): when `bind_via = "options_field"` is configured for a trait bridge,
  extract the visitor from the options struct before conversion, create the bridge, and
  manually inject it into the converted core options. This ensures visitor callbacks
  are preserved across the NAPI FFI boundary instead of being dropped during the
  JsConversionOptions -> ConversionOptions Into conversion.
- fix(backend-pyo3): emit a `_coerce_enum(enum_cls, value)` helper in the generated
  api.py wrapper and use it in `_to_rust_*` converters instead of attempting
  `_rust.<Enum>(value)`. PyO3 unit-enum pyclasses do not expose a string-accepting
  `__new__`, so the previous codegen raised `TypeError: cannot create '<Enum>'
  instances` when callers passed string aliases like `'atx'`. The helper coerces
  strings, snake_case, UPPER_CASE, and CamelCase to the canonical class attribute.
- fix(backend-java): stop emitting illegal `Optional<String>` accessor methods that
  shadow the canonical record component accessor. Java records auto-generate the
  accessor with the field's declared type and disallow overrides with a different
  signature. Callers that want `Optional` can wrap with `Optional.ofNullable(...)`.
- fix(e2e/java): wrap nullable record-component accessors with `Optional.ofNullable`
  before calling `.orElse(...)`, so the existing assertion templates work whether the
  underlying type is `Optional<X>` or `@Nullable X`.
- fix(cli/build): resolve mvn working directory by walking up from `[crates.output].java`
  to the nearest `pom.xml`, so configurations that point output at `src/main/java/`
  still build via maven from the project root.
- fix(cli/build): when the cargo `[crates.output].<lang>` directory is itself inside a
  standalone crate that declares `[workspace]` (extendr's `packages/r/src/rust/`), `cd`
  into that crate and run `cargo build` instead of treating it as a workspace member.
- fix(cli): keep the test module after all items so clippy's `items_after_test_module` check
  passes in the full pre-commit suite.
- fix(e2e/typescript): simplify generated `is_empty` assertion emission so clippy remains
  warning-clean under `-D warnings`.
- fix(backend-pyo3): import native unit enums referenced only through data-enum aliases in generated
  `options.py`, keeping runtime aliases such as `ToolChoice = ToolChoiceMode | str | SpecificToolChoice`
  import-safe and ruff-clean.
- fix(e2e/go): derive generated Go e2e imports and local `replace` paths from `[go]` package
  configuration when no explicit `[e2e.packages.go]` entry is present, import `testify/assert`
  for every assertion kind that emits `assert.*`, and only emit direct Go calls when an explicit
  Go callable override exists.
- fix(cli/test): resolve e2e-only test languages in `alef test --e2e` without leaking the
  test-only resolver into scaffold generation.
- fix(e2e/elixir): emit fixture IDs as generated ExUnit test labels so long fixture
  descriptions cannot exceed Elixir's test name limit.
- fix(scaffold/ruby): omit workspace lints from standalone Ruby native extension crates.
- fix(e2e/csharp): default generated project references to Alef's nested C# package
  project path.
- fix(e2e/csharp): read namespace from `[crates.csharp].namespace` in alef.toml when
  generating test imports, fixing mismatch with generated package namespace.
- fix(e2e/go): render string containment assertions through `fmt.Sprint` so structured
  slices such as `[]StructureItem` compile.
- fix(backend-csharp): emit free P/Invoke declarations for all true opaque handle
  wrappers, including handles that only appear in generated wrapper classes.
- fix(backend-magnus): borrow field types when detecting thread-unsafe visitor
  handles so the backend compiles cleanly.
- fix(e2e/wasm): declare Rollup in generated WASM Vitest harnesses so Vite plugins
  can load in isolated installs.
- fix(e2e/go): compute imports from executable Go fixtures only so skip-only files
  compile without unused imports.
- fix(backend-go): decode Rust data-enum JSON into Go discriminated structs with
  a printable variant name instead of silently returning nil conversion results.
- fix(e2e/go): stringify structured array fields as JSON for containment assertions
  so pointer fields compare by value instead of address.
- fix(backend-go): marshal decoded data-enum variants back to their variant names so
  generated JSON-based e2e assertions can inspect them.

## [0.14.3] - 2026-05-03

### Added

- feat(codegen/doc-emission): shared `parse_rustdoc_sections`,
  `parse_arguments_bullets`, and `replace_fence_lang` helpers in
  `alef-codegen::doc_emission`. Rustdoc Markdown H1 headings (`# Arguments`,
  `# Returns`, `# Errors`, `# Panics`, `# Safety`, `# Example`/`# Examples`)
  are parsed into a typed `RustdocSections` struct that every host renderer
  consumes. ` ```rust ` fences inside `# Example` bodies are rewritten to
  host-native language tags (`typescript`, `php`, `csharp`, …); pound signs
  inside non-Rust fences (e.g. shell `# install …`) are preserved.
- feat(codegen/doc-emission): per-host section renderers
  `render_jsdoc_sections`, `render_javadoc_sections`,
  `render_csharp_xml_sections`, `render_phpdoc_sections`, and
  `render_doxygen_sections`. JSDoc emits `@param` / `@returns` / `@throws` /
  `@example` with ` ```typescript ` fences. JavaDoc emits `@param` / `@return`
  / `@throws KreuzbergRsException`. C# XML doc emits `<param>` / `<returns>`
  / `<exception cref="…">` / `<example><code language="csharp">`. PHPDoc emits
  `@param` / `@return` / `@throws` / ` ```php ` example fences. Doxygen emits
  `\param` / `\return` / `\code` / `\endcode`.
- feat(codegen/trait-bridge): shared `host_function_path` helper in
  `alef-codegen::generators::trait_bridge` replaces the duplicated per-backend
  `<lang>_host_function_path` inline helpers. pyo3, napi, magnus, ext-php-rs,
  rustler, extendr, and wasm now consume the same resolution logic.
- feat(backend-magnus/unregister-clear): emit `pub fn unregister_<name>(name:
  String) -> Result<(), magnus::Error>` and `pub fn clear_<plural>() ->
  Result<(), magnus::Error>` when `bridge_config.unregister_fn` / `clear_fn`
  are set. Errors are wrapped through `Ruby::exception_runtime_error`.
- feat(backend-php/unregister-clear): emit `#[php_function] pub fn
  unregister_<name>(name: String) -> ext_php_rs::prelude::PhpResult<()>` and
  the matching `clear_<plural>()` function. Errors are mapped through
  `PhpException::default(format!("{}", e))`.
- feat(backend-rustler/unregister-clear): emit `#[rustler::nif] pub fn
  unregister_<name>(env: rustler::Env<'_>, name: String) -> rustler::Atom`
  and `clear_<plural>()` returning `:ok` / `:error` atoms.
- feat(backend-wasm/unregister-clear): emit `#[wasm_bindgen] pub fn
  unregister_<name>(name: String) -> Result<(), JsValue>` and matching
  `clear_<plural>()` wrappers; surfaced via `gen_bindings/mod.rs` so the
  generated bridge re-exports them in `index.ts`.
- feat(backend-extendr/unregister-clear): emit `#[extendr] pub fn
  unregister_<name>(name: String)` / `clear_<plural>()` for R bindings.
- feat(backend-pyo3/host_function_path): pyo3 register/unregister/clear
  bridges now share the centralised host-function path resolver, eliminating
  the duplicated inline helper and matching the resolution rules used by all
  other backends.
- feat(backend-napi/host_function_path): napi register/unregister/clear
  bridges now share the centralised resolver.

### Fixed

- fix(backend-napi): preserve Rust `Default` values when converting omitted
  fields on defaultable JavaScript config objects instead of falling back to
  per-field zero values.
- fix(backends): remove dead PyO3 and Magnus backend code paths so workspace
  builds are warning-clean.
- fix(e2e): make TypeScript assertions handle `null` empty results and string
  containment checks against arrays of structured objects.
- fix(backend-napi/errors): JSDoc rendering for `KreuzbergError` variants now
  goes through the shared per-host renderer instead of leaking raw rustdoc
  Markdown headings into emitted error class JSDoc blocks.

## [0.14.2] - 2026-05-02

### Added

- feat(extract/normalize-rustdoc): doc comments parsed from Rust source are now run through
  a `normalize_rustdoc` pass before any backend sees them. Two specific rustdoc artefacts
  that previously leaked into every binding are removed:
  - Rustdoc-hidden lines inside ```rust /```rust,no_run fences (`# tokio_test::block_on(async {`,
    `# Ok::<(), Error>(())`, `# }`, `# async fn example()`) are dropped.
  - Intra-doc-link syntax `[`crate::Foo`]` / `[`super::Bar`] /`[`self::X`]` is rewritten to
    plain `` `Foo` `` / `` `Bar` `` / `` `X` ``.
  Per-host renderers (Javadoc, KDoc, PHPDoc, JSDoc, Dartdoc, Swift-DocC, roxygen2, Doxygen)
  continue to translate `# Errors`/`# Returns`/etc. as before — full per-host code-fence
  translation deferred to v0.14.3.

- feat(error-gen/strip-thiserror-placeholders): added `strip_thiserror_placeholders` and
  `acronym_aware_snake_phrase` helpers in `alef-codegen::error_gen`. Display strings emitted
  by binding error renderers (Go sentinels, Dart `String get message`) no longer contain raw
  `{name}` substitution markers (`OCR error: {message}` becomes `OCR error`, `extraction
  cancelled` is preserved). Variant names with technical acronyms (IO, OCR, PDF, URL, HTTP, …)
  render as `IO error` / `OCR error`, not `iO error` / `oCR error`.

- feat(backend-dart/error-render): Dart sealed-class exceptions now use the new placeholder
  stripping so `String get message => 'Parsing error: {message}'` is emitted as
  `'Parsing error'`. Surviving prose still has `'`, `\`, and `$` escaped per Dart string
  literal rules.

- feat(backend-zig/trait-bridge): the `make_<trait>_vtable` doc comment was missing a
  `format!()` call, leaking the literal `{snake}` template string. Substitution now
  happens correctly so generated docs read `make_post_processor_vtable(MyType, …)` etc.

- feat(trait-bridge/unregister-clear): `[[trait_bridges]]` schema gains optional
  `unregister_fn` and `clear_fn` fields. When set, alef emits matching host-language
  wrappers alongside `register_fn`. Pyo3 and napi backends opt in this release; remaining
  backends (magnus, php, rustler, gleam, extendr, dart, swift, kotlin, csharp, java,
  wasm) fall through to the default `None` (no emission) — those will be added in v0.14.3.
  The host-language wrappers call the user's plugin module function (e.g.
  `kreuzberg::plugins::ocr_backend::unregister_ocr_backend(&name)`); the path is derived
  from the bridge's `registry_getter` (`get_*_registry` → `*::*_fn_name`).

### Fixed

- fix(error-gen/go-sentinel): Go sentinel error strings no longer contain literal
  `{message}` / `{plugin_name}` / `{elapsed_ms}` placeholders. Variants like `IoError`,
  `OcrError`, `PdfError` render as `errors.New("IO error")`, `errors.New("OCR error")`,
  `errors.New("PDF error")` instead of the previous `errors.New("iO error")`,
  `errors.New("oCR error: {message}")`.

### Deferred to v0.14.3

- Per-host doc-comment section translation (`# Arguments` → `@param`, `# Returns` →
  `@returns`, `# Errors` → `@throws`, etc.) for JSDoc, JavaDoc, C# XML, PHPDoc, dartdoc,
  roxygen2, KDoc, Swift-DocC, Doxygen, Elixir doctests. Foundation in
  `alef-codegen::doc_emission` is unchanged.
- `gen_unregistration_fn` / `gen_clear_fn` for magnus, php, rustler, gleam, extendr,
  dart, swift, kotlin, csharp, java, wasm. Schema is in place; backends opt in incrementally.

### Fixed (carry-forward from unreleased v0.14.1 follow-ups)

- fix(cli): fix alef-verify ↔ host-formatter circular drift in `generate` and `all` commands.
  `format_generated` was called with only the bindings slice, so languages where only stubs
  changed (cache-hit bindings) were never formatted before `finalize_hashes` ran. Host
  formatters (ruff, mix format, php-cs-fixer, gofmt) would then reformat the unformatted
  stubs, making the embedded `alef:hash:` line stale and causing `alef verify` to report
  drift on every run. Fix: pass `bindings + stubs` combined to `format_generated` in both
  `Commands::Generate` and `Commands::All`, and track stub-changed languages in
  `changed_languages` in `Commands::All`.

- fix(e2e): add `node`/`wasm` default formatters (`npx oxfmt {dir}`) to `alef_e2e::format`.
  `run_formatters` had no entry for TypeScript e2e output, so hashes were embedded over raw
  codegen bytes; prek's oxfmt hook then reformatted those files, making `alef verify` report
  28 stale TS files after every `alef e2e generate` / `alef e2e generate --registry` run.

- fix(cli,e2e): add `cargo sort` to Ffi/Wasm/rust-e2e formatter pipelines before hash
  finalisation. `cargo-sort` normalises `Cargo.toml` dependency-table ordering and feature
  indentation; without it, prek's `cargo-sort` hook reformatted those files after
  `finalize_hashes` ran, causing `alef verify` to report `crates/*-wasm/Cargo.toml`,
  `e2e/rust/Cargo.toml`, and `test_apps/rust/Cargo.toml` as stale on every run.
  - `Language::Ffi` formatter now runs `cargo fmt --all` then `cargo sort -w` (workspace).
  - `Language::Wasm` formatter now runs `cargo fmt --manifest-path` then `cargo sort <crate>`.
  - E2e rust default formatter now runs `cargo fmt --all && cargo sort .` inside the crate dir.

- fix(php): respect namespace config in php_autoload_namespace. After v0.14.0, the namespace
  emission logic regressed and ignored the `[crates.php] namespace` configuration, instead
  splitting the extension name on underscores. Now respects explicit namespace override.

- fix(backend-wasm): bridge type aliases (e.g. `VisitorHandle`) are now treated as opaque in
  WASM binding-to-core `From` conversions. Fields of these types now emit `Default::default()`
  instead of `val.visitor.map(Into::into)` (which failed E0277 because `WasmVisitorHandle`
  has no `From` impl to the core `Rc<RefCell<dyn HtmlVisitor>>`). Builder methods on
  has-default structs with bridge parameters also use `.visitor(None)` instead of
  `.visitor(visitor.as_ref().map(|v| &v.inner))` to avoid E0308 type mismatch.

- fix(backend-php): bridge type aliases (e.g. `VisitorHandle`) are now included in
  `opaque_type_names` so that `gen_php_struct` emits `#[serde(skip)]` for fields of those
  types. Without this, structs like `ConversionOptions` that derive `serde::Serialize` +
  `serde::Deserialize` failed E0277 because `VisitorHandle` (an `Arc<Rc<RefCell<dyn
  HtmlVisitor>>>` wrapper) does not implement those traits. Builder methods with bridge
  parameters also use `.visitor(None)` to avoid E0308 type mismatch.

- fix(backend-pyo3): fixed three remaining codegen bugs for PyO3 bindings: (1) `PyVisitorRef::extract` now uses `Borrowed::to_owned()` instead of non-existent `Borrowed::clone()` to avoid E0507 "cannot move out of dereference"; (2) builder methods on has_default structs no longer attempt to access `.inner` for bridge type parameters—visitor parameter skips the core builder's `.visitor()` call and uses `None` instead to avoid E0308 type mismatch; (3) wrapper functions that pass has_default parameters to core functions now wrap the deserialized value in `Some()` when core expects `Option<T>` to fix E0308 "expected Option, found T".

- fix(backend-pyo3): `PyVisitorRef` now uses PyO3 0.28 API correctly. Updated `FromPyObject<'a, 'py>` to accept two lifetimes and use `extract(Borrowed<'a, 'py, PyAny>)` instead of the deprecated `extract_bound()`. Fixed `IntoPyObject` error type to use `Infallible` (conversion cannot fail). Resolves E0407, E0107, E0277, E0308 errors when building against PyO3 0.28.3.

- fix(backend-pyo3): trait types now map to `PyVisitorRef` (a custom wrapper) instead of `Arc<Py<PyAny>>`.
  `Arc<Py<PyAny>>` broke PyO3's `IntoPyObject`/`FromPyObject` field traits (E0277). `PyVisitorRef`
  wraps `Py<PyAny>` in `Arc<>` for cheap cloning while maintaining PyO3 compatibility at struct
  field and parameter boundaries. The wrapper implements Clone via Arc, FromPyObject/IntoPyObject
  for binding integration, and exposes `.inner: Arc<Py<PyAny>>` for trait bridge code generation.

- fix(backend-go): `NodeContext.NodeType` field type is now emitted as `NodeType` (the package-defined
  type alias for `string`) instead of the raw `string` type. The redundant `type NodeType = string`
  declaration in `visitor.go` that caused a redeclaration compile error with `binding.go`'s
  `type NodeType string` has been removed.

- fix(backend-rustler): `gen_native_ex` now emits a `convert_with_visitor/3` NIF stub when
  a `bind_via = "options_field"` bridge is present. Previously only `FunctionParam` bridges
  triggered the visitor-variant stub, causing an `on_load` failure at runtime.

- fix(backend-rustler): `gen_bridge_field_function` now correctly handles optional options
  params. When the core function expects `Option<ConversionOptions>`, the plain NIF no longer
  calls `.unwrap_or_default()` (preserving `None`), and the visitor NIF wraps `options_core`
  in `Some(...)` before passing to the core function.

- fix(alef-codegen/enums): resolved merge-conflict marker in `gen_enum` that broke builds.
  The `#[default]` attribute is now placed on the `is_default`-marked variant (falling back to
  the first variant), matching the Rust core's explicit `#[default]` placement.

- fix(napi): `core_to_binding` conversion now emits `Default::default()` for opaque Named fields
  with `CoreWrapper::None` (e.g. `visitor: Object<'static>`) instead of trying to wrap them
  with `Arc::new` — mirrors the same fix previously applied to `binding_to_core`.

- fix(napi): `gen_opaque_struct_methods` no longer emits `#[napi]` methods whose parameters
  include opaque-typed values by value; NAPI class types don't implement `FromNapiValue`
  and cannot appear as plain method params. Methods without an explicit adapter body are skipped.

- fix(napi/types): opaque Named types in `#[napi(object)]` struct fields are now mapped to
  `napi::bindgen_prelude::Object<'static>` instead of the NAPI class type, which fails the
  `FromNapiValue` bound required by `#[napi(object)]`.

- fix(napi/dts): opaque trait types (e.g. `JsHtmlVisitor`) are now included in the generated
  `index.d.ts`; previously they were excluded by the `!is_trait` filter.

- fix(backend-java): optional record fields now use `@Nullable` annotation instead of
  `Optional<T>` wrapper, making the Java API idiomatically correct. Fields are typed as
  nullable boxed types (e.g. `@Nullable Long` instead of `Optional<Long>`), improving
  compatibility with IDE inspections and code generators.

- fix(backend-java): Javadoc generation now strips Rust-specific syntax (intra-doc links
  like `[`Type`]`, code block fences, and Rust sections like `# Arguments`/`# Errors`)
  to produce valid Java documentation. The `emit_javadoc` helper function applies the
  transformation uniformly across all public API methods.

- fix(backend-java): builder `build()` methods now correctly extract values from `Optional`
  fields using `.orElse(null)` before passing to nullable record constructors.

- fix(backend-java): `convertWithVisitor` method now constructs `ConversionResult` with
  nullable fields instead of `Optional.of()` and `Optional.empty()` calls.

- fix(napi/dts): optional fields in `#[napi(object)]` structs are now correctly marked `?`
  when the field type is `TypeRef::Optional`, `field.optional` is set, or the parent type
  has `has_default = true`.

- fix(e2e/node): removed `"strictNullChecks": false` from e2e tsconfig; strict mode is now
  fully enabled.

- fix(e2e/node): visitor method template double-brace bug fixed and string literals in
  `Custom` callback returns are now properly quoted.

- fix(e2e/node): visitor method parameters are annotated `: any` to satisfy `noImplicitAny`.

- fix(java): bare `catch (Throwable ignored) { return 0; }` in VisitorBridge upcall stubs swallowed
  visitor exceptions silently. The catch clause now captures the first throwable in a sticky
  `volatile Throwable visitorError` field, returns `VISIT_RESULT_ERROR` (4), and surfaces the
  exception via `rethrowVisitorError()` in the `finally` block of `convertWithVisitor`.

- fix(java): `encodeVisitResult` allocated `Custom` markdown and `Error` message buffers into
  `Arena.global()`, leaking them permanently. The method is now a non-static instance method
  that allocates into `this.arena` (the bridge's confined arena), so all buffers are freed on
  `VisitorBridge.close()`.

- fix(java): `checkLastError()` always threw the base exception class. The method now
  dispatches on the native error code: code 1 throws `InvalidInputException`, code 2 throws
  `ConversionErrorException`, and any other code throws the base `{Class}Exception`.

- fix(java): `createObjectMapper()` was called once per FFI invocation, allocating a new
  `ObjectMapper` on every `convert()` call. The factory is now called once at class-load time
  to populate a `private static final ObjectMapper MAPPER` field.

- fix(java): stale generated files `IHtmlVisitor.java`, `HtmlVisitorBridge.java`,
  `TestVisitor.java`, `TestVisitorAdapter.java`, and `VisitContext.java` were emitted by the
  trait-bridge and gen-visitor paths. They are superseded by `Visitor.java` and
  `VisitorBridge.java` in the Panama FFM visitor pattern. All five files are removed from
  generation and deleted from committed output.

- fix(java): `has_visitor_pattern` was evaluated using `config.ffi.visitor_callbacks` which is
  always `None` in the Java backend's `ResolvedCrateConfig`. The check now also activates when
  any `[[trait_bridges]]` entry uses `bind_via = "options_field"`, which is the Java-specific
  visitor activation path.

- fix(java): output path defaulted to `packages/java/` from the workspace template, causing
  generated files to land in `packages/java/dev/kreuzberg/htmltomarkdown/` instead of the
  correct `packages/java/src/main/java/dev/kreuzberg/htmltomarkdown/`. The Java output path is
  now resolved via an explicit `java = "packages/java/src/main/java/"` entry in `[crates.output]`.

## [0.14.1] - 2026-05-02

### Added

- feat(trait-bridge): `[[trait_bridges]]` schema now supports optional `unregister_fn` and
  `clear_fn` fields. When present, alef emits the corresponding host-language wrappers
  alongside the existing `register_fn` codegen. Pyo3 backend implements both new methods;
  remaining backends fall through to the default `None` (no emission).

- feat(error-gen): error-Display strings now have template placeholders (`{message}`,
  `{plugin_name}`, `{elapsed_ms}`, `{limit_ms}`, `{0}`-style positional, etc.) stripped
  before emission. Acronym-aware variant-name splitting recognizes 40+ technical
  acronyms (IO, OCR, PDF, URL, HTTP, TCP, …) and preserves them in the rendered text:
  `IoError` → "IO error", not "iO error". Applies across all error-handling backends.

- feat(pyo3): `register_*` plugin docstrings now use a humanized noun derived from the
  trait name. Previously emitted `"""Register a register_ocr_backend backend."""`
  (placeholder leaked); now emits `"""Register a OCR backend implementation as a runtime plugin."""`.

### Fixed

- fix(extendr): `generate_public_api` no longer emits a duplicate `convert()` wrapper calling
  `.Call("htm_convert", ...)`. extendr generates `extendr-wrappers.R` with the correct
  `.Call(wrap__convert, ...)` symbol; the backend now emits only `@useDynLib` + `options.R`.

- fix(extendr): generated `options.R` now includes all `ConversionOptions` fields from the IR,
  including `exclude_selectors`, `max_image_size`, `capture_svg`, `infer_dimensions`, `max_depth`,
  `skip_images`, `link_style`, `output_format`, `include_document_structure`, `extract_images`,
  and `visitor` (previously missing).

- fix(extendr): visitor pairlist keys no longer carry a leading underscore. Rust uses `_ctx` /
  `_href` for unused default-impl parameters; R callers write `function(ctx, ...)` without the
  prefix. Keys are now trimmed of leading `_`.

- fix(e2e/r): `CallbackAction::Custom` / `CustomTemplate` output values are now quoted in generated
  R: `list(custom = "[AUDIO: podcast.mp3]")` instead of the previous unquoted form that caused a
  parse error.

- fix(e2e/r): `visitor` is now passed inside `options = list(visitor = visitor)` rather than as a
  top-level parameter to `convert()`.

- fix(backend-wasm): use `is_default` variant for generated `Default` impl on wasm enums instead
  of always using the first variant, fixing incorrect defaults for `HeadingStyle`, `CodeBlockStyle`,
  and `PreprocessingPreset`.

- fix(backend-wasm): visitor bridge methods now dispatch `{custom: "..."}` JS objects to
  `VisitResult::Custom(s)` and `{error: "..."}` objects to `VisitResult::Continue`, making the
  full `VisitResult` variant set reachable from JavaScript.

- fix(e2e/wasm): merge visitor into the options object (2nd arg) rather than appending it as a
  standalone 3rd argument, matching the wasm binding's single-object options API.

- fix(backend-napi): use generated `From` conversions for named reference parameters instead of
  JSON round-tripping when let-binding delegation is possible.

- fix(backend-pyo3): generate `__str__` and `__repr__` for Rust-backed enum wrappers so returned
  enum values are inspectable from Python.

- fix(e2e/python): render string containment assertions over configured array fields by checking common DTO text
  attributes instead of comparing directly against the object list.

- fix(alef-cli/format): format WASM binding crates with `cargo fmt --manifest-path`
  derived from the resolved output path so renamed or workspace-excluded crates are handled correctly.

- fix(codegen): use generated `From` conversions for named reference parameters instead of JSON round-tripping when a
  direct let-binding is possible. This preserves fields such as Python `ProcessConfig.language` in free functions.
- fix(codegen): convert Cow-backed string fields with `.into()` when reconstructing `core_self` for binding methods.

- fix(cli): run language-native formatters on stubs before finalising the embedded `alef:hash:` line.
  `alef stubs` previously skipped the format step and computed the hash over raw codegen output.
  When host-language tools (ruff, php-cs-fixer, mix format, cargo fmt, …) reformatted those files,
  the embedded hash no longer matched the on-disk content, causing `alef verify` to report them as stale.

- fix(cli): track stub-changed languages in `changed_languages` during `alef generate`.
  When stubs changed but no bindings changed for a language, the formatter gate
  (`if any_written && !changed_languages.is_empty()`) would skip formatting entirely,
  leaving stub files unformatted and the hash stale after the next formatter run.

- fix(cli): always add stub file paths to `current_gen_paths` before the orphan-sweep pass
  in `alef generate`. When the stub cache was warm, stub paths were not registered,
  causing the sweep to delete freshly-generated stub files (e.g. `.pyi`) as orphans
  on the second generate run.

- fix(csharp): `ConversionOptionsBuilder.Dispose()` and `VisitorHandle.Dispose()` were
  no-op stubs. Opaque handles now emit a `{Name}SafeHandle : SafeHandle` inner class for
  deterministic, exception-safe cleanup. The public wrapper delegates `Dispose()` to it.

- fix(csharp): `optionsHandle` and `visitorHandle` were leaked when `Convert()` or
  `ConvertWithVisitor()` threw. Both handles are now freed in a unified `try/finally` block.

- fix(csharp): `Marshal.PtrToStringAnsi` used throughout — replaced everywhere with
  `Marshal.PtrToStringUTF8` so non-ASCII characters (e.g. Unicode URLs, CJK text) are
  decoded correctly on all platforms.

- fix(csharp): typed exceptions (`InvalidInputException`, `ConversionErrorException`) were
  never thrown. `GetLastError()` now dispatches on the native error code and emits `if`
  branches for each registered exception type.

- fix(csharp): `NodeContext.NodeType` was typed `int` instead of the `NodeType` enum.
  `DecodeNodeContext` now casts `Marshal.ReadInt32` to `(NodeType)`.

- fix(csharp): stale generated files `IVisitor.cs` and `VisitorCallbacks.cs` were still
  emitted by `gen_visitor_files`. They are superseded by `IHtmlVisitor`/`HtmlVisitorBridge`
  in the trait-bridge pattern. Both files are removed from generation and deleted from
  committed output.

- fix(csharp): bare `catch { return 0; }` in visitor upcall thunks swallowed exceptions
  silently. The catch clause now captures the exception and returns the Error discriminant
  (4) with the message encoded via `EncodeString`.

- fix(csharp): outer `packages/csharp/HtmlToMarkdown.csproj` was a NuGet wrapper that
  double-included the inner project's sources, producing duplicate type definitions.
  Added `<Compile Remove="HtmlToMarkdown/**" />`, `<ProjectReference>`, and
  `<GenerateAssemblyInfo>false</GenerateAssemblyInfo>`.

- fix(csharp): `<TreatWarningsAsErrors>true</TreatWarningsAsErrors>` was absent from
  `Directory.Build.props`. Added alongside `<Nullable>enable</Nullable>`.

- fix(csharp): `using System.Threading.Tasks` was emitted unconditionally. It is now
  conditionalized on whether the API has any `async` functions or methods.

- fix(csharp): `ConvertWithVisitor` accepted `IVisitor` (old generated interface) instead
  of `IHtmlVisitor` (trait-bridge interface from `TraitBridges.cs`). The method now takes
  `IHtmlVisitor` and wraps it in `HtmlVisitorBridge` instead of the removed `VisitorCallbacks`.

- fix(napi): `mapper.map_type` returns `String`, not `Cow<str>`; removed stale `.into_owned()`
  calls in `crates/alef-backend-napi/src/gen_bindings/types.rs`.

- fix(extendr): removed dead reference to `gen_conversion_options_r` (kreuzberg-specific dead
  code from an earlier project-coupled state).

- fix(codegen): collapsed clippy `if-same-then-else` warning in `config_gen.rs` by combining
  enum-variant and Named-type branches that emit the same body.

- chore(deps): dropped unused `tracing` dep from `alef-core` (only `tracing-test` is used,
  and that's a dev-dep).

## [0.14.0] - 2026-05-02

### Changed

- **BREAKING (config schema): `alef.toml` is now multi-crate.** The old single-crate `[crate]` schema is rejected on load with a curated migration message. Run `alef migrate --write` to convert. Internally the loader returns `Vec<ResolvedCrateConfig>` and every backend, codegen pass, scaffold step, e2e generator, and publish step now consumes `&ResolvedCrateConfig` instead of `&AlefConfig`. The `Backend` trait signature changed accordingly. Workspace-wide settings live under `[workspace]`; per-crate settings live under `[[crates]]`.
- alef-cli now iterates every crate in the workspace by default. Add a top-level `--crate <name>` (repeatable) to restrict commands to a subset.
- `alef-backend-swift` / `alef-backend-kotlin`: an explicit `[crates.output] {swift,kotlin}` value is now treated as the verbatim package directory; only the template-derived default constructs the canonical `Sources/<Module>/` (swift) or `src/main/kotlin/<package>/` (kotlin) layout. Predictable rule: explicit override = exactly that path; default template = backend builds the canonical structure.

### Added

- feat(alef-cli): `alef migrate --write` rewrites legacy `alef.toml` in place using an atomic temp-file + rename and refuses to overwrite symlinks. The dry-run default prints a unified diff.
- feat(alef-cli): top-level `--crate <name>` CLI filter (repeatable) selects a subset of the workspace; absent processes every crate. New `crates/alef-cli/src/dispatch.rs` exposes `select_crates` / `is_multi_crate`.
- feat(alef-core): new `WorkspaceConfig` / `RawCrateConfig` / `ResolvedCrateConfig` / `OutputTemplate` types with documented per-field precedence; `OutputTemplate::resolve` rejects path traversal segments and NUL bytes; `WorkspaceConfig`, `RawCrateConfig`, and `NewAlefConfig` use `#[serde(deny_unknown_fields)]` so typos surface as parse errors.
- feat(alef-core): per-crate `compute_crate_sources_hash(&ResolvedCrateConfig)` replaces the legacy `&[PathBuf]` overload so multi-source-crate workspaces produce a single stable hash across all contributing source files.

### Fixed

- fix(e2e-go): avoid double-wrapping `len(...)` in `min_length` / `max_length`
  assertions and generate valid guards for array element length checks.
- fix(e2e-rust): honor call-level `result_is_simple` and `result_is_option`
  flags so optional scalar results are unwrapped before equality assertions.
- fix(codegen): preserve sanitized `Cow<str>` fields in Rust binding DTO serde and binding-to-core conversions.
  Python bindings previously dropped required string fields such as `ProcessConfig.language`, causing generated e2e
  tests to fail with missing-field deserialization errors.
- fix(alef-backend-dart, alef-backend-swift): honor `[crates.dart] frb_version` and `[crates.swift] swift_bridge_version` overrides — the fields were deserialized but every callsite hardcoded the compiled-in default constant.
- fix(alef-cli/migrate): preserve legacy `[crate]` scalars at the top of the generated `[[crates]]` entry by recursively clearing toml_edit position metadata so the resulting document is well-formed even when many language sub-tables follow.
- fix(alef-cli/version-pin): write `[workspace] alef_version` after a successful generate instead of a top-level `version =` line. The legacy detector rejected the top-level form on the next run, breaking re-generation.
- fix(alef-cli/cache): scope IR / lang / stage caches per-crate (`.alef/<crate>/ir.json`, `.alef/<crate>/hashes/<lang>.{hash,manifest}`, `.alef/<crate>/hashes/<stage>.{hash,manifest}`) so multi-crate workspaces no longer poison or overwrite each other's cache entries. `validate_cache_crate_name` rejects path separators, NUL, `..`, `.`.

## [0.13.10] - 2026-05-02

### Added

- feat(codegen): auto-exclude trait-bridge registration functions via `collect_trait_bridge_registration_fn_names()`.
  Each binding backend now automatically excludes `register_*` / `unregister_*` functions to prevent double-emit
  compile errors — downstream consumers no longer need manual `exclude` entries in `alef.toml` for trait-bridge registration fns.

- feat(e2e-go): support disk-path fixture loading in Go e2e harness in addition to HTTP URLs.
  New test infrastructure in `crates/alef-e2e/tests/go_bytes_file_path_fixtures.rs` demonstrates the pattern.

### Fixed

- fix(cli): qualify type-name exclusion entries by Rust path. `apply_filters` previously matched
  `[exclude].types`/`enums`/`errors` entries against the short type name only, so a fully-qualified entry
  like `kreuzberg::core::config::formats::OutputFormat` never actually filtered anything when an
  ambiguously-named type appeared in two pub-use chains. Entries containing `::` are now matched against
  the type's `rust_path` (with hyphens normalised); plain entries continue to match by short name.
  Three regression tests cover plain-name match, qualified-path match, and hyphen normalisation.

- fix(codegen): resolve C# signature mismatch in generated binding code that surfaced during trait-bridge auto-exclusion wiring.

- fix(backend-rustler): emit `if(...)` function-call form instead of `(if ...)` outer-parens form for JSON-encoding options.
  `mix format` rewrites `(if x, do: y, else: z)` → `if(x, do: y, else: z)`, so the outer-parens form caused a hash drift
  on every prek run. The generator now emits the formatter-stable `if(...)` form directly.

- fix(e2e): honor call-level Rust option result flags and use the crate version for registry-mode Rust e2e dependencies when no package override is set.
- fix(e2e): generate valid Go array guards for length assertions on array element fields.
- fix(e2e): honor call-level simple/optional result flags in Go tests and render contains assertions for struct slices.
- fix(e2e): emit nil checks for Go optional simple-result empty assertions.
- fix(backend-java): correct four compile errors in the generated Java convert wrapper
  when `bind_via = "options_field"` is used with an opaque-handle bridge field:
  - B1: the generated `convert()` no longer calls the non-existent
    `htm_conversion_options_from_json` FFI symbol; options are now serialised to JSON
    and passed as a C string directly via `coptionsJsonSeg`.
  - B2: the bridge attachment code now detects that the bridge Java type is an opaque
    handle (e.g. `VisitorHandle`) and calls `options.visitor().handle()` to pass the
    raw `MemorySegment` to the FFI setter, instead of constructing a `VisitorHandleBridge`
    (which does not exist and would require a `Visitor` interface argument).
  - B3: the FFI call argument for the options param is now `coptionsJsonSeg` (matching
    the emitted variable name), not the stale `cOptions` name from the old `_FROM_JSON`
    path.
  - B4: `gen_builder_class` now only emits `.orElse(null)` for optional fields that
    are bridge fields (where the record stores the raw handle type); regular
    `Optional<T>` fields are passed directly to avoid "incompatible types" errors.
- fix(backend-go): preserve `None` as nil for optional string returns.
- fix(backend-go): decode externally tagged Rust enums with string payload fallback
  variants (for example `Other(String)`) as string-like Go enums instead of empty
  structs, and match Rust serde's default unit-variant wire names.
- fix(e2e-go): render `contains` assertions for structured values through JSON
  instead of `%v`, so pointer fields are checked by value.

- fix(backend-go): correct two codegen bugs that produced uncompilable Go bindings when
  `bind_via = "options_field"` is used with an options type that has an Update sibling:
  - Bug A: `ffi_c_struct_name` previously inserted an extra PascalCase prefix segment,
    producing double-prefixed C type names (e.g. `HTMHtmNodeContext`) that do not exist
    in the cbindgen header; the formula now maps Rust struct basenames directly to their
    cbindgen names (e.g. `HTMNodeContext`).
  - Bug B: visitor.go is no longer emitted when all `[[trait_bridges]]` entries use
    `bind_via = "options_field"`; the stale FFI symbols `htm_conversion_options_from_json`,
    `htm_visitor_create`, `htm_visitor_free`, and `htm_convert_with_visitor` are absent
    from the options-field bridge API and caused undefined-symbol link errors.
  - Additionally, non-opaque Named parameters and method receivers whose type has a
    corresponding `{Name}Update` sibling now use the two-step
    `{prefix}_{snake}_update_from_json` + `{prefix}_{snake}_from` pattern instead of the
    removed `{prefix}_{snake}_from_json` helper.

- fix(backend-pyo3): emit SCREAMING_SNAKE_CASE aliases alongside PascalCase variants in
  `.pyi` enum stubs so mypy resolves `CodeBlockStyle.BACKTICKS` without `attr-defined`
  errors. Previously only the raw PyO3 names (e.g. `Backticks`) were declared in the stub
  while `options.py` adds the aliases at runtime.

- fix(backend-pyo3): use `object | None` for bridge fields in the `__init__` stub parameter
  list, matching the field annotation. Previously the constructor parameter was typed
  `VisitorHandle | None`, causing a mypy `arg-type` error when passing `value.visitor`
  (typed `object`) from the generated `api.py` conversion helper.

- fix(backend-rustler): wrap inline `if/do/else` JSON-encoding expressions in parentheses
  so the generated NIF call args parse as Elixir when the conditional is followed by a
  visitor parameter. Previously emitted `convert_with_visitor(html, if ..., do: ..., else: ..., visitor)`
  which is invalid syntax — Elixir keyword lists must be the final argument.

- fix(backend-php): null-guard visitor access and correct PSR-4 namespace in generated PHP:
  - `generate_public_api` now emits `$options?->visitor` (null-safe operator) instead of
    `$options->visitor` when the options parameter is optional, eliminating the PHPStan
    "Cannot access property on null" error.
  - Added `generate_scaffold` to `PhpBackend` that emits a `functions.php` convenience
    wrapper using the namespace from `config.php_autoload_namespace()` (which honours
    `[php].namespace` in `alef.toml`), replacing the previously hard-coded wrong
    namespace derived by mechanical case-splitting of the crate name.

- fix(backend-php): resolve compile errors when `bind_via = "options_field"` is used
  in `[[trait_bridges]]` with an options type that has an opaque handle field:
  - `gen_php_struct` now receives `opaque_type_names` from `mod.rs` so
    `gen_struct_with_per_field_attrs` can emit `#[serde(skip)]` on fields whose type
    is an opaque handle (e.g. `VisitorHandle`), fixing 8 `E0277` serde trait errors.
  - `gen_bridge_field_function` now emits `let mut {name}_core` for optional params,
    enabling `options_core.as_mut()` in the generated `visitor_attach` call.
  - `find_bridge_field` (alef-codegen) now sets
    `param_is_optional: is_optional || param.optional`, covering the IR pattern where
    the options param is stored as `ty: Named("T") + optional: true` rather than
    `ty: Optional(Named("T"))`. This caused the wrong `visitor_attach` branch to be
    selected, producing a `no field 'visitor' on type Option<...>` compile error.
  - `gen_php_call_args` now emits `visitor.map(|v| (*v.inner).clone())` for optional
    opaque params, replacing the former `visitor.as_ref().map(|v| &v.inner)` which
    produced a type mismatch (`Option<&Arc<...>>` vs `Option<Rc<...>>`).

- fix(backend-wasm): resolve four compile errors when `bind_via = "options_field"` is
  used in `[[trait_bridges]]` with an options type that has a handle field:
  - `gen_new_method` now appends `..Default::default()` to the struct literal when
    bridge fields are excluded from constructor params, fixing missing-field errors.
  - `gen_opaque_struct_methods` now skips methods whose params contain a
    `type_alias` of an `options_field` bridge (e.g. the builder's `visitor()` method),
    preventing uncompilable Arc-vs-Rc type mismatches in the generated call args.
  - `field_conversion_from_core_cfg` (alef-codegen) now emits `None` for sanitized
    `Named` / `Optional(Named)` fields in WASM mode (`map_uses_jsvalue = true`)
    instead of the Debug-format fallback that produced `Option<String>` rather than
    `Option<JsValue>`.
  - `bridge_fields_map` is extended to cover `{options_type}Update` types so that
    binding→core `From` impls for Update types default the bridge field via
    `Default::default()` instead of calling `val.visitor.map(Into::into)`, which
    fails because `WasmVisitorHandle` has no `Into<VisitorHandle>` impl.
  Also fixes `convertible_types` (alef-codegen) to allow optional sanitized fields
  whose type is `Named(T)` with `optional = true`: the binding stores `Option<T>`
  which is always `Default`, so these types correctly remain in the convertible set.

- fix(backend-ffi): suppress duplicate `{prefix}_convert` symbols in options-field
  bridge mode. When `bind_via = "options_field"` is set, the free-function loop now
  skips ALL `convert` variants (not just sanitized ones), and `gen_convert_no_visitor`
  - `gen_visitor_bindings` are not emitted. The single authoritative `{prefix}_convert`
  comes exclusively from `gen_convert_with_options_field_bridge`. Removes the three
  `htm_convert` definitions that caused a duplicate `#[no_mangle]` compile error in
  html-to-markdown. Also fixes `gen_convert_with_options_field_bridge` to call
  `core::convert(html, options)` with 2 arguments matching the current API (visitor
  is embedded in options, not passed as a third argument).

- fix(e2e): visitor codegen now assigns the synthesized visitor object to
  `options.visitor` (language-idiomatic field assignment) instead of passing it
  as a third positional argument. This aligns e2e test generation with the h2m
  API change where `convert(html, options)` accepts visitor as an optional
  ConversionOptions field rather than a separate parameter. Implemented for:
  - Rust: mutate options binding after construction to attach visitor
  - Ruby: insert visitor key into options hash inline
  - PHP: merge visitor as separate options object
  - Python, TypeScript: already correct (kwargs/object-field pattern)
  Remaining languages (Go, Java, C#, Elixir, R, Gleam, Kotlin, Zig, Swift,
  WASM, Dart, C) require equivalent updates to move visitor from positional
  arg to options field assignment (patterns will vary per language syntax).

- fix(backend-napi): resolve all compile errors when `bind_via = "options_field"`
  is used in `[[trait_bridges]]` with a NAPI-RS binding:
  - `gen_struct` bridge-field substitution (emit `Option<Object<'static>>` instead of
    `Option<JsVisitorHandle>`) is now applied BEFORE the `field.sanitized` guard, so it
    correctly fires for bridge fields that are NOT sanitized (e.g. `visitor` in
    `ConversionOptions` has `sanitized: false`).
  - `gen_struct` now also builds a set of all bridge `type_alias` values and applies the
    `Option<Object<'static>>` substitution to ANY field whose type is a bridge alias
    (e.g. `ConversionOptionsUpdate.visitor`), not only fields in the declared
    `options_type`. This prevents `JsVisitorHandle: FromNapiValue not satisfied` errors
    in Update-pattern structs.
  - `bridge_fields_by_type` (used to populate `force_default_fields` for From
    conversions) is extended by case (b): any type with a field whose type is a bridge
    type alias is now included, so binding→core and core→binding `From` impls emit
    `Default::default()` instead of `val.visitor.map(Into::into)` for those fields.
  - `gen_opaque_struct_methods` now skips methods whose params contain a `type_alias`
    of an `options_field` bridge (e.g. `ConversionOptionsBuilder::visitor()`),
    preventing uncompilable Arc-vs-Rc type mismatches in the generated call args.

- fix(backend-pyo3): resolve all compile errors when `bind_via = "options_field"`
  is used in `[[trait_bridges]]` with a PyO3 binding:
  - `extra_field_attrs` now returns `vec![]` for bridge fields, preventing
    duplicate `#[pyo3(get)]` and `#[serde(skip)]` attributes.
  - `rewrite_bridge_field_type` now accepts `type_alias: Option<&str>` and
    also rewrites `Option<{alias}>` patterns (e.g. `Option<VisitorHandle>`),
    not only the legacy `Option<String>` placeholder.
  - `rewrite_bridge_field_impl` now rewrites constructor parameter types
    (`visitor: Option<VisitorHandle>` → `visitor: Option<Py<PyAny>>`).
  - Struct generation for both the primary options type and related types
    (e.g. `ConversionOptionsUpdate`) that share the same bridge field name
    now applies the full set of rewrites: field type override, `frozen` →
    `unsendable`, and manual `Clone` impl using `Python::attach`/`clone_ref`.
  - `bridge_field_name_for_type` extends From-conversion post-processing to
    cover related types (`ConversionOptionsUpdate`) that are not listed as
    `options_type` in `alef.toml` but share the bridge field name.
  - Opaque types whose name matches a bridge `type_alias` (e.g. `VisitorHandle`)
    and builder types for bridge options types (e.g. `ConversionOptionsBuilder`)
    now use `#[pyclass(unsendable)]` because they transitively contain
    `Rc<RefCell<dyn Trait>>`.
  - Builder opaque impl: `visitor.as_ref().map(|v| &v.inner)` rewritten to
    `visitor.map(|v| (*v.inner).clone())` to pass `Rc<RefCell<...>>` (not a
    reference into Arc) to the core builder's `.visitor()` method.
  - `gen_bridge_field_function` in `trait_bridge.rs` now uses
    `Python::attach(|py| v.clone_ref(py))` to extract the visitor from
    `Option<Py<PyAny>>`, replacing the former `.clone()` call that requires
    the `py-clone` feature (not enabled in most consumers).

- fix(backend-csharp): drop stale `[DllImport]` declarations and fix the visitor
  attach path when `bind_via = "options_field"` is active:
  - `gen_native_methods` no longer emits `{prefix}_conversion_options_from_json`
    for options types owned by an options-field bridge; those types are constructed
    via the `UpdateFromJson` + `FromUpdate` pair that still exists in the FFI surface.
  - `gen_native_methods_visitor` now returns an empty string when an options-field
    bridge is present, suppressing the stale `{prefix}_convert_with_visitor`,
    `{prefix}_visitor_create`, and `{prefix}_visitor_free` P/Invoke declarations.
  - `gen_wrapper_function` now emits `{opts}.{Field}._vtable` directly instead of
    double-wrapping the already-constructed `{bridge_type}Bridge` in a second `new
    {bridge_type}Bridge(...)`, eliminating the `CS1503` type mismatch compile error.
  - `ffi_symbol()` on `OptionsFieldBridgeInfo` now produces
    `{prefix}_options_set_{field}` (matching `alef-backend-ffi::gen_bridge_field`)
    instead of the former `{prefix}_{opts_snake}_set_{field}` mismatch.

### Added

- feat(backend-pyo3): wire `bind_via = "options_field"` bridge support. When a
  `[[trait_bridges]]` entry sets `bind_via = "options_field"`, the PyO3 backend now:
  - Emits `visitor: Option<Py<PyAny>>` on the `ConversionOptions` pyclass (overriding
    the IR-sanitized `String` type via a targeted post-process rewrite).
  - Switches the `ConversionOptions` pyclass attribute from `frozen` to `unsendable`
    because the struct embeds `Rc<RefCell<dyn HtmlVisitor>>` via `Py<PyAny>` and is
    `!Send`; `unsendable` enforces single-thread GIL access.
  - Generates a `convert` wrapper that extracts `options.visitor`, builds a
    `PyHtmlVisitorBridge`, sets it on the core `ConversionOptions.visitor` field via
    `std::rc::Rc::new(std::cell::RefCell::new(bridge)) as VisitorHandle`, then calls
    core convert — no separate `convert_with_visitor` export.
  - Updates `.pyi` stubs: the visitor field on `ConversionOptions` is typed as
    `object | None` instead of `str | None`.
  Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
  the pyo3 `trait_bridge` module.

- feat(backend-napi): wire `bind_via = "options_field"` bridge support — visitor field on
  `JsConversionOptions` is emitted as `Option<Object<'static>>`, the `convert` wrapper
  extracts it, builds the `JsHtmlVisitorBridge`, sets it on core options, and calls core;
  the `.d.ts` interface shows `visitor?: HtmlVisitor`; no separate `convertWithVisitor`
  export is emitted in this mode.
- feat(backend-extendr): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the R/extendr backend now:
  - Renders the bridge field as `Option<extendr_api::Robj>` on the binding options struct
    (with `#[serde(skip)]` so JSON round-trips ignore it).
  - Emits a custom `From<BindingOptions> for core::Options` impl that leaves the bridge
    field at `Default::default()`; the convert wrapper sets it explicitly after building
    the bridge from the R object.
  - Updates the `new_<options>` kwargs constructor to accept the visitor as
    `Option<extendr_api::Robj>` and assign it via `Some(v)`.
  - Emits a `convert` wrapper (via `gen_bridge_field_function`) that takes
    `options: Option<ConversionOptions>`, pulls the visitor field off the binding,
    constructs `RHtmlVisitorBridge`, attaches it to `options_core.visitor` as
    `Rc<RefCell<...>> as VisitorHandle`, and calls core convert.
  - Does NOT emit a separate `convert_with_visitor` extendr export.
  Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
  the extendr `trait_bridge` module.
- feat(backend-rustler): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the Elixir Rustler backend
  now emits:
  - A regular `convert` NIF (ignores any visitor field in the options JSON).
  - A `convert_with_visitor` NIF with the visitor appended as `Option<rustler::Term<'_>>`,
    generated by the new `gen_bridge_field_function` helper.
  - A `visitor` field (nil default) on the `ConversionOptions` Elixir struct.
  - A `convert/2` Elixir wrapper that calls `Map.pop(options, :visitor)` and dispatches
    to either the async NIF+receive-loop (visitor present) or the plain NIF (no visitor).
  - `convert_with_visitor` is not re-exported as a public Elixir function.
  Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
  the `trait_bridge` module.
- feat(backend-ffi): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a bridge is configured in options-field mode, `alef-backend-ffi` now:
  - Emits `{prefix}_options_set_{field}(options, visitor)` — a setter that wraps
    the vtable bridge in a thin `Rc<RefCell<VtableRef>>` and stores it on the
    options struct's visitor field.
  - Emits `{prefix}_convert(html, options)` (options carries the embedded visitor)
    instead of the generic sanitized stub.
  - Does NOT emit `{prefix}_convert_with_visitor` — the single `{prefix}_convert`
    entry point replaces it.
  - Suppresses `{prefix}_{options_type}_from_json` for types that own a bridge
    field, because the visitor cannot survive a JSON round-trip.
  New symbol emitted for h2m: `htm_options_set_visitor`.
- feat(docs): render trait-bridged fields as struct fields on options types when
  configured with `bind_via = "options_field"`. The visitor bridge type is rendered
  with language-appropriate syntax (e.g. `HtmlVisitor(Protocol)` for Python,
  `HtmlVisitor (interface)` for TypeScript), and the field description is
  auto-generated if not explicitly documented.
- feat(core/codegen): trait bridges may now declare `bind_via = "options_field"`
  on `[[trait_bridges]]` (with `options_type` and optional `options_field`)
  to indicate that the bridge handle lives as a struct field on a function
  parameter rather than as its own positional argument. Adds the
  `BridgeBinding` enum, the `find_bridge_field` codegen helper, and updates
  `find_bridge_param` to skip bridges configured for options-field binding.
  Existing configs default to `function_param` (the legacy mode), so this is
  fully backwards compatible.
- feat(backend-wasm): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the WASM/wasm-bindgen backend now:
  - Emits the bridge field as `Option<JsValue>` on the binding struct (e.g. `visitor` on
    `WasmConversionOptions`) with a matching `#[wasm_bindgen(getter)]` / `setter`.
  - Excludes the bridge field from the `new()` constructor — callers set it via the setter.
  - Marks the bridge field `sanitized` in the cloned IR so the auto-generated
    `From<WasmConversionOptions>` emits `Default::default()` for it.
  - Generates a `convert` wrapper (via `gen_bridge_field_function`) that extracts the
    `Option<JsValue>` from the binding options, converts the rest to core options via
    `From`, builds `WasmHtmlVisitorBridge`, wraps it as
    `Rc<RefCell<...>> as VisitorHandle`, sets it on core options, and calls core convert.
  - Does NOT emit a standalone `convertWithVisitor` function in this mode.
  Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
  the wasm `trait_bridge` module.
- feat(backend-csharp): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the C# backend now:
  - Emits a `[JsonIgnore] public {Trait}Bridge? {Field}` property on the options
    record type so the bridge object is excluded from JSON serialization.
  - Declares an `internal static extern void {Options}Set{Field}(IntPtr options, IntPtr vtable)`
    P/Invoke entry-point in `NativeMethods.cs` whose FFI symbol is
    `{prefix}_{options_snake}_set_{field_snake}`.
  - Injects a bridge-attachment block in the wrapper `Convert` method: when
    `options.Visitor != null`, creates the bridge, calls the setter, then calls the
    standard two-arg `convert(html, options)` FFI function.
  - Does NOT emit a separate `ConvertWithVisitor` overload in this mode.
- feat(backend-php): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the PHP/ext-php-rs backend now:
  - Renders the bridge field as `?HtmlVisitor` on the `ConversionOptions` type stub
    (overriding the IR-sanitized `?string` type).
  - Generates a `convert` wrapper (via `gen_bridge_field_function`) that takes all original
    params plus a hidden `{field}_obj: Option<&mut ZendObject>` extra param, builds
    `PhpHtmlVisitorBridge::new`, wraps it in `Rc<RefCell<...>> as VisitorHandle`, attaches it
    to the core options struct, and calls core convert.
  - The PHP facade passes `$options->visitor` as the extra hidden arg when delegating to the
    native extension class.
  - Does NOT emit a standalone `convertWithVisitor` function.
  Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
  the PHP `trait_bridge` module.
- feat(backend-go): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the Go/cgo backend now:
  - Emits a lightweight `type {Trait} interface` (trait methods only, no plugin lifecycle) in `binding.go`.
  - Injects a synthetic `{Field} {Trait} \`json:"-"\`` field on the Go options struct,
    skipping the raw IR `Option<VisitorHandle>` field so JSON marshaling ignores it.
  - Adds a `WithConversionOptions{Trait}` functional-option constructor.
  - Adds `"runtime/cgo"` to the imports when any options-field bridge is active.
  - In the `Convert` wrapper, after the C options handle is created, checks
    `options.{Field} != nil`, creates a `cgo.NewHandle`, defers `handle.Delete()`,
    and calls `C.{prefix}_options_set_{field}(cOptions, ...)` to pass the visitor to Rust.
  - Does NOT emit a separate `ConvertWithVisitor` function in this mode.
- feat(backend-java): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the Java/Panama FFM backend now:
  - Emits the bridge field as the bridge interface type (e.g. `HtmlVisitor`) annotated with
    `@JsonIgnore` on the `ConversionOptions` record so Jackson excludes it from serialization.
  - Emits a `static final MethodHandle {PU}_OPTIONS_SET_{FIELD}` in `NativeLib.java` using
    `orElse(null)` so class initialization succeeds even when the dylib lacks the symbol.
  - Generates a `convert` wrapper that marshals the options record normally (bridge field is
    skipped by `@JsonIgnore`), then—if both the setter handle and `options.visitor()` are
    non-null—creates the `{Trait}Bridge`, converts it to a `MemorySegment` via `toSegment`,
    and calls the setter before the main FFI invocation.
  - Does NOT emit a standalone `convertWithVisitor` method in this mode.
- feat(backend-magnus): support `bind_via = "options_field"` in `[[trait_bridges]]`.
  When a visitor bridge is configured in options-field mode the Ruby/Magnus backend now:
  - Renders the bridge field as `Option<magnus::Value>` on the binding options struct
    (with `#[serde(skip)]` so JSON round-trips ignore it).
  - Generates a custom `From<BindingOptions> for core::Options` impl that skips the
    bridge field and lets the convert wrapper set it explicitly.
  - Emits a `convert` wrapper (via `gen_bridge_field_function`) that deserializes the
    options hash via `to_json`/`serde_json`, extracts the visitor Ruby object, builds
    `RbHtmlVisitorBridge`, wraps it in `Rc<RefCell<...>>`, and sets it on the core
    options before calling the core function.
  - Does NOT emit a standalone `convert_with_visitor` function.
  Re-exports `find_bridge_field` and `BridgeFieldMatch` from `alef-codegen` through
  the Magnus `trait_bridge` module.

### Fixed

- fix(backend-java): public facade and opaque-type Javadocs now use the shared
  Javadoc escaping path, preventing Rustdoc examples from being emitted as raw
  `&...`/fenced-code content that breaks `mvn verify` Javadoc generation.
- fix(backend-php): generated public PHP wrapper files now include the
  formatter-required blank line after `<?php` before the Alef header, keeping
  `php-cs-fixer` from mutating generated files after hashes are finalized.
- fix(scaffold/java,scaffold/php): generated Java Checkstyle suppressions now
  use a repo-root path in `checkstyle.xml` so both Maven and repo-root
  pre-commit invocations find the suppressions file, and the PHP CS Fixer
  scaffold is itself fixer-clean while still tolerating packages without a
  `tests/` directory.
- fix(cli/format): the default WASM formatter now derives the generated crate
  package from Alef's core crate directory (`core_crate_dir-wasm`) instead of
  the public Rust crate name. This fixes repos where the public crate
  (`tree-sitter-language-pack`) differs from the internal core crate directory
  (`ts-pack-core`), avoiding `cargo fmt -p <missing-crate>` during generation.
- fix(scaffold/generated-output): generated downstream projects now pass common pre-commit checks after trimming a public API surface: Python `options.py` no longer imports native data-enum aliases before redefining their Python-side aliases, Elixir/Ruby Cargo scaffolds omit `async-trait`/`tokio` when no generated code uses them, Java Checkstyle suppressions resolve from both Maven and repo-root hook invocations, PHP CS Fixer tolerates packages without a `tests/` directory, and Rustler `native.ex` target lists are emitted in `mix format` style.
- fix(cli/format): the default Node formatter path now uses the Oxc toolchain (`npx oxfmt .` followed by `npx oxlint --fix .`) instead of invoking Biome. The Node scaffold, lint defaults, and generated pre-commit config already used `oxfmt`/`oxlint`; this removes the remaining stale Biome fallback that `alef all --clean` could hit in downstream repos with vendored Biome configs.
- fix(backend-napi): `gen_dts` now applies the same filtering as `gen_function` (drops names listed in `[node].exclude_functions`, drops `sanitized` functions without a trait_bridge). Previously every public function in the API surface was declared in `index.d.ts`, even when its NAPI binding was filtered out of `lib.rs` because it took a tuple-typed param like `Vec<(Vec<u8>, String, Option<FileExtractionConfig>)>`. The mismatch surfaced as TS2614 / TS2345 in downstream e2e suites that imported the phantom names. The d.ts is now generated from the same filtered set as the lib.rs, keeping the two in lockstep.
- fix(e2e/go): json_object args with `element_type` now emit a Go slice type derived from the element (e.g. `Vec<String>` → `[][]string`). Previously the codegen always declared `var parts []string` regardless of `element_type`, producing `cannot use parts (variable of type []string) as [][]string value` on `kreuzberg.GenerateCacheKey([][]string)` and similar nested-slice signatures.
- fix(backend-java): generate `TestVisitor`, `TestVisitorAdapter`, and `VisitContext` types for visitor-pattern bindings. `TestVisitor` is a test-friendly interface using `VisitContext` (same fields as `NodeContext`) instead of the raw FFI `NodeContext`, and `TestVisitorAdapter` converts `NodeContext` → `VisitContext` before delegating. `VisitResult` gains a `continue_()` alias used by generated e2e tests. `VisitorBridge.encodeVisitResult` now uses `Arena.global()` (not the try-with-resources arena) so allocated buffers survive past the callback return, and calls `.reinterpret(ValueLayout.ADDRESS.byteSize())` before `.set()` on the 0-byte upcall `outCustom`/`outLen` segments. The public facade's `generate_public_api` now forwards `has_visitor_pattern` so `HtmlToMarkdown.convert(html, options, TestVisitor)` is emitted. Trait bridge register/unregister handles in `NativeLib` use `.map(...).orElse(null)` instead of `.orElseThrow()` to avoid `ExceptionInInitializerError` when the symbol is absent.
- fix(e2e/java): `CustomTemplate` actions now extract `{placeholder}` names, convert them to camelCase Java variable names, replace each `{name}` with `%s`, and emit `String.format("fmt", var1, ...)` instead of the unformatted literal. `visit_blockquote`'s `depth` parameter is now `long` (matching the `JAVA_LONG` C layout) instead of `int`. Visitor e2e test files now import `TestVisitor`, `VisitContext`, and `VisitResult` from the binding package when any fixture uses visitor callbacks.
- fix(backend-pyo3): emit `setattr(Cls, "FOO", getattr(Cls, "Bar"))` instead of bare `Cls.FOO = Cls.Bar` when the Rust variant name is a Python keyword. Previously enum variants like `HighlightStyle::None` rendered as `HighlightStyle.NONE = HighlightStyle.None` in `options.py` — `None` is a reserved word, so this produced a `SyntaxError: invalid syntax` at import time, breaking every Python e2e test for any binding whose enum has a `None`/`True`/`False`/etc. variant.
- fix(backend-pyo3): use the python-ident-escaped runtime name in the `getattr` call for keyword-named enum variants. PyO3's `#[pyclass]` enum derive renames variants whose name collides with a Python keyword by appending `_` (e.g. `None` → the runtime attribute is `None_`), so `getattr(HighlightStyle, "None")` raises `AttributeError`. The alias emission now resolves the runtime name via `alef_core::keywords::python_ident` and also triggers the `setattr`/`getattr` form when the SCREAMING_SNAKE_CASE alias itself is a Python keyword (e.g. a variant `As` would alias to `AS` which is fine, but `Is` → `IS` and a hypothetical `Continue` → `CONTINUE` are guarded for symmetry).
- fix(e2e/rust): pass visitor arg and unwrap Result for content access. Generated tests now call `convert(html, Some(options), None)` (3-arg form) instead of the stale 2-arg form, emit `.expect("should succeed")` so field access is on the unwrapped `ConversionResult` not on `Result<ConversionResult, ConversionError>`, derive `Debug` on `_TestVisitor` structs (required by the `HtmlVisitor: Debug` bound), import `HtmlVisitor` from the `::visitor` sub-module where it lives, and fix all `HtmlVisitor` method parameter types (e.g. `u8` → `u32` for heading level, `&str` → `Option<&str>` for audio/video/iframe src, `usize` for blockquote depth). `CustomTemplate` actions now bind the referenced parameter identifiers so `format!("{text}")` etc. compile; `Option<&str>` parameters referenced in a template are unwrapped via `let name = name.unwrap_or_default()`. The `alef.toml` Rust override for html-to-markdown gains `wrap_options_in_some = true`, `options_type = "ConversionOptions"`, `extra_args = ["None"]`, and `returns_result = true`.
- fix(e2e/elixir): the e2e mix.exs now uses `[elixir].app_name` as the dep atom (e.g. `:html_to_markdown`), not the crate name with `-` → `_` replacement (e.g. `:html_to_markdown_rs`). When the dep atom doesn't match the path-dep's own `app:` value, mix's resolution silently misroutes — the path-dep's transitive deps (notably `:rustler_precompiled`) are not loaded during compilation of the path-dep itself, and the parent build fails with `(CompileError) cannot compile module ... module RustlerPrecompiled is not loaded and could not be found`.

- fix(backend-php): add `[php].namespace` field to `PhpConfig` that overrides the derived PHP namespace verbatim. Previously the namespace was always derived from `extension_name` by splitting on `_` and converting each segment to PascalCase (e.g. `html_to_markdown` → `Html\To\Markdown`), which was inconsistent with the single-segment `HtmlToMarkdown` namespace expected by `[e2e.call.overrides.php].module`. The new field is used verbatim in class registration (`#[php(name = ...)]`), the PHP facade, type stubs, PSR-4 autoload keys, and e2e test imports — ensuring all four places agree. The three duplicated inline namespace-derivation blocks in `gen_bindings/mod.rs` are collapsed to a single `config.php_autoload_namespace()` call that honors the override.

- fix(backend-extendr): emit `impl From<BindingType> for core::Type` (and the reverse direction) for binding↔core struct and enum conversions, mirroring the existing pyo3/magnus emission paths. Without these, every `*_core: core::Type = binding.into()` site in the generated R lib.rs failed to compile with E0277 unsatisfied trait bounds (e.g. `From<PreprocessingOptions> not implemented for html_to_markdown_rs::PreprocessingOptions`). Lossy `From` impls are now also emitted for data-variant enums, since the extendr binding flattens them to unit variants but containing structs still call `.into()` on the variant value.
- fix(codegen/extendr-kwargs): the `gen_extendr_kwargs_constructor` helper used to emit Rust function-parameter defaults (`name: String = "default"`), which is a syntax error — Rust does not accept default values in function signatures. The constructor now accepts each field as `Option<T>`, instantiates the type via its `Default` impl, and overlays caller-supplied values, producing valid Rust source that the `#[extendr]` macro accepts. Existing `extendr(default = …)` semantics on the R side are preserved by the `generate_public_api` wrapper.
- fix(e2e/c): emit shfmt-compliant case statement style in `download_ffi.sh` so the generated file passes the `shfmt` pre-commit hook without modification and `alef verify` stays consistent.
- fix(e2e/elixir): the e2e mix.exs now uses `[elixir].app_name` as the dep atom (e.g. `:html_to_markdown`), not the crate name with `-` → `_` replacement (e.g. `:html_to_markdown_rs`). When the dep atom doesn't match the path-dep's own `app:` value, mix's resolution silently misroutes — the path-dep's transitive deps (notably `:rustler_precompiled`) are not loaded during compilation of the path-dep itself, and the parent build fails with `(CompileError) cannot compile module ... module RustlerPrecompiled is not loaded and could not be found`. Fixes the elixir e2e suite for any binding whose `[elixir].app_name` differs from the kebab-case crate name.
- fix(e2e/typescript): `options_type` casts and imports are now resolved per-fixture. Previously the renderer used the top-level `[e2e.call.overrides.<lang>].options_type` for every fixture in a test file, even fixtures that overrode the call (`fixture.call = "chunk_text"`) and declared their own per-call `options_type` (e.g. `JsChunkingConfig`). The renderer now reads `e2e.calls.<name>.overrides.<lang>.options_type` for fixtures that override the call, and only the top-level value is used for fixtures using the default call. The header import block emits `type X` declarations for the *union* of all options types referenced by the active fixtures (e.g. both `JsExtractionConfig` and `JsChunkingConfig` when one file mixes `extract_file` and `chunk_text` fixtures), eliminating TS2304 "Cannot find name 'JsChunkingConfig'" on chunking fixtures and stopping the wrong cast (`JsExtractionConfig` applied to a `JsChunkingConfig`-shaped object).
- fix(e2e/typescript): `as <OptionsType>` casts are now emitted only for *object*-shaped json_object args. Previously every json_object arg (including `paths: [...]`, `texts: [...]`, `parts: [...]`) was cast to the call's options type, producing `["a.pdf"] as JsExtractionConfig` on `batchExtractFileSync(paths, config)` calls — TS2352 "Conversion of type 'string[]' to type 'JsExtractionConfig' may be a mistake". The renderer now inspects the fixture value: arrays/scalars are emitted plain (typed as `Array<string>` / etc by the binding), only objects receive the cast.
- fix(e2e/typescript): `bytes` arg type now classifies the fixture string value (file path / inline / base64) and emits a runtime load instead of passing the string as-is. Previously fixtures like `extract_bytes` with `data: "pdf/fake_memo.pdf"` produced `extractBytes("pdf/fake_memo.pdf", ...)` — TS2345 "Argument of type 'string' is not assignable to parameter of type 'Uint8Array<ArrayBufferLike>'". The renderer now classifies via the same rules as python/rust codegens (`<`/`{`/`[`/whitespace → inline-text → `Buffer.from(s, "utf-8")`; `dir/file.ext` → file-path → `readFileSync(path)`; otherwise → base64 → `Buffer.from(s, "base64")`). When any active fixture's resolved call uses a bytes file-path arg, the test file imports `{ readFileSync } from 'node:fs'`. The existing `setup.ts` chdir to `test_documents/` is preserved, so relative paths resolve at runtime.
- fix(e2e/typescript): cast through `unknown` (`as unknown as <Type>`) for both inline configs and the empty-default placeholder. NAPI-RS marks Rust `Option<T>` config params as `T | undefined | null` but bare `T` as required, while the alef.toml `optional` flag does not always match — fixtures like `extract_file_with_no_config` produced calls with no third argument and TS2554 "Expected 3 arguments, but got 2". When the json_object arg is missing (or `null`) and an `options_type` is configured, the renderer now emits `{} as unknown as <Type>` to satisfy the binding's required parameter; partial config literals are also cast through `unknown` to avoid TS2352 "neither type sufficiently overlaps". The runtime binding still validates fields.
- fix(e2e/typescript): `result_is_simple = true` is now honored. Previously the flag was only used by the rust/csharp/java/ruby/r codegens, so fixtures whose binding returns a primitive (e.g. `generateCacheKey: string`, `validateCacheKey: boolean`) emitted `result.<field>` access and triggered TS2339 "Property 'result' does not exist on type 'string'". The TypeScript renderer now resolves the per-call `result_is_simple` flag from `[e2e.calls.<name>.overrides.<lang>]` and (a) uses the bare `result_var` for assertions, (b) skips assertions on non-`result` fields with a `// skipped:` comment.
- fix(e2e/typescript): `arg_order` per-language override is now honored. The flag has been documented in `alef-core` for some time but no codegen reordered the `args` slice — so kreuzberg's `[e2e.call.overrides.node] arg_order = ["path", "config", "mime_type"]` was silently ignored and the renderer emitted args in the canonical Rust order, producing TS2345 "Argument of type 'string' is not assignable to parameter of type 'JsExtractionConfig'" on every `extractFile(path, mime, config)` call (NAPI's binding takes `path, config, mime_type?`). The renderer now reorders the per-fixture call args via the language override before passing to `build_args_and_setup`.
- fix(e2e/typescript): per-fixture `options_type` resolution now falls back to the top-level `[e2e.call.overrides.<lang>]` when a per-call override doesn't declare its own. Most kreuzberg per-call entries (e.g. `extract_bytes`) rely on the same `JsExtractionConfig` shape as the default `extract_file` call and therefore intentionally omit the per-call `options_type` — but the prior renderer treated this as "no options_type", emitted no cast, and produced TS2345 on inline config literals. The fallback chain is: per-call `options_type` → top-level `options_type` → no cast.
- fix(e2e/typescript): emit a bare call (no arguments) when the resolved `[e2e.call]` has `args = []` and the fixture has no input. Previously the renderer fell through to `json_to_js(input)` and stringified the entire input object as a single positional arg, producing `listExtractors({})` for `args=[]` calls and triggering TS2554 "Expected 0 arguments, but got 1".
- fix(e2e/typescript): support per-call `result_is_vec = true` by indexing `[0]` into the result variable for field-targeted assertions. Mirrors the existing csharp behavior so fixtures asserting `mime_type` / `content` on `batch_extract_*` calls (which return `Array<JsExtractionResult>`) compile cleanly under strict tsconfig.
- fix(e2e/typescript+wasm): honor per-call `skip_languages` when filtering active fixtures. The CallConfig field has been documented in alef-core for some time but no codegen consulted it — so kreuzberg's `[e2e.calls.batch_extract_*] skip_languages = ["wasm"]` was silently ignored and the wasm renderer emitted test files importing `batchExtractBytesSync` etc. that the wasm binding doesn't export, producing TS2614 / TS2724. Both the typescript (node) and wasm renderers now drop fixtures whose resolved call's `skip_languages` contains the target language.
- fix(e2e/typescript): `not_empty` / `is_empty` on non-array fields no longer emits `.length`. The renderer cannot statically know whether a struct field (e.g. `metadata` on JsExtractionResult — type `JsMetadata`) is a string or an object, so it now emits a runtime IIFE that branches on `typeof v === 'string' || Array.isArray(v)` (uses `v.length`) vs object (uses `Object.keys(v).length`). Casting through `unknown` avoids TS narrowing the unreachable branch to `never`. Behavior on array fields (declared in `fields_array`) is unchanged.
- fix(e2e/typescript): `has_later_arg_value` now also returns true for optional `json_object` args with `options_type` configured, since those emit a typed `{} as unknown as <Type>` placeholder. Previously a fixture with only `path` set (no `mime_type`, no `config`) on a 3-arg `extractFile(path, mime_type?, config)` produced `extractFile(path, {})` (2 args), tripping TS2554 against the wasm signature `extractFile(path, mime_type, config)`. The fix makes preceding optional args emit `undefined` placeholders so positional order is preserved.

### Changed

- feat(e2e/codegen): result-shape flags (`result_is_simple`, `result_is_vec`, `result_is_array`, `result_is_bytes`, `result_is_option`) are now read from the call-level `[e2e.calls.<n>]` (and the default `[e2e.call]`) in addition to per-language overrides. The rust, csharp, ruby, r, and typescript codegens all OR the call-level value with their per-language override, so a single call-level declaration applies consistently to every binding (return-type shape is a property of the Rust core function, not of any binding). Per-language overrides remain accepted for backwards compatibility but are no longer required.

### Changed

- feat(core/config): `result_is_simple`, `result_is_vec`, `result_is_array`, `result_is_bytes`, and `result_is_option` are now first-class fields of `CallConfig` itself, not only `CallOverride`. These flags describe the Rust core's return type and therefore apply identically to every binding — declaring them at call level eliminates the need for redundant per-language `[e2e.calls.<n>.overrides.<lang>] result_is_simple = true` blocks (kreuzberg's alef.toml had ~30 such call entries duplicated across rust/node/csharp/java/python/wasm). The typescript codegen now ORs the call-level value with the per-language override to preserve backwards compatibility; other backends will adopt the same pattern in subsequent releases. Per-language overrides remain accepted but should be removed in favor of the call-level field.

## [0.13.6] - 2026-05-02

### Added

- feat(scaffold): manage workspace `.cargo/config.toml` via opt-in `[scaffold.cargo]` section in `alef.toml`. When the section is present, alef writes the full canonical file with hash-based drift detection (same `# alef:hash:` pattern as other alef-managed files). Default config produces a 6-target template covering macOS dynamic_lookup (required for PyO3/ext-php-rs cdylibs to link on macOS — the symptom is `ld: symbol(s) not found for architecture arm64` on `_zend_*`/`ext_php_rs::zend::*` and PyO3 symbols at runtime since macOS ld is strict and these symbols are resolved at extension-load time, not link time), Windows MSVC `rust-lld` for x86_64 + i686, `aarch64-linux-gnu-gcc` for ARM64 Linux cross-compile, `x86_64-unknown-linux-musl` for static Linux, and the `wasm32-unknown-unknown` bulk-memory + getrandom_backend cfg. Per-target opt-out via `[scaffold.cargo.targets]`; repo-specific `[env]` entries via `[scaffold.cargo.env]` (supports both bare strings and `{ value, relative }` form). When `[scaffold.cargo]` is absent, the legacy create-if-missing wasm32-only behavior is preserved unchanged. New types in `alef-core::config`: `ScaffoldCargo`, `ScaffoldCargoTargets`, `ScaffoldCargoEnvValue`. New public function `alef_scaffold::render_cargo_config(&ScaffoldCargo) -> String` for deterministic rendering, with 11 unit tests covering golden output, env injection, per-target opt-out, deterministic re-render, key sorting, escaping, and the legacy-fallback gate.

### Added

- feat(e2e/elixir,e2e/gleam): migrate the BEAM languages onto the shared `TestClientRenderer` + `render_http_test` driver, completing Phase 2B. Elixir defines an `ElixirTestClientRenderer<'a>` (carrying `fixture_id` and `expected_status` so `render_call` can build `/fixtures/<id>` URLs and disable Req's auto-redirect for 3xx fixtures) implementing all 8 trait methods; the `FINCH_UNSUPPORTED_METHODS` (TRACE/CONNECT) skip stub stays as a thin pre-hook before delegating to the shared driver. Gleam defines a unit `GleamTestClientRenderer` whose `render_call` uses `gleam_httpc` and whose `render_assert_partial_body` / `render_assert_validation_errors` use `string.contains` (Gleam's stdlib has no JSON decoder); the renderer also overrides `sanitize_test_name` to strip leading `_`/digits so gleeunit names are valid Gleam identifiers. Behavioral notes: elixir's `describe`/`test` labels are now `(fixture_id, description)` rather than `("METHOD PATH - description", "")`, matching all reference impls (cosmetic only); both languages now emit `body_partial`/`validation_errors` assertions when fixtures carry them (old monoliths never called these); WASM was already routing through the migrated TypeScript renderer and required no changes.
- feat(e2e/go,e2e/java,e2e/csharp,e2e/kotlin): migrate the JVM/Go/.NET languages onto the shared `TestClientRenderer` + `render_http_test` driver. These languages cannot expose the consumer's `TestClient` cleanly via FFI, so they continue to drive the binding's `App.serve()` over a TCP-loopback `baseUrl` env var, but the per-test rendering now goes through the shared driver. Per-language deltas: go now unconditionally emits `_ = bodyBytes` to silence the unused-var compile error on tests with no body assertion (the old monolith conditionally omitted `io.ReadAll`); java/kotlin/csharp now correctly emit `<<present>>`/`<<absent>>`/`<<uuid>>` header-token assertions (old code silently skipped them); java now appends query params to the URI string (old code never handled query params); kotlin/go/csharp now sort headers and cookies for deterministic output (old code used HashMap insertion order); go/kotlin/csharp/java now also emit `body_partial` and `validation_errors` assertions when fixtures carry them (old monoliths never called these).
- feat(e2e/php,e2e/dart,e2e/swift,e2e/zig): migrate the second batch of TestClient-friendly languages (php, dart, swift, zig) onto the shared `TestClientRenderer` + `render_http_test` driver. Each language now defines a `<Lang>TestClientRenderer` struct + `impl client::TestClientRenderer` capturing only the language-native primitives (PHPUnit `public function test*(): void` + Guzzle, Dart `test('...', () async {})` over `dart:io`, XCTest `func testFoo() throws` over `URLSession`, Zig `test "..."` over `std.http.Client`). Per-language deltas: php now emits validation-error assertions even when the fixture also has a `body` (the old gate suppressed them); dart's `DartTestClientRenderer` carries a small `Cell<bool>` for `is_redirect` + `in_skip` because `CallCtx` doesn't carry expected-status; swift binds `_resp`/`_responseData` in `render_call` and references those names from subsequent assertion methods (matching the synchronous-via-DispatchSemaphore wrapper); zig's `render_assert_header` and `render_assert_partial_body`/`render_assert_validation_errors` are stubs (Zig's stdlib HTTP client doesn't expose response headers from `client.fetch(...)` results — same TODO pattern already used for `method_result`/`matches_regex` in the file).
- feat(e2e/python,e2e/typescript,e2e/ruby,e2e/rust): migrate the four pilot languages (python, typescript/node, ruby, rust) onto the shared `TestClientRenderer` + `render_http_test` driver. Each language now defines a small `<Lang>TestClientRenderer` struct + `impl client::TestClientRenderer` capturing only the language-native primitives (`def test_X(client):` / `it(...)` / RSpec `it` block / `#[tokio::test] async fn`); the canonical sequence (open → call → status → headers → body → validation → close), header sorting, content-encoding stripping, skip-directive handling, and `null`/empty-string body sentinels are now driven once from the shared driver. The four files `crates/alef-e2e/src/codegen/{python,typescript,ruby,rust}.rs` lose ~145-line monolithic HTTP-test renderers in favor of trait impls, leaving `render_http_test_function` (or equivalent) as a thin pre-hook + delegate to `client::http_call::render_http_test`. Behavioral notes: rust now also emits body/header/validation assertions from the shared driver (previous code only asserted status); typescript request headers are now emitted in deterministic sorted order rather than HashMap insertion order; python's one fixture with both `body` and `validation_errors` (`content_type_validation_invalid_type`) now emits both assertions instead of suppressing the latter (the validation assertion is a strict subset of the body equality, so no false failures).
- feat(e2e): shared `TestClientRenderer` trait and `render_http_test` driver in `crates/alef-e2e/src/codegen/client/`. Per-language e2e codegen previously duplicated the structural shape of every HTTP test (function header, request build, status/header/body assertions) across 18 monolithic per-language files (359-2965 lines each); the shape only differed in syntax. The new `client::TestClientRenderer` trait captures the canonical sequence — `render_test_open` → `render_call` → `render_assert_status` → per-header `render_assert_header` → `render_assert_json_body` / `render_assert_partial_body` → `render_assert_validation_errors` → `render_test_close` — and the shared `client::http_call::render_http_test` driver invokes those primitives in order, with built-in handling of skip directives, content-encoding header stripping, deterministic header ordering, and the `null`/empty-string body sentinels. Languages migrate one at a time: each implements the trait, replaces its monolithic `render_http_test_function` with a one-line call to the shared driver, and may optionally split its file into a `<lang>/{mod,client,ws,helpers}.rs` directory. 9 new unit tests use a tag-emitting mock renderer to lock down the driver's call sequence and special-case handling. A companion `client::ws_script::WebSocketScriptRenderer` trait is stubbed out for Phase 2B/2C.

### Changed

- chore: scrub library-specific terminology from alef source comments, doc-strings, test fixtures, and skill references. alef is a generic polyglot bindgen tool used by multiple downstream consumer libraries; comments, examples, and test fixture data that previously named one specific consumer ("spikard", `dev.spikard`, `:spikard`, etc.) are renamed to neutral placeholders (`mylib`, `mylib-core`, `consumer_core`, `com.example`, `:mylib`, `MyHandler`). Changes are limited to documentation/test data — no production code logic or generator output is affected. Historical published `## [0.x.y]` CHANGELOG entries are intentionally not rewritten (they describe shipped releases).
- fix(adapters/pyo3): callback-bridge codegen for Python no longer emits hardcoded `prepare_request`/`interpret_response` stub methods referencing a specific consumer's `handler_trait::RequestData` type. The stubs were dead code (always returning `Err(...)` "not implemented") that imposed a 3-method trait shape on every consumer; other language bridges (Node, Ruby, PHP, ...) only implement the user-defined method and were already correct. The Python branch now matches that pattern. If a consumer's actual trait requires additional methods, they must declare default impls in their trait definition or extend the AdapterConfig schema.

### Fixed

- fix(taskfile): `task set-version` now also bumps the workspace `alef.toml` (alef dogfoods its own config). Also switched from BSD-only `sed -i ''` to portable `sed -i.bak ... && rm -f` so the task works on Linux CI as well as macOS.
- fix(scaffold/node): emit `index.d.ts` at the package root in addition to `src/index.d.ts`. The generated `package.json` declares `"types": "index.d.ts"` (and lists `"index.d.ts"` in `files`), but the scaffold previously only emitted the type-shim under `packages/{node}/src/`, leaving the npm package without discoverable declarations at the path consumers actually resolve. Downstream `import 'kreuzberg'` (or any other alef-managed npm package) typed under the strict tsconfig produced `TS7016: Could not find a declaration file for module 'kreuzberg'` and degraded to implicit `any`, propagating to every test/file that referenced binding-emitted types like `JsExtractionConfig`. The fix emits both copies — the `src/` copy is preserved for in-tree tsconfig "include": ["src"] tooling; the new root copy is what npm exposes via `package.json#types`. Scaffold tests updated to assert both files are present (file count rises from 7 → 8 in `test_scaffold_node` and 10 → 11 in `test_scaffold_multiple`).
- fix(backend/pyo3): correct `Arc<T: !Copy>` field codegen for clone. When a struct field had type `Option<Arc<serde_json::Value>>` (or any non-String `Arc`-wrapped type), the core→binding `From` impl chained an extra `.map(|v| (*v).clone().into())` on top of a base conversion that already handled the `Option<Arc<T>>` via Display/Deref coercion (e.g. `val.X.as_ref().map(ToString::to_string)`). This produced `(*String).clone()` at the call site — dereferencing a `String` to `str`, then calling `str::clone()` which fails since `str: !Clone`. The fix detects when the base conversion is already a complex expression (not a simple `val.{name}` passthrough) and returns it unchanged, relying on `Arc<T>: Display + Deref<Target=T>` to correctly produce the binding-side value.
- fix(backend/pyo3): emit `Arc<tokio::sync::Mutex<T>>` (instead of `Arc<std::sync::Mutex<T>>`) for opaque types whose `&mut self` methods are all `async`. `std::sync::MutexGuard` is `!Send`, so holding a guard across `.await` makes the surrounding future `!Send`, which fails to compile against PyO3's `Send` future bound on `pyo3_async_runtimes::tokio::future_into_py`. `tokio::sync::MutexGuard` IS `Send`. New helper `alef_codegen::generators::type_needs_tokio_mutex` returns true only when every `&mut self` method on a mutex-needing type is async (mixed sync+async `&mut self` methods stay on `std::sync::Mutex` since `tokio::sync::Mutex::lock()` returns a future). The PyO3 backend rewrites the rendered struct + impl block to swap the lock type and `.lock().unwrap()` → `.lock().await` for these types. Verified end-to-end against an opaque type wrapping `axum-test`'s `TestWebSocket` in `Arc<Mutex<>>` with all-async `&mut self` methods: with the previously-required exclusions removed from `alef.toml`, the consumer's PyO3 cdylib compiles cleanly and the generated future satisfies the `Send` bound.
- fix(e2e/rust): rust e2e codegen now honors `owned = true` on `bytes` and `string` arg configs. Pre-this-change the renderer always emitted `&{name}` for bytes (via `.as_bytes()`) and a bare `&str` literal for string, regardless of whether the underlying kreuzberg-core function took the value by reference or by ownership (e.g. `detect_image_format(data: Vec<u8>)`, `detect_mime_type(path: String, ...)`). The codegen now flips both binding type (`Some(...).to_vec()` / `String::new()`) and pass expression (`name.to_string()` / `name.to_vec()` / `name`) when `owned = true`, so call sites match the function signature without forcing every owned-by-value Rust API into a borrowed wrapper. Two regression tests added in `crates/alef-e2e/tests/rust_function_call_fixtures.rs` (owned bytes file-path, owned string).
- fix(backend-magnus): functions registered via the trait_bridge path now use fixed arity in `function!(...)` instead of `-1` (variadic). The bridge generator emits a fixed-arg signature like `fn convert(html: String, options: Option<String>, visitor: Option<Value>)`, but the registration loop previously emitted `function!(convert, -1)` whenever any param was optional. Magnus's `function!` macro requires the fn to take `&[Value]` for arity `-1`, so the trait-bridge fn failed to satisfy `FunctionCAry<_>`/`Fn<(&[Value],)>`, breaking compilation for every binding that exposed a top-level function with an `HtmlVisitor`-style trait param (e.g. `html-to-markdown`'s ruby gem failed to build on linux/aarch64/macos with E0599 "method `call_handle_error` exists but trait bounds were not satisfied"). Bridge functions now register with `function!(name, params.len())`; non-bridge optional-param functions still use variadic `-1` with `scan_args` as before.

## [0.13.5] - 2026-05-02

### Fixed

- fix(e2e/rust): rust e2e codegen no longer short-circuits all non-HTTP, non-mock-server fixtures to a `// TODO: implement when a callable API is available` stub. Pre-0.13.5 the renderer assumed any fixture without an `http` block or a `mock_response` was a schema/spec validation fixture (asyncapi/grpc/graphql_schema) with no callable Rust API, so libraries whose fixtures invoke a plain function (e.g. `kreuzberg::extract_file(path, mime, config)`) emitted zero real test bodies. The stub gate now triggers only when the resolved call config has no function name; fixtures pointing at a configured `[e2e.call]` (or `[e2e.calls.<n>]`) render real function invocations with the right imports.
- fix(e2e/rust): `use {crate}::{fn};` import lines are now emitted for any non-HTTP fixture whose resolved call config has a function name, not only `mock_response`-bearing fixtures. Previously plain function-call fixtures rendered the call site but omitted the `use` line, producing E0425 "cannot find function" on every test.
- fix(e2e/rust): `bytes` arguments whose fixture value is a relative file path (e.g. `"pdf/fake_memo.pdf"`, classified the same way as in the python codegen) are now loaded at runtime via `std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../test_documents/", "<path>")).expect(...)` and passed by reference. Previously the path string itself was embedded as `r#"…"#` and `.as_bytes()`-ed, which compiled but always passed the path text as the file contents.

## [0.13.4] - 2026-05-01

### Added

- feat(e2e/wasm): the wasm e2e codegen now emits real `extractFile`/`extractBytes`-style test cases for non-HTTP fixtures, not just HTTP-server fixtures. Pre-0.13.4 the wasm renderer filtered `f.http.is_some()` and dropped every function-call fixture, leaving `e2e/wasm/` with only `globalSetup.ts`+`package.json` and zero tests against the binding's core async API. The wasm codegen now reuses the typescript renderer (parameterised by `lang = "wasm"`) and emits a `setup.ts` chdir to `test_documents/` when any active fixture takes a `file_path` or `bytes` arg, plus `globalSetup.ts` only when at least one HTTP fixture is in scope. Function-call resolution now goes through `[e2e.calls.<name>.overrides.wasm]` (auto-camelCasing the rust snake_case name when no explicit override is set), so wasm-specific renames work the same way they do for node.

### Changed

- chore(codegen): `resolve_node_function_name` (in `alef-e2e::codegen::typescript`) now takes a `lang: &str` parameter so the wasm codegen can reuse it. Existing call sites unaffected — typescript codegen passes `"node"` and behaves identically.

## [0.13.3] - 2026-05-01

### Fixed

- fix(backend/php): always emit `#![cfg_attr(windows, feature(abi_vectorcall))]`
  in the generated PHP binding lib.rs. ext-php-rs requires the `vectorcall`
  calling convention for PHP entry points on Windows, but the generator
  previously only emitted the cfg_attr when `[php].feature_gate` was set.
  Without it, downstream crates fail with E0658 ("the extern \"vectorcall\"
  ABI is experimental"). The cfg_attr is a no-op on non-windows so it costs
  nothing on Linux/macOS builds.

## [0.13.2] - 2026-05-01

### Fixed

- fix(publish/php): use `[php].extension_name` (composer.json's `php-ext.extension-name`)
  for PIE archive filenames and inner `.so`/`.dll` name, not the binding crate name. PIE
  installs binaries by extension-name, so `package_php` previously emitted
  `php_{crate}_php-...` archives that PIE couldn't find. Cargo's compiled artifact filename
  (still based on the crate name with `-php` suffix) is now resolved separately and renamed
  to `{ext_name}.so` / `{ext_name}.dll` inside the archive.

- fix(backend/ffi): honor `[ffi] exclude_types` in generated lib.rs. Previously the `FfiConfig::exclude_types`
  field was parsed from `alef.toml` but never applied during code generation — all types in the API surface
  were emitted regardless. The FFI backend now filters `api.types` through `ffi_exclude_types` before emitting
  struct wrappers, field accessors, and method wrappers, matching the behavior of the elixir and wasm backends.

## [0.13.1] - 2026-05-01

### Fixed

- fix(cli/e2e): `alef e2e generate` now sweeps orphan alef-generated files left under the e2e (and `--registry` test_apps) output root when a category or fixture leaves the generation set (e.g. all fixtures in a category resolve to skipped for the active binding surface, or wasm fixture-filtering drops a file). Previously these files lingered on disk with stale `alef:hash:` headers and `alef verify --exit-code` reported them as stale forever, blocking pre-commit hooks even after a clean regen. The sweep only deletes files whose first 10 lines contain the `auto-generated by alef` marker, so user-owned fixtures, scaffolded manifests, and lockfiles are never touched. Dependency directories (`target`, `node_modules`, `_build`, `vendor`, `deps`, `.venv`, `build`, `dist`, `.git`, `Pods`) are skipped during the walk.

## [0.13.0] - 2026-05-01

### Changed (BREAKING)

- feat(publish/php): `alef publish package --lang php` now produces PIE-conventional archives.
  Filename: `php_{ext}-{ver}_php{phpVer}-{arch}-{os}-{libc}-{ts}.tgz` (Unix) /
  `php_{ext}-{ver}-{phpVer}-{ts}-{compiler}-{arch}.zip` (Windows).
  Archive contains only `{ext_name}.so`/`.dll` at the archive root — no `composer.json`,
  `pie.json`, `INSTALL.md`, or `ext/` subdirectory. Requires new flags:
  `--php-version` (required), `--php-ts` (default `nts`), `--php-libc` (auto-detected),
  `--windows-compiler` (required on Windows targets). A SHA-256 sidecar
  `{archive}.sha256` is always written alongside the archive.

### Added

- feat(template-versions): `ENVOY_VERSION_RANGE` for the `envoy` Hex package — used by gleam e2e tests to read `MOCK_SERVER_URL` since neither `gleam_stdlib` nor `gleam_erlang` expose env-var access.
- feat(e2e/gleam): inject `gleam_http` as a direct dep and emit `import gleam/result` so generated tests can compile under gleam_stdlib >= 1.0.

### Fixed

- fix(codegen/conversions): preserve `Some(...)` wrap when the binding optionalizes a non-optional core field of type `Cow<'_, str>` (e.g. NAPI `optionalize_defaults`); previously the Cow `to_string()` branch dropped the upstream `Some(...)` wrap, producing `String` where `Option<String>` was required and breaking compile of NAPI bindings whose core type holds `Cow<'static, str>` (`ArchiveMetadata.format`, `ExtractionResult.mime_type`)
- fix(e2e/wasm): drop non-HTTP fixtures, fixtures with a literal `Content-Length` header, and fixtures using TRACE/CONNECT before code generation; node `fetch` (undici) rejects pre-set Content-Length and disallows TRACE/CONNECT, and empty `describe(...)` blocks fail vitest

- fix(e2e/elixir): emit a `@tag :skip` ExUnit test for non-HTTP non-`mock_response` fixtures (AsyncAPI, WebSocket, OpenRPC schema-only) so generated suites compile without referencing a non-existent binding `handle_request`/`handle_request_async` callable.
- fix(e2e/elixir): drop `content-encoding` and `connection` from generated header assertions — Req auto-decompresses gzip/brotli responses and strips the encoding header, and `connection` is a hop-by-hop header stripped by the response pipeline.
- fix(e2e/gleam): import `gleam/httpc` (not the package name `gleam_httpc`) and call `httpc.send/1`; the previous import path produced "unknown module" errors at compile time.
- fix(e2e/gleam): replace `gleam/os.get_env` (removed from gleam_stdlib >= 1.0) with `envoy.get/1` for `MOCK_SERVER_URL` lookup.
- fix(e2e/gleam): annotate the `list.find` predicate with `fn(h: #(String, String))` so the gleam type checker can resolve tuple field access on `resp.headers`.
- fix(e2e/gleam): use `result.is_ok` (not `option.is_some`) for header-presence assertions — `list.find` returns `Result(a, Nil)`, not an Option.
- fix(e2e/gleam): strip leading underscores/digits from HTTP test function names so numeric-prefixed fixture IDs (`13_json_with_charset_utf16`) produce valid gleam identifiers.
- fix(e2e/gleam): always emit the `e2e_gleam_test.gleam` entry module so `gleam test` discovers the per-category test files.
- fix(e2e/gleam): drop WebSocket-upgrade fixtures (request advertises `Upgrade: websocket`) — gleam_httpc cannot follow HTTP/1.1 protocol upgrades and errors with `ResponseTimeout`.
- fix(e2e/wasm): drop non-HTTP fixtures, fixtures with a pre-set `Content-Length` header, and fixtures using `TRACE`/`CONNECT` methods — node's undici fetch rejects the first two and refuses to dispatch the latter.

## [0.12.16] - 2026-05-01

### Added

- feat(backend-csharp,core): support `[languages.csharp.exclude_functions]` to skip generating selected free functions from the C# binding surface
- feat(backend-java): emit `JsonInclude(NON_NULL)` on optional fields and `JsonInclude(NON_ABSENT)` at the class level so Jackson omits null/empty optionals when serializing Java DTOs across the FFI boundary
- feat(backend-java): wrap optional FFI parameters as `TypeRef::Optional` for null-safety; non-zero integer defaults emit a compact constructor that initializes them when callers pass null

### Fixed

- fix(backend-magnus): use variadic arity (-1) with `scan_args` for free functions that have optional or promoted parameters; previously such functions were registered with a fixed arity equal to the total parameter count, causing Ruby callers that omit trailing optional arguments to get an argument count error
- fix(e2e/ruby): emit `nil` placeholder for skipped optional positional args when a later arg is present; previously omitting an optional String arg before a json_object arg produced a 2-arg call where the config object landed in the string slot, causing a `TypeError: no implicit conversion` runtime error
- fix(e2e/elixir): add kreuzberg path dep and rustler direct dep to generated mix.exs so NIF force-build works in e2e tests
- fix(e2e/elixir): replace `map_or(false, ...)` with `is_some_and(...)` to clear `clippy::unnecessary_map_or`
- fix(e2e/java): set `workingDirectory` in the generated maven-surefire plugin block so tests run from the project basedir, fixing relative fixture path resolution
- fix(backend-extendr): derive R wrapper output path as packages/r/R/ instead of packages/r/src/rust/src/
- fix(backend-pyo3): emit `pass` body for empty structs that pass through the codegen; combined with the v0.12.15 fix this allows empty structs to be referenced in tagged-union aliases without import-time `NameError`
- fix(backend-ffi): preserve numeric error code metadata on the FFI error type through the type generators so host-side error conversions can attach `code`/`message` consistently
- fix(scaffold/r): include `<R_ext/Visibility.h>` in generated entrypoint.c to define `attribute_visible`
- fix(scaffold/php): align scaffolded PHP package metadata and dev tooling with current backend expectations
- fix(cli/extract): drop debug `eprintln` from the extract pipeline so generation output is no longer polluted on stderr
- fix(e2e/go): include `mock_response` in unit-test fixture builder so `test_go_method_name_uses_go_casing` exercises the real codegen path instead of the non-HTTP skip stub
- fix(e2e/rust): include `mock_response` in unit-test fixture builders so `rust_call_overrides` regression tests exercise the real codegen path
- fix(scaffold): assert on `config.ext_dir = 'native'` (the current `rb_sys/mkmf` API) instead of the deprecated `cargo_manifest` form

## [0.12.15] - 2026-05-01

### Fixed

- fix(backend-pyo3): emit empty Python dataclasses for empty Rust structs (e.g. `pub struct ExcelMetadata {}`) when they have `Default`. Previously, the codegen skipped them entirely, leaving dangling references in tagged-union type aliases (`FormatMetadata = ... | ExcelMetadata | ...`) and breaking module import with `NameError: name 'ExcelMetadata' is not defined`.
- fix(e2e/escape): escape ASCII control characters (`\x00`–`\x1f`) in Python string literals using `\xHH` so generated Python source remains valid. Previously, fixtures with embedded NUL bytes (e.g. `clean_text_basic`) produced source files Python's parser rejected with `SyntaxError: source code string cannot contain null bytes`.
- fix(e2e/csharp): use PascalCase HTTP method names for `System.Net.Http.HttpMethod` static properties; route header assertions to `.Headers` vs `.Content.Headers` correctly to avoid "Misused header name" runtime errors; uniquify per-assertion variable names so `out var` does not redeclare in method scope.
- fix(e2e/go): clarify the non-HTTP stub message and document why it fires (Go bindings without a callable matching `[e2e.call].function`).
- fix(e2e/go): emit `os` import when any HTTP fixture is present (HTTP tests read `MOCK_SERVER_URL` via `os.Getenv`); previously only mock_url args triggered the import.
- fix(e2e/go): only emit `io` import when at least one HTTP fixture has a body assertion; eliminates `"io" imported and not used` build errors in test files where no fixture asserts on the body.
- fix(e2e/go): only declare `bodyBytes` when the fixture has a body assertion; eliminates `declared and not used: bodyBytes` build errors.
- fix(e2e/go): use a separate `want :=` declaration plus `%q` formatting in the body-mismatch `t.Fatalf` instead of interpolating the (already-Go-quoted) literal into the format string; previously emitted unparsable Go for fixtures whose expected body contained double-quotes (e.g. JSON-as-string responses) and for headers whose names/values contained backticks.
- fix(e2e/go): decode HTTP response and expected bodies into `any` (not `map[string]any`) so JSON arrays in the expected body decode without `cannot unmarshal array into ... map` runtime errors.

## [0.12.14] - 2026-05-01

### Fixed

- fix(e2e/rust): translate synthetic assertion fields (`embeddings`, `embedding_dimensions`, `embeddings_valid`, `embeddings_finite`, `embeddings_non_zero`, `embeddings_normalized`, `chunks_have_embeddings`, `keywords`, `keywords_count`) to correct Rust expressions instead of treating them as struct field accesses; `keywords`/`keywords_count` map to `extracted_keywords` on `ExtractionResult`.
- fix(e2e/python): intercept synthetic embedding and keyword assertion fields before struct field emission; skip `keywords`/`keywords_count` (no Python binding for `extracted_keywords`).
- fix(e2e/typescript): intercept synthetic embedding and keyword assertion fields; emit Vitest `expect` assertions using array methods; skip `keywords`/`keywords_count`.
- fix(e2e/go): intercept synthetic embedding and keyword assertion fields; emit inline immediately-invoked functions for predicate checks on `*[][]float32`; skip `keywords`/`keywords_count`.
- fix(e2e/ruby): intercept synthetic embedding and keyword assertion fields in the binding assertion path; emit RSpec `expect` assertions using Ruby enumerable methods; skip `keywords`/`keywords_count`.
- fix(e2e/php): intercept synthetic embedding and keyword assertion fields; emit PHPUnit assertions using `array_reduce`/`array_filter`; skip `keywords`/`keywords_count`.
- fix(e2e/csharp): intercept synthetic embedding and keyword assertion fields; emit xUnit assertions using LINQ; skip `keywords`/`keywords_count`.
- fix(e2e/elixir): intercept synthetic embedding and keyword assertion fields; emit ExUnit assertions using `Enum` functions; skip `keywords`/`keywords_count`.
- fix(e2e/r): intercept synthetic embedding and keyword assertion fields; emit testthat `expect_*` assertions using `sapply`/`all`; skip `keywords`/`keywords_count`.
- fix(e2e/wasm): intercept synthetic embedding and keyword assertion fields (defensive, embed is WASM-skipped); skip `keywords`/`keywords_count`.
- fix(e2e/ruby): use title-cased `Net::HTTP` request class names (`Delete`, `Head`, `Patch`, `Put`) instead of all-caps variants that don't exist in Ruby's stdlib; skip `content-encoding` header assertions; handle plain-string and empty bodies correctly; skip HTTP 101 WebSocket upgrade tests; emit `skip` stubs for non-HTTP fixtures (WebSocket, SSE) that cannot be tested via Net::HTTP; avoid `JSON.parse(nil)` on 204 No Content responses; deduplicate `Content-Type` header when body is already set.
- fix(backend-magnus): use `.to_vec()` instead of `.into()` when wrapping `&Bytes` return values from non-opaque struct methods; `Vec<u8>: From<&Bytes>` is not implemented so `.into()` failed to compile for methods like `as_bytes()` on `UploadFile`.
- fix(e2e/gleam): filter out HTTP-type fixtures and call-based fixtures without a Gleam-specific override; emit a `src/` placeholder module (required by the Gleam toolchain) and a smoke test when no fixture-driven tests are generated; strip leading digits from test function names.
- fix(e2e/kotlin): generate proper `java.net.http.HttpClient` HTTP tests for all 559 HTTP fixtures; hit `MOCK_SERVER_URL/fixtures/<id>` per the mock server protocol; URL-encode query params at runtime to handle JSON array values; escape `$` in string literals to prevent Kotlin string interpolation; skip Java's restricted headers (`Connection`, `Content-Length`, `Expect`, `Host`, `Upgrade`); skip `content-encoding` header assertions (mock server doesn't compress); skip 101 status tests (Java HttpClient doesn't handle protocol upgrades); emit `assumeTrue(false)` stubs for non-HTTP fixtures without a Kotlin-specific call override; add `kotlin("test")` to build.gradle.kts dependencies.
- fix(escape): add `escape_kotlin` function that escapes `$` in addition to standard Java escapes to prevent Kotlin string interpolation on embedded values.

## [0.12.13] - 2026-05-01

### Fixed

- fix(extract): `validate_call_export` now accepts method-on-type references (e.g., `function = "chat"` where `chat` is a method on a public type). Previously rejected legitimate method-style entry points used by `liter-llm` and similar libraries.

## [0.12.12] - 2026-05-01

### Fixed

- fix(codegen/binding_helpers): apply `k.into()` to Map keys in `gen_lossy_binding_to_core_fields` so the binding-side `is_empty()` helper round-trips wrapped string keys (`Cow`, `Box<str>`, `Arc<str>`) correctly. Previously the helper emitted `(k, …)` directly, which broke when the core key type was `AHashMap<Cow<'static, str>, Value>`.
- fix(backend-wasm): also deserialize `Vec<Vec<T>>` parameters in the `can_delegate` branch via `serde_wasm_bindgen::from_value`. Previously only the Result-returning serde-recovery branch handled this, so non-Result functions like `generate_cache_key` received the raw `JsValue`.
- fix(codegen/binding_to_core): emit `k.into()` for non-Json keys in the generic `Map` arm, completing the Cow-key round-trip fix.

### Changed

- feat(e2e/go): emit `t.Skip("TODO: ...")` stubs for all fixtures that lack a `mock_response`; omit the package import when no test uses it to avoid the Go "imported and not used" compile error.
- feat(e2e/java): emit `Assumptions.assumeTrue(false, "TODO: ...")` stubs for all fixtures that lack a `mock_response` so the test class compiles and is skipped cleanly.
- feat(e2e/csharp): emit `[Fact(Skip = "TODO: ...")]` stubs for all fixtures that lack a `mock_response` so the test class compiles and all tests are reported as skipped.
- fix(e2e/php): skip JSON decode and body assertions when `expected_response.body` is the empty-string sentinel, avoiding `JsonException` on empty bodies.
- fix(e2e/php): skip `markTestSkipped` for non-HTTP fixtures with no assertions; add `allow_redirects => false` to Guzzle client; skip HTTP 101 WebSocket upgrade tests; handle plain-string response bodies with raw `(string)$response->getBody()` comparison; suppress redundant `validation_errors` assertions when a full `body` assertEquals is generated; skip `content-encoding` header assertions since the mock server always returns uncompressed bodies.
- fix(e2e/rust): handle `<<absent>>` sentinel by omitting that header from mock responses; handle `<<uuid>>` sentinel by generating a real UUID v4 value; use `text/plain` default content-type when body is not JSON.
- fix(e2e/python): skip content-encoding assertions; handle plain-string and empty/null bodies without json.loads; skip HTTP 101 tests; disable redirect following; fix validation errors field name ("errors" not "detail").
- fix(e2e/typescript): skip content-encoding assertions; handle plain-string and empty/null bodies; skip HTTP 101 tests; add redirect: 'manual'; suppress import when no-assertion non-HTTP fixtures; emit it.skip stubs; fix validation errors field name.

## [0.12.11] - 2026-05-01

### Changed

- feat(e2e/swift): emit test files into `packages/swift/Tests/<Module>Tests/` instead of a standalone `e2e/swift` package. SwiftPM 6.0 forbids local `.package(path:)` references between packages in the same git repository; placing the generated tests inside the existing library package sidesteps the restriction while retaining a skeletal `e2e/swift/Package.swift` for CI reference.
- feat(e2e/zig): use `std.heap.DebugAllocator(.{}) = .init` (Zig 0.16+); emit compilable stub test bodies (`_ = testing;`) for fixtures with no assertions instead of calling a non-existent `handle_request` function; omit allocator setup when no setup lines are needed.
- feat(scaffold/swift): upgrade generated `Package.swift` to `swift-tools-version: 6.0`; remove `unsafeFlags` from `RustBridge` target's `linkerSettings` so the package can be used as a SwiftPM dependency.

### Fixed

- fix(e2e/rust/tests): pass the new `needs_http_tests` flag through `cargo_toml_generation.rs` test fixtures so the suite compiles after the `render_cargo_toml` signature was extended.
- fix(codegen/conversions): emit `k.to_string()` for Map keys in core→binding conversion and `k.into()` for the reverse direction so `Cow<'_, str>`/`Box<str>`/`Arc<str>` keys (which the type resolver normalizes to `TypeRef::String`) round-trip correctly. Without this fix the generated `From` impls produced `(Cow<'_, str>, …)` iterators feeding `HashMap<String, String>::from_iter`, breaking pyo3/napi/php bindings on `Metadata.additional`.
- fix(e2e/elixir): add missing commas between deps entries in generated `mix.exs`; without them Elixir emits a syntax error before deps like `{:req, ...}`.
- fix(e2e/kotlin): fall back to `alef_config.resolved_version()` when `[e2e.packages.kotlin]` has no explicit `version`, avoiding a stale `0.1.0` jar reference in `build.gradle.kts`.
- fix(e2e/gleam): strip leading underscores from generated test-function names; fixture IDs with numeric prefixes (e.g. `13_json_...`) produced `_json_..._test()` which Gleam rejects.
- fix(backend-dart): emit `lib/<pkg>.dart` barrel file re-exporting `src/<pkg>.dart` so `package:<pkg>/<pkg>.dart` imports resolve correctly in e2e tests.
- fix(e2e/elixir): normalize scientific-notation floats for Elixir syntax (`1e-10` → `1.0e-10`, strip `e+` → `e`); use `response.body` directly instead of `Jason.decode!` since Req auto-decodes JSON; fall back to `Req.request(method: :METHOD, ...)` for non-standard HTTP verbs like OPTIONS/TRACE; extract first value from Req's `[{name, [values]}]` header lists; preserve original JSON map keys instead of snake_casing them.
- fix(e2e/rust): skip `content-encoding` headers in mock server responses — server returns uncompressed bodies so forwarding a `content-encoding` header would cause clients to attempt decompression and fail; also skip setting default `content-type: application/json` when the fixture already specifies one.
- fix(extract/type_resolver): treat bare `Value` as `TypeRef::Json` to match the idiomatic `use serde_json::Value;` re-import. Without this, `HashMap<String, Value>` fields (e.g. `ProblemDetails.extensions`) sanitized to `Map<String, String>` and downstream From-impls emitted `.into_iter().collect()` between mismatched value types, breaking pyo3/napi bindings. `serde_json::Value` (full path) was already handled.

## [0.12.10] - 2026-05-01

### Added

- feat(backend-wasm/config): add `custom_rust_modules`, `exclude_fields`, and `source_crate_remaps` options for hand-written Rust modules, per-type field exclusions for `cfg(not(target_arch = "wasm32"))`-gated source fields, and rewriting `<original_crate>::TypeName` references to `<override_crate>::TypeName` when `core_crate_override` is set.
- feat(codegen/conversions): introduce `core_type_path_remapped` / `apply_crate_remaps` so generated `From` impls reference the override crate when `source_crate_remaps` is configured, avoiding orphan-rule violations across re-export facades.

### Fixed

- fix(extract): preserve `Map<K, V>` structure during sanitization. Previously a `Map<Cow<'static, str>, serde_json::Value>` field was flattened to `TypeRef::String` whenever the key resolved through a sanitized `Named` type; the field now stays a Map so binding backends emit the correct iterator-based conversion (or `serde_wasm_bindgen::to_value` for WASM) instead of `format!("{:?}", val.<field>)`. Fixes `Metadata.additional` mismatches in pyo3, napi, php, and wasm bindings.
- fix(backend-napi): widen nested `Vec<Vec<primitive>>` element-wise in return conversion when the binding declares the wider type (`f32` → `f64`, `u64`/`usize`/`isize` → `i64`). Mirrors the existing single-Vec arm so `embed_texts` and similar functions returning `Vec<Vec<f32>>` compile.
- fix(backend-php): widen nested `Vec<Vec<u64/usize/isize>>` element-wise in return conversion to match the i64 cast emitted for single-Vec return types.
- fix(backend-php): `gen_stub_return` now respects the function's actual error variance — non-Result functions get a type-appropriate default (e.g. `String::new()`, `Vec::new()`, `None`) instead of an `Err(PhpException::default(...))` body that violates the function signature.
- fix(backend-wasm): emit `serde_wasm_bindgen::from_value` deserialization for `Vec<Vec<T>>` parameters that arrive as `JsValue`. Without it, `generate_cache_key` and similar functions taking nested-vec composites passed the raw `JsValue` into the core fn.
- fix(core/version): add `to_r_version()` converting SemVer prereleases to CRAN-compatible four-component form.
- fix(scaffold/r): generate `packages/r/src/Makevars`, `Makevars.in`, `Makevars.win.in`, and `src/entrypoint.c`.
- fix(scaffold/r): change scaffolded `Cargo.toml` crate-type from `["cdylib"]` to `["staticlib", "lib"]`.
- fix(scaffold/r): generate `packages/r/NAMESPACE` with `useDynLib` bootstrap.
- fix(scaffold/r): use `to_r_version()` in `DESCRIPTION` `Version:` field.
- fix(backend-extendr): `bridge_imports()` returns bare paths, not full `use` statements.
- fix(cli/sync-versions): write CRAN-compatible version to `packages/r/DESCRIPTION`.
- fix(cli/validate-versions): compare R DESCRIPTION against CRAN-compatible version.

## [0.12.9] - 2026-04-30

### Fixed

- fix(e2e/go): add `returns_void` field to `CallConfig` so functions returning only `error` (e.g. `validate_host`, `validate_port`, `validate_language_code`) emit `err := func()` instead of `_, err := func()` in generated Go tests.
- fix(e2e/go): Go override `returns_result = true` now respected for `dedup_text` and similar functions returning `(value, error)`; previously the call-level default (`false`) overrode the per-language override.

## [0.12.8] - 2026-04-30

### Fixed

- fix(backend-dart): convert nested `Vec<Vec<f32>>` to `Vec<Vec<f64>>` in bridge so embed_texts return type matches binding declaration.

## [0.12.7] - 2026-04-30

### Fixed

- fix(backend-rustler): `force_build` honors `Application.compile_env(:rustler_precompiled, [:force_build, :<app>], false)` in addition to env var + Mix.env() heuristics, so consumers can opt into source builds via mix config.
- fix(extract): empty struct with `#[derive(Default, Serialize, Deserialize)]` is treated as a transparent NifMap data type rather than an opaque resource handle (e.g. `ExcelMetadata{}` is data, not a `ResourceArc`).
- fix(scaffold/precommit): generated `.pre-commit-config.yaml` includes the shared `kreuzberg-dev/pre-commit-hooks` repo (shfmt/shellcheck/hadolint/textlint), tracked via the new `KREUZBERG_PRECOMMIT_HOOKS_REV` template version.
- fix(e2e/java): per-fixture `options_type` overrides now collected for ObjectMapper imports/usage; previously only class-level options_type was honored, leaving overrides un-imported.
- fix(backend-magnus): optional Ruby parameters typed as `Option<magnus::Value>` (not `magnus::Value`) — handler now matches `Some(_v) if !_v.is_nil()` before calling `.funcall(...)`, fixing a compile error when an optional struct arg is omitted.
- fix(backend-pyo3): union alias types used directly in `options.py` runtime expressions (e.g. `FormatMetadata = str | ExcelMetadata | ...`) are now imported unconditionally instead of under `TYPE_CHECKING`. Because union alias RHS is evaluated at module load, all names must resolve at import time; previously the TYPE_CHECKING guard caused `NameError` for data-enum payload struct types not defined locally.
- fix(codegen/core-to-binding): `Map<String, Json>` fields now emit `.to_string()` per-value conversions (same as `Map<String, String>`). Previously the map-value branch only matched `TypeRef::String`, so `serde_json::Value` values were not converted, causing a type mismatch in NAPI-RS and other backends.
- fix(e2e/go): `go.mod` now includes `github.com/stretchr/testify` as a required dependency so generated Go test modules compile without a missing-import error.
- fix(e2e/go): `bytes`-type args now decode from base64 at runtime via `base64.StdEncoding.DecodeString`; the `encoding/base64` import is emitted only when needed.
- fix(e2e/go): optional `string` args are now passed as `*string` (address of a local) matching Go binding signatures that take pointer-to-string for nullable strings.
- fix(e2e/go): functions that return only `error` (no result value) now emit `err := fn(...)` instead of `_, err := fn(...)`, fixing a compile error when the function signature is `func(...) error`.
- fix(e2e/go): `result_is_simple` functions (returning `*string`, `*bool`, etc.) now dereference the pointer into a `value` local before assertions and emit a nil guard, rather than asserting directly on a pointer type.
- fix(e2e/go): `greater_than` assertions on optional (pointer) fields now emit a nil guard before the comparison, preventing a nil-pointer dereference.
- fix(e2e/go): `contains` assertions on optional array fields now use `strings.Join(*field_expr, " ")` instead of incorrectly dereferencing an array pointer as a string.
- fix(e2e/go): `contains_any` is now recognized as a string-assertion type that requires the `strings` import.
- fix(e2e/typescript): `package.json` local dependency now uses `file:<pkg_path>` instead of `workspace:*`, fixing resolution when the e2e package is not inside a pnpm workspace.
- fix(e2e/typescript): options-type imports now collect all distinct types across all per-fixture call overrides, not only the default call's options type; each type is imported exactly once.
- fix(e2e/typescript): `resolve_node_function_name` now converts snake_case function names to camelCase (NAPI-RS convention) when no explicit node override is set, eliminating `undefined is not a function` errors at runtime.
- fix(e2e/typescript): optional config args with no fixture value now emit `{} as unknown as <OptionsType>` instead of being skipped, matching NAPI-RS binding signatures that require the config parameter.
- fix(e2e/typescript): config `json_object` args are now cast via `as unknown as <T>` to suppress TS2352 type-overlap errors for partial config objects.
- fix(e2e/typescript): `bytes`-type args now emit `Buffer.from("<b64>", "base64")` so base64 fixture values are decoded to `Uint8Array` at test time.
- fix(e2e/typescript): `not_empty` assertions now handle array fields with `.length` checks and non-string struct fields with `!= null` presence checks, rather than emitting `.length` on every type unconditionally.
- fix(e2e/core): `CallOverride` gains an `arg_order` field to allow per-language parameter reordering when a binding's function signature differs from the canonical `args` list order.
- fix(e2e/escape): `escape_python` now escapes NUL (`\x00`) and other ASCII control characters (U+0001–U+001F) as `\xNN` sequences, preventing malformed Python string literals from non-printable fixture bytes.

## [0.12.6] - 2026-04-30

### Fixed

- fix(e2e/rust): `Option<Vec<T>>` fields in `fields_array` now emit `as_deref().unwrap_or(&[])` instead of `unwrap_or("")` in the unwrap binding. Previously, `as_deref()` on `Option<Vec<T>>` yields `Option<&[T]>`, so the `&str` fallback produced an `E0308` type mismatch (e.g. `detected_languages`, `metadata.sheet_names`).
- fix(e2e/rust): `greater_than_or_equal` assertions on optional non-array fields (e.g. `Option<usize>`) now emit `.unwrap_or(0) >= N` instead of a bare `>= N` comparison. Comparing `Option<usize> >= 2` directly is a type error (e.g. `metadata.sheet_count >= 2`).
- fix(e2e/rust): `equals` assertions with string values on optional string fields that were not pre-unwrapped (e.g. inside a `result_is_vec` loop) now emit `.as_deref().unwrap_or("").trim()` instead of `.trim()` directly on `Option<String>`, fixing `E0599` no method `trim` on `Option<T>` (e.g. `metadata.output_format`).
- fix(e2e/rust): `field: "result"` used as a sentinel to refer to the whole return value no longer emits `result.result`. When the field path exactly matches the result variable name, the codegen now references the result variable directly.
- fix(e2e/c): `result_type_name` fallback now derives from the base `call.function` name instead of the C-overridden function name. When `[e2e.call.overrides.c]` sets `function = "htm_convert"` and `prefix = "htm"`, the fallback type was `HtmConvert`, which combined with the `HTM` prefix produced `HTMHtmConvert` (doubled prefix). It now correctly produces `HTMConvert`.
- fix(backend-go): `TypeRef::Bytes`-returning methods and functions no longer emit `defer C.{prefix}_free_string(ptr)`. Bytes pointers alias internal FFI storage and must not be freed by the caller; freeing via `_free_string` (which expects `*C.char`) would corrupt the parent handle and caused cgo type errors. The `unmarshalBytes` helper copies the data without freeing the source pointer.
- fix(backend-php): PHP type stub methods with non-void return types now emit `throw new \RuntimeException('Not implemented.');` instead of an empty `{ }` body. Empty bodies are invalid for non-void methods under PHPStan level 9 (`implicitlyImpureMethod` / missing return), which broke `composer check` on downstream repos (e.g. `getErrorCode(): int`, getter methods, and `*Api` static stubs).

## [0.12.5] - 2026-04-30

### Added

- **Per-language `core_import` derivation.** When `[wasm|dart|swift].core_crate_override` is set, the wasm backend now also routes generated Rust `use` paths and `From`/`Into` impls through the override (e.g. `spikard_core::ProblemDetails` instead of `spikard::ProblemDetails`). Without this, the override only flipped the Cargo dep key but left source paths pointing at the umbrella crate, producing E0433 unresolved-crate errors at compile time. New helper: `AlefConfig::core_import_for_language(lang)` in `crates/alef-core/src/config/mod.rs`. Defaults to `core_import()` when no override is set, so existing configs are unaffected.
- **Wasm backend respects `exclude_types` for error converters.** `gen_wasm_error_converter` calls and the WASM exports list now skip any error whose `name` is listed in `[wasm].exclude_types`, mirroring the existing exclusion behavior for structs/enums/functions. Lets a wasm-safe surface (e.g. schema validation only) drop sibling-crate errors like `GraphQLError` or `SchemaError` without the binding referencing the unlinked crate.

## [0.12.4] - 2026-04-30

### Fixed

- fix(backend-php): inherent-method delegation helper (`gen_php_lossy_binding_to_core_fields`) now applies per-value `.into()` and preserves the Option layer for `Map<K, Named>` and `Option<Map<K, Named>>` fields. Same bug class as 0.12.3 fix in `alef-codegen::binding_helpers`, but PHP has its own helper that needed the same patch.

## [0.12.3] - 2026-04-30

### Fixed

- fix(codegen/binding-helpers): `gen_lossy_binding_to_core_fields` now applies per-value `.into()` for `Map<K, Named>` and `Option<Map<K, Named>>` fields. The 0.12.2 round only patched `field_conversion_to_core` (used by the `From<Binding> for Core` impl), but the inherent-method delegation path (used for `with_chunking`, `all`, `minimal`-style wrappers on PyO3/PHP/NAPI/Magnus/Rustler) is a parallel codegen path that constructs `core_self` directly. Both paths now emit per-value `(k, v.into())` so a binding-wrapper Map value converts to the core type.

## [0.12.2] - 2026-04-30

### Fixed

- fix(e2e/rust): `not_empty` / `is_empty` assertions on a field that is BOTH `fields_optional` AND `fields_array` now emit `as_ref().is_some_and(|v| !v.is_empty())` and `as_ref().is_none_or(|v| v.is_empty())` respectively, so `Option<Vec<T>>` struct fields (e.g. `chunks`, `images`, `pages`, `extracted_keywords`) compile against the assertion. Previously the codegen emitted `result.<field>.is_empty()` against the bare `Option<Vec<_>>`, which failed `E0599`.
- fix(e2e/rust): when a call returns `Vec<T>` (`result_is_vec` rust override), the call-site optional-string unwrap pass is skipped so per-element iteration emitted by `render_assertion` is the only place fields are accessed; previously the call-site emitted `let metadata_x = result.metadata.x.as_deref().unwrap_or("")` against `Vec<ExtractionResult>` (no `metadata` field on the Vec).
- fix(e2e/rust): non-optional `string` arguments are no longer prefixed with `&` in the call expression. The fixture binding is already a `&'static str` literal; adding another reference produced `&&str`, mismatching `&str` parameter slots (e.g. `extract_email_content(data, mime_type, ...)`). For `impl AsRef<Path>` parameters both forms still satisfy the bound thanks to `&&str: AsRef<Path>`.
- fix(e2e/rust): `not_empty` on a bare `Option<T>` result (no field) now emits `result.is_some()` when the call is annotated `result_is_option`, matching the existing `is_some()`/`is_none()` mapping for field-level checks.
- fix(backend-pyo3): python_field_type now resolves Named(DataEnum) types correctly per emit context (options.py dataclass field vs _native.pyi stub) — introduces EmitContext enum and threads it through all collection branches (Map, Vec, Optional) so that in the OptionsModule context the bare name resolves to the locally defined union type alias rather than to the native module import, eliminating the mypy type mismatch between caller-supplied `dict[str, options.ExtractionPattern]` and the annotation `dict[str, _native.ExtractionPattern]`.
- fix(codegen/core-to-binding): emit explicit arms for `Map<K, Named>`, `Option<Map<K, Named>>`, `Vec<Named>`, `Option<Vec<Named>>` instead of falling through to the binding-to-core helper (was emitting wrong-direction conversions; broke every backend that uses the shared converter for high-level `Option<Map<Named>>` fields).
- fix(backend-rustler): split native.ex `force_build:` keyword across three lines so `mix format` accepts it without reformatting (was 114 chars, exceeded Elixir's 98-char default).
- fix(codegen/binding-to-core): apply per-value `.into()` when emitting `Option<Map<K, Named>>` field conversions (was dropping the wrapper conversion, causing rustc type mismatch in PyO3/PHP/Magnus/Rustler).
- fix(codegen/binding-to-core): preserve `Option` layer in optionalized-field path when field is genuinely IR-optional (was using `unwrap_or_default` and dropping `Option`, breaking NAPI `Option<Map<K, Named>>` round-trip).

### Added

- **Per-language `core_crate_override`.** `[wasm].core_crate_override`, `[dart].core_crate_override`, and `[swift].core_crate_override` let a Rust binding crate point at a sub-crate other than `[crate.name]` (e.g. a wasm-safe `spikard-core` instead of the umbrella `spikard` facade). When set, the binding's generated `Cargo.toml` depends on `<override> = { path = "../<override>" }` (wasm) or the equivalent `crates/<override>` path (dart/swift) and the override key replaces the umbrella crate as the core dep. (`crates/alef-core/src/config/languages.rs`, `crates/alef-core/src/config/mod.rs`, `crates/alef-backend-wasm/src/gen_bindings.rs`, `crates/alef-backend-dart/src/gen_rust_crate/cargo.rs`, `crates/alef-backend-swift/src/gen_rust_crate/`)
- **Per-language `exclude_extra_dependencies`.** `[wasm|dart|swift].exclude_extra_dependencies` filters specific keys out of the merged `[crate.extra_dependencies]` set for that language only — useful when shared sibling crates (e.g. `spikard-http`, `spikard-graphql`) cannot be linked into the wasm target. (`crates/alef-core/src/config/mod.rs::extra_deps_for_language`)

Both fields default to unset/empty so existing configs produce byte-identical output.

- test(backend-wasm): round-trip test for `Map<K, Named>` field through `serde_wasm_bindgen`, locks in the contract that Named values inside a Map must have symmetric serde impls (`crates/alef-backend-wasm/tests/gen_bindings_test.rs`).

### Changed

- BREAKING(cli): `alef generate` now runs language formatters by default; pass `--no-format` to skip. The previous `--format` flag is removed (was opt-in and easily forgotten, causing generated output to fail downstream linters like `mix format`). `alef all` behaves the same way — pass `--no-format` to suppress formatters.
- refactor(codegen): introduce `TypeMapper` trait in `alef-codegen`; every backend now implements it with exhaustive `TypeRef` matching. The Go, Java, C#, and FFI backends have been migrated to `GoMapper`, `JavaMapper`/`JavaBoxedMapper`, `CsharpMapper`, and `FfiParamMapper`/`FfiReturnMapper` structs respectively — all implementing `TypeMapper`. Adding a new `TypeRef` variant now produces a compile error in every backend that hasn't handled it, retiring the silent-fallthrough bug class that previously hid `AHashMap` as `Named("AHashMap")`. The dead `extendr/type_map.rs` stub (comment-only file) has been removed; `ExtendrBackend` already implemented `TypeMapper` in `gen_bindings.rs`.

## [0.12.1] - 2026-04-30

### Fixed

- **e2e mock-server respects fixture `expected_response.headers`.** The alef-generated mock HTTP server (both the test-embedded `mock_server` module and the standalone `mock-server` binary used by cross-language e2e suites) now applies fixture-declared response headers to the served response. Previously, headers from `http.expected_response.headers` (spikard schema) and `mock_response.headers` (liter-llm schema) were silently dropped, causing consumer header assertions (CORS, request-id, auth challenge, compression, etc.) to come back as `null`. `MockResponse` gained a `headers: HashMap<String, String>` field, `Fixture::as_mock_response()` bridges headers from both schemas, and the mock-server route handlers iterate the map and apply each entry via `Response::builder().header(name, value)`. Repeated `.header()` calls preserve multi-value semantics for headers like `Set-Cookie`. (`crates/alef-e2e/src/fixture.rs`, `crates/alef-e2e/src/codegen/rust.rs`)

- **go: skip method emission on opaque error types.** When a `TypeDef` is both opaque and registered as an error, `gen_go_error_struct` already emits it as a value-type struct with `Code`/`Message` fields (no `ptr`). Previously the codegen still tried to emit method bodies that dispatch through `h.ptr`, producing uncompilable Go (e.g. `GraphQLError.StatusCode` referencing `h.ptr` against a value struct). Methods on these types are now skipped. (`crates/alef-backend-go/src/gen_bindings.rs`)

### Added

- **Rust e2e call-override fields.** `[e2e.call.overrides.rust]` learns `wrap_options_in_some`, `extra_args`, `returns_result`, `result_is_vec`, and `result_is_option` to support fallible signatures whose options slot is owned `Option<T>` and which take additional trailing positional args. (`crates/alef-core/src/config/e2e.rs`, `crates/alef-e2e/src/codegen/rust.rs`)

### Reverted

- **Wasm HTTP-fixture auto-skip** (introduced in fa2d03b1 then reverted in d8ff34c7). The wasm e2e target needs a real fix — a wasm-safe dispatch entrypoint that exercises HTTP fixtures in-process — rather than skipping them.

## [0.12.0] - 2026-04-30

### Fixed

- fix(extract): recognise `AHashMap`, `IndexMap`, and `FxHashMap` as map types in the type resolver (previously fell through to `TypeRef::Named`, causing every binding backend to emit a string/opaque type instead of a real map).

### Added

- **Blocker A: HTTP fixture mock-server support for spikard-style fixtures.** The e2e generator now emits the `mock-server` binary and per-language bootstrap code for projects whose fixtures use the `http.expected_response` schema (spikard shape), not just `mock_response` (liter-llm shape). Changes:
  - `fixture.rs`: `needs_mock_server()` returns `true` for HTTP fixtures; new `as_mock_response()` bridges both schemas to a unified `MockResponse`.
  - `codegen/rust.rs`: embedded `Fixture` struct in the mock-server binary now deserializes both `mock_response` and `http.expected_response`; route-loading uses `as_mock_response()` accessor.
  - `codegen/typescript.rs`: HTTP test cases now call `fetch(MOCK_SERVER_URL/fixtures/<id>)` instead of `app.request(bare_path)`.
  - `codegen/python.rs`: conftest.py now spawns the mock-server binary and exposes `MOCK_SERVER_URL`; HTTP test functions use `urllib.request` against `/fixtures/<id>`.
  - `codegen/ruby.rs`: generates `spec/spec_helper.rb` that starts the mock-server; HTTP examples use `Net::HTTP` against `/fixtures/<id>`.
  - `codegen/php.rs`: `bootstrap.php` spawns the mock-server; HTTP tests use `MOCK_SERVER_URL` and `/fixtures/<id>`.
  - `codegen/elixir.rs`: `test_helper.exs` starts the mock-server via Port; HTTP tests use `Req` against `/fixtures/<id>`.

### Changed

- Bump templated `ext-php-rs` pin to `0.15.12` for PHP 8.5 compatibility (downstream consumers regenerate to pick up upstream's PHP 8.5 build fixes).
- **BREAKING: `[e2e.calls.X].returns_result` default flipped from `true` to `false`.** Most e2e fixture call configs target functions whose Rust signatures do not return `Result<T, E>` (e.g. `String`, `Cow<'_, str>`, `bool`, builder types). The previous default emitted `.expect("should succeed")` unconditionally, producing `no method named expect` compile errors against non-Result returns. Existing configs that genuinely call Result-returning functions (`extract_*`, `batch_extract_*`, `chunk_*`, `render_*`, `validate_*`, `detect_languages`, `blake3_hash_*`, `compute_hash`, etc.) must now set `returns_result = true` explicitly. (`crates/alef-core/src/config/e2e.rs`)

### Added

- **`alef_extract::validate_call_export(surface, module_path, function_name)` public API.** Returns `ExportValidation::Ok` when the function is exported at the declared path, `WrongPath` when the function exists but not at the given module path (includes all actual `rust_path`s found), or `NotFound` when the function is absent from the surface entirely. Used by `alef generate` to fail fast on C1 (function not re-exported at crate root) and C2 (wrong definition selected by dedup). (`crates/alef-extract/src/lib.rs`)
- **`alef_extract::return_type_fields(surface, function_name)` public API.** Returns the public fields of the struct returned by a named free function. Returns `None` for primitive, `String`, `Vec`, unit, or opaque (no-field) return types. Allows the Rust codegen to validate `result.field` assertions at generation time. (`crates/alef-extract/src/lib.rs`)
- **`[e2e.calls.X].args[].owned` flag.** When `true`, the Rust codegen emits the argument as an owned binding and passes it by value rather than by reference. Use for parameters whose Rust signature is `Vec<T>` (not `&Vec<T>` / `&[T]`) — for example `batch_extract_file(items: Vec<(PathBuf, Option<FileExtractionConfig>)>, config: &ExtractionConfig)`. Defaults to `false`. (`crates/alef-core/src/config/e2e.rs`)
- **`[e2e.calls.X].args[].element_type` field.** For `json_object` args whose Rust target is `&[T]`, set to the element type literal (`"String"`, `"f32"`, etc.) so the codegen emits `let name: Vec<element_type> = serde_json::from_value(...).unwrap();`. Without this annotation `serde_json::from_value` cannot infer the unsized slice type and the generated test fails to compile with E0277. (`crates/alef-core/src/config/e2e.rs`)

### Fixed

- **Go backend: duplicate sentinel error declarations across multiple `ErrorDef`s.** When two error enums in the same crate shared variant names (e.g. `GraphQLError::ValidationError` and `SchemaError::ValidationError`), the Go binding emitted two top-level `var (...)` blocks each declaring `ErrValidationError`, breaking compilation with `redeclared in this block`. The Go backend now emits a single consolidated sentinel block; colliding variant names are disambiguated by qualifying with the parent error's stripped base name (`ErrGraphQLValidationError` and `ErrSchemaValidationError`). Unique variant names continue to use the unqualified `Err{Variant}` form. New public APIs `alef_codegen::error_gen::gen_go_sentinel_errors` and `gen_go_error_struct` allow callers to control sentinel/struct emission independently. (`crates/alef-codegen/src/error_gen.rs`, `crates/alef-backend-go/src/gen_bindings.rs`)

- **Magnus backend: typed-options params now accept `magnus::Value`.** Functions/methods with `Option<Named>` (or `Named`) parameters previously generated a `Option<String>` ABI that forced Ruby callers to `Hash#to_json` explicitly — and any failure to do so raised `TypeError`. The binding now accepts `magnus::Value` and calls `to_json` internally before `serde_json` deserialization, so a plain Ruby Hash works directly. Closes the upstream regression in `kreuzberg-dev/html-to-markdown#334`.

- **Phase 1: Rust e2e codegen A1/A3/A4/A5 fixes** — Eliminate `E0308 expected &T found &Option<_>`, `E0308 expected Vec<T> found &_`, and `E0277 trait bound` errors via correct optional handling, owned-param passing, slice-type annotation, and simple-return-type detection.
  - **A1:** No longer wraps optional `json_object` args in `Some(...)`; desers as `T` directly, passes `&T`. (`crates/alef-e2e/src/codegen/rust.rs`)
  - **A3:** Respects `owned = true`, passes by value instead of reference. (`crates/alef-e2e/src/codegen/rust.rs`)
  - **A4:** Emits `Vec<element_type>` annotation for slice args when `element_type` is set. (`crates/alef-e2e/src/codegen/rust.rs`)
  - **A5 (partial):** `result_is_simple = true` in call overrides redirects field-access assertions to the result variable directly. (`crates/alef-e2e/src/codegen/rust.rs`)

- **Phase 2: Per-language e2e codegen fixes** — Parallel fixes across Python, TypeScript, Go, Java, C#, Ruby, PHP, Elixir, and other languages to match Rust A1-A5 patterns.
  - **A1 (Python):** No longer wraps optional `json_object` args; passes values directly. When optional arg value is null, skip argument (function default handles None).
  - **A2 (Python, TypeScript, and all languages):** Respect `returns_result=false` — skip error-handling try/except/await for non-Result calls.
  - **Python codegen (`crates/alef-e2e/src/codegen/python.rs`):** A1 and A2 fixes.
  - **TypeScript codegen (`crates/alef-e2e/src/codegen/typescript.rs`):** A2 fix (A1 already handled correctly).

- **Phase 2: Validation hardening in `validate_fixtures_semantic()`** — Add semantic checks to catch configuration errors at `alef generate` time rather than downstream build failures.
  - **D1:** Validate call arg arity and types against IR function signatures (planned: integrate in follow-up with alef-extract).
  - **D2:** Validate return-type field paths against IR struct definitions; reject fixture assertions on fields not in the return type (planned: integrate in follow-up with alef_extract::return_type_fields).
  - **D3:** Integrate module-path validation into generate flow (D3 already implemented in alef-cli; validate.rs updated for consistency).

- **alef-cli: fixed `replace('-', "_")` Rust 2024 edition char-literal compile error.** The Rust 2024 edition requires `&str` for the first argument of `str::replace`; passing `'-'` (char) caused an E0308 in the validate-call path. (`crates/alef-cli/src/main.rs`)

## [0.11.26] - 2026-04-30

### Fixed

- **PyO3 stubs now emit `async def` for async functions and methods so mypy accepts the `await` in the generated `api.py` wrapper.** The 0.11.24 fix changed `api.py` to use `async def fn(...): return await _rust.fn(...)` for pyo3-async functions, but the corresponding `.pyi` stub kept declaring the underlying `_rust.fn` as plain `def fn(...) -> T`. mypy then errored at every wrapper call site with `Incompatible types in "await" (actual type "T", expected type "Awaitable[Any]")`. The stub generator now reads `FunctionDef.is_async` / `MethodDef.is_async` and emits `async def` so the underlying `_rust` symbol is typed as a coroutine; the generated `api.py` wrapper's `await` then type-checks. Both free functions and instance methods (including static methods) are covered. (`crates/alef-backend-pyo3/src/gen_stubs.rs`)
- **E2E TypeScript generator now auto-produces `globalSetup.ts` for HTTP test fixtures.** The generator was only creating `globalSetup.ts` when `client_factory` was configured, leaving HTTP test suites without proper mock server setup. Tests would fail with "app is not defined" at runtime. The generator now checks `has_http_fixtures` and generates `globalSetup.ts` unconditionally when HTTP tests are present (regardless of `client_factory`). The setup creates a fetch-wrapped HTTP client (`createApp`) and exposes it as `global.app` to all test suites. Vitest's `globalSetup` config is also auto-enabled whenever `needs_global_setup` is true. (`crates/alef-e2e/src/codegen/typescript.rs`)

## [0.11.25] - 2026-04-30

### Fixed

- **C# JSON converter switch-case block braces now match `dotnet format`'s default style.** The `gen_csharp_json_converter` writer emitted block braces aligned with the `case` keyword (col 12), but `dotnet format` reformats them one level deeper (col 16). On every commit the `csharp-format` pre-commit hook reshuffled the braces, which then broke `alef-verify`'s per-file hash check on the very next run. The generator now matches the formatter's expected output, eliminating the hook ping-pong. (`crates/alef-backend-csharp/src/gen_bindings.rs`)

## [0.11.24] - 2026-04-30

### Fixed

- **PyO3 backend: sanitized struct fields now emit `#[serde(skip)]` to prevent JSON round-trip failures (`#44 item 1`).** Fields whose types were sanitized to `String` placeholders (e.g. `CancellationToken → String`) were included in the derived `serde::Deserialize` impl. Deserializing a binding struct from JSON would fail with "unknown field 'cancel_token'" because the core type never expects that field. The fix adds `serde(skip)` alongside the existing opaque-field skip logic, so sanitized fields are excluded from JSON serialization/deserialization. (`crates/alef-codegen/src/generators/structs.rs`)
- **PyO3 backend: sanitized enum-like fields no longer cause "unknown variant ''" round-trip errors (`#44 item 2`).** A non-`Option` enum field sanitized to `String` (e.g. `result_format: OutputFormat → String`) derived `Default` as `String::default()` → `""`. Round-tripping through serde then failed with "unknown variant ''". The same `serde(skip)` fix from item 1 applies: the field is excluded from JSON, so the in-memory default is used silently. (`crates/alef-codegen/src/generators/structs.rs`)
- **`api.py` wrappers now forward arguments by keyword to match pyo3 signature order (`#44 item 3`).** The Python wrapper called `_rust.fn(path, config, mime_type)` in wrapper-declaration order, which misaligned with the pyo3 `#[pyo3(signature = (path, mime_type=None, config=None))]` declaration. The call is now `_rust.fn(path=path, mime_type=mime_type, config=config)` so slot alignment is independent of order. (`crates/alef-backend-pyo3/src/gen_bindings.rs`)
- **Async pyo3 functions now produce `async def` + `await` wrappers in `api.py` (`#44 item 4`).** Functions with `is_async = true` were emitted as plain `def fn(...)` wrappers that returned a coroutine directly. Callers who assigned the result without awaiting got a coroutine object instead of the resolved value, and type checkers saw the wrong return type. The generated wrapper is now `async def fn(...): ... return await _rust.fn(...)`. (`crates/alef-backend-pyo3/src/gen_bindings.rs`)
- **Trait-bridge `register_*` helpers are now exported via `api.py` and included in `__init__.py` `__all__` (`#44 item 5`).** `register_embedding_backend` and `register_ocr_backend` were added as `#[pyfunction]` to the pyo3 module but were absent from `api.py` and `__all__`, so `kreuzberg.register_ocr_backend(...)` raised `ImportError`. Pass-through wrappers are now emitted in `api.py` for all trait bridges that declare `register_fn`, and both `__init__.py` imports and `__all__` are updated accordingly. (`crates/alef-backend-pyo3/src/gen_bindings.rs`)

### Changed

- **PyO3 and NAPI generated bindings suppress pedantic/nursery clippy lints that don't apply to autogenerated FFI code.** Downstream projects that opt into `clippy::pedantic = "deny"` and `clippy::nursery = "deny"` (e.g. spikard's `[workspace.lints.clippy]`) were getting 240+ errors in `crates/<crate>-py/src/lib.rs` and `crates/<crate>-node/src/lib.rs` — none of them real bugs, all of them stylistic complaints about generated wrappers (every accessor wanting `#[must_use]`, every `-> CrateName` wanting `-> Self`, every `as` cast at the JS/Python boundary, every `Deserialize` derive on a type that has unsafe FFI methods). Fixing each in the generators would require per-template rewrites with no functional impact, so both backends now emit a documented `#![allow(clippy::*)]` block covering the FFI-specific false-positives: `unsafe_derive_deserialize`, `must_use_candidate`, `return_self_not_must_use`, `use_self`, `missing_const_for_fn`, `missing_errors_doc`, `needless_pass_by_value`, `doc_markdown`, `derive_partial_eq_without_eq`, `uninlined_format_args`, `redundant_clone`, `implicit_clone`, `redundant_closure_for_method_calls`, `wildcard_imports`, `option_if_let_else`, `too_many_lines`. NAPI also picks up the cast-family lints (`cast_possible_wrap`, `cast_possible_truncation`, `cast_sign_loss`) that pyo3 already had. Each entry is annotated with the rationale in the source. (`crates/alef-backend-pyo3/src/gen_bindings.rs`, `crates/alef-backend-napi/src/gen_bindings.rs`)

## [0.11.23] - 2026-04-30

### Fixed

- **C# csproj `<None Include="../../../LICENSE" />` had three `../` segments; flat csproj layout (`packages/csharp/<Namespace>.csproj`) only needs two to reach the repo root.** With three `../` segments, `dotnet pack` resolved the LICENSE path to one directory *above* the repo and bailed with `error NU5019: File not found`. The csproj is scaffold-once so existing repos keep their hand-fixed value, but new scaffolds now emit `../../LICENSE`. (`crates/alef-scaffold/src/languages/csharp.rs`)

## [0.11.22] - 2026-04-29

### Fixed

- **Rustler NIF emission for `&self` opaque methods no longer requires `T: Clone`.** The 0.11.21 fix replaced `resource.inner.as_ref().clone().method(...)` with `(*resource.inner).clone().method(...)` to silence `noop_method_call`, but that pattern requires the underlying opaque type to implement `Clone` — and many real opaque types (e.g. tree-sitter-language-pack's `DownloadManager`) intentionally don't, since they wrap non-cloneable resources like dynamic-library handles. Compilation failed with `error[E0599]: no method named clone found for struct DownloadManager` across every emitted opaque-method call. The emit is now `ReceiverKind`-aware: `ReceiverKind::Ref` produces `resource.inner.method(...)` (Arc<T> derefs to &T, no clone needed); `ReceiverKind::RefMut` and `ReceiverKind::Owned` keep `(*resource.inner).clone().method(...)` (still requires `T: Clone`, but those receiver kinds are uncommon for opaque types and callers can use `[<lang>] exclude_functions` if the type isn't cloneable). Same fix applied to the sync (`gen_nif_method`) and async (`gen_nif_async_method`) call paths.

## [0.11.21] - 2026-04-29

### Fixed

- **Rustler NIF instance-method calls no longer trip `noop_method_call` under `clippy -D warnings`.** `gen_bindings/functions.rs` emitted `resource.inner.as_ref().clone().method(...)` for opaque instance methods. `Arc::as_ref()` returns `&T`; `.clone()` then resolves to `<&T as Clone>::clone` (a pointless reference-clone returning another `&T`), and the lint flagged every emitted NIF call site under `-D warnings`. tree-sitter-language-pack's Elixir NIF builds failed across all 3 platforms (linux-x86_64, linux-aarch64, macos-arm64) with dozens of `error: call to .clone() on a reference in this situation does nothing` at every generated method. The emission now uses `(*resource.inner).clone().method(...)` — the `*` dereferences `Arc<T>` to `T`, so `.clone()` resolves to `<T as Clone>::clone` and produces an owned `T` the method can consume. Same fix applied to both the sync (`gen_nif_method`) and async (`gen_nif_async_method`) emission paths.

## [0.11.20] - 2026-04-29

### Fixed

- **`alef sync-versions` no longer skips work based on a stale `.alef/last_synced_version` cache.** The previous warm-path short-circuit returned early whenever the cached version matched `Cargo.toml`'s, on the assumption that "same version → all manifests still in sync." That assumption breaks in three real cases: a manifest hand-edited to the wrong version, a *new* manifest added after the last sync (e.g. `e2e/rust/Cargo.toml` introduced by `alef e2e generate`), or a stale `alef:hash:` line whose content drifted. CI runs without the cache and re-derives the correct state, so the local hook stayed silent while the `alef-sync-versions` pre-commit hook failed in CI for downstream consumers (most recently: liter-llm rc.14, kreuzcrawl, html-to-markdown, tree-sitter-language-pack — all required `rm -rf .alef` to reproduce the diff). The function now always walks every manifest. The scan is sub-second on kreuzberg-sized repos and the underlying writes are idempotent when nothing is actually stale, so the cost is invisible. The `.alef/last_synced_version` stamp is still written for forward-compatible introspection but is no longer consulted as a gate. (`crates/alef-cli/src/pipeline/version.rs`)

### Added

- **`[csharp].package_id` config field for NuGet `<PackageId>`, decoupled from `[csharp].namespace`.** The csharp scaffold previously emitted both `<RootNamespace>` and `<PackageId>` from `csharp_namespace()`, conflating two distinct identifiers. That conflation silently broke html-to-markdown's NuGet publish: the consumer-facing namespace `HtmlToMarkdown` is owned on nuget.org by an unrelated third party (Enrico Rossini), but the historical `KreuzbergDev.HtmlToMarkdown` package was owned by us. The alef migration overwrote the published-artifact id, and every release returned `403 (does not have permission to access the specified package)` until the csproj was hand-edited. Setting `[csharp] package_id = "KreuzbergDev.HtmlToMarkdown"` now lets the project publish to the owned coordinate while the in-code namespace stays the short, idiomatic form. When unset, `package_id` defaults to `namespace` — existing configs keep their behaviour. New accessor: `AlefConfig::csharp_package_id()`.

### Changed

- **PHP composer name and Swift Package URL no longer hardcode `kreuzberg`.** The PHP e2e composer.json `"name"` field now derives `<vendor>/e2e-php` from the consumer-binding pkg vendor, and the PSR-4 autoload namespace uses the configured PHP namespace (`<configured>\E2e\\`). The PHP README's `composer require <vendor>/<crate>` line uses the same derivation. The PHP e2e `pkg_name` fallback for `[e2e.packages.php].name` derives `<org>/<module>` from `[scaffold] repository` (instead of `kreuzberg/<module>`). The Swift e2e `Package.swift` `.package(url: ...)` for registry mode now uses the configured repository URL with a `.git` suffix, falling back to a vendor-neutral `https://example.invalid/<module>.git` placeholder. New `derive_repo_org()` helper exposed in `alef_core::config`.
- **`alef init` no longer writes `dev.kreuzberg` / `kreuzberg-dev` literals into freshly generated `alef.toml`.** When invoked in a project whose `Cargo.toml` declares `[package] repository = "..."`, the generated config now seeds `[scaffold] repository`, derives `[go] module` from it, and derives `[java] package` via the same reverse-DNS rule as `AlefConfig::java_package()` — so `repository = "https://github.com/foo-org/bar"` produces `module = "github.com/foo-org/bar"` and `package = "com.github.foo_org"`. When the repository is unset, the affected sections are emitted with the offending fields commented out and a `# TODO: set this` marker, instead of writing a plausible-looking but wrong default. When `[go].module` is unset, alef now derives the module path from `[scaffold] repository` / `[e2e.registry] github_repo` by stripping the `https://` scheme (`https://github.com/foo/bar` → `github.com/foo/bar`). When no repository is configured at all, the fallback is a vendor-neutral `example.invalid/<crate>` placeholder that fails `go build` loudly. The internal `package_name()` helper's last-resort `unwrap_or("kreuzberg")` is now `"binding"` (this branch is unreachable in practice — `next_back()` on a non-empty `split('/')` always returns `Some` — but the literal is gone). New `try_go_module()` accessor returns `Result<String, String>` for callers that should error rather than emit the placeholder.
- **Java/Kotlin package fallback no longer hardcodes `dev.kreuzberg`.** When `[java].package` / `[kotlin].package` is unset, alef now derives a reverse-DNS package from the configured repository URL (`[scaffold] repository` or `[e2e.registry] github_repo`): `https://<host>/<org>/<rest>` becomes `<reversed-host>.<org>` (host labels reversed, hyphens replaced with underscores). For kreuzberg-dev consumers (`https://github.com/kreuzberg-dev/<crate>`) this produces `com.github.kreuzberg_dev` — *different* from the previous `dev.kreuzberg` literal. **Action required:** kreuzberg-dev consumers should set `[java] package = "dev.kreuzberg"` and `[kotlin] package = "dev.kreuzberg"` in their `alef.toml` to keep the existing namespace. When no repository URL is configured at all, `java_package()` / `kotlin_package()` fall back to `unconfigured.alef` so the build fails loudly. New `try_java_package()` / `try_kotlin_package()` accessors return `Result<String, String>` for callers that should error rather than emit the placeholder.
- **README scaffolding no longer falls back to `https://github.com/kreuzberg-dev/<crate>` when no repository is configured.** Alef is meant to be vendor-neutral; consumers outside the kreuzberg-dev org were silently picking up that URL in 13 places across `alef-readme` (the README header link plus 12 per-language "See <repo> for usage examples." pointers). The 13 inline `format!("https://github.com/kreuzberg-dev/{name}")` calls now route through a single `AlefConfig::github_repo()` accessor whose fallback is `https://example.invalid/<crate>` — an obviously-broken URL that surfaces in code review instead of smuggling another organization's link into the output. Set `[scaffold] repository = "..."` (or `[e2e.registry] github_repo`) in your `alef.toml` to resolve. A new `AlefConfig::try_github_repo() -> Result<String, String>` accessor is available for callers that should fail hard on missing config.

### Fixed

- **C# wrapper class methods now thread `IntPtr handle` for non-static methods.** `gen_pinvoke_for_method` and `gen_wrapper_method` previously only emitted the visible parameter list, ignoring the receiver. The cbindgen-emitted FFI signature for an instance method is `fn(this: *const T, ...)`, so the C# P/Invoke and the wrapper call site were one argument short — `dotnet build` failed with `CS7036: There is no argument given that corresponds to the required parameter 'ptr'`. Both functions now detect `!method.is_static && method.receiver.is_some()` and prepend `IntPtr handle` (P/Invoke signature) / `handle` (wrapper-to-native call argument) so the surfaces line up.
- **Kotlin error typealiases now use the `Exception` suffix to avoid colliding with same-named structs.** When an `errors` variant shares a name with a struct in `api.types` (e.g. an error variant `Foo` and a struct `Foo`), the previous `typealias Foo = pkg.FooException` would clash with the struct's own `typealias Foo = pkg.Foo`, and `compileKotlin` failed with `Redeclaration:`. The error alias now uses `typealias FooException = pkg.FooException`, mirroring the Java facade's class name; struct typealiases are unchanged.
- **Kotlin scaffold sample object renamed to plain `Sample` so its filename is not project-specific.** ktlint's `filename` rule requires a file with a single top-level declaration to match the declaration name. The sample object is now named `Sample` (matching `Sample.kt`) instead of `<ProjectName>Sample` (which would require `<ProjectName>Sample.kt`); the project name still appears in the body's `println` for context.
- **Gleam scaffold emits a valid `manifest.toml`.** Previous output was a comment-only stub, which gleam's TOML parser rejects with `missing field 'requirements'` on every `gleam check` / `gleam build` invocation. The scaffolded manifest now contains the minimum gleam expects (`packages = []` plus an empty `[requirements]` table); `gleam build` repopulates it on the first run.
- **Java scaffold's checkstyle config now resolves `${checkstyle.suppressions.file}`.** The pom.xml `<configuration>` block references a checkstyle config that requires the `checkstyle.suppressions.file` property to be set, but the config never told maven-checkstyle-plugin where to load that property from. Adds `<propertiesLocation>${project.basedir}/checkstyle.properties</propertiesLocation>` to the plugin config. Also fixes the property's value in `checkstyle.properties` to be relative to packages/java/ (the maven cwd) rather than to repo root.

## [0.11.19] - 2026-04-29

### Fixed

- **`sync-versions` now updates `packages/ruby/Gemfile.lock` alongside the gemspec.** After a version bump the gemspec received the new version but the lockfile was not touched, causing `bundle install` to abort with "The gemspecs for path gems changed, but the lockfile can't be updated because frozen mode is set" on every CI run. `sync_versions` now textually replaces all `<gem-name> (<old-version>)` entries for path gems in Gemfile.lock (both the `PATH > specs` block and the `CHECKSUMS` block) with the new RubyGems-format version. The replacement is idempotent — a second sync with the same version is a no-op.
- **`has_derive` now recognises namespaced derive paths** like `#[derive(serde::Serialize, serde::Deserialize)]`. The bare-ident form (`#[derive(Serialize)]`) was already detected, but the namespaced form fell through and IR types ended up with `has_serde: false`. Backends that gate `_to_json` / `_from_json` emission on `has_serde` (which is correct) then produced binding code referencing FFI functions that don't exist (e.g. `C.spikard_sse_event_to_json` for spikard's `SseEvent`, which derives `serde::Serialize`). The cfg_attr branch already used last-segment matching; the direct-derive branch now mirrors that.
- **Go backend C type references now preserve all-caps abbreviations.** Mirror of the FFI cbindgen-forward-decl fix: `alef-backend-go` ran type names through `heck::ToPascalCase` in seven places (free-method receiver, opaque-handle marshalling, return-value unwrapping, trait bridge trampolines, the `type_name` helper used in unmarshalFoo function names). For types like `GraphQLError` this produced `C.SPIKARDGraphQlError` in the generated `binding.go`, which doesn't match cbindgen's actual `SPIKARDGraphQLError` and causes `go build` to fail with "could not determine what C.SPIKARDGraphQlError refers to". IR type names are already PascalCase from Rust source — the conversion was both unnecessary and harmful for acronym-bearing types. Field-name renaming (`field_name.to_pascal_case()` for `serde_rename_all = "PascalCase"`) and method-name conversion are unchanged.
- **FFI cbindgen forward declarations now match cbindgen's actual emit for types with all-caps abbreviations.** The forward-declaration block in the generated `cbindgen.toml` ran type names through `heck::ToPascalCase`, which mangles abbreviations: `GraphQLError` becomes `GraphQlError`, but cbindgen emits the actual struct in the C header as `SPIKARDGraphQLError` (literal Rust name + prefix). Consumers compiling against the header saw two different type names for the same struct and the build failed (`unknown type name 'SPIKARDGraphQLRouteConfig'; did you mean 'SPIKARDGraphQlRouteConfig'?`). The pre-declarations now use the IR type name verbatim.
- **Java/Kotlin e2e codegen now respects `[java].package` and `[kotlin].package`.** Previously the e2e generators hardcoded `dev.kreuzberg` in three places: the pom.xml `<groupId>`, the test-file `package` declarations, and the generated filesystem path (`src/test/{java,kotlin}/dev/kreuzberg/e2e/...`). Projects whose Java/Kotlin package config used a different group id (e.g. spikard's `dev.spikard`) ended up with package declarations that disagreed with their filesystem location, breaking compilation. The generators now use `alef_config.java_group_id()` / `alef_config.kotlin_package()` consistently — pom.xml `<groupId>`, gradle `group =`, the `package` line, and the path segments all derive from the configured value.

## [0.11.18] - 2026-04-29

A patch release that fixes Javadoc emission so HTML inside backticks survives the Eclipse-formatter Spotless pipeline.

### Fixed

- **Javadoc `{@code …}` content now HTML-escapes its inner `<`, `>`, `&` characters.** A Rust doc comment like `` /// Determines how code blocks (`<pre><code>`) are rendered `` previously emitted `{@code <pre><code>}` with raw HTML inside the tag. Eclipse-formatter Spotless (used by html-to-markdown's `packages/java/pom.xml`) interprets the inner `<pre>` as a real block-level HTML element and shatters the doc comment across multiple `* ` rows — which then breaks `alef-verify` on the very next prek run. The codegen now emits `{@code &lt;pre&gt;&lt;code&gt;}` so Spotless leaves the line alone; readers see the same text since Javadoc renders `{@code}` literally regardless. Both `alef-codegen::doc_emission::escape_javadoc_line` and `alef-backend-java::gen_bindings::helpers::escape_javadoc_line` carry the fix.

### Fixed

- **Python e2e codegen no longer triggers ruff `F401` on `import pytest`.**

## [0.11.17] - 2026-04-29

PHP backend: fix flat-enum codegen to emit correct code instead of no-op conversions.

### Fixed

- **PHP flat-enum `From` impls no longer emit no-op `.into()` for primitive and `String` fields.** `flat_enum_core_to_binding_field_expr` and `flat_enum_binding_to_core_field_expr` previously fell through to `.map(Into::into)` / `.into()` for all types not explicitly handled. For primitives (`u8`, `u16`, `u32`, `i32`, `bool`, `f32`, `f64`, etc.) and `String`, the PHP binding type equals the core type, so `Into::into` is a no-op and triggered `clippy::useless_conversion`. Both functions now emit direct assignment for these same-type cases.
- **PHP flat-enum getter methods no longer call `.clone()` on `Copy` types.** Getters for `Option<u32>`, `Option<u8>`, `Option<bool>`, `Option<i64>` etc. previously always emitted `self.field.clone()`, which triggered `clippy::clone_on_copy`. Getters for Copy fields (`is_php_copy_type` helper: `Primitive` and `Option<Primitive>`) now emit `self.field` directly.
- **PHP flat-enum `From` impls no longer emit `..Default::default()` when all struct fields are covered.** When a variant's explicit field assignments already cover every field in the flat struct, the trailing `..Default::default()` is redundant and triggered `clippy::needless_update`. The codegen now pre-computes the complete set of flat field names and omits the struct update when the variant sets all of them. When `e2e.call.async = true` (or any test in the file is async/skipped/has error assertions), the python e2e generator emits `import pytest` at module level. Pytest is needed for `pytest.fixture` / `pytest.mark.*` decorators, but ruff's `F401` rule strips the import when no symbol is statically referenced in the file body — which then causes `alef verify` to fail on subsequent runs because the file's hash no longer matches the generated content. The import is now suppressed with `# noqa: F401`.

## [0.11.16] - 2026-04-29

WASM trait bridges and FFI multi-crate codegen fixes.

### Added

- **`[wasm.extra_dependencies]` table in `alef.toml`.** Mirrors `[crate.extra_dependencies]` but applies only to the WASM binding `Cargo.toml`. Initial use case: `async-trait = "0.1"` so generated wasm bridges can use `#[async_trait::async_trait(?Send)]` without callers having to declare it manually.

### Fixed

- **WASM trait bridge async impls now correctly emit `#[async_trait::async_trait(?Send)]`.** `gen_bridge_trait_impl` in `alef-codegen` always emitted `#[async_trait::async_trait]` (the default `+ Send` variant), which produced `error[E0053]: method ... has an incompatible type for trait` on `wasm32-unknown-unknown`: the underlying trait future was not `Send`-bounded but the macro-rewritten impl signature was. Added `async_trait_is_send() -> bool` (default: `true`) on the `TraitBridgeGenerator` trait; the wasm backend overrides it to `false`, and `gen_bridge_trait_impl` selects between `#[async_trait::async_trait]` and `#[async_trait::async_trait(?Send)]` accordingly. WASM bindings (kreuzberg-wasm and any other wasm-target generator) now compile clean for `OcrBackend`, `PostProcessor`, `Validator`, and `EmbeddingBackend` bridges.
- **FFI scaffold now merges `[crate.extra_dependencies]` into the generated `crates/<crate>-ffi/Cargo.toml`.** Previously the scaffold emitted only the umbrella crate (e.g. `kreuzberg = { path = "../kreuzberg" }`) plus `serde_json` and `tokio`, which worked when the public API surface lived in a single workspace crate. For multi-crate workspaces (e.g. spikard's `spikard-core`, `spikard-http`, `spikard-graphql`), the FFI bindings codegen emits qualified paths like `spikard_http::ServerConfig` and `spikard_graphql::QueryOnlyConfig` — the cdylib failed to compile because those crates were not direct dependencies. The scaffold now merges entries from `[crate.extra_dependencies]` (sorted, alphabetised) into the `[dependencies]` block, matching the behaviour of the wasm backend.
- **FFI codegen no longer double-prefixes sibling-crate paths in field accessors.** The path-qualification logic in `gen_field_accessor` previously only treated `module_prefix` as already-qualified when it equalled `core_import` or started with `core_import::`. For sibling workspace crates whose names share the project's prefix (e.g. `core_import = "spikard"`, sibling crate `spikard_http`), `module_prefix.starts_with("spikard::")` was false and the codegen emitted `spikard::spikard_http::openapi::OpenApiConfig` — a path that doesn't exist. The check now also accepts `{core_import}_` as a sibling-crate marker, so sibling-crate paths render verbatim.
- **FFI field accessors now look up Named types in the per-binding `path_map`.** When a struct field's `type_rust_path` is `None` in the IR (which alef emits when the type is referenced by short name), the field accessor previously fell back to `core_import` and produced paths like `spikard::ContactInfo` — which fails when the type lives in a sibling crate (`spikard_http::ContactInfo`) and is not re-exported through the umbrella crate. `gen_field_accessor` now threads the `path_map` built in `gen_bindings::generate` and uses `c_return_type_with_paths`, matching the behaviour of `gen_method_wrapper` and `gen_free_function`.
- **FFI codegen now uses `.map(str::to_owned)` for `Option<&str>` returns.** Methods returning `Option<&str>` (e.g. a `get_description() -> Option<&str>` accessor) previously emitted `let result = result.cloned();`, which fails to compile because `str: !Sized` and `Option::cloned` requires `T: Clone` on a sized type. The codegen now special-cases `TypeRef::Optional(TypeRef::String)` to emit `.map(str::to_owned)` instead. The existing `.cloned()` path still applies to `Option<&NamedType>`, `Option<&Vec<...>>`, and `Option<&char>`.
- **FFI codegen now uses `.to_owned()` for `&str` returns (was `.clone()`, a no-op).** Methods returning `&str` previously emitted `let result = result.clone();` — which compiles but is a no-op (str: !Sized doesn't impl Clone, so the call is a noop on the reference) and triggers Rust's `noop_method_call` lint. The codegen now emits `.to_owned()`, producing the owned `String` the FFI layer needs.
- **FFI no longer emits duplicate `to_json` / `from_json` exports when the type defines those methods.** Types that derive both `serde::Serialize` and a manual `to_json` / `from_json` method (e.g. `ProblemDetails::to_json`) caused two FFI functions with the same C name (`{prefix}_{type}_to_json`) — one from the auto-serde codegen path and one from `gen_method_wrapper` — and the cdylib failed with `E0428: defined multiple times`. The auto-serde codegen now skips emitting `to_json` / `from_json` when the type already defines a method of that name.

## [0.11.15] - 2026-04-29

PHP backend: fix codegen errors in tagged data enum `From` impls.

### Fixed

- **PHP flat data enum `From` impls now handle all field kinds correctly.** `gen_flat_data_enum_from_impls` previously emitted `.into()` unconditionally for every variant field, which compiled only when `From<BindingType>` happened to exist. Four cases were missing:
  - **`sanitized: true` fields** (e.g. `TableGrid`, `ImageMetadata`, `PdfMetadata`, `ProcessResult`, `[(u32,u32);4]`): now emits `None` / `Default::default()` instead of trying to convert an opaque or complex core type through a String.
  - **`is_boxed: true` fields** (e.g. `FormatMetadata::Docx(Box<DocxMetadata>)`, `::Html(Box<HtmlMetadata>)`): now wraps the core→binding result in `Some((*val).into())` and the binding→core result in `Box::new(...)`.
  - **`TypeRef::Path` fields** (e.g. `ChunkSizing::Tokenizer { cache_dir: Option<PathBuf> }`): now uses `PathBuf::from(s)` for binding→core and `.to_string_lossy().into_owned()` for core→binding instead of `.into()` (no `From<PathBuf> for String` exists).
  - **`TypeRef::Primitive(Usize | U64 | Isize)` fields** (e.g. `EmbeddingModelType::Custom { dimensions: usize }`): now emits `v as usize` / `v as i64` explicit casts instead of `.into()` (no `From<i64> for usize` exists).
  - Struct variant destructuring patterns for sanitized fields now use `field: _field` syntax (not `_field` alone) to satisfy the Rust struct-pattern completeness requirement.

## [0.11.14] - 2026-04-29

CLI ergonomics, generation performance, and live output for long-running commands. Warm `alef generate` on a 16-language consumer (kreuzberg) drops from ~14s to ~1s.

### Added

- **Standard CLI affordances.** `alef --version` (and `-V`) prints the binary version. New global flags `--verbose` / `-v` (repeatable: `-v` info, `-vv` debug, `-vvv` trace), `--quiet` / `-q` (errors only), and `--no-color` (disable ANSI in log output). Tracing now defaults to `info`-level output on stderr — previously the CLI was effectively silent unless `RUST_LOG` was set, which meant users had no signal during long-running commands.
- **Live streamed output for long-running commands.** `alef setup`, `alef update`, `alef update --latest`, `alef lint`, `alef fmt`, `alef test`, and `alef clean` now stream stdout/stderr to the terminal in real time, line-prefixed with `[<lang>]` when multiple languages run in parallel. Previously output was captured and only re-emitted via `tracing::info!` after the command finished — producing a multi-minute blackout for `pnpm install` / `bundle install` / `cargo update`. Failures are now also surfaced via an explicit `✗ <command> failed: <lang> — <error>` summary line on stderr.

### Changed

- **`alef generate` no longer runs formatters by default.** Formatting was the dominant cost of `alef generate` on multi-language projects (e.g. `cargo fmt --all`, `ruff check --fix .`, `biome format --write .`, `dotnet format`) and ran on the full package directory every invocation — even when only one language regenerated. The behaviour is now opt-in: pass `--format` to `alef generate`, `alef all`, or `alef init` to run formatters. When `--format` is passed, formatters run *only* for languages that actually regenerated this run (other languages skip), making warm `alef generate --format` proportional to changed source files. The tradeoff: projects that previously relied on the implicit format pass to keep `alef verify` green should either (a) pass `--format`, (b) keep formatters in pre-commit hooks, or (c) run `alef fmt` explicitly after generate.

### Performance

- **Public API codegen is now cached.** `pipeline::generate_public_api` output is hashed and skip-written like binding files; previously every warm run rewrote 200+ Python `api.py` / `options.py` / `__init__.py` files for no net change.
- **Deterministic Python public-API imports.** The PyO3 backend's `gen_api_py` collected import names into an `AHashSet` then emitted them via `.join(", ")` — `AHashSet` iteration order is non-deterministic, so the codegen output flipped between runs and the content-hash cache always missed. Imports are now sorted before emit.
- **Idempotent `sync_versions`.** `replace_version_pattern` previously returned `Some(new_content)` whenever its regex matched, even when the rewrite was byte-identical (e.g. when Magnus emits `VERSION = "x"` with double-quotes and the replacement template uses single-quotes). Each `alef generate` then ping-pong-rewrote `version.rb`, marked it as drifted, and triggered a full README regeneration. The function now extracts the version literal from the match and short-circuits when it already equals the target — quote style irrelevant.
- **`sync_versions` short-circuits on warm runs.** Stamps `.alef/last_synced_version` after each successful sync; the next warm run with no `--bump` and an unchanged canonical version skips the entire glob+regex+stat pass over package manifests. When a sync does run, only `readme/docs/scaffold` stage caches are invalidated; the IR cache and per-language binding hashes are preserved.
- **`sources_hash` mtime-prefilter.** A per-source `(mtime_nanos, size)` memo at `.alef/sources_hash.cache` lets warm runs return the previous aggregate hash directly when nothing in the source tree has changed — skipping the read+blake3 pass over every `[crate].sources` file.
- **Hot extractor optimisations.** `extract_serde_rename_all` parses with `attr.parse_nested_meta` instead of stringifying the entire token stream and substring-scanning. `normalize_type_string` now scans bytes directly instead of materialising a `Vec<char>` per call. Combined, these halve allocations on the cold extraction path for large API surfaces.
- **Single-pass `to_pep440`.** Rewritten to build the output in one pre-allocated `String` instead of chaining five `.replace()` calls, each of which allocates a fresh intermediate.
- **`extract_version` regex cache.** The verify path's `extract_version` helper now caches compiled regexes in `OnceLock<Mutex<HashMap>>` so the ~15 verify patterns aren't recompiled per file.
- **Gleam variant collision map** uses `ahash::AHashMap` with `&str` keys; only colliding names allocate owned `String`s (was: `HashMap<String, _>` with per-variant clone).

## [0.11.13] - 2026-04-29

A patch release that addresses two regressions surfaced by the tree-sitter-language-pack v1.8.0-rc.14 release: copy-paste duplication in generated Java FFI methods and Spotless/`alef-verify` hash drift.

### Fixed

- **Java backend deduplicates the FFI Vec-return path through a shared `readJsonList` helper.** Every Vec-returning method previously inlined an identical ~15-line null-check → reinterpret → free → JSON deserialize block, which `cpd` correctly flagged as duplication. The boilerplate now lives in a single private static helper emitted by the helper-emitter; per-method call sites collapse to one line.
- **Java auto-format prefers Spotless when `packages/java/pom.xml` configures `spotless-maven-plugin`.** When detected, `alef generate` runs `mvn -f packages/java/pom.xml spotless:apply -q` instead of `google-java-format`, so the embedded `alef:hash:` value matches what the project's `mvn spotless:check` will see at verification time. Previously the hash drifted on every `alef generate` → prek cycle for any project whose Spotless config diverges from google-java-format defaults (e.g., Eclipse formatter).

## [0.11.12] - 2026-04-29

A patch release that fixes the Java backend so functions returning `PathBuf` emit a properly-typed `java.nio.file.Path` value instead of a raw `String`.

### Fixed

- **Java backend now wraps FFI string returns with `java.nio.file.Path.of(str)` when the declared return type is `Path`.** Previously, methods like `cacheDir()` were declared to return `java.nio.file.Path` but the body returned the raw `String` result of the FFI call, causing `javac` to reject the generated code with "incompatible types". Both `Path` and `Optional<Path>` are now handled correctly.

## [0.11.11] - 2026-04-29

A patch release that adds a `returns_result: bool` field to e2e `[CallConfig]` so the Rust generator can skip the `.expect("should succeed")` unwrap for native non-Result-returning calls.

### Added

- **`[e2e.calls.<name>] returns_result = false`** lets fixture authors mark a call whose Rust function signature returns `T` directly (e.g. `String`, `Vec<u8>`, `bool`, a hash) rather than `Result<T, _>`. The Rust code generator now binds the value with no unwrap when this is set; previous behavior always emitted `.expect("should succeed")`, which failed to compile against any non-Result function. Defaults to `true` to preserve existing behavior.

## [0.11.10] - 2026-04-29

A patch release that fixes the NAPI/Node backend so functions whose only sanitized parameter is a configured `[[trait_bridges]]` param (e.g. `Option<VisitorHandle>`) are emitted via the bridge instead of being silently skipped.

### Fixed

- **NAPI backend now emits top-level functions that take a configured trait-bridge parameter, even when the function is marked sanitized.** Previously the sanitized check ran before `find_bridge_param`, so html-to-markdown's top-level `convert(html, options, Option<VisitorHandle>)` was dropped from the Node binding, causing downstream `import { convert }` to fail at TypeScript compile time. The check now skips a function only when it's sanitized AND no trait-bridge param applies — matching how the PyO3 backend already handles this case (which is why Python had `convert` and Node didn't).

## [0.11.9] - 2026-04-29

A patch release that fixes the PHP backend's lowering of tagged enums with **tuple variants** holding distinct types.

### Fixed

- **PHP backend now gives each tuple variant of a tagged enum its own distinct field instead of collapsing them all to a shared `_0` field.** Previously `pub enum Message { System(SystemMessage), User(UserMessage), … }` lowered to a single `_0: Option<SystemMessage>` (the first variant's type) and the From impls then tried to assign `UserMessage`/`AssistantMessage`/etc to that one field, producing N trait-bound errors per non-first variant. Single-field tuple variants now use the variant's snake_case name as the field name (`system: Option<SystemMessage>`, `user: Option<UserMessage>`, …); multi-field tuple variants get `{variant}_0`, `{variant}_1`, …. Struct variants are unchanged.

## [0.11.8] - 2026-04-29

A patch release that fixes the PHP and Rustler/Elixir backends to preserve struct-variant fields when lowering tagged enums, plus two smaller release-pipeline fixes.

### Fixed

- **PHP backend now generates a real wrapper class for tagged enums with struct variants instead of emitting only string constants.** Previously `pub enum SecuritySchemeInfo { Http { scheme, bearer_format }, ApiKey { location, name } }` was lowered to a few `pub const` strings, the surrounding struct field was demoted to `HashMap<String, String>`, and the generated `From<core::Outer> for php::Outer` impl emitted `(k, v.into())` against the original enum value — which fails to compile with `the trait bound \`String: From<SecuritySchemeInfo>\` is not satisfied`. The backend now emits a flat php_class with a`type_tag` discriminator plus optional fields for every variant, full `#[php_impl]` getters, a `from_json` constructor, and proper `From` impls in both directions. Fixes spikard-php builds with `--features extension-module`.
- **Rustler/Elixir backend now preserves struct-variant fields in tagged enums instead of silently dropping them.** Previously every enum was lowered to `rustler::NifUnitEnum`, so `SecuritySchemeInfo::Http { scheme, bearer_format }` became unit variants `Http`/`ApiKey` with the inner fields gone, and the generated `From` impls fabricated `Default::default()` for every missing field — round-tripping any real value through Elixir produced empty defaults (silent data corruption). The backend now detects struct-variant enums and lowers them to `rustler::NifTaggedEnum` with the original variant fields preserved; the From impls now destructure variants instead of fabricating defaults. Unit-only enums continue to use `NifUnitEnum`.
- **WASM binding crate generation now sets `default-features = false` on the core dep when `[wasm].features` is configured.** Cargo would otherwise OR the explicit feature set with the core crate's defaults, sneaking host-only defaults like `download` back into the `wasm32-unknown-unknown` build and failing on `getrandom`/`mio`.
- **`alef validate-versions` now normalizes per-format before comparing.** It applies `to_pep440` to the canonical version when reading `pyproject.toml`, applies `to_rubygems_prerelease` for Ruby `version.rb`, and skips manifests that exist but don't declare a `version` field (e.g. private pnpm workspace roots). Eliminates spurious mismatches like canonical `1.4.0rc8` vs Python `1.4.0rc8` vs Ruby `1.4.0.pre.rc.8`.

## [0.11.7] - 2026-04-28

A patch release that fixes Rust e2e codegen so optional string/bytes args are passed via `.as_deref()` (yielding `Option<&str>`/`Option<&[u8]>`) rather than `&Option<...>`.

### Fixed

- **Rust e2e codegen no longer generates `&mime_type` for an `Option<String>` argument when the target signature expects `Option<&str>`.** Previously `let mime_type = None;` followed by `extract_file(&path, &mime_type, ...)` produced an `expected Option<&str>, found &Option<_>` E0308 mismatch on every fixture using the `extract_file` call. Optional string args now bind to a typed `Option<String>`/`Option<Vec<u8>>` (so `None` resolves) and pass via `.as_deref()` (or `.as_ref().map(|v| v.as_slice())` for bytes). Non-string optional non-string args use `.as_ref()` to avoid moving the binding.

## [0.11.6] - 2026-04-28

A patch release that prevents the python e2e codegen from emitting test files for categories whose fixtures are 100% skipped for python.

### Fixed

- **Python e2e codegen no longer generates a `tests/test_<category>.py` file when every fixture in that category has `skip.languages` containing `"python"`.** Previously the emitter would still write the file with `@pytest.mark.skip` markers, but the file's `from kreuzberg import ...` line would then reference APIs that aren't bound in Python — failing at module import before pytest could honor the skip marker. The other language emitters (node, ruby, java, go, csharp, php, elixir, r, wasm, dart, gleam, kotlin, swift, zig, typescript) already filtered skipped fixtures upstream and were not affected.

## [0.11.5] - 2026-04-28

A patch release that fixes the GitHub-release existence check so the publish pipeline no longer skips its own builds when only a release tag exists with no binaries attached.

### Fixed

- **`alef check-registry --registry github-release` now optionally verifies asset presence.** Two new flags — `--asset-prefix <STR>` and `--required-assets <a,b,c>` — make the check fail (return `exists=false`) when the release tag exists but the requested binaries have not been uploaded yet. Without this, alef's own `check-github-release` job returned `exists=true` for v0.11.4's empty release (the tag had been created by `gh release create` before the build matrix uploaded artifacts) and the dependent build jobs skipped — producing another tag with no binaries. The action wrapper (`kreuzberg-dev/actions/check-registry`) already exposed `asset-prefix` and `assets` inputs but was previously dropping them on the floor.

## [0.11.4] - 2026-04-28

A patch release that ships v0.11.3's content plus a fix for alef's own publish pipeline. v0.11.3 tagged but never produced binaries because validate-versions aborted on a half-empty `alef.toml`.

### Fixed

- **`alef.toml` in this repo now carries the minimal `[crate]` section and `languages = []` that `load_config` requires.** `alef validate versions` (run by the publish workflow's pre-flight) was failing with `missing field crate`, causing the whole publish run to skip without ever building binaries. Without a `[crate]` table the parser refuses the file even though the subcommand only needs the canonical version from `Cargo.toml`. v0.11.0 through v0.11.3 all share this issue — none of those tags has binary assets on the release page; v0.11.4 is the first installable build of the v0.11.x line.

## [0.11.3] - 2026-04-28

A patch release that turns alef.toml's `version` field into a real lifecycle pin: writes are stamped on every successful generate, and a config pointing to a future alef now refuses to run instead of silently producing stale output.

### Added

- **`alef generate` and `alef all` enforce alef.toml ↔ CLI version compatibility.** Before doing any work, both commands parse the top-level `version = "X.Y.Z"` field (if set) and compare it semver-style against the running CLI. If the pin is greater than the CLI, the command aborts with `alef.toml pins version = "..." but installed alef CLI is X.Y.Z. Upgrade alef ...`. This catches the case where a downstream repo bumps `alef.toml` and tries to regenerate against an older binary still on disk — the regenerate would otherwise quietly skip new emitters and corrupt the output.
- **`alef generate` and `alef all` stamp alef.toml with the CLI version after a successful run.** The top-level `version = "..."` line is rewritten (or inserted if missing) to match `env!("CARGO_PKG_VERSION")`. Downstream consumers (install-alef, CI verify) now have an authoritative record of which alef produced the on-disk artifacts, so a mismatch between alef.toml and the headers in generated files becomes impossible. The rewrite is line-anchored, so dependency `version = "..."` specs inside inline tables are never touched.

## [0.11.2] - 2026-04-28

A patch release fixing the alef self-publish bootstrap and rolling up several major dependency upgrades.

### Fixed

- **`alef`'s own publish workflow no longer races against itself.** The `prepare`, `validate-versions`, and `check-registry` jobs use the alef-backed composite actions, which previously defaulted `alef-version: latest` — install-alef would resolve that to the just-bumped `alef.toml` pin (the very version being published) and try to download a binary that didn't exist yet (it's built later in the same run). All four alef-side action calls now pass `alef-version: main` so install-alef builds from source via `cargo install --git --locked`. Costs ~2-3 min per job but breaks the chicken-and-egg loop.

### Changed

- **`alef check-registry` ported to ureq v3.** The previous v2 API (`AgentBuilder`, `RequestBuilder::set`, `Error::Status(404, _)`, `Response::into_string`) was rewritten to the v3 equivalents (`Agent::config_builder().new_agent()`, `RequestBuilder::header`, `Error::StatusCode(404)`, `Response::into_body().read_to_string()`).

### Bumped (template_versions)

- `JUNIT` 5.14.4 → 6.0.3 (Maven scaffold templates)
- `MICROSOFT_NET_TEST_SDK` 17.14.1 → 18.4.0
- `XUNIT_RUNNER_VISUALSTUDIO` 2.8.2 → 3.1.5
- `PRE_COMMIT_HOOKS_REV` v0.9.5 → v6.0.0
- `TYPOS_REV` v0.7.10 → v1.45.2
- Plus auto-merged Renovate PRs landed since v0.11.1: `criterion` 0.7 → 0.8, `extendr-api` 0.8 → 0.9, `microsoft.net.test.sdk` 17.12.0 → 17.14.1, JVM tooling pins, pre-commit hook revisions.

## [0.11.1] - 2026-04-28

A patch release fixing path discovery and version-extraction edge cases in the new `alef validate versions` subcommand surfaced when running it against kreuzberg's repo layout.

### Fixed

- **`alef validate versions` now skips manifests that don't exist** rather than treating absence as a mismatch. Previously, every repo was flagged as having "missing" Ruby/Go/PHP manifests if it didn't follow alef's default lib-based layout. The check is opt-in per file: only manifests that physically exist are validated.
- **Ruby version files are discovered via globs** matching the same patterns alef's `sync-versions` writes to: `packages/ruby/lib/*/version.rb`, `packages/ruby/ext/*/src/*/version.rb`, `packages/ruby/ext/*/native/src/*/version.rb`. Repos that ship the Ruby gem with rb-sys-style ext layout no longer produce false negatives.
- **PHP composer.json is only validated when a `version` field is actually declared**. Composer relies on Git tags for versioning, and most polyglot manifests omit it; the validator no longer flags missing-by-design as a mismatch.
- **`mix.exs` reader now accepts both `@version "X.Y.Z"` (module-attribute form) and `version: "X.Y.Z"` (keyword form inside `def project do`).** The previous reader only matched the `@version` constant style.
- **Root `package.json` and `crates/{name}-{wasm,node}/package.json` are now part of the validation set.** Repos that ship a top-level npm package or per-crate package.json no longer silently bypass the check.

## [0.11.0] - 2026-04-28

A minor release that absorbs a large slice of polyglot publish-pipeline machinery into alef so consumers (kreuzberg, html-to-markdown, liter-llm, …) can stop duplicating it across `kreuzberg-dev/actions` shims and per-repo `scripts/publish/` shell. `alef-publish` now owns end-to-end packaging for seven languages it previously didn't, and the `alef` binary gains four new top-level subcommands that consolidate cross-manifest validation, release-event metadata extraction, multi-registry version checks, and Go submodule tagging.

### Added

- **`alef-publish` end-to-end packagers for Python, Wasm, Node, Ruby, Elixir, Java, and C#.** Each adds a `package_<lang>` function under `crates/alef-publish/src/package/` and is dispatched from `alef publish package --lang <lang> --target <triple>`. Python invokes maturin for wheels and sdist; Wasm runs `wasm-pack` then `npm pack`; Node runs `napi build` and emits per-platform npm sub-packages following the `napi.triples.additional` convention (with the platform list configurable via the new `[publish.languages.node].npm_subpackage_platforms`); Ruby compiles via rb-sys/`bundle exec rake compile` and assembles a platform-tagged `.gem`; Elixir produces RustlerPrecompiled-style NIF tarballs (one per `nif_versions` × target combination) plus a `checksum-Elixir.{App}.exs` aggregator; Java stages the JNI shared library under `native/{classifier}/` and runs Maven to produce a classifier-suffixed JAR; C# stages `runtimes/{rid}/native/` and runs `dotnet pack`.

- **`alef validate versions`** — cross-manifest version consistency checker. Reads `alef.toml` to discover each language's manifest location (Cargo.toml / pyproject.toml / package.json / gemspec / mix.exs / pom.xml / .csproj / composer.json / gleam.toml / build.zig / Package.swift / pubspec.yaml / build.gradle.kts / DESCRIPTION) and compares declared versions to the canonical Cargo.toml workspace version. Exits non-zero with a clear mismatch list; emits JSON with `--json`. Replaces the Python-based `kreuzberg-dev/actions/validate-versions` action and the per-repo shell scripts that did the same job.

- **`alef release-metadata`** — emits release metadata JSON in the exact shape consumed by GitHub Actions matrix dispatch (`tag`, `version`, `npm_tag`, `is_tag`, `is_prerelease`, `release_<target>` flags, …). Args: `--tag`, `--targets`, `--git-ref`, `--event`, `--dry-run`, `--force-republish`, `--json`. The set of valid targets is discovered from `alef.toml`'s `languages` array. Replaces `kreuzberg-dev/actions/prepare-release-metadata`.

- **`alef check-registry`** — version-existence check across PyPI, npm, crates.io, RubyGems, Hex, Maven Central, NuGet, Packagist, a Homebrew tap, and GitHub Releases. Output: `exists=true|false` (and JSON with `--json`). Replaces `kreuzberg-dev/actions/check-registry` plus the per-registry `scripts/publish/check_*.sh` scripts.

- **`alef go-tag`** — creates and pushes the two Git tags Go's submodule-versioning convention requires (the top-level `vX.Y.Z` and the `packages/go/v<major>/vX.Y.Z` submodule tag). Supports `--dry-run` and `--remote`.

- **`PublishLanguageConfig` schema additions** (all `Option<...>`, fully back-compatible): `npm_subpackage_platforms`, `cibuildwheel_environment`, `jni_classifier`, `csharp_rid`, `wheel`, `sdist`. Existing `[publish.languages.*]` tables continue to load unchanged.

### Fixed

- **Scaffolded GitHub workflows for Swift, Dart, Gleam, Kotlin, and Zig were broken on first run.** The Swift workflow referenced `actions/setup-swift@v4`, which doesn't exist (the working action is `swift-actions/setup-swift@v2`). The Dart, Gleam, Kotlin, and Zig workflows passed unsupported flags to the per-language CLIs (`--working-dir`, `--project-path`, `--directory`) instead of `cd`-ing into the package directory. All five workflow templates now use `defaults.run.working-directory: packages/<lang>` and let each CLI run from its own root, and Swift uses `swift-actions/setup-swift@v2`.

## [0.10.4] - 2026-04-27

A patch release fixing three orchestration / e2e-codegen bugs that surfaced during a clean `alef all --clean` regenerate of every downstream polyglot repo: standalone e2e Rust crates were getting absorbed into parent workspaces, the orchestrated `alef all` skipped the language-default formatter pass, and the Rust visitor codegen was producing unparsable trait-impl blocks.

### Fixed

- **e2e Rust crate's `Cargo.toml` was silently inherited by any parent workspace**, so running `cargo fmt`/`cargo build` from inside `e2e/rust/` failed with `current package believes it's in a workspace when it's not` whenever the consuming repo had a top-level workspace `Cargo.toml` that didn't explicitly list `e2e/rust` in `members` or `exclude`. The default formatter `(cd {dir} && cargo fmt --all)` therefore exited 1 and left `e2e/rust/tests/*.rs` unformatted, breaking the next `alef verify`. The generated `e2e/rust/Cargo.toml` now starts with an empty `[workspace]` table so the e2e crate is its own workspace root and is unaffected by any parent — consumers no longer have to remember to add `e2e/rust` to `workspace.exclude`.

- **`alef all` skipped `format_generated`**, leaving every language-default formatter (mix format, ruff format, biome format, php-cs-fixer, etc.) unrun on the freshly emitted bindings. Prek's check-mode formatting hooks (`mix format --check-formatted`, etc.) therefore failed against `packages/elixir/lib/<lib>/native.ex` and similar files in every repo that ran the orchestrated `alef all` instead of `alef generate`. `Commands::All` now invokes `pipeline::format_generated` before `fmt_post_generate`, mirroring the order in `alef generate`.

- **e2e Rust visitor codegen emitted untyped trait method parameters** (`fn visit_custom_element(&self, ctx, tag_name, html) -> VisitResult`), producing files that could not parse. Three coupled fixes:
  - Each visitor parameter is now bound to a `_` pattern with the explicit `&NodeContext` / `&str` / `bool` / `u8` / `u32` / `Option<&str>` / `&[String]` type from the `HtmlVisitor` trait, so the body needn't introduce unused bindings.
  - The receiver is now `&mut self` to match the trait, not `&self`.
  - `CallbackAction::Custom` was missing the surrounding string-literal quotes — `VisitResult::Custom([CUSTOM WIDGET].to_string())` was the literal output for `output: "[CUSTOM WIDGET]"`. The codegen now wraps the escaped value in `"…"` before calling `.to_string()`, so the emitted expression is a well-formed `&str` literal.
  - The test file now imports `HtmlVisitor`, `NodeContext`, and `VisitResult` whenever any fixture in the file declares a `visitor` block.

## [0.10.3] - 2026-04-27

A patch release fixing `alef e2e generate` so it only emits test projects for languages the consumer has actually scaffolded, plus two `alef verify`-vs-`prek` drift fixes uncovered by a clean v0.10.2 regenerate of the downstream polyglot repos.

### Fixed

- **`alef e2e generate` emitted test projects for every supported backend** when neither `--lang` nor `[e2e].languages` was set, including languages with no scaffolded binding (e.g. `gleam`, `kotlin`, `dart`, `swift`, `zig`, `brew`). The resulting `e2e/<lang>/` directories couldn't compile because the package they reference doesn't exist. The default now mirrors `alef generate` / `alef scaffold`: derive the e2e language list from the top-level `[languages]` array, mapping `Language::Ffi` → the `c` e2e harness and always including `rust` for the source-crate suite. Generators without a matching `Language` variant (`brew`) require explicit opt-in via `[e2e].languages`.

  Migration: after upgrading, run `alef e2e generate` once, then manually delete any stale `e2e/<lang>/` directories for languages you never scaffolded — the cleanup pass only revisits dirs the current run touched, so untouched stale dirs from prior runs are not auto-removed.

- **e2e Rust default formatter command was invalid**: v0.10.2 introduced a built-in default of `cargo fmt --manifest-path {dir}/Cargo.toml`, but `--manifest-path` is not a stable global flag for `cargo fmt` (cargo prints `Specify message-format: short|json|human / --all / --check` and exits 1). The default is now `(cd {dir} && cargo fmt --all)`, which formats the standalone e2e crate from inside its own directory and works regardless of whether the crate is a workspace member. Without this fix, `e2e/rust/tests/*.rs` were left unformatted by `alef e2e generate`, and prek's cargo-fmt hook then rewrote them post-finalisation, breaking `alef verify`.

- **`normalize_content` skipped trailing-whitespace stripping for `.rs` files** when rustfmt could not parse them — for example, cextendr's `packages/r/src/rust/src/lib.rs` uses the non-standard `name: T = "default"` parameter-default syntax, which rustfmt rejects, so `format_rust_content` falls back to the raw codegen output. The raw output contains trailing whitespace on blank lines (e.g. `    \n` between `#[must_use]` and the next `pub fn`), which prek's `trailing-whitespace` hook then strips, breaking `alef verify`. `normalize_content` now always pipes through `normalize_whitespace` after rustfmt, so the embedded `alef:hash` always reflects whitespace-clean content regardless of whether rustfmt parsed the file.

### Added

- **`alef_e2e::default_e2e_languages(&[Language])`** — the public helper that maps the scaffolded language list to e2e generator names. Exposed so consumers and downstream tooling can resolve the same default the CLI uses.

## [0.10.2] - 2026-04-27

A patch release fixing four codegen and pipeline bugs surfaced by a clean regenerate of the five downstream polyglot repos against v0.10.1.

### Fixed

- **Swift binding `Cargo.toml` was missing `serde_json`** even though the generated `lib.rs` emitted `::serde_json::to_value(...)` / `::serde_json::from_value(...)` calls for `Codable` propagation, breaking compilation with ~1k+ `E0433: cannot find serde_json in the crate root` errors. The Swift backend now always includes `serde_json = "1"` in the binding crate's `[dependencies]`.
- **Dart binding `Cargo.toml` listed an unused `anyhow = "1"`** — the Dart trait-bridge codegen returns `source_crate::Result<T>` directly and never imports anyhow, so `cargo machete` rejected the crate. The Dart backend's `extra_deps` no longer emits anyhow.
- **Go e2e codegen emitted fixture strings containing NUL bytes inside raw string literals** (`` `…\0…` ``), which `gofmt` rejects with `illegal character NUL`. Affected ~50 generated test files in repos with NUL-bearing fixture data. The `go_string_literal` helper now switches to interpreted (double-quoted) form whenever a string contains characters Go raw strings cannot represent (NUL, `\r`, or backtick), and emits `\xNN` hex escapes for any ASCII control byte.
- **e2e generated files drifted from their embedded `alef:hash` after `prek run --all-files`** because `alef e2e generate` skipped trailing-newline / trailing-whitespace normalization and didn't run a Rust or Python formatter on standalone e2e manifests by default. Three coupled fixes:
  - `pipeline::write_scaffold_files_with_overwrite` now runs every emitted file through `normalize_content` (ensure exactly one trailing newline, strip trailing whitespace per line) before hash finalization, matching what `prek`'s `end-of-file-fixer` and `trailing-whitespace` hooks would have done.
  - `alef-e2e::format::run_formatters` now falls back to a built-in default formatter set (`cargo fmt` inside `e2e/rust/`, `ruff format` on `e2e/python/`) when `[e2e].format` is unconfigured, instead of silently no-oping.
  - `pipeline::sync_versions` now invokes `finalize_hashes` on every version-synced file (e.g. `version.rb`) so the embedded `alef:hash` line stays consistent with the rewritten content.



A patch release reworking `alef verify` to be **idempotent across alef versions** and bundling six small generator/scaffold fixes that landed since v0.10.0. The verify hash no longer encodes the alef CLI version or `alef.toml`; it is now a per-file fingerprint derived purely from the rust sources and the on-disk file content, so a green `alef verify` stays green after upgrading the alef CLI as long as nothing else changed.

### Changed

- **`alef verify` is now per-file source+output deterministic**. The `alef:hash:<hex>` line embedded in every generated file is computed as `blake3(sources_hash || file_content_without_hash_line)`, where `sources_hash = blake3(sorted(rust_source_files))` — no alef version dimension, no `alef.toml` dimension. `alef generate` finalises the hash *after* every formatter has run (so the embedded hash describes the actual on-disk byte content), and `alef verify` is now a pure read+strip+rehash+compare with no regeneration and no writes. Previously the hash incorporated the alef CLI version, which forced every consumer repo to re-run `alef generate` after every alef bump even when nothing else had changed.
- **`alef_core::hash::compute_generation_hash` is removed**; use `compute_sources_hash` + `compute_file_hash` instead. The IR cache (`.alef/ir.json`) now keys on `compute_sources_hash` alone — pass `--clean` to bust the cache when the alef extractor itself has changed.
- **`pipeline::write_files` / `write_scaffold_files` no longer take a `generation_hash` argument**; the hash is finalised separately by the new `pipeline::finalize_hashes(paths, sources_hash)` after formatters run.

  Migration: after upgrading, every existing alef-generated file still carries the old input-deterministic hash. Run `alef generate` once (then `alef e2e generate` if the repo uses `[e2e]`) to refresh embedded hashes to the new per-file scheme.

### Fixed

- **`alef scaffold` emitted Dart and Swift `Cargo.toml` files without a `license` field and pulled in an unused `serde_json` dep** — both languages now emit the same `license = "..."` line as the other backends, and the spurious `serde_json = "1"` is gone.
- **Go e2e fixtures used raw `to_pascal_case` for method names** instead of routing through `to_go_name`, so generated test code referenced `result.Html` while the binding declared `result.HTML` (golangci-lint rejected the package). All Go method emission now goes through `alef_codegen::naming::to_go_name`.
- **Dart codegen emitted reserved keywords / numeric idents / unescaped `$` and `\`** in field names, parameter names, and string literals — Dart now escapes reserved keywords (`async`, `class`, …), prefixes leading-digit identifiers with `_`, and escapes `$` / `\` / `"` inside generated string literals.
- **`alef readme` panicked when a snippet section was missing from the language template** — missing snippet keys default to an empty Tera object instead of raising.
- **`fmt_post_generate` did not run the configured formatters for WASM, C#, and Java** because the lint/format dispatch was hardcoded to a static set of languages — the dispatch is now driven entirely by `LintConfig::format` so any backend can opt in.
- **NAPI `.d.ts` emission placed optional parameters before required ones**, which TypeScript rejects with `TS1016`. NAPI generators now reorder optional parameters last in the emitted `.d.ts` signatures.

## [0.10.0] - 2026-04-27

A major release expanding alef's target-language coverage from 11 to 16 backends. Adds full code-generation support for **Kotlin** (JNA), **Swift** (swift-bridge), **Dart** (flutter_rust_bridge), **Gleam** (Rustler-on-BEAM), and **Zig** (C ABI), each with scaffold, lint, format, build, test, setup, update, and clean defaults wired through `alef-core`. The five new backends share the existing IR, trait-bridge round-trip API, and `alef-docs` language-native doc emission paths — there is no per-language fork of the codegen pipeline. Also adds language-native doc comments (PHPDoc, C# XML, `@doc`, roxygen2) to the existing PHP/C#/Elixir/R backends, and fills in numerous correctness gaps surfaced by the kreuzberg verification worktree.

### Added

- **Five new language backends** — `alef-backend-kotlin`, `alef-backend-swift`, `alef-backend-dart`, `alef-backend-gleam`, `alef-backend-zig`. Each implements the `Backend` trait, ships per-crate snapshot tests via `insta`, and is wired into `alef-cli`'s dispatch table. Trait bridges are supported across all five (Dart abstract classes, Gleam per-method response shims, Zig comptime `make_*_vtable` helpers, Kotlin/Swift host-side typealiases + Codable propagation).
- **Language-native scaffolds for the new backends** — `alef-scaffold` writes Kotlin (Gradle KTS + Detekt + ktlint), Swift (Swift Package Manager + swift-format + SwiftLint), Dart (pub + flutter_rust_bridge + lints), Gleam (`gleam.toml` + gleeunit), and Zig (`build.zig` + `build.zig.zon`) toolchain configs, README stubs, and CI templates. `alef-publish` gains per-language packaging and validation modules for the same five.
- **`alef-docs` language emitters for PHP/C#/Elixir/R/Kotlin/Swift/Dart/Gleam/Zig** — PHPDoc, XML `<summary>`/`<param>`/`<returns>`, Elixir `@doc` heredocs, roxygen2 `@param`/`@return`, KDoc, Swift markdown, Dart `///`, Gleam `///`, and Zig `///` are now emitted for all public functions/methods. Shared `doc_emission` module handles language-specific escaping (e.g. `*/` in PHP, XML entities in C#, triple-quote escapes in Elixir).
- **`has_serde` IR flag on `EnumDef`** — extracted from `derive(Serialize)` + `derive(Deserialize)` so backends can decide whether to JSON-roundtrip or pattern-match conversion. Used by Swift to emit `Codable` and propagate it to non-derived enum references whose containing struct is `Codable`.
- **`return_sanitized` IR flag on `FunctionDef`** — laid down by `alef-extract` when the return type was sanitized by the unknown-type pass. Backends can use it to decide whether a roundtrip is recoverable for the return value (currently consumed by snapshot/doc tests; full backend wiring is a follow-up).
- **`alef-readme` v0.10.0 README templates** for the new languages.

### Changed

- **Backend count: 11 → 16** in README, skill docs, and the supported-languages table. Kotlin/Swift/Dart/Gleam/Zig are no longer flagged "in progress".
- **Trait bridges span all 16 backends** — round-trip callbacks (host language implements a trait, Rust core invokes it through the bridge) work end-to-end for every supported language.
- **`alef-core::config` defaults** for `LintConfig`, `BuildConfig`, `TestConfig`, `SetupConfig`, `UpdateConfig`, and `CleanConfig` now cover Kotlin/Swift/Dart/Gleam/Zig with per-language preconditions and command sequences. The Phase-1 skip clauses in the per-language test loops are gone.
- **Centralized version pins** — `alef-core::template_versions` gains `pub_dev`, `toolchain`, and Kotlin/Swift entries in `cargo`/`maven` modules. Toolchain pins (Zig 0.16, Dart SDK `>=3.0 <4.0`, JVM 21, swift-bridge `0.1.59`, flutter_rust_bridge `2.12.0`, Kotlin 2.1.10, kotlinx-coroutines 1.9.0, JNA 5.14.0) are now single-source.

### Fixed

- **Swift typealias closure for #59** — non-trait Kreuzberg types are emitted as `typealias Kreuzberg.X = RustBridge.X`, host-language wrapper functions are removed, and consumers call `RustBridge` functions directly. `RustString`/`RustVec` returns and `Data → [UInt8]` arguments are converted at the wrapper boundary. Conversion initializers (`init(rustVal: RustBridge.X)`) are emitted for non-`Codable` enums; `Codable` enums skip them and use JSON. `swift_name(camelCase)` is emitted on every bridge fn so swift-bridge generates idiomatic Swift symbols. Reduces `kreuzberg-swift` build errors from ~190 to 0.
- **Swift unbridgeable surfaces** — getters/wrappers for fields that can't survive the swift-bridge round-trip (excluded fields, JSON containers with non-serde inners, `Vec<non-Primitive>` in non-serde structs) now skip emission entirely with a `// alef: skipped` marker instead of emitting `unimplemented!()`.
- **Swift constructor / lints / keyword escaping** — constructor is omitted (instead of `unimplemented!()`) for structs that need `Default` but don't derive it. Generated `lib.rs` carries an `#![allow]` block for the swift-bridge codegen pattern's clippy artifacts. Reserved Swift keywords used as field/param names are escaped at the extern block, wrapper, and constructor sites. A phantom `Vec<FooBox>` accessor is forced per trait so swift-bridge emits the C symbols its Vec<T> accessors call.
- **Dart trait abstract classes + unbridgeable surfaces** — Dart-side `abstract class` is emitted per trait for the round-trip API. Functions with unbridgeable params (e.g. `Vec<(Vec<u8>, …)>`, lossy through JSON) are skipped instead of emitting panicking shims. Tuple-variant fields are renamed `_N` → `fieldN` to survive flutter_rust_bridge's underscore stripping. Generator-only clippy lints (`map_identity`, `let_and_return`, `collapsible_match`, `manual_flatten`, `too_many_arguments`, `unit_arg`, `type_complexity`) are silenced via `#![allow]` in the emitted `lib.rs`.
- **Gleam per-method response shims** — each trait method gets a typed `{trait}_{method}_response(call_id, Result(T, E))` shim so callback modules can reply through the Rustler reply-registry without raw JSON.
- **Zig vtable codegen** — comptime `make_{trait}_vtable(comptime T, *T)` helper generated per trait bridge; thunks reconstruct byte slices, fallible methods return `i32 + out_error`, lifecycle stubs (`name_fn`, `version_fn`, `initialize_fn`, `shutdown_fn`) emitted when `super_trait` is configured. Trait-bridge thunks now discard unused `out_result` params, avoid shadowing method params named `result`, and only emit `return 0;` when the success path actually flows through.
- **WASM `useless_conversion`** — generated `lib.rs` adds the lint to its crate-wide allow list, since the `kreuzberg::T::from(t_core).into()` identity conversion is only an artifact of code generation.
- **Rustler doc emission** — NIF Rust source emits standard `///` Rustdoc instead of literal Elixir `@doc """…"""` heredocs (which produced 380 parse errors when run through `cargo build`). Reserved Elixir module-attribute names (`@behaviour`, `@callback`, …) are no longer used as GenServer attributes. The lossy `Vec<String>` → `Vec<&str>` conversion is dropped — `&Vec<T>` already derefs to `&[T]`, so the simpler emission preserves caller signatures like `&[String]`.
- **Swift scaffold indent** — Swift test stubs use 2-space indent to match swift-format defaults.

## [0.9.2] - 2026-04-27

A patch fixing three generator bugs that surfaced when alef v0.9.1 was used to regenerate downstream consumer repos. Each one blocked a different language's pre-commit hook from passing on freshly regenerated bindings; with these fixes a full `alef generate` + `alef e2e generate` produces output that passes mix credo (Elixir), golangci-lint (Go), and `dotnet format` / `dotnet build` (C#) without further hand-editing.

### Fixed

- **Elixir trait-bridge module names emitted broken PascalCase for hyphenated crate names**: `alef-scaffold`'s Elixir generator used `capitalize_first(&app_name)`, which only uppercased the first character and left underscores in place. For a crate named `html-to-markdown` the generated Rustler trait bridge declared `defmodule Html_to_markdownHtmlVisitorBridge do`, which `mix credo` correctly rejects ("Module names should be written in PascalCase."). All three call sites (mix.exs `defmodule`, the bridge `module_name`, and `native_mod`) now use `heck::ToPascalCase`, splitting on both `-` and `_`. `html-to-markdown` → `HtmlToMarkdown`, `tree-sitter-language-pack` → `TreeSitterLanguagePack`.
- **Go binding and Go e2e tests disagreed on initialism casing**: `alef-backend-go` already routes Go field/parameter names through an idiomatic-initialism transformer (`HTML`, `URL`, `ID`, `JSON`, …) but `alef-e2e`'s Go fixture renderer used a plain `to_pascal_case` helper, so generated test code referenced `result.Html` and `result.Url` while the binding declared `HTML` and `URL`. golangci-lint rejected the generated package with "result.Html undefined". Both crates now route through `alef_codegen::naming::to_go_name` and `go_param_name`; the duplicate `GO_INITIALISMS` table in `alef-e2e/src/codegen/go.rs` was deleted.
- **C# `.csproj` scaffold was placed inside the source subdirectory instead of the package root**: `alef-scaffold` wrote the project file at `packages/csharp/{Namespace}/{Namespace}.csproj`, but consumer Taskfile targets and the prek `csharp-format` / `csharp-lint` hooks expected it at `packages/csharp/{Namespace}.csproj` (alongside `Directory.Build.props`, matching the convention used for the gemspec at `packages/ruby/*.gemspec`). The path is now `packages/csharp/{Namespace}.csproj`, with `<ItemGroup>` `Include` paths adjusted (one level up the relative path) so `LICENSE` and `runtimes/**` still resolve correctly. The scaffold remains create-once.
- **C# async docs/signature emitted invalid `Task<void>`**: `alef-docs::render_csharp_fn_sig` formatted async return types as `Task<{ret}>` unconditionally, so for void-returning async functions the documented signature read `public static async Task<void> ...`, which doesn't compile under `<TreatWarningsAsErrors>true</TreatWarningsAsErrors>`. It now emits `Task` for void returns and `Task<{ret}>` otherwise.

## [0.9.1] - 2026-04-27

A patch release fixing the publish pipeline. v0.9.0 (and prior releases since `alef-publish` was added) failed to publish `alef-cli` to crates.io because `alef-publish` was missing from the publish workflow's crate list — `alef-cli` declares `alef-publish = "^X.Y.Z"` and crates.io still served `alef-publish 0.7.2`, so the upload of `alef-cli` errored with `failed to select a version for the requirement alef-publish = "^0.9.0"`. The 19 other workspace crates published cleanly each time; only `alef-cli` (the binary used by `cargo install` / `cargo binstall`) was affected. v0.9.0 was rescued by publishing `alef-publish` and `alef-cli` manually with `cargo publish`; v0.9.1 makes the workflow self-sufficient.

### Fixed

- **`alef-publish` was missing from the Publish workflow's crate list**: `.github/workflows/publish.yaml` listed 20 crates but not `alef-publish`, so the workflow never uploaded it to crates.io. `alef-cli`'s dependency on `alef-publish ^X.Y.Z` therefore failed to resolve at publish time. The crate is now listed in dependency order (after `alef-scaffold`, before the backends), matching the rest of the workspace.

## [0.9.0] - 2026-04-27

A major fix release eliminating ~40 generated "Not implemented" stubs across the PHP, Ruby, C FFI, and R backends. Every batch extraction API (`batch_extract_file_sync`, `batch_extract_bytes_sync`, plus async variants), `extract_file`/`extract_file_sync`, and most of the Ruby gem's surface previously failed at runtime with error code 99. Five distinct generator bugs collapsed into a single class of bad output. Also makes `alef verify` input-deterministic so downstream formatters can reformat generated content freely without breaking verify, and exposes the canonical input-hash recipe via `alef_core::hash::compute_generation_hash`.

### Changed

- **`alef verify` is now input-deterministic**. The embedded `alef:hash:<hex>` line in every generated file is no longer a per-file hash of the normalised generated content; it is a single fingerprint of the **inputs** that produced the run:
  - `blake3(sorted(rust_source_files) + alef.toml + alef_version)`

  `alef generate` computes this hash once and writes the same value into every alef-headered file. `alef verify` recomputes the same input hash and compares it to the disk hash without inspecting any file body. As a result, downstream formatters (rustfmt, rubocop, dotnet format, spotless, biome, mix format, php-cs-fixer, taplo, ruff, …) can reformat alef-generated content freely without breaking verify — only changes to the generation inputs invalidate the embedded hash. The previous output-deterministic semantics caused `alef verify` to flag bindings as stale on every commit in repos with active language formatters.

  Migration: after upgrading, every existing alef-generated file still carries the old per-file hash. Run `alef generate` once (then `alef e2e generate` if the repo uses `[e2e]`) to refresh all embedded hashes — a single regenerate pass writes the new uniform input hash everywhere.

- **`alef verify` ignores `--lang`, `--compile`, `--lint`**. The flags are still accepted for backwards compatibility but no longer affect the check, since verify no longer regenerates per-language. Use `alef build`, `alef lint`, `alef test` for those concerns.

### Added

- **`alef_core::hash::compute_generation_hash(sources, config_path, alef_version)`** — public function exposing the canonical input-hash recipe so other consumers (cache invalidation, custom build tools) can reuse it. `alef-cli`'s `cache::generation_hash` is now a thin wrapper passing `env!("CARGO_PKG_VERSION")` for the version dimension.

### Fixed

- **Bare `Path` resolved to `Named("Path")` and silently sanitized to `String`**: `alef-extract`'s `resolve_path_type` only recognized `PathBuf` (and `&Path` via the reference path). Functions taking `path: impl AsRef<Path>` resolved the inner generic arg `Path` to `Named("Path")`, then `sanitize_unknown_types` (which considers stdlib `Path` an unknown user type) replaced it with `TypeRef::String` and marked the param as sanitized. The result was that every binding emitted "Not implemented" stubs for `extract_file`, `extract_file_sync`, and any other API using `impl AsRef<Path>`. Bare `Path` now maps to `TypeRef::Path` like `PathBuf`.
- **PHP backend stubbed sanitized functions even when the params were JSON-roundtripable**: `alef-backend-php`'s `gen_function_body` short-circuited to `gen_stub_return` whenever `func.sanitized` was true, ignoring the existing serde-roundtrip helper. `gen_php_serde_let_bindings`, `gen_php_named_let_bindings`, and `gen_php_call_args_with_let_bindings` now handle `Vec<String>` params that came from `Vec<tuple>` (each element is a JSON-encoded tuple), and the function/method body generators fall through to the serde path when a recovery is possible. Affects all batch APIs (`batch_extract_*`).
- **Magnus (Ruby) backend stubbed every extraction function**: the shared `can_auto_delegate_function` rejected functions with a non-opaque Named ref param (e.g. `&ExtractionConfig`), so `extract_bytes`, `extract_bytes_sync`, and 19 other functions emitted "Not implemented" stubs even though the existing deser-preamble could handle them. `alef-backend-magnus` now adds a magnus-specific `magnus_serde_recoverable` gate that allows JSON-roundtrip on Named non-opaque params and on sanitized `Vec<String>` (originally `Vec<tuple>`), and uses `gen_call_args_with_let_bindings` so the call site borrows `&{name}_core` correctly.
- **C FFI backend stubbed all functions with sanitized params**: `alef-backend-ffi` set `will_be_unimplemented = func.sanitized` and bailed out before parameter conversion, even though the existing `Vec` JSON deserialization path could handle `Vec<String>` params with type inference at the call site. The FFI now exempts sanitized functions from the unimplemented bail-out when every sanitized param is a recoverable `Vec<String>` (with `original_type` set), routing them through the standard JSON-roundtrip Vec conversion. Affects `kreuzberg_batch_extract_*` and other tuple-batch FFI exports consumed by the Go, Java, and C# bindings.
- **R (extendr) backend panicked with `todo!("async not supported by backend")` for every async function**: extendr was configured with `AsyncPattern::None`. R is single-threaded but async functions are still callable from R via a per-call tokio runtime. The backend now uses `AsyncPattern::TokioBlockOn` and reports `supports_async: true`, generating `tokio::runtime::Runtime::new()?.block_on(...)` wrappers like the other sync-on-async backends.

### Known limitations

- Functions whose return type is a Named struct from outside the API surface (e.g. `get_preset` returning `&'static EmbeddingPreset`) are still stubbed: alef has no way to know whether the core type derives `Serialize`, so it can't safely JSON-roundtrip the value back to the binding side. The new `return_sanitized` IR field is laid down for a future fix.

## [0.8.7] - 2026-04-27

A patch fixing a CI-hang in the C# `alef setup` default and replacing the `// TODO: add usage example` placeholder in alef-generated READMEs with a pointer to the main repository docs.

### Fixed

- **`alef setup --lang csharp` walks the entire repo when no .csproj is at the top level**: `dotnet restore packages/csharp` searches for project files recursively under the given directory, including `target/`, `node_modules/`, and other artifact dirs. On CI this took longer than the 600s timeout. The C# setup default now applies the same precondition + `find` strategy as the upgrade default — both `dotnet` and a discoverable `.sln`/`.csproj` (depth 3) must exist, otherwise setup is skipped, and the resolved project path is passed explicitly to `dotnet restore`.
- **README usage examples were `// TODO: add usage example` placeholders**: the alef-generated per-language READMEs in the FFI/WASM crate dirs and `packages/{lang}/` for hardcoded fallbacks now redirect readers to the main repository's documentation instead of emitting a literal TODO.

## [0.8.6] - 2026-04-27

A patch hardening the C# upgrade default introduced in v0.8.5. The original `$(ls ... || find ... || echo …)` chain didn't actually skip when no project file existed — `ls foo/*.csproj 2>/dev/null` always exits 0 with empty stdout, so the `||` fallbacks never triggered and `dotnet outdated` ran with no path argument and errored out at the repo root.

### Fixed

- **C# upgrade falls through and fails when no `.csproj` exists**: alef-core's `update_defaults.rs` C# precondition now requires BOTH `dotnet` AND a discoverable `.sln`/`.csproj` under the output dir (depth 3). When no project exists, the upgrade is skipped (precondition warning) instead of erroring out. The command itself is simplified to just `find … | head -1`, which the precondition has already validated returns a path.

## [0.8.5] - 2026-04-27

A patch fixing two `alef update --latest` (`task upgrade`) failures observed across all four consumer repos.

### Fixed

- **Java `versions:use-latest-releases` aborted on missing rules file**: `alef-core/src/config/update_defaults.rs` unconditionally appended `-Dmaven.version.rules=file://${PWD}/{output_dir}/versions-rules.xml`, but most consumers don't ship that file. Maven aborted with `ResourceDoesNotExistException`, failing the whole upgrade pipeline. The flag is now wrapped in `$([ -f .../versions-rules.xml ] && echo "...")` so it's only appended when the file exists.
- **C# `dotnet outdated packages/csharp` rejected the directory**: `dotnet outdated` requires a `.sln` or `.csproj` path, or a directory that contains one at the top level. Most consumers nest projects under `packages/csharp/<ProjectName>/`, so the call failed with "The directory 'packages/csharp' does not contain any solutions or projects." The default now resolves to the first `.sln`/`.csproj` found under the output dir (depth 3) before invoking `dotnet outdated`.

## [0.8.4] - 2026-04-27

A follow-up to v0.8.3 fixing two cases where downstream tooling reformatted alef-generated `crates/{lib}-wasm/Cargo.toml` and silently invalidated the alef hash header.

### Fixed

- **WASM `Cargo.toml` cargo-sort canonical layout**: `alef-backend-wasm`'s `gen_cargo_toml` previously emitted `[lib]`/`[dependencies]` *before* the `[package.metadata.*]` blocks and listed dependencies in declaration order. cargo-sort, run as a pre-commit hook in every consumer, then rewrote the file in canonical alphabetical / TOML-section order — invalidating the embedded `alef:hash:` line, so the next `alef verify` reported the wasm Cargo.toml as stale on every commit. The template now emits sections and dependencies in cargo-sort canonical order (`[package]` → `[package.metadata.*]` → `[lib]` → `[dependencies]` sorted alphabetically including the dynamic core crate dep), so cargo-sort is a no-op.
- **WASM cargo-machete unused-dep flag for `serde_json`**: the wasm `Cargo.toml` always declared `serde_json = "1"` because trait-bridge / function generators may use it, but consumers without those code paths would fail `cargo-machete` on every commit. Added `serde_json` to the existing `[package.metadata.cargo-machete] ignored` list alongside the already-ignored `futures-util` and `wasm-bindgen-futures`.

## [0.8.3] - 2026-04-26

A follow-up to v0.8.2 making `alef verify` formatter-agnostic and fixing the Ruby version-sync footgun. Verify is now a pure hash comparison: alef computes the canonical hash at generation time, embeds it in the file header, and on `alef verify` only compares the embedded hash against a freshly-computed hash of the canonicalised generated content. External formatters (php-cs-fixer, rubocop, ruff, biome, …) can reformat the body freely without ever causing a verify diff.

### Changed

- **`alef verify` is now hash-only.** The legacy "diff against on-disk content" path is removed. Generated files carry an `alef:hash:<blake3>` header line; verify reads that hash and compares against the hash of the freshly-generated canonical content. No file body inspection, no normalization-against-disk, no formatter sensitivity. Files without an alef hash header (user-owned scaffolds like `Cargo.toml`, `composer.json`) are skipped — alef has no claim over them.
- **`alef generate` auto-runs `sync_versions`** at the end of every run, so user-owned manifests (gemspec, composer.json, package.json, etc.) track `Cargo.toml` automatically without a separate `alef sync-versions` invocation.

### Fixed

- **Ruby version.rb double-conversion via `[sync].extra_paths`**: when consumers listed `version.rb` in `[sync].extra_paths` (a common pattern for non-default extension layouts), the generic `SEMVER_RE.replace_all` matched the `0.3.0` prefix of the gem-formatted `0.3.0.pre.rc.2` and replaced it with the cargo dash-form `0.3.0-rc.2`, producing the corrupted `VERSION = "0.3.0-rc.2.pre.rc.2"`. The extra-paths handler now recognises `version.rb` and `*.gemspec` filenames and applies the gem-aware replacer (`to_rubygems_prerelease` + targeted regex) instead of the generic semver regex.
- **Ruby gemspec/version.rb gem format**: scaffold (`alef-scaffold`) and binding writer (`alef-backend-magnus`) now both call `alef_core::version::to_rubygems_prerelease` so cargo prereleases like `0.3.0-rc.2` are emitted as RubyGems-canonical `0.3.0.pre.rc.2`. RubyGems rejects the dash form (`gem build` raises `Gem::Version "0.3.0-rc.2" is not a valid version`).
- **PHP stub PSR-12 blank line**: generated `.phpstub` files now emit `<?php\n\n` so php-cs-fixer doesn't reformat the file on every commit.

### Added

- **`alef_core::version` module**: `to_rubygems_prerelease(version: &str) -> String` extracted from the CLI's `pipeline::version` so backends and scaffolds can reuse it without duplicating the conversion logic.

## [0.8.2] - 2026-04-26

A follow-up release to v0.8.1 fixing post-generation formatter invocation. Generated bindings now reliably pass `cargo fmt --check` and `ruff check` in CI.

### Fixed

- **FFI cargo fmt silently no-op'd**: `format_generated` ran `cargo fmt` in `packages/ffi/`, which has no `Cargo.toml`. Generated `crates/{lib}-ffi/src/lib.rs` was therefore never formatted, and CI's `cargo fmt --all -- --check` failed on diffs like `use std::ffi::{CStr, CString, c_char};` vs the rustfmt-canonical `use std::ffi::{c_char, CStr, CString};`. Now runs `cargo fmt --all` from the project root, which formats every generated Rust crate (FFI, PyO3, NAPI-RS, Magnus, ext-php-rs, Rustler, wasm-bindgen) in the consumer workspace.
- **Python ruff missed lint autofixes**: only `ruff format` was run, so generated stubs/wrappers retained `I001` unsorted imports, `F401` unused imports (e.g. `Any`), stale `# noqa: F401` comments on used imports, and `TC008` missing `TypeAlias` annotations on union aliases. `format_generated` now runs `ruff check --fix .` before `ruff format .` in `packages/python/`.
- **Ruby formatter wrong tool**: ran `cargo fmt` in `packages/ruby/`, which has no `Cargo.toml` (the Magnus crate lives under `packages/ruby/ext/{lib}_rb/native/` and is covered by the FFI-driven `cargo fmt --all`). Now runs `rubocop -A --no-server` in `packages/ruby/` to auto-correct generated `.rb` and `.gemspec` files.
- **R formatter wrong tool**: ran `cargo fmt` in `packages/r/`. Now runs `Rscript -e "styler::style_pkg('packages/r')"`.
- **WASM formatter scope tightened**: was `cargo fmt` in `packages/wasm/` (works because the wasm-bindgen crate lives there). Now uses `cargo fmt -p wasm` so the explicit package selection survives if package layout changes.

### Changed

- `FormatterSpec` now holds a `&'static [FormatterCommand]` instead of a single command, so a language can run multiple formatter steps in sequence (used by Python's `ruff check --fix` → `ruff format`). On first failure within a sequence the remaining steps are skipped (warning logged) — formatter errors never fail `alef generate`.
- `work_dir: ""` is now treated as the project root (no path join), so language-agnostic invocations like `cargo fmt --all` can run from the consumer's workspace root.

## [0.8.1] - 2026-04-26

A follow-up release to v0.8.0 focused on closing remaining clippy/build-correctness gaps surfaced by Kreuzberg's full workspace build. All alef-generated bindings (Python, Node, Ruby, PHP, FFI, WASM, Elixir, R, Go, Java, C#) now compile cleanly with `-D warnings` against the kreuzberg verification worktree. Adds an `is_copy` IR flag so the FFI backend can correctly distinguish Copy enums from Clone-but-not-Copy data-bearing enums.

### Added

- **`is_copy` IR flag on `TypeDef` and `EnumDef`**: extracted from `#[derive(Copy)]`. Lets backends distinguish Copy types from Clone-but-not-Copy types when emitting field accessors and method returns.

### Fixed

- **WASM trait-bridge regression**: shared `gen_bridge_trait_impl` emitted `#[async_trait::async_trait]` unconditionally, but `kreuzberg-wasm` has no `async_trait` dep. On non-wasm32 targets the trait declaration uses `#[cfg_attr(not(target_arch = "wasm32"), async_trait)]`, producing E0195 lifetime mismatches and E0433 missing-crate errors. WASM trait bridges now compile inside a private `#[cfg(target_arch = "wasm32")]` mod with a public re-export — the bridge is wasm-only by nature (wraps `wasm_bindgen::JsValue`), so host workspace builds skip the impl entirely. The same `cfg(target_arch="wasm32")` gate is also applied to bridge-using free functions.
- **WASM scaffold deps**: generated `kreuzberg-wasm/Cargo.toml` now ensures `js-sys` is present, and serde-style `Vec<&str>` parameters are correctly bridged via `serde_wasm_bindgen` instead of failing JsValue→Vec deserialization.
- **FFI clone-on-copy lints (~25 occurrences)**: field accessors and method returns previously emitted `Box::into_raw(Box::new(obj.field.clone()))` for every Named-typed field, tripping `clippy::clone_on_copy` on the `Copy` enums (`TableModel`, `ChunkerType`, `LayoutClass`, `BBox`, etc.). Codegen now consults `enum.is_copy` / `type.is_copy` to emit auto-copy/deref for Copy types and `.clone()` for Clone-but-not-Copy types. Non-Clone opaques fall back to a raw-pointer alias.
- **FFI move-out-of-borrow errors (E0507)**: data-bearing enums (`Provider`, `HtmlTheme`, `PdfBackend`, `NodeContent`, `OcrBoundingGeometry`, etc.) are not Copy; previous codegen emitted `Box::new(obj.field)` (no clone), causing E0507. Now correctly emits `.clone()` when `is_clone && !is_copy`.
- **FFI let_and_return in vtable wrappers**: infallible primitive/Duration vtable returns previously emitted `let _rc = unsafe { fp(args) }; _rc`, tripping `clippy::let_and_return`. Skip the binding for that case and emit `unsafe { fp(args) }` as the tail expression.
- **FFI useless conversion in PathBuf::from on Option<PathBuf>**: codegen unconditionally wrapped `cache_dir_rs.map(std::path::PathBuf::from)` even when `cache_dir_rs` was already `Option<PathBuf>`. Now passes through directly.
- **PHP plugin bridge clippy cleanup**: 33 lint errors eliminated across `alef-backend-php`'s trait bridge generator — `let_and_return` in async wrappers, `clone_on_copy` on `*mut _zend_object` (raw ptr is Copy), `useless_conversion` on `&str`/`PhpException` self-conversions, `vec_init_then_push` replaced with `vec![…]` literals, and a duplicate `let texts = texts;` redundant local.
- **Rustler/Elixir useless `.into()`**: NIF function wrappers no longer wrap owned `String` / `Vec<u8>` returns in `.into()` (which clippy flags as `String → String` / `Vec<u8> → Vec<u8>` identity). The `gen_rustler_wrap_return` helper now consults `returns_ref` to decide.
- **Rustler reserved Elixir attribute names**: emitted GenServer modules no longer use Elixir-reserved module-attribute names (`@behaviour`, `@callback`, etc.), preventing `mix compile` errors.
- **Rustler/Go/scaffold misc**: Elixir builtins handling fix, Go cbindgen prefix correction for re-exported types, Cargo `tokio` dep added when async functions are present.
- **Rustler test fixture**: `EnumVariant` test data updated for the new `is_tuple` field added in v0.8.0.
- **PyO3 / WASM static method wrappers**: `wrap_return_with_mutex` now wraps the inner core type in the binding wrapper for `default()` / `from()` static methods on non-opaque types (was previously skipping the conversion via the broken `n == type_name` shortcut, producing 24 `mismatched types` errors in PyO3).
- **PyO3 useless `.into()` on owned String returns**: `wrap_return` in the shared serde path no longer emits `.into()` for `TypeRef::String | TypeRef::Bytes` (both are identity in all backends). Eliminates `.map(Into::into)` chains on `Result<String, _>` returns in generated PyO3 functions like `reduce_tokens`, `serialize_to_toon`, `serialize_to_json`.
- **`useless_conversion` on extracted `From<X> for Y` static methods**: kreuzberg's `From<html_to_markdown_rs::HtmlMetadata> for HtmlMetadata` impl is extracted as a static method on `HtmlMetadata` whose param type is normalized to `Self`. The generated FFI/PyO3 wrapper calls `Y::from(arg: Y)`, which Rust resolves to the blanket `From<T> for T` (identity). The wrapper is preserved for ABI stability; `clippy::useless_conversion` is suppressed at the FFI and PyO3 file level.
- **C# nullable double-coalesce**: visitor field accessors emitted `x?.Property ?? null ?? defaultValue` on already-nullable types. The redundant `?? null` is now dropped.
- **C# Name property on visitor interface**: `IVisitor` was missing the `Name` property on some codegen paths; now always emitted so trait bridge dispatch works.
- **C# csproj scaffolding**: `Kreuzberg.csproj` is scaffolded once into the package subdir (not the language root) and not overwritten on subsequent runs. User-tuned project settings (target framework, package metadata) survive regeneration.
- **Elixir NIF Cargo.toml**: emits `[workspace]` block so the NIF crate is its own cargo workspace, plus `futures-util` dep for the async bridge dispatch path.
- **Java primitive trait method parameters**: trait bridges now correctly box primitives (`int` → `Integer`, `bool` → `Boolean`) before passing through `Object[]` callbacks; primitive return values are unboxed via `.intValue()` / `.booleanValue()` rather than direct cast.
- **Magnus tagged-union `Vec<Named>` field marshalling**: Vec-of-Named fields preserve `Vec<Named>` type in generated Rust enum variants instead of collapsing to `String`. JSON array round-trip now works for tagged unions like `Multi(Vec<Item>)`. Map fields still collapse to `String` for serde_json indirection.
- **Magnus Option<T> double-wrap in kwargs**: kwargs builders no longer emit `Option<Option<T>>` for nullable struct fields when the field type is already `Option<T>` and the kwarg flag also makes it optional. Single-Option emission across kwargs and getters.
- **Go/WASM NodeContext cbindgen prefix**: cbindgen-emitted C struct names use the configured crate prefix (e.g. `KZBNodeContext` instead of `NodeContext`), matching the declared FFI symbol convention.
- **Go binding `futures-util` dep**: generated Go FFI's `Cargo.toml` declares `futures-util` when the IR exposes async functions.

## [0.8.0] - 2026-04-26

This release closes a long-standing gap in alef's polyglot generator: bindings for every supported language now compile cleanly with zero errors and zero warnings against real-world Rust crates that exercise trait bridges, tagged-union enums, async functions, and feature-gated modules. Most of the surface area changes are codegen-internal — public APIs are unchanged — but the cumulative effect is that downstream projects (Kreuzberg in particular) can now consume alef-emitted code without any post-generation patching across Java, C#, Go, Ruby, Elixir, R, WASM, and the previously-stable Python/Node/PHP/FFI bindings.

### Added

- **Trait bridges (Java, C#, Magnus/Ruby)**: full plugin-style register/unregister codegen with proper FFI-side vtable struct names and method dispatch. Previously gated behind `exclude_languages = ["..."]` per-bridge; the generated code is now compile-clean across all backends.
- **Trait bridges (Rustler/Elixir)**: new `LocalPid`-based dispatch using `OwnedEnv::send_and_clear` plus a global oneshot reply registry (`TRAIT_REPLY_COUNTER`, `TRAIT_REPLY_CHANNELS`) for synchronous and asynchronous trait method calls. The `alef-scaffold` Elixir generator emits a companion GenServer module per trait so consumer code can `use` the bridge directly.
- **Auto-format**: `alef generate` now runs the language-native formatter (`ruff format`, `mix format`, `cargo fmt`, `biome format --write`, `gofmt -w`, `php-cs-fixer fix`, `dotnet format`, `google-java-format -i`) on the emitted output by default. `--no-format` opts out. Per-language overrides via `[format.<lang>]` in `alef.toml`.
- **Setup timeouts**: `alef setup` accepts `--timeout <seconds>` (default 600 per language). Per-language `timeout_seconds` settable via `[setup.<lang>]` in `alef.toml`. Hangs in tooling that ignores Ctrl-C now fail cleanly via `try_wait` deadline + `child.kill`.
- **Sync-versions (RubyGems prerelease format)**: `alef sync-versions` now writes Bundler-canonical prerelease strings (`1.8.0-rc.2` → `1.8.0.pre.rc.2`) to gemspecs and `version.rb` files. Previously emitted SemVer dashes were rejected by RubyGems.
- **Sync-versions (README regeneration)**: after manifest updates, alef extracts current IR and regenerates per-language READMEs so embedded version strings (e.g. `<version>1.8.0</version>` in pom.xml snippets) refresh without a separate `alef readme` invocation.
- **Sync-versions (cache invalidation)**: removes `.alef/` after manifest updates so subsequent generation steps see fresh IR.
- **Publish `after` hooks**: `[publish.<lang>] after = "..."` is now executed at the success-path end of `prepare`, `build`, and `package` stages, mirroring the existing `before`/`precondition` hook contract. The `after` field on `PublishLanguageConfig` was previously dead config.
- **Feature-gated extraction**: `alef-extract` now captures `#[cfg(feature = "...")]` annotations on `pub use` re-exports and `pub mod` declarations, propagating the cfg to all re-exported items.
- **WASM feature filtering**: `alef-backend-wasm` reads enabled feature flags from `alef.toml` and automatically excludes types/functions/enums whose cfg references a disabled feature, preventing broken references like `ServerConfig` (gated behind `api`) from leaking into the WASM crate.
- **Orphan file cleanup**: after `alef generate` writes the current binding output, alef scans the per-language output directories for files containing the alef-generated header that are not in the current run's emission set, and deletes them. Prevents stale alef-emitted files from blocking builds when types are removed from the public Rust API.
- **Magnus tuple variant codegen**: tagged-union enum variants like `Multiple(Vec<String>)` now emit valid `Multiple(Vec<String>)` Rust syntax. Previously emitted invalid `Multiple { _0: ... }` struct-style braces.
- **FFI enum from_json/free**: enums that flow through FFI as opaque pointer parameters now get `*_from_json` and `*_free` exports so backends (C#, Java, etc.) can construct enum values across the FFI boundary.
- **C# Directory.Build.props**: emitted on every `alef generate` (always overwritten) with `<Nullable>enable</Nullable>` and `<LangVersion>latest</LangVersion>`. MSBuild auto-imports it from the package directory so the setting survives user edits to `Kreuzberg.csproj`.

### Changed

- **Magnus serde marshalling for tagged-union enum fields**: `Vec<Item>` (where `Item` is a Named type) is now emitted as `Vec<Item>` in the generated Rust enum — preserving JSON array round-trip — instead of being collapsed to `String`. Map fields still collapse to `String` for serde_json indirection. Vec of primitives stays `Vec<T>`.
- **Java facade Optional unwrap**: facade methods declared with non-Optional return types (e.g. `String getPreset(...)`) now call `.orElseThrow()` on FFI results that come back as `Optional<T>`, eliminating the type mismatch between the facade and the Panama FFM layer.
- **C# wrapper `result` shadowing**: emitted local `result` variables are renamed to `nativeResult` to avoid CS0136/CS0841 collisions when the wrapper method takes a parameter named `result` (e.g. `SerializeToJson(ExtractionResult result)`).
- **C# tagged-enum string defaults**: when a serde-tagged enum field maps to `string` in C#, the default is emitted as the variant's JSON tag (`"Plain"`) rather than the non-existent `string.Plain` static.
- **C# async unit returns**: `async Task` methods with unit returns no longer emit `return await Task.Run(() => …);` — the `return` is dropped to satisfy CS1997.
- **Go trait_bridges.go**: now generated independently of the `visitor_callbacks` flag so plugin-style bridges work without enabling visitor codegen. The vtable struct name uses cbindgen's `{CRATE_UPPER}{CratePascal}{TraitPascal}VTable` convention; register/unregister calls use `{prefix}_register_{trait_snake}` / `{prefix}_unregister_{trait_snake}`. Function pointer fields are now wrapped via `(*[0]byte)(unsafe.Pointer(C.export))` so cgo treats them as C function pointer types. JSON-decoded `interface{}` parameters are type-asserted to `map[string]interface{}` before being handed to the impl method. `bool` parameters are converted to `C.uchar` via a conditional rather than direct cast.
- **C# stale-visitor cleanup**: when `visitor_callbacks = false`, alef now deletes `IVisitor.cs`, `VisitorCallbacks.cs`, `NodeContext.cs`, and `VisitResult.cs` from the output dir if they remain from a prior run.
- **Hash injection (XML)**: `alef_core::hash::inject_hash_line` now correctly handles `<!-- … -->` comment style for XML/csproj files; `pipeline::write_files` and `pipeline::diff_files` respect the per-file `generated_header` flag rather than always assuming Rust line comments.
- **alef-cli helpers**: `run_command_captured_with_timeout` uses a deadline-driven `try_wait` poll loop to enforce setup timeouts without adding new dependencies.

### Fixed

- **Magnus**: emits `Vec<T>` and `Map<K,V>` as their actual JSON-shaped types in serde-marshalled enum variants. Collapsing to bare `String` previously broke deserialization of tagged-union variants like `StopSequence::Multiple(Vec<String>)`.
- **Magnus**: deprecated `magnus::value::qnil()` and `magnus::RString::new()` API calls replaced with `Ruby::qnil()` / `Ruby::str_new` in trait bridge and binding emit. Crate-level inner attributes silence the remaining cosmetic warnings.
- **Java**: `gen_facade_class` now wraps FFI Optional returns with `.orElseThrow()` for non-Optional facade signatures (`getPreset`, `detectLanguages`).
- **Java**: `*_TO_JSON` symbol duplicates in `NativeLib.java` are deduplicated via a tracking set, mirroring the existing `_from_json` and `_free` dedup paths.
- **Java**: output-path doubling fixed when `output.java` already contains the `dev/<group>/` prefix.
- **C#**: `_vtable` field promoted to `internal` so cross-class dispatch from the wrapper compiles.
- **C#**: `sizeof(IntPtr)` replaced with the const `IntPtr.Size` so the call site no longer requires `unsafe`.
- **C#**: `bool` parameters cast to `int` (`(arg ? 1 : 0)`) at FFI call sites.
- **C#**: F32 default values emit the `f` suffix; F64 emits the `d` suffix where required for unambiguous typing.
- **C# scaffold**: removed `<NoWarn>` suppression — every generated `.cs` file now has `#nullable enable` and the project enables nullable reference types via `Directory.Build.props`.
- **Rustler**: trait bridge dispatch was previously stubbed; now wired to the reply registry with proper error_constructor template substitution. Atom creation uses `rustler::types::atom::ok()` (the previous `Env::new()` call did not exist).
- **Rustler**: error constructor template now substituted (`{msg}`) into `Err(...)` paths in both sync and async method bodies.
- **CLI**: `alef sync-versions` now updates `packages/ruby/lib/{gem}/version.rb` files in addition to gemspec/pyproject/package.json/pom.xml, completing the previously-incomplete Ruby version sync.
- **e2e/rust**: dropped the unconditional `[workspace]` block from generated `e2e/rust/Cargo.toml`. Parent workspace `[workspace.exclude]` now governs whether cargo trips on two-workspace-roots.
- **Go visitor.go**: regenerated visitor.go now compiles (`HTMHtmNodeContext`/`goVisitText` errors fixed at the alef-backend-go source).
- **Setup pipeline**: closure no longer fights `?`-propagation against rayon's parallel iterator; setup errors now surface with proper context.
- **alef-cli**: setup timeouts now actually kill the spawned process group on deadline rather than logging a spurious "skeleton" error.

### Removed

- C# scaffold no longer emits `<NoWarn>CS1591,CS8618,CS0168,CS0219,CS8625,CS0414,CS8632,CS8866</NoWarn>`. All formerly-suppressed warnings are now fixed at codegen.

## [0.7.11] - 2026-04-26

### Changed

- **E2E**: bump pinned `vitest` in generated TypeScript and WASM `package.json` from `^3.0.0` to `^4.1.5`, matching the version dependabot already pulls into the alef repo's own e2e lockfile.
- **Codegen/Scaffold**: all hardcoded third-party dependency version strings used in scaffold and e2e templates are centralized in `alef_core::template_versions` (110 constants grouped by ecosystem: npm, cargo, maven, gem, packagist, nuget, hex, pypi, cran, precommit). Each const that should auto-bump is annotated with a `// renovate: datasource=...` marker, and a new `renovate.json` at the repo root wires up the custom regex manager so Renovate can open version-bump PRs. Pure refactor — no value changes.
- **Tooling**: `task set-version` now targets the centralized `ALEF_REV` constant in `crates/alef-core/src/template_versions.rs` instead of the stale path in `precommit.rs`.

### Fixed

- **Update**: `alef update --latest` no longer hangs when Node and Wasm are both configured. Commands shared across multiple language default configs (e.g. `pnpm up --latest -r -w`) are now deduplicated — only the first language claiming a command runs it, preventing pnpm lockfile races under parallel execution.
- **Update**: R `upgrade` command now passes `ask = FALSE` to `remotes::update_packages()`, preventing an interactive prompt that blocked the non-interactive runner.
- **E2E (Python)**: skip-reason strings on generated `@pytest.mark.skip` decorators are now escaped before interpolation, so reasons containing quotes or backslashes no longer produce syntactically invalid Python test files.

### Removed

- **Hooks**: `alef-fmt` and `alef-lint` pre-commit hooks. Both used `pass_filenames: false` with a broad `files:` regex, so any matching commit cold-started every configured language toolchain (mvn, dotnet, mypy, etc.) regardless of which file changed — making the hooks unusably slow. The `alef fmt` and `alef lint` CLI commands are unchanged. Scaffold's generated `.pre-commit-config.yaml` no longer emits the removed hook ids.

## [0.7.10] - 2026-04-25

### Fixed

- **Validation**: redundant-default warnings no longer fire for `precondition` fields when the section has custom main commands (format/check/typecheck/command/e2e). Previously, `alef verify` would warn to remove the precondition while simultaneously requiring it — a contradiction.
- **Verify**: legacy README comparison now normalizes whitespace before hashing, preventing false-positive "stale bindings" reports from trailing-space or blank-line differences.

## [0.7.9] - 2026-04-25

### Added

- **Backends**: Kotlin/JVM backend (`alef-backend-kotlin`) — emits `data class` for IR structs, `enum class` / `sealed class` for IR enums, and an `object` wrapping top-level functions. Consumes the same Java/Panama FFM `.so` produced by the Java backend. Function bodies are `TODO()` stubs; FFI bridge wiring lands in Phase 1C. Kotlin/Native and Multiplatform paths deferred to Phase 3.
- **Backends**: Gleam backend (`alef-backend-gleam`) — emits `pub type Foo { Foo(field: T, ...) }` records and `@external(erlang, "<nif>", "<fn>")` declarations targeting the Rustler-emitted Erlang NIF. No new Rust crate generated; Gleam shims an existing Rustler NIF library.
- **Backends**: Zig backend (`alef-backend-zig`) — emits `pub const T = struct {}` types, `pub const E = enum {}` / `union(enum)` enums, and thin wrappers calling into the C ABI via `pub const c = @cImport(@cInclude("<header>.h"))`. Marshalling for non-trivial return types lands in Phase 1C.
- **Core**: `BuildDependency` enum (`None | Ffi | Rustler`) replaces `BuildConfig.depends_on_ffi: bool`. All 11 existing backends migrated; `BuildConfig::depends_on_ffi()` accessor preserved for callers.
- **Core**: `PostBuildStep::RunCommand { cmd, args }` variant — needed by Phase 2 Dart's flutter_rust_bridge codegen step.
- **Core**: Five new `Language` enum variants — `Kotlin`, `Swift`, `Dart`, `Gleam`, `Zig`. All per-language match arms across `crates/alef-core/src/config/` (build/clean/lint/setup/test/update defaults, naming, scaffold, publish, docs, adapters) wired with sensible defaults. `Swift` and `Dart` panic in the registry pending Phase 2.
- **CLI**: `alef verify` now checks README freshness — regenerated READMEs are compared with on-disk files.
- **Hooks**: `alef-readme` pre-commit hook to regenerate README files for all configured languages.
- **Scaffold**: scaffolded `.pre-commit-config.yaml` includes `alef-readme` hook.
- **Config**: top-level `[tools]` section selects per-language tool variants — `python_package_manager` (uv | pip | poetry), `node_package_manager` (pnpm | npm | yarn), and `rust_dev_tools` (list of `cargo install` targets). Per-language pipeline defaults dispatch on these choices.
- **Pipelines**: every per-language default (lint, test, build, setup, update, clean) now declares a POSIX `command -v <tool>` precondition so steps gracefully warn-and-skip when the underlying tool is missing on the user's system.
- **Setup**: Rust `setup` default now installs the full polyrepo dev-tool set — `cargo-edit`, `cargo-sort`, `cargo-machete`, `cargo-deny`, `cargo-llvm-cov` — plus `rustfmt` and `clippy` rustup components. The list is overridable via `[tools].rust_dev_tools`.
- **Init**: `alef init` emits a commented `[tools]` block in the generated `alef.toml` with all alternatives documented.
- **Validation**: `alef.toml` is validated at load time. Custom `[lint|test|build_commands|setup|update|clean].<lang>` tables that override a main command field must declare a `precondition` so the warn-and-skip behavior is preserved on user systems.
- **Scaffold**: FFI Cargo.toml gains a `[dev-dependencies]` block with `tempfile`. Dependency pins audited and policy documented.
- **Config**: per-language `run_wrapper`, `extra_lint_paths`, and `project_file` knobs reduce override boilerplate. `[python] run_wrapper = "uv run --no-sync"` prefixes default tool invocations across lint and test; `[python] extra_lint_paths = ["scripts"]` appends paths to default lint commands; `[csharp] project_file = "Foo.csproj"` (and `[java] project_file = "pom.xml"`) makes default lint/test/build commands target the file instead of the package directory. Each absorbs a common override pattern observed in consumer repos without forcing a full `[lint.<lang>]` redefinition.
- **Validation**: `alef.toml` is now scanned for redundant defaults at load time. When a user-supplied `[lint|test|build_commands|setup|update|clean].<lang>` field equals the built-in default verbatim, alef emits a `tracing::warn!` naming the section and field so users can keep the file minimal.

### Changed

- **Pipelines**: `default_*_config` functions now take a `&LangContext` argument bundling `&ToolsConfig`, `run_wrapper`, `extra_lint_paths`, and `project_file` — replacing the prior `&ToolsConfig`-only signature so per-language defaults can dispatch on every relevant knob.
- **Generate**: post-generation formatting is now best-effort. `alef generate` (and `alef all`) call a new `fmt_post_generate` that swallows three classes of post-gen formatter trouble — a missing tool (precondition miss → "Skipping" warning, language skipped), a failing `before` hook (warning, language skipped), and a non-zero formatter exit (warning, formatter loop continues). Formatters are *expected* to modify generated files, so non-zero exits there must not abort the run. The explicit `alef fmt` command keeps strict failure semantics.

### Fixed

- **Python**: classify enum imports correctly in generated `api.py` — data enums (tagged unions) and enums not referenced by `has_default` config structs now import from the native module instead of `options.py`, fixing missing-attribute errors at runtime.
- **Python**: emit converter locals in required-first, optional-last order so positional calls to the native function match the pyo3 signature.
- **Python**: generate runtime conversion for data-enum and `has_default` parameters in `api.py` wrappers, including `Vec<…>` and `Optional<Vec<…>>` shapes — list comprehensions are emitted to coerce each element. Previously only scalar (`Named` / `Optional<Named>`) parameters were converted, leaving collection parameters silently un-coerced.
- **Performance (pyo3)**: maintain a parallel `AHashSet` for the opaque-types transitive-closure dedupe check, restoring O(1) membership testing in the hot loop.

## [0.7.8] - 2026-04-25

### Changed

- **Codegen**: extract shared trait bridge helpers (`bridge_param_type`, `visitor_param_type`, `prim`, `find_bridge_param`, `to_camel_case`) into `alef-codegen::generators::trait_bridge`, removing ~600 lines of duplication across 7 backend crates.
- **Codegen**: remove duplicate `format_type_ref` from extendr backend in favor of the shared implementation.
- **Codegen**: consolidate `PYTHON_KEYWORDS` constant — delete duplicate from `alef-codegen::generators::enums`, import from `alef-core::keywords`.
- **E2E**: extract shared `resolve_field()` helper from `rust.rs` and `python.rs` into `codegen/mod.rs`.

### Fixed

- **Codegen**: avoid redundant `.into()` on owned String/Bytes return values in generated bindings.
- **Codegen**: use `to_string_lossy()` for Path return values in generated method wrappers.
- **Codegen**: optimize generated `.map(|val| val.into())` to `.map(Into::into)` in function/method wrappers.
- **Go**: skip functions/methods that use enum types lacking FFI JSON helpers (`_from_json`/`_to_json`/`_free`).
- **Go**: respect `exclude_functions` from FFI config when generating Go bindings.
- **Go**: fix `visitor_callbacks` in trait bridge test config.

## [0.7.7] - 2026-04-25

### Added

- **Config**: top-level `version` field in `alef.toml` to pin the alef CLI version per project. `alef init` now emits this field automatically.
- **Install action**: `install-alef` reads the pinned version from `alef.toml` when input is `"latest"`, falling back to the latest GitHub release if not specified.

### Fixed

- **Config**: use top-level `version` key instead of `[alef]` section to avoid TOML scoping issues where subsequent keys were captured inside the section.
- **Python bindings**: sanitize doc strings with `sanitize_python_doc()` consistently across all generated code (options enums, dataclasses, TypedDict, API functions, exceptions) to prevent ruff RUF001/RUF002 lint errors.
- **Python stubs**: fix test assertions for builtin-shadowing parameter names (`input`, `id`) that now generate multi-line signatures with `# noqa: A002`.

## [0.7.6] - 2026-04-25

### Added

- **Config**: `[alef]` section in `alef.toml` with `version` field to pin the alef CLI version per project (superseded by top-level `version` in 0.7.7).

### Fixed

- **Python bindings**: sanitize doc strings with `sanitize_python_doc()` consistently across all generated code.

## [0.7.5] - 2026-04-25

### Fixed

- **Codegen**: suppress `clippy::redundant_closure` and `clippy::useless_conversion` lints in generated `From` impl blocks.
- **Python bindings**: add `noqa` comments to generated `typing` imports.
- **Python bindings/stubs**: force multi-line function signatures when a parameter name shadows a Python builtin (e.g. `id`, `type`, `input`), and annotate those parameters with `# noqa: A002`.

## [0.7.4] - 2026-04-25

### Added

- **E2E assertions**: `min_length`, `max_length`, `ends_with`, and `matches_regex` assertion types across all language codegen backends (Bash/Brew, C, C#, Elixir, Go, Java, PHP, Python, R, Ruby, WASM).

### Fixed

- **Python bindings**: prevent duplicate `| None` annotation on optional parameters whose base type already contains `| None` (e.g. `Option<Option<T>>`).
- **Python stubs**: use `ends_with("| None")` instead of `contains("| None")` for the same check in type-init, method, and function stubs.
- **Scaffold(C#)**: XML-escape author names in `.csproj` to avoid malformed XML.
- **Scaffold(Java)**: parse `"Name <email>"` author strings into separate `<name>` and `<email>` elements; XML-escape both; omit placeholder email when not provided.
- **Scaffold(PHP)**: derive Composer vendor from GitHub repository URL instead of hardcoding `kreuzberg-dev`.
- **Scaffold(Ruby)**: use double-quoted strings for gemspec authors array.
- **Scaffold**: default repository URL uses `example` org instead of `kreuzberg-dev`.
- **Elixir bindings**: escape parameter names that collide with Elixir reserved words (`do`, `end`, `fn`, etc.) by appending `_val` suffix.

## [0.7.3] - 2026-04-25

### Added

- **Keyword-aware field renaming for Python bindings** — struct fields whose names collide with Python reserved keywords (e.g. `class`, `from`, `type`) are automatically escaped in the generated binding struct (`class_`) while preserving the original name via `#[pyo3(get, name = "class")]` and `#[serde(rename = "class")]` attributes. Configurable per-field via `rename_fields` in language configs.
- Shared keyword list in `alef-core::keywords` with `python_ident()` helper — single source of truth for Python reserved words, replacing duplicated lists in `gen_bindings.rs` and `gen_stubs.rs`.
- `ConversionConfig::binding_field_renames` for keyword-escaped field name substitution in `From`/`Into` conversion impls.
- `gen_struct_with_rename` and `gen_impl_block_with_renames` codegen generators for per-field attribute customization.
- `AlefConfig::resolve_field_name()` for config-driven field name resolution per language and type.
- Scaffold: emit `rust-toolchain.toml` (Rust 1.95) and `.cargo/config.toml` with `wasm32-unknown-unknown` target for WASM projects.
- Scaffold: include WASM target in clippy pre-commit hook instead of excluding it.
- Config: `corepack up` added to Node.js update defaults; `corepack use pnpm@latest` for pnpm upgrade.

### Fixed

- Publish: derive PHP extension name from output path instead of hardcoding.
- Publish: derive crate names from output paths instead of hardcoding.
- FFI: replace deprecated `clippy::drop_ref` lint with `dropping_references`.
- Scaffold: Ruby `Rakefile` template uses correct Bundler 4 API.

## [0.7.2] - 2026-04-24

### Added

- **`alef publish` command group** — vendoring, building, and packaging artifacts for distribution across language package registries (issue #9).
  - `alef publish prepare` — vendors core crate (Ruby, Elixir, R) and stages FFI artifacts (Go, Java, C#).
  - `alef publish build` — cross-compilation-aware build with `--target` support, auto-selects cargo/cross/maturin/napi/wasm-pack per language.
  - `alef publish package` — creates distributable archives (C FFI tarball with pkg-config/CMake, PHP PIE archive, Go FFI tarball).
  - `alef publish validate` — checks version readability, package directory existence, and manifest file presence.
- New `alef-publish` crate with `platform::RustTarget` for parsing Rust target triples and mapping to per-language platform conventions (Go, Java, C#, Node, Ruby, Elixir).
- `[publish]` config section in `alef.toml` with per-language `vendor_mode`, `nif_versions`, `build_command`, `package_command`, `precondition`, `before` hooks, `pkg_config`, `cmake_config`.
- `vendor::vendor_core_only()` — copies core crate, rewrites Cargo.toml via `toml_edit` to inline workspace inheritance and dependency specs, removes workspace lints, generates vendor workspace manifest.
- `vendor::vendor_full()` — core-only + `cargo vendor` of all transitive deps with test/bench cleanup.
- `ffi_stage::stage_ffi()` / `stage_header()` — copies built FFI shared library and C header to language-specific directories.

### Fixed

- CLI: `alef build` now respects `[build_commands.<lang>]` overrides for non-Rust languages.
- CLI: `before` hooks in lint/fmt now fire even when a language has `check`/`typecheck` commands but no `format` commands.
- CLI: `before` hook output (stdout/stderr) is now logged instead of silently discarded.
- Config: Ruby lint defaults use `cd {output_dir} && bundle exec rubocop` instead of running from project root.
- Config: FFI lint defaults search entire output directory instead of assuming `tests/` subdirectory.
- Scaffold: pre-commit config now uses `alef-fmt` + `alef-lint` hooks instead of per-language hooks.
- Codegen(Go): local variable naming, error string conventions, enum acronym rules, `nodeTypeFromC` acronym naming, visitor doc comments, golangci scaffold config.

## [0.7.1] - 2026-04-24

### Added

- Config: `precondition` field on all command configs (`LintConfig`, `TestConfig`, `SetupConfig`, `UpdateConfig`, `BuildCommandConfig`, `CleanConfig`) — a shell command that must exit 0 for the main command to run; skips the language with a warning on failure.
- Config: `before` field on all command configs — command(s) that run before the main command; aborts on failure. Supports `StringOrVec` (single string or list).
- Config: `GOWORK=off` in default Go setup command to avoid workspace interference.
- Config: Maven version rules file reference in default Java update commands.
- CLI: Rust is now a first-class language in `alef build` — builds via configurable `[build_commands.rust]` instead of panicking on missing backend.
- FFI: derive `Copy` and `Clone` on generated vtable structs.

### Fixed

- FFI: trait bridge generation fixes for kreuzberg integration.
- Scaffold: pre-commit config simplification — removed per-language hooks.

## [0.7.0] - 2026-04-24

### Added

- CLI: `alef fmt` command — run only format commands on generated output.
- CLI: `alef update` command — orchestrate per-language dependency updates with sensible defaults. `--latest` flag for aggressive upgrades (incompatible/major version bumps).
- CLI: `alef setup` command — install dependencies per language using per-language defaults or `[setup.<lang>]` config.
- CLI: `alef clean` command — clean build artifacts per language using per-language defaults or `[clean.<lang>]` config.
- CLI: `alef test --coverage` flag — run coverage commands defined in `[test.<lang>].coverage`.
- Config: `StringOrVec` type for lint/update/test/setup/clean/build commands — supports both `format = "cmd"` and `format = ["cmd1", "cmd2"]` in alef.toml.
- Config: `[update.<lang>]` sections in alef.toml with `update` (safe) and `upgrade` (latest) commands.
- Config: `[setup.<lang>]` sections in alef.toml with `install` commands for dependency installation.
- Config: `[clean.<lang>]` sections in alef.toml with `clean` commands for removing build artifacts.
- Config: `[build_commands.<lang>]` sections in alef.toml with `build` and `build_release` commands (replaces hard-coded tool invocations).
- Config: `coverage` field on `[test.<lang>]` — `TestConfig` fields migrated to `StringOrVec`; `command` and `e2e` also accept arrays.
- Config: default lint commands for all 12 languages with autofixes enabled (ruff --fix, rubocop -A, clippy --fix, oxlint --fix).
- Config: default update commands for all 12 languages (cargo update, pnpm up, uv sync, bundle update, composer update, go get, mvn versions, dotnet outdated, mix deps.update, etc.).
- Config: default setup and clean commands for all 12 languages.
- Config: add `exclude_languages` field to `TraitBridgeConfig` for per-language trait bridge opt-out.
- Config: add `exclude_functions` and `exclude_types` fields to `RubyConfig` for per-type/function exclusion.
- Magnus: honor `exclude_functions`/`exclude_types` from `[ruby]` config in binding, conversion, and module init generation.
- Magnus: detect absent `Named` types in enum variant fields and route them through `serde_json` deserialization.
- Rustler: filter trait bridges by `exclude_languages` so excluded bridges are omitted from Elixir output.
- Codegen: handle `EnumVariant` defaults for `String`-mapped fields in Magnus hash constructors.
- Codegen: deduplicate `Bytes` conversion when base conversion already emits `.into()` or `.map(Into::into)`.
- Codegen: handle sanitized and excluded-type fields in enum binding-to-core match arms with `serde_json` deserialization and `Box` wrapping.
- Scaffold (Node): generate `.oxfmtrc.json` (120 printWidth, tabs, import sorting) and `.oxlintrc.json` (correctness=error, suspicious=warn, style=off, typescript+import plugins).

### Changed

- CLI: `alef init` now runs full project bootstrap — generates alef.toml, extracts API, generates bindings, scaffolds manifests, and formats in one command.
- CLI: `alef lint` refactored with `LintPhase` enum, uses `config.lint_config_for_language()` with per-language defaults instead of raw map lookup.
- CLI: `alef generate` and `alef all` use built-in `fmt` instead of prek for post-generation formatting.
- Scaffold (Node): replace Biome with Oxc toolchain — `oxfmt` + `oxlint` in package.json devDeps and scripts.
- Scaffold (Java): strip cosmetic whitespace checks from checkstyle.xml (Spotless handles formatting). Remove WhitespaceAfter, WhitespaceAround, GenericWhitespace, EmptyBlock, NeedBraces, MagicNumber, JavadocPackage.
- Scaffold (Go): update `.golangci.yml` to v2 format with full settings (errcheck exclude-functions, govet enable-all, misspell locale, revive rules, exclusions, formatters).
- Scaffold (pre-commit): replace biome-format/biome-lint hooks with oxlint + oxfmt local hook.

### Removed

- CLI: `run_prek()` and `run_prek_autoupdate()` — prek is no longer auto-invoked during generation. Users can still use prek independently.

### Fixed

- Java: remove dead code left over from modularization (duplicate helpers in `mod.rs`, `helpers.rs`, `marshal.rs`).
- Java: fix empty-line-after-doc-comment clippy warnings in `marshal.rs` and `types.rs`.
- Java: suppress `too_many_arguments` clippy warnings on `gen_main_class` and `gen_facade_class`.
- Java: add missing `is_ffi_string_return()` and `java_ffi_return_cast()` marshal functions.
- Java: fix `RECORD_LINE_WRAP_THRESHOLD` visibility (`pub(crate)`).
- Docs: fix `render_method_signature` line length by splitting parameters across lines.
- Tests: add missing `exclude_languages` field to all `TraitBridgeConfig` test initializers.
- Tests: add missing `exclude_functions`/`exclude_types` fields to `RubyConfig` test initializers.

## [0.6.1] - 2026-04-23

### Fixed

- FFI: generate `{prefix}_unregister_{trait_snake}` extern C function for trait bridges (was documented but missing).
- Codegen: use fully-qualified `std::result::Result` in trait bridge return types.
- Codegen: tuple sanitization, Go `gofmt` column alignment, error gen cleanup.
- Go: `gofmt`-compliant struct field alignment with tab-separated columns.
- Go: apply Go acronym uppercasing (`to_go_name`, `go_type_name`, `go_param_name`) per Go naming conventions.

### Changed

- Tests: align all backend test assertions with actual codegen output (Go, Magnus, NAPI, PHP, PyO3, Rustler, FFI, scaffold).
- Tests: remove unimplemented Magnus plugin bridge tests (visitor bridge tests retained).
- Scaffold: update file count expectations for new tsconfig.json (Node), .golangci.yml (Go), and Java config files.

## [0.6.0] - 2026-04-23

### Added

- Scaffold: add ruff + mypy config and `[dependency-groups] dev` to Python `pyproject.toml`.
- Scaffold: add `biome.json`, `tsconfig.json`, and `@biomejs/biome`/`typescript` devDeps to Node/TypeScript.
- Scaffold: add `.golangci.yml` for Go with standard linter configuration.
- Scaffold: add `.php-cs-fixer.php` for PHP with PSR-12 and PHP 8.2 migration rules.
- Scaffold: add `.credo.exs` for Elixir with strict mode and cyclomatic complexity limit.
- Scaffold: add `.lintr` for R with 120-char line length; add `lintr`/`styler` to DESCRIPTION Suggests.
- Scaffold: add `.editorconfig` for C# with 120-char line length.
- Scaffold (Java): add `versions-rules.xml` filtering alpha/beta/RC/milestone/snapshot versions.
- Scaffold (Java): add `pmd-ruleset.xml` with standard PMD categories and exclusions.
- Scaffold (Java): add `maven-enforcer-plugin`, `maven-checkstyle-plugin`, `maven-pmd-plugin`, `versions-maven-plugin`, `jacoco-maven-plugin` to `pom.xml`.
- Scaffold (Java): add `jspecify` and `assertj-core` dependencies.

### Changed

- Scaffold (Java): update `maven-compiler-plugin` 3.14.0 → 3.15.0, `maven-source-plugin` 3.3.1 → 3.4.0, `maven-site-plugin` 3.21.0 → 4.0.0-M16.
- Scaffold (Java): update surefire argLine with `-XX:-ClassUnloading`, parallel test execution, and 600s timeout.
- Scaffold (Java): change javadoc doclint from `none` to `all,-missing`.
- Scaffold (Java): add whitespace, block, and coding checks to `checkstyle.xml`; add FFI and test suppressions.

### Fixed

- Codegen: fix `Map` with `Named` value conversion in optionalized `binding_to_core` path.
- Codegen: fix WASM `js_sys` import generation, Go `gofmt` compliance, Rust e2e `is_none` assertions, PHP builder pattern.
- Codegen: fix C# bytes pinning and `IntPtr.Zero` guard, Go `gofmt` output, Rust e2e `is_none`, Java e2e camelCase naming.
- Codegen: fix `Vec<String>` `_refs` binding and sanitized tuple-to-vec clone in `core_to_binding`.
- Codegen: fix `Vec<Primitive>` sanitized passthrough and enum sanitized serde round-trip.
- Magnus: fix `funcall` API usage, visitor bridge argument passing, `Vec` conversion, optional field flattening, and `default_types` handling.
- Magnus: simplify `gen_trait_bridge` signature — remove inline error type/constructor args, derive from config.
- PyO3: pass enum fields directly instead of `str()` + map lookup.
- PyO3: add `license`/`credits`/`copyright` to Python builtin names list.
- Rustler: fix Elixir `@spec` line wrapping to respect 98-char formatter default.
- Elixir: add `.formatter.exs` with 120-character line length; update stub wrap threshold to 120.
- Scaffold: add `steep`, `rubocop-performance`, `rubocop-rspec` to Ruby Gemfile template.

## [0.5.9] - 2026-04-23

### Fixed

- Rustler: replace `Pid::spawn_monitor` with `env.call` for sync NIF dispatch; add `Encoder` import.
- Rustler: visitor bridge receive loop — emit `_with_visitor` NIF variant and Elixir-side `do_visitor_receive_loop/1` for trait bridge callbacks.
- Rustler: fix double-optional struct fields — skip outer `Option` wrap when `field.ty` is already `Optional`.
- Rustler: add explicit type-annotated let bindings for named params in static NIF methods to resolve ambiguous `From` conversions.
- PyO3: strong typing for data enums in `options.py` — use concrete union type aliases instead of `dict[str, Any]`.
- PyO3: transitively expand `needed_enums` so nested data enums (e.g. `ContentPart` inside `UserContent`) are defined in `options.py`.
- PyO3: topological sort of data enum union aliases to ensure dependencies are emitted before dependents.
- PyO3: `needs_any` check now scans for `TypeRef::Json` instead of data enum fields.
- PyO3: assert non-optional converters for data enum variant fields.
- PyO3: escape single quotes in string literals and add `noqa` comments for assertion expressions.
- Go: remove zero-argument `New{TypeName}()` constructors from opaque types — handles are created by factory functions, not bare allocators.
- WASM: add `type_contains_json` helper for data enum field scanning.
- E2E (TypeScript): escape single quotes in string literals, handle large integers with `BigInt()`, add `noqa` assertions.
- CLI: simplify post-generation hash logic — store generation hashes from in-memory content pre-formatter instead of re-extracting and re-hashing on-disk files.
- Scaffold: fix Ruby `Rakefile` for Bundler 4 — use `Bundler::GemHelper.install_tasks` instead of `require 'bundler/gem_tasks'`.
- Core IR: add `TypeRef::references_named` method for recursive named type detection.
- Codegen: add `type_contains_json` helper to `binding_helpers` for checking `TypeRef::Json` presence in nested types.

## [0.5.8] - 2026-04-23

### Added

- E2E: `is_false`, `method_result` assertion types and `resolve_call` per-fixture call routing across all 14 language generators.

### Fixed

- PyO3: exclude return types from `local_type_names` in `options.py` (prevents false re-exports).
- PyO3/Magnus: remove stale `type: ignore` comments; add `VERSION` constant to RBS type signatures.
- E2E (Python): add `ruff` per-file-ignores to generated `pyproject.toml`; add `noqa` comments for B017 and S108; add `method_result` helper imports.
- E2E (C): add missing `find_nodes_by_type` and `is_error` check types to `run_query`.
- E2E (Rust): add top-level `is_false` assertion support.
- C#/Elixir: format generated code to pass `dotnet format` and `mix format`.
- FFI: fix doc comment prefixes, duplicate imports, and optional param types.
- Codegen: use serde round-trip for tuple-to-vec `From` impls.
- Codegen: fix async return type wrapping, `Vec` element conversion, and `as_deref` option handling.
- Codegen: fix serde derives for opaque fields, `error_constructor` propagation, `BridgeOutput` fields, and trait bridge generation.
- Codegen: fix `Result` qualification, non-`Result` method returns, complex return types, PHP raw pointer handling, WASM scaffold paths.
- Scaffold: wire `scaffold_output` from config; remove stale `serde(default)` on non-serde structs.
- Python: dedup exception classes in `gen_exceptions_py`.
- NodeContent derive forwarding and test fixture resolution.
- Java: add `final` params, javadoc, `eclipse-formatter.xml`, `checkstyle.xml` scaffold with 120 line width, `spotless-apply` pre-commit hook.
- C#: fix `VisitResult.Continue()` to `new VisitResult.Continue()`.
- Elixir: add `ELIXIR_BUILTIN_TYPES` deny-list, suffix colliding type names with `_variant`.
- PHP: fix double-escaped quotes in `get_property` calls in trait bridge.

## [0.5.7] - 2026-04-22

### Fixed

- Python stubs (.pyi): add `Any` to typing imports (used by `dict[str, Any]` fields).
- Python stubs (.pyi): rename tagged union TypedDict variants with `Variant` suffix to avoid name collision with enum classes (e.g. `ToolChoiceModeVariant` instead of `ToolChoiceMode`).

## [0.5.6] - 2026-04-22

### Fixed

- Go backend: skip streaming adapter methods whose FFI signature uses callbacks (CGO incompatible).
- C# backend: skip streaming adapter methods in P/Invoke declarations and wrapper class generation.
- PHP backend: scaffold generates `[features] extension-module = []` for ext-php-rs compatibility.
- Map value conversion: fix optional fields with Named values in binding-to-core and core-to-binding conversions.

## [0.5.5] - 2026-04-22

### Added

- FFI backend: callback-based streaming via `LiterLlmStreamCallback` function pointer pattern. Replaces the `compile_error!` stub with a working implementation that invokes a callback per JSON chunk.
- FFI backend: `gen_streaming_method_wrapper` generates correct C signature (`client`, `request_json`, `callback`, `user_data` → `i32`).

### Fixed

- WASM backend: streaming adapter return type overridden to `Result<JsValue, JsValue>` (was `Result<String, JsValue>`, causing type mismatch with `serde_wasm_bindgen::to_value`).
- WASM backend: don't underscore-prefix params when adapter body is present (`_req` → `req`).
- Python backend: move type imports out of `TYPE_CHECKING` block — types used in function signatures must be available at runtime.
- Go backend: suppress CGO const warnings for generated enum-to-string functions.
- PHP backend: skip undelegatable methods instead of emitting broken stubs.
- Trait bridge: propagate `error_constructor` to all backends.

## [0.5.4] - 2026-04-22

### Fixed

- NAPI, Magnus, PHP, Rustler backends: skip sanitized methods entirely instead of emitting stubs, panics, or `[unimplemented: ...]` placeholders. Matches PyO3 behavior.
- Scaffold: Ruby `extconf.rb` now sets `cargo_manifest` path for `rb_sys`.
- Scaffold: Ruby and Elixir `Cargo.toml` `[lib]` section now includes `name` and `path` pointing to alef-generated binding source.
- E2E: TypeScript codegen no longer escapes `$` in double-quoted strings (fixes Biome `noUselessEscapeInString` lint).
- Go backend: remove invalid `?` syntax from vtable function pointer assignments.
- Rustler: keep `force_build` on single line for `mix format` compatibility.

## [0.5.3] - 2026-04-22

### Added

- CLI: `alef version` command and `alef --version` flag
- Scaffold: generate `py.typed` marker for Python, `Steepfile` for Ruby, `.d.ts` barrel for TypeScript
- Test coverage: ~500 new tests across alef-docs, alef-codegen, alef-extract, alef-readme, and all backends

### Fixed

- Adapters: streaming body fixes for all backends, core type let bindings
- Magnus: use snake_case for Ruby file paths in `generate_public_api`
- Scaffold: use snake_case for Ruby file paths regardless of `gem_name`
- Go: remove colons from Go parameter syntax in trait bridge
- Fix `error_type` field propagation across all test initializers

## [0.5.2] - 2026-04-22

### Added

- Trait bridges: plugin-pattern support across all 11 backends (issue #4)
  - PyO3: de-hardcoded plugin bridge via shared `TraitBridgeGenerator`
  - NAPI, WASM, PHP, Magnus, Rustler, Extendr: implement `TraitBridgeGenerator` for plugin bridges
  - FFI: vtable + `user_data` ABI with `#[repr(C)]` function pointer struct
  - Go: cgo trampolines wrapping C FFI vtable via `cgo.Handle`
  - Java: Panama FFM upcall stubs wrapping C FFI vtable
  - C#: P/Invoke declarations for C FFI trait bridge registration
- Codegen: shared `TraitBridgeGenerator` trait with `type_paths`, flexible registration, configurable super-trait
- Codegen: `format_param_type` respects `is_ref` for reference-typed params (`&str`, `&[u8]`, `&Path`)
- E2E: HTTP server fixture support for testing HTTP endpoints
- E2E: HTTP test codegen for Python, Node, Ruby, PHP, Elixir
- Scaffold: add `[lints] workspace = true` to all generated Cargo.toml templates
- Extendr: opaque type support with Arc wrappers and mutex types
- Skills: add designing-alef-toml guide with real-world configuration patterns

### Fixed

- Codegen: `gen_bridge_all` now emits constructor so `Wrapper::new()` exists for registration
- Codegen: `gen_bridge_trait_impl` filters `trait_source` methods (super-trait handled separately)
- Codegen: `gen_bridge_plugin_impl` strips body indentation to prevent double-indent
- PyO3: remove `.expect()` panics in `gen_registration_fn`, use graceful fallback
- PyO3: fix async double-`Result` flattening (`spawn_blocking` → `??`)
- NAPI/WASM/PHP/Magnus/Rustler/Extendr: remove `.expect()` panics in `gen_registration_fn`
- Magnus: implement actual registration logic (was a non-functional stub)
- Magnus: add missing async `String` clone for blocking closures
- Magnus/Rustler: fix `build_arg` ordering for optional `&str` (unreachable branch)
- Rustler: replace `spawn_monitor` with direct `Term::apply` for sync dispatch
- PHP: fix `&*self.php_obj` to `&mut *self.php_obj` for `try_call_method`
- WASM: handle poisoned `RwLock` in registration instead of panicking
- FFI: cache `version()` at construction (was leaking via `Box::leak` every call)
- FFI: return error on NUL bytes in serialized params (was `unwrap_or_default`)
- FFI: add `Sync` impl on async `_LocalBridge`
- Go: use correct `ffi_prefix` parameter instead of deriving from `register_fn`
- Go: fix `unsafe.Pointer(handle)` → `unsafe.Pointer(uintptr(handle))`
- Go: fix `outResult` ABI mismatch (emit when `return_type != Unit`, not just `error_type`)
- Go: fix syntax error in `C.GoString(` missing closing paren
- Go: add `FreeUserData` trampoline to prevent `cgo.Handle` leak
- Java: implement actual result serialization (was returning `MemorySegment.NULL`)
- Java: fix use-after-free by storing bridge in static map (not `try-with-resources`)
- Java: uncomment and implement FFI registration call
- Java: conditionally emit super-trait vtable slots based on config
- C#: add `[MarshalAs(UnmanagedType.LPUTF8Str)]` for UTF-8 string marshaling
- C#: change `ref IntPtr outError` to `out IntPtr outError`
- Docs: use filtered IR for API reference generation
- Docs: use `dict[str, Any]` instead of bare `Any` for Python JSON types
- Docs: box Java primitives in `Optional` (e.g. `Optional<Integer>` not `Optional<int>`)
- Docs: emit `async Task<T>` for C# async method signatures
- Docs: use `&str`/`&[u8]` for Rust method params matching free-function style
- Docs: pluralize nouns after stripping `_count` suffix
- Docs: thread `Language` param through `clean_doc_inline`
- Codegen: skip `Default`/`Serialize`/`Deserialize` derives for structs with opaque fields
- Codegen: add serde recovery path to Rustler and PHP backends
- Adapters: emit core type let bindings in Python streaming adapter
- C#: deterministic DllImport ordering in NativeMethods.cs
- CLI: use filtered IR for docs, remove dead unfiltered extraction code

## [0.5.1] - 2026-04-21

### Added

- Extract: per-crate extraction for multi-crate workspaces via `source_crates` config
- IR: `original_rust_path` field for orphan-safe `From` impls
- Codegen: dedup after path mapping, generic/enum generation improvements

### Fixed

- Codegen: Python workspace source path, C# paths/TFM, Go version resolution
- Codegen: only omit opaque inner field for unresolvable generic types (not all sanitized types)
- Codegen: skip serde derives for structs with opaque (Arc-wrapped) fields
- Codegen: use post-mapping `rust_path` for `From` impls, treat data enums as known types
- PyO3: don't import streaming `item_type` that shadows wrapper structs
- PyO3: async stub inference, Mutex default, trait type handling, error path fixes
- PyO3: group consecutive opaque type stubs in `.pyi` output
- Cache: strip all whitespace in content hash for formatter agnosticism
- CI: add `contents: write` permission to publish-homebrew job for bottle upload

## [0.5.0] - 2026-04-21

### Added

- Scaffold: render `extra_dependencies` in scaffold, auto path mappings
- CI: rewrite CI and Publish workflows using shared `kreuzberg-dev/actions` composite actions
- CI: add `typos` (spelling) and `actionlint` (GitHub Actions linting) pre-commit hooks

### Changed

- Publish: replace GoReleaser with native per-platform CLI builds via `build-rust-cli@v1`
- Publish: phased publish pipeline (prepare, validate, check, build, publish, finalize)
- Publish: OIDC authentication for crates.io via `publish-crates@v1`
- Publish: draft-then-undraft GitHub release pattern

### Fixed

- Codegen: opaque `Default` fallback uses `todo!()` instead of `Default::default()` for opaque return types
- PyO3: scan opaque type methods for `HashMap` usage to ensure import is added
- NAPI: pass `opaque_types` to `gen_unimplemented_body` calls
- E2E: make tokio dependency conditional in Rust e2e generator
- Core: remove unused `ahash` dependency, fix `clippy::manual_strip` warning

## [0.4.6] - 2026-04-21

### Added

- Config: `exclude_functions` and `exclude_types` support for Python (PyO3), FFI, and Node (NAPI) backends

### Fixed

- WASM: always import `js_sys` (error converters use `js_sys::Object`/`js_sys::Reflect`)
- WASM: skip free functions referencing excluded types (prevents undefined type references)
- WASM: add `clippy::unused_unit` to allowed lints for generated code
- Node: wire `exclude_functions`/`exclude_types` from config into NAPI gen_bindings
- Python: wire `exclude_functions`/`exclude_types` into both struct generation and pymodule init
- FFI: wire `exclude_functions` into free function generation loop
- Rustler: fix f32/f64 stub return values (`0` → `0.0`)
- Rustler: add `clippy::unused_unit` to allowed lints
- PyO3: add `clippy::map_identity` to allowed lints
- Codegen: use `format!("{:?}", i)` instead of `.to_string()` for sanitized `Vec<String>` conversions (fixes Display trait bound failures)
- Codegen: extend `gen_serde_let_bindings` to handle `Vec<Named>` params (fixes undefined `_core` variables)
- Rustler: Elixir module spec line-wrapping for long type signatures

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
