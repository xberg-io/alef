# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

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
