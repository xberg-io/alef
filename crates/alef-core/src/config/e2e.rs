//! E2E test generation configuration types.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Controls whether generated e2e test projects reference the package under
/// test via a local path (for development) or a registry version string
/// (for standalone `test_apps` that consumers can run without the monorepo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyMode {
    /// Local path dependency (default) — used during normal e2e development.
    #[default]
    Local,
    /// Registry dependency — generates standalone test apps that pull the
    /// package from its published registry (PyPI, npm, crates.io, etc.).
    Registry,
}

/// Configuration for registry-mode e2e generation (`alef e2e generate --registry`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Output directory for registry-mode test apps (default: "test_apps").
    #[serde(default = "default_test_apps_dir")]
    pub output: String,
    /// Per-language package overrides used only in registry mode.
    /// Merged on top of the base `[e2e.packages]` entries.
    #[serde(default)]
    pub packages: HashMap<String, PackageRef>,
    /// When non-empty, only fixture categories in this list are included in
    /// registry-mode generation (useful for shipping a curated subset).
    #[serde(default)]
    pub categories: Vec<String>,
    /// GitHub repository URL for downloading prebuilt artifacts (e.g., FFI
    /// shared libraries) from GitHub Releases.
    ///
    /// Falls back to `[scaffold] repository` when not set, then to
    /// `https://github.com/kreuzberg-dev/{crate.name}`.
    #[serde(default)]
    pub github_repo: Option<String>,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            output: default_test_apps_dir(),
            packages: HashMap::new(),
            categories: Vec::new(),
            github_repo: None,
        }
    }
}

fn default_test_apps_dir() -> String {
    "test_apps".to_string()
}

/// Root e2e configuration from `[e2e]` section of alef.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2eConfig {
    /// Directory containing fixture JSON files (default: "fixtures").
    #[serde(default = "default_fixtures_dir")]
    pub fixtures: String,
    /// Output directory for generated e2e test projects (default: "e2e").
    #[serde(default = "default_output_dir")]
    pub output: String,
    /// Repo-root-relative directory holding binary file fixtures referenced by
    /// `file_path` / `bytes` fixture args (default: "test_documents").
    ///
    /// Backends that emit chdir / setup hooks for file-based fixtures resolve
    /// the relative path from the test-emission directory via
    /// [`E2eConfig::test_documents_relative_from`]. The default matches the
    /// kreuzberg convention; downstream crates whose fixtures don't reference
    /// files (e.g. liter-llm, which uses pure mock-server fixtures) can leave
    /// the default in place — backends conditionally emit the setup only when
    /// fixtures actually need it.
    #[serde(default = "default_test_documents_dir")]
    pub test_documents_dir: String,
    /// Languages to generate e2e tests for. Defaults to top-level `languages` list.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Default function call configuration.
    pub call: CallConfig,
    /// Named additional call configurations for multi-function testing.
    /// Fixtures reference these via the `call` field, e.g. `"call": "embed"`.
    #[serde(default)]
    pub calls: HashMap<String, CallConfig>,
    /// Per-language package reference overrides.
    #[serde(default)]
    pub packages: HashMap<String, PackageRef>,
    /// Per-language formatter commands.
    #[serde(default)]
    pub format: HashMap<String, String>,
    /// Field path aliases: maps fixture field paths to actual API struct paths.
    /// E.g., "metadata.title" -> "metadata.document.title"
    /// Supports struct access (foo.bar), map access (foo[key]), direct fields.
    #[serde(default)]
    pub fields: HashMap<String, String>,
    /// Fields that are Optional/nullable in the return type.
    /// Rust generators use .as_deref().unwrap_or("") for strings, .is_some() for structs.
    #[serde(default)]
    pub fields_optional: HashSet<String>,
    /// Fields that are arrays/Vecs on the result type.
    /// When a fixture path like `json_ld.name` traverses an array field, the
    /// accessor adds `[0]` (or language equivalent) to index into the first element.
    #[serde(default)]
    pub fields_array: HashSet<String>,
    /// Fields where the accessor is a method call (appends `()`) rather than a field access.
    /// Rust-specific: Java always uses `()`, Python/PHP use field access.
    /// Listed as the full resolved field path (after alias resolution).
    /// E.g., `"metadata.format.excel"` means `.excel` should be emitted as `.excel()`.
    #[serde(default)]
    pub fields_method_calls: HashSet<String>,
    /// Known top-level fields on the result type.
    ///
    /// When non-empty, assertions whose resolved field path starts with a
    /// segment that is NOT in this set are emitted as comments (skipped)
    /// instead of executable assertions.  This prevents broken assertions
    /// when fixtures reference fields from a different operation (e.g.,
    /// `batch.completed_count` on a `ScrapeResult`).
    #[serde(default)]
    pub result_fields: HashSet<String>,
    /// Fixture categories excluded from cross-language e2e codegen.
    ///
    /// Fixtures whose resolved category matches an entry in this set are
    /// skipped by every per-language e2e generator — no test is emitted at
    /// all (no skip directive, no commented-out body). The fixture files stay
    /// on disk and remain available to Rust integration tests inside the
    /// consumer crate's own `tests/` directory.
    ///
    /// Use this to keep fixtures that exercise internal middleware (cache,
    /// proxy, budget, hooks, etc.) out of bindings whose public surface does
    /// not expose those layers.
    ///
    /// Example:
    /// ```toml
    /// [e2e]
    /// exclude_categories = ["cache", "proxy", "budget", "hooks"]
    /// ```
    #[serde(default)]
    pub exclude_categories: HashSet<String>,
    /// C FFI accessor type chain: maps `"{parent_snake_type}.{field}"` to the
    /// PascalCase return type name (without prefix).
    ///
    /// Used by the C e2e generator to emit chained FFI accessor calls for
    /// nested field paths. The root type is always `conversion_result`.
    ///
    /// Example:
    /// ```toml
    /// [e2e.fields_c_types]
    /// "conversion_result.metadata" = "HtmlMetadata"
    /// "html_metadata.document" = "DocumentMetadata"
    /// ```
    #[serde(default)]
    pub fields_c_types: HashMap<String, String>,
    /// Fields whose resolved type is an enum in the generated bindings.
    ///
    /// When a `contains` / `contains_all` / etc. assertion targets one of these
    /// fields, language generators that cannot call `.contains()` directly on an
    /// enum (e.g., Java) will emit a string-conversion call first.  For Java,
    /// the generated assertion calls `.getValue()` on the enum — the `@JsonValue`
    /// method that all alef-generated Java enums expose — to obtain the lowercase
    /// serde string before performing the string comparison.
    ///
    /// Both the raw fixture field path (before alias resolution) and the resolved
    /// path (after alias resolution via `[e2e.fields]`) are accepted, so you can
    /// use either form:
    ///
    /// ```toml
    /// # Raw fixture field:
    /// fields_enum = ["links[].link_type", "assets[].category"]
    /// # …or the resolved (aliased) field name:
    /// fields_enum = ["links[].link_type", "assets[].asset_category"]
    /// ```
    #[serde(default)]
    pub fields_enum: HashSet<String>,
    /// Dependency mode: `Local` (default) or `Registry`.
    /// Set at runtime via `--registry` CLI flag; not serialized from TOML.
    #[serde(skip)]
    pub dep_mode: DependencyMode,
    /// Registry-mode configuration from `[e2e.registry]`.
    #[serde(default)]
    pub registry: RegistryConfig,
}

impl E2eConfig {
    /// Resolve the call config for a fixture. Uses the named call if specified,
    /// otherwise falls back to the default `[e2e.call]`.
    pub fn resolve_call(&self, call_name: Option<&str>) -> &CallConfig {
        match call_name {
            Some(name) => self.calls.get(name).unwrap_or(&self.call),
            None => &self.call,
        }
    }

    /// Resolve the call config for a fixture, applying `select_when` auto-routing.
    ///
    /// When the fixture has an explicit `call` name, that named config is returned
    /// (same as [`resolve_call`]).  When the fixture has no explicit call, the method
    /// scans named calls for a [`SelectWhen`] condition that matches the fixture input
    /// and returns the first match.  If no condition matches, it falls back to the
    /// default `[e2e.call]`.
    pub fn resolve_call_for_fixture(&self, call_name: Option<&str>, fixture_input: &serde_json::Value) -> &CallConfig {
        if let Some(name) = call_name {
            return self.calls.get(name).unwrap_or(&self.call);
        }
        // Auto-route by select_when condition.
        for call_config in self.calls.values() {
            if let Some(SelectWhen::InputHas(key)) = &call_config.select_when {
                let val = fixture_input.get(key.as_str()).unwrap_or(&serde_json::Value::Null);
                if !val.is_null() {
                    return call_config;
                }
            }
        }
        &self.call
    }

    /// Resolve the effective package reference for a language.
    ///
    /// In registry mode, entries from `[e2e.registry.packages]` are merged on
    /// top of the base `[e2e.packages]` — registry overrides win for any field
    /// that is `Some`.
    pub fn resolve_package(&self, lang: &str) -> Option<PackageRef> {
        let base = self.packages.get(lang);
        if self.dep_mode == DependencyMode::Registry {
            let reg = self.registry.packages.get(lang);
            match (base, reg) {
                (Some(b), Some(r)) => Some(PackageRef {
                    name: r.name.clone().or_else(|| b.name.clone()),
                    path: r.path.clone().or_else(|| b.path.clone()),
                    module: r.module.clone().or_else(|| b.module.clone()),
                    version: r.version.clone().or_else(|| b.version.clone()),
                }),
                (None, Some(r)) => Some(r.clone()),
                (Some(b), None) => Some(b.clone()),
                (None, None) => None,
            }
        } else {
            base.cloned()
        }
    }

    /// Return the effective output directory: `registry.output` in registry
    /// mode, `output` otherwise.
    pub fn effective_output(&self) -> &str {
        if self.dep_mode == DependencyMode::Registry {
            &self.registry.output
        } else {
            &self.output
        }
    }

    /// Relative path from a backend's emission directory to the
    /// `test_documents_dir` at the repo root.
    ///
    /// `emission_depth` counts the number of additional `../` segments needed
    /// to reach `<output>/<lang>/` from where the file is being emitted:
    ///
    /// * `0` — emitted directly at `e2e/<lang>/` (e.g. dart, zig `build.zig`)
    /// * `1` — emitted at `e2e/<lang>/<sub>/` (e.g. ruby `spec/`, R `tests/`)
    /// * `2` — emitted at `e2e/<lang>/<sub1>/<sub2>/`
    ///
    /// The base prefix is two segments above `<output>/<lang>/` (i.e.
    /// `../../`), matching the canonical layout where `<output>` (default
    /// `"e2e"`) sits at the repo root next to the configured
    /// `test_documents_dir`.
    pub fn test_documents_relative_from(&self, emission_depth: usize) -> String {
        let mut up = String::from("../../");
        for _ in 0..emission_depth {
            up.push_str("../");
        }
        format!("{up}{}", self.test_documents_dir)
    }
}

fn default_fixtures_dir() -> String {
    "fixtures".to_string()
}

fn default_output_dir() -> String {
    "e2e".to_string()
}

fn default_test_documents_dir() -> String {
    "test_documents".to_string()
}

/// Hand-rolled `Default` so the `test_documents_dir` field receives its
/// `default_test_documents_dir()` value (`"test_documents"`) when callers use
/// `..Default::default()` to construct an `E2eConfig` literally rather than
/// going through `serde::Deserialize`. Without this, `derive(Default)` would
/// fall back to `String::default()` (i.e. the empty string), and any backend
/// computing `test_documents_relative_from(0)` would emit `"../../"` (no dir
/// component), breaking generated chdir hooks.
impl Default for E2eConfig {
    fn default() -> Self {
        Self {
            fixtures: default_fixtures_dir(),
            output: default_output_dir(),
            test_documents_dir: default_test_documents_dir(),
            languages: Vec::new(),
            call: CallConfig::default(),
            calls: HashMap::new(),
            packages: HashMap::new(),
            format: HashMap::new(),
            fields: HashMap::new(),
            fields_optional: HashSet::new(),
            fields_array: HashSet::new(),
            fields_method_calls: HashSet::new(),
            result_fields: HashSet::new(),
            exclude_categories: HashSet::new(),
            fields_c_types: HashMap::new(),
            fields_enum: HashSet::new(),
            dep_mode: DependencyMode::default(),
            registry: RegistryConfig::default(),
        }
    }
}

/// Configuration for the function call in each test.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CallConfig {
    /// The function name (alef applies language naming conventions).
    #[serde(default)]
    pub function: String,
    /// The module/package where the function lives.
    #[serde(default)]
    pub module: String,
    /// Variable name for the return value (default: "result").
    #[serde(default = "default_result_var")]
    pub result_var: String,
    /// Whether the function is async.
    #[serde(default)]
    pub r#async: bool,
    /// HTTP endpoint path for mock server routing (e.g., `"/v1/chat/completions"`).
    ///
    /// Required when fixtures use `mock_response`. The Rust e2e generator uses
    /// this to build the `MockRoute` that the mock server matches against.
    #[serde(default)]
    pub path: Option<String>,
    /// HTTP method for mock server routing (default: `"POST"`).
    ///
    /// Used together with `path` when building `MockRoute` entries.
    #[serde(default)]
    pub method: Option<String>,
    /// How fixture `input` fields map to function arguments.
    #[serde(default)]
    pub args: Vec<ArgMapping>,
    /// Per-language overrides for module/function/etc.
    #[serde(default)]
    pub overrides: HashMap<String, CallOverride>,
    /// Whether the function returns `Result<T, E>` in its native binding.
    /// Defaults to `true`. When `false`, generators that distinguish Result-returning
    /// from non-Result-returning calls (currently Rust) will skip the
    /// `.expect("should succeed")` unwrap and bind the raw return value directly.
    #[serde(default = "default_returns_result")]
    pub returns_result: bool,
    /// Whether the function returns only an error/unit — i.e., `Result<(), E>`.
    ///
    /// When combined with `returns_result = true`, Go generators emit `err := func()`
    /// (single return value) rather than `_, err := func()` (two return values).
    /// This is needed for functions like `validate_host` that return only `error` in Go.
    #[serde(default)]
    pub returns_void: bool,
    /// skip_languages
    #[serde(default)]
    pub skip_languages: Vec<String>,
    /// When `true`, the function returns a primitive (e.g. `String`, `bool`,
    /// `i32`) rather than a struct.  Generators that would otherwise emit
    /// `result.<field>` will fall back to the bare result variable.
    ///
    /// This is a property of the Rust core's return type and therefore identical
    /// across every binding — set it on the call, not in per-language overrides.
    /// The same flag is also accepted under `[e2e.calls.<name>.overrides.<lang>]`
    /// for backwards compatibility, but the call-level value takes precedence.
    #[serde(default)]
    pub result_is_simple: bool,
    /// When `true`, the function returns `Vec<T>` / `Array<T>`.  Generators that
    /// support per-element field assertions (rust, csharp) iterate or index into
    /// the result; the typescript codegen indexes `[0]` to mirror csharp.
    ///
    /// As with `result_is_simple`, this is a Rust-side property — set it on the
    /// call, not on per-language overrides. Per-language overrides remain
    /// supported for backwards compatibility.
    #[serde(default)]
    pub result_is_vec: bool,
    /// When `true` (combined with `result_is_simple`), the simple return is a
    /// slice/array (e.g., `Vec<String>` → `string[]` in TS).
    #[serde(default)]
    pub result_is_array: bool,
    /// When `true`, the function returns a raw byte array (`Vec<u8>` →
    /// `Uint8Array` / `[]byte` / `byte[]`).
    #[serde(default)]
    pub result_is_bytes: bool,
    /// Three-valued opt-in/out for streaming-virtual-field auto-detection.
    ///
    /// - `Some(true)`: force streaming semantics regardless of fixture shape.
    /// - `Some(false)`: disable streaming auto-detection — assertions referencing
    ///   fields like `chunks` / `chunks.length` / `tool_calls` / `finish_reason`
    ///   are treated as plain field accessors on the result, not streaming
    ///   adapters. Use this when your API has a `chunks` field that is a regular
    ///   list (not an async stream).
    /// - `None` (default): auto-detect — treat as streaming when either the
    ///   fixture provides a streaming `mock_response` or any assertion references
    ///   a hard-coded streaming-virtual-field name.
    #[serde(default)]
    pub streaming: Option<bool>,
    /// When `true`, the function returns `Option<T>`.
    #[serde(default)]
    pub result_is_option: bool,
    /// Automatic fixture-routing condition.
    ///
    /// When set, a fixture whose `call` field is `None` is routed to this named call config
    /// if the condition is satisfied.  This avoids the need to tag every fixture with
    /// `"call": "batch_scrape"` when the fixture shape already identifies the call.
    ///
    /// Example (`alef.toml`):
    /// ```toml
    /// [e2e.calls.batch_scrape]
    /// select_when = { input_has = "batch_urls" }
    /// ```
    #[serde(default)]
    pub select_when: Option<SelectWhen>,
}

fn default_result_var() -> String {
    "result".to_string()
}

fn default_returns_result() -> bool {
    false
}

/// Condition for auto-selecting a named call config when the fixture matches.
///
/// When a fixture does not specify `"call"`, the codegen normally uses the default
/// `[e2e.call]`.  A `SelectWhen` condition on a named call allows automatic routing
/// based on the fixture's input shape:
///
/// ```toml
/// [e2e.calls.batch_scrape]
/// select_when = { input_has = "batch_urls" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectWhen {
    /// Select this call when the fixture input contains the named key with a non-null value.
    InputHas(String),
}

/// Maps a fixture input field to a function argument.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgMapping {
    /// Argument name in the function signature.
    pub name: String,
    /// JSON field path in the fixture's `input` object.
    pub field: String,
    /// Type hint for code generation.
    #[serde(rename = "type", default = "default_arg_type")]
    pub arg_type: String,
    /// Whether this argument is optional.
    #[serde(default)]
    pub optional: bool,
    /// When `true`, the Rust codegen passes this argument by value (owned) rather than
    /// by reference. Use for `Vec<T>` parameters that do not accept `&Vec<T>`.
    #[serde(default)]
    pub owned: bool,
    /// For `json_object` args targeting `&[T]` Rust parameters, set to the element type
    /// (e.g. `"f32"`, `"String"`) so the codegen emits `Vec<element_type>` annotation.
    #[serde(default)]
    pub element_type: Option<String>,
    /// Override the Go slice element type for `json_object` array args.
    ///
    /// When set, the Go e2e codegen uses this as the element type instead of the default
    /// derived from `element_type`. Use Go-idiomatic type names including the import alias
    /// prefix where needed, e.g. `"kreuzberg.BatchBytesItem"` or `"string"`.
    #[serde(default)]
    pub go_type: Option<String>,
}

fn default_arg_type() -> String {
    "string".to_string()
}

/// Per-language override for function call configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CallOverride {
    /// Override the module/import path.
    #[serde(default)]
    pub module: Option<String>,
    /// Override the function name.
    #[serde(default)]
    pub function: Option<String>,
    /// Maps canonical argument names to language-specific argument names.
    ///
    /// Used when a language binding uses a different parameter name than the
    /// canonical `args` list in `CallConfig`. For example, if the canonical
    /// arg name is `doc` but the Python binding uses `html`, specify:
    ///
    /// ```toml
    /// [e2e.call.overrides.python]
    /// arg_name_map = { doc = "html" }
    /// ```
    ///
    /// The key is the canonical name (from `args[].name`) and the value is the
    /// name to use when emitting the keyword argument in generated tests.
    #[serde(default)]
    pub arg_name_map: HashMap<String, String>,
    /// Override the crate name (Rust only).
    #[serde(default)]
    pub crate_name: Option<String>,
    /// Override the class name (Java/C# only).
    #[serde(default)]
    pub class: Option<String>,
    /// Import alias (Go only, e.g., `htmd`).
    #[serde(default)]
    pub alias: Option<String>,
    /// C header file name (C only).
    #[serde(default)]
    pub header: Option<String>,
    /// FFI symbol prefix (C only).
    #[serde(default)]
    pub prefix: Option<String>,
    /// For json_object args: the constructor to use instead of raw dict/object.
    /// E.g., "ConversionOptions" — generates `ConversionOptions(**options)` in Python,
    /// `new ConversionOptions(options)` in TypeScript.
    #[serde(default)]
    pub options_type: Option<String>,
    /// How to pass json_object args: "kwargs" (default), "dict", "json", or "from_json".
    ///
    /// - `"kwargs"`: construct `OptionsType(key=val, ...)` (requires `options_type`).
    /// - `"dict"`: pass as a plain dict/object literal `{"key": "val"}`.
    /// - `"json"`: pass via `json.loads('...')` / `JSON.parse('...')`.
    /// - `"from_json"`: call `OptionsType.from_json('...')` (Python only, PyO3 native types).
    #[serde(default)]
    pub options_via: Option<String>,
    /// Module to import `options_type` from when `options_via = "from_json"`.
    ///
    /// When set, a separate `from {from_json_module} import {options_type}` line
    /// is emitted instead of including the type in the main module import.
    /// E.g., `"liter_llm._internal_bindings"` for PyO3 native types.
    #[serde(default)]
    pub from_json_module: Option<String>,
    /// Override whether the call is async for this language.
    ///
    /// When set, takes precedence over the call-level `async` flag.
    /// Useful when a language binding uses a different async model — for example,
    /// a Python binding that returns a sync iterator from a function marked
    /// `async = true` at the call level.
    #[serde(default, rename = "async")]
    pub r#async: Option<bool>,
    /// Maps fixture option field names to their enum type names.
    /// E.g., `{"headingStyle": "HeadingStyle", "codeBlockStyle": "CodeBlockStyle"}`.
    /// The generator imports these types and maps string values to enum constants.
    #[serde(default)]
    pub enum_fields: HashMap<String, String>,
    /// Maps result-type field names to their enum type names for assertion routing.
    /// Per-call so e.g. `BatchObject.status` (enum) and `ResponseObject.status` (string)
    /// can be disambiguated.
    #[serde(default)]
    pub assert_enum_fields: HashMap<String, String>,
    /// Module to import enum types from (if different from the main module).
    /// E.g., "html_to_markdown._html_to_markdown" for PyO3 native enums.
    #[serde(default)]
    pub enum_module: Option<String>,
    /// Maps nested fixture object field names to their C# type names.
    /// Used to generate `JsonSerializer.Deserialize<NestedType>(...)` for nested objects.
    /// E.g., `{"preprocessing": "PreprocessingOptions"}`.
    #[serde(default)]
    pub nested_types: HashMap<String, String>,
    /// When `false`, nested config builder results are passed directly to builder methods
    /// without wrapping in `Optional.of(...)`. Set to `false` for bindings where nested
    /// option types are non-optional (e.g., html-to-markdown Java).
    /// Defaults to `true` for backward compatibility.
    #[serde(default = "default_true")]
    pub nested_types_optional: bool,
    /// When `true`, the function returns a simple type (e.g., `String`) rather
    /// than a struct.  Generators that would normally emit `result.content`
    /// (or equivalent field access) will use the result variable directly.
    #[serde(default)]
    pub result_is_simple: bool,
    /// When `true` (and combined with `result_is_simple`), the simple result is
    /// a slice/array type (e.g., `[]string` in Go, `Vec<String>` in Rust).
    /// The Go generator uses `strings.Join(value, " ")` for `contains` assertions
    /// instead of `string(value)`.
    #[serde(default)]
    pub result_is_array: bool,
    /// When `true`, the function returns `Vec<T>` rather than a single value.
    /// Field-path assertions are emitted as `.iter().all(|r| <accessor>)` so
    /// every element is checked. (Rust generator.)
    #[serde(default)]
    pub result_is_vec: bool,
    /// When `true`, the function returns a raw byte array (e.g., `byte[]` in Java,
    /// `[]byte` in Go). Used by generators to select the correct length accessor
    /// (field `.length` vs method `.length()`).
    #[serde(default)]
    pub result_is_bytes: bool,
    /// When `true`, the function returns `Option<T>`. The result is unwrapped
    /// before any non-`is_none`/`is_some` assertion runs; `is_empty`/`not_empty`
    /// assertions map to `is_none()`/`is_some()`. (Rust generator.)
    #[serde(default)]
    pub result_is_option: bool,
    /// When `true`, the R generator emits the call result directly without wrapping
    /// in `jsonlite::fromJSON()`. Use when the R binding already returns a native
    /// R list (`Robj`) rather than a JSON string. Field-path assertions still use
    /// `result$field` accessor syntax (i.e. `result_is_simple` behaviour is NOT
    /// implied — only the JSON parse wrapper is suppressed). (R generator only.)
    #[serde(default)]
    pub result_is_r_list: bool,
    /// When `true`, the Zig generator treats the result as a `[]u8` JSON string
    /// representing a struct value (e.g., `ExtractionResult` serialized via the
    /// FFI `_to_json` helper). The generator parses the JSON with
    /// `std.json.parseFromSlice(std.json.Value, ...)` before emitting field
    /// assertions, traversing the dynamic JSON object for each field path.
    /// (Zig generator only.)
    #[serde(default)]
    pub result_is_json_struct: bool,
    /// When `true`, the Rust generator wraps the `json_object` argument expression
    /// in `Some(...).clone()` to match an owned `Option<T>` parameter slot rather
    /// than passing `&options`. (Rust generator only.)
    #[serde(default)]
    pub wrap_options_in_some: bool,
    /// Trailing positional arguments appended verbatim after the configured
    /// `args`. Used when the target function takes additional positional slots
    /// (e.g. visitor) the fixture cannot supply directly. (Rust generator only.)
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Per-rust override of the call-level `returns_result`. When set, takes
    /// precedence over `CallConfig.returns_result` for the Rust generator only.
    /// Useful when one binding is fallible while others are not.
    #[serde(default)]
    pub returns_result: Option<bool>,
    /// Maps handle config field names to their Python type constructor names.
    ///
    /// When the handle config object contains a nested dict-valued field, the
    /// generator will wrap it in the specified type using keyword arguments.
    /// E.g., `{"browser": "BrowserConfig"}` generates `BrowserConfig(mode="auto")`
    /// instead of `{"mode": "auto"}`.
    #[serde(default)]
    pub handle_nested_types: HashMap<String, String>,
    /// Handle config fields whose type constructor takes a single dict argument
    /// instead of keyword arguments.
    ///
    /// E.g., `["auth"]` means `AuthConfig({"type": "basic", ...})` instead of
    /// `AuthConfig(type="basic", ...)`.
    #[serde(default)]
    pub handle_dict_types: HashSet<String>,
    /// Elixir struct module name for the handle config argument.
    ///
    /// When set, the generated Elixir handle config uses struct literal syntax
    /// (`%Module.StructType{key: val}`) instead of a plain string-keyed map.
    /// Rustler `NifStruct` requires a proper Elixir struct — plain maps are rejected.
    ///
    /// E.g., `"CrawlConfig"` generates `%Kreuzcrawl.CrawlConfig{download_assets: true}`.
    #[serde(default)]
    pub handle_struct_type: Option<String>,
    /// Handle config fields whose list values are Elixir atoms (Rustler NifUnitEnum).
    ///
    /// When a config field is a `Vec<EnumType>` in Rust, the Elixir side must pass
    /// a list of atoms (e.g., `[:image, :document]`) not strings (`["image"]`).
    /// List the field names here so the generator emits atom literals instead of strings.
    ///
    /// E.g., `["asset_types"]` generates `asset_types: [:image]` instead of `["image"]`.
    #[serde(default)]
    pub handle_atom_list_fields: HashSet<String>,
    /// WASM config class name for handle args (WASM generator only).
    ///
    /// When set, handle args are constructed using `ConfigType.default()` + setters
    /// instead of passing a plain JS object (which fails `_assertClass` validation).
    ///
    /// E.g., `"WasmCrawlConfig"` generates:
    /// ```js
    /// const engineConfig = WasmCrawlConfig.default();
    /// engineConfig.maxDepth = 1;
    /// const engine = createEngine(engineConfig);
    /// ```
    #[serde(default)]
    pub handle_config_type: Option<String>,
    /// PHP client factory method name (PHP generator only).
    ///
    /// When set, the generated PHP test instantiates a client via
    /// `ClassName::factory_method('test-key')` and calls methods on the instance
    /// instead of using static facade calls.
    ///
    /// E.g., `"createClient"` generates:
    /// ```php
    /// $client = LiterLlm::createClient('test-key');
    /// $result = $client->chat($request);
    /// ```
    #[serde(default)]
    pub php_client_factory: Option<String>,
    /// Client factory function name for instance-method languages (WASM, etc.).
    ///
    /// When set, the generated test imports this function, creates a client,
    /// and calls API methods on the instance instead of as top-level functions.
    ///
    /// E.g., `"createClient"` generates:
    /// ```typescript
    /// import { createClient } from 'pkg';
    /// const client = createClient('test-key');
    /// const result = await client.chat(request);
    /// ```
    #[serde(default)]
    pub client_factory: Option<String>,
    /// Verbatim trailing arguments appended after the fixed `("test-key", ...)` pair
    /// when calling the `client_factory` function.
    ///
    /// Use this when the factory function takes additional positional parameters
    /// beyond the API key and optional base URL that the generator would otherwise
    /// emit.  Each element is emitted verbatim, separated by `, `.
    ///
    /// Example — Gleam `create_client` takes five positional arguments:
    /// `(api_key, base_url, timeout_secs, max_retries, model_hint)`.  Set:
    /// ```toml
    /// [e2e.call.overrides.gleam]
    /// client_factory = "create_client"
    /// client_factory_trailing_args = ["option.None", "option.None", "option.None"]
    /// ```
    /// to produce `create_client("test-key", option.Some(url), option.None, option.None, option.None)`.
    #[serde(default)]
    pub client_factory_trailing_args: Vec<String>,
    /// Fields on the options object that require `BigInt()` wrapping (WASM only).
    ///
    /// `wasm_bindgen` maps Rust `u64`/`i64` to JavaScript `BigInt`. Numeric
    /// values assigned to these setters must be wrapped with `BigInt(n)`.
    ///
    /// List camelCase field names, e.g.:
    /// ```toml
    /// [e2e.call.overrides.wasm]
    /// bigint_fields = ["maxTokens", "seed"]
    /// ```
    #[serde(default)]
    pub bigint_fields: Vec<String>,
    /// Static CLI arguments appended to every invocation (brew/CLI generator only).
    ///
    /// E.g., `["--format", "json"]` appends `--format json` to every CLI call.
    #[serde(default)]
    pub cli_args: Vec<String>,
    /// Maps fixture config field names to CLI flag names (brew/CLI generator only).
    ///
    /// E.g., `{"output_format": "--format"}` generates `--format <value>` from
    /// the fixture's `output_format` input field.
    #[serde(default)]
    pub cli_flags: HashMap<String, String>,
    /// C FFI opaque result type name (C only).
    ///
    /// The PascalCase name of the result struct, without the prefix.
    /// E.g., `"ChatCompletionResponse"` for `LiterllmChatCompletionResponse*`.
    /// If not set, defaults to the function name in PascalCase.
    #[serde(default)]
    pub result_type: Option<String>,
    /// Override the argument order for this language binding.
    ///
    /// Lists argument names from `args` in the order they should be passed
    /// to the target function. Useful when a language binding reorders parameters
    /// relative to the canonical `args` list in `CallConfig`.
    ///
    /// E.g., if `args = [path, mime_type, config]` but the Node.js binding
    /// takes `(path, config, mime_type?)`, specify:
    /// ```toml
    /// [e2e.call.overrides.node]
    /// arg_order = ["path", "config", "mime_type"]
    /// ```
    #[serde(default)]
    pub arg_order: Vec<String>,
    /// When `true`, `json_object` args with an `options_type` are passed as a
    /// pointer (`*OptionsType`) rather than a value.  Use for Go bindings where
    /// the options parameter is `*ConversionOptions` (nil-able pointer) rather
    /// than a plain struct.
    ///
    /// Absent options are passed as `nil`; present options are unmarshalled into
    /// a local variable and passed as `&optionsVar`.
    #[serde(default)]
    pub options_ptr: bool,
    /// Alternative function name to use when the fixture includes a `visitor`.
    ///
    /// Some bindings expose two entry points: `Convert(html, opts)` for the
    /// plain case and `ConvertWithVisitor(html, opts, visitor)` when a visitor
    /// is involved.  Set this to the visitor-accepting function name so the
    /// generator can pick the right symbol automatically.
    ///
    /// E.g., `"ConvertWithVisitor"` makes the Go generator emit:
    /// ```go
    /// result, err := htmd.ConvertWithVisitor(html, nil, visitor)
    /// ```
    /// instead of `htmd.Convert(html, nil, visitor)` (which would not compile).
    #[serde(default)]
    pub visitor_function: Option<String>,
    /// Rust trait names to import when `client_factory` is set (Rust generator only).
    ///
    /// When `client_factory` is set, the generated test creates a client object and
    /// calls methods on it. Those methods are defined on traits (e.g. `LlmClient`,
    /// `FileClient`) that must be in scope. List the trait names here and the Rust
    /// generator will emit `use {module}::{trait_name};` for each.
    ///
    /// E.g.:
    /// ```toml
    /// [e2e.call.overrides.rust]
    /// client_factory = "create_client"
    /// trait_imports = ["LlmClient", "FileClient", "BatchClient", "ResponseClient"]
    /// ```
    #[serde(default)]
    pub trait_imports: Vec<String>,
    /// Raw C return type, used verbatim instead of `{PREFIX}Type*` (C only).
    ///
    /// Valid values: `"char*"`, `"int32_t"`, `"uintptr_t"`.
    /// When set, the C generator skips options handle construction and uses the
    /// raw type directly. Free logic is adjusted accordingly.
    #[serde(default)]
    pub raw_c_result_type: Option<String>,
    /// Free function for raw `char*` C results (C only).
    ///
    /// Defaults to `{prefix}_free_string` when unset and `raw_c_result_type == "char*"`.
    #[serde(default)]
    pub c_free_fn: Option<String>,
    /// C FFI engine factory pattern (C only).
    ///
    /// When set, the C generator wraps each test call in a
    /// `{prefix}_create_engine(config)` / `{prefix}_crawl_engine_handle_free(engine)`
    /// prologue/epilogue using the named config type as the "arg 0" handle type.
    ///
    /// The value is the PascalCase config type name (without prefix), e.g.
    /// `"CrawlConfig"`. The generator will emit:
    /// ```c
    /// KCRAWLCrawlConfig* config_handle = kcrawl_crawl_config_from_json("{json}");
    /// KCRAWLCrawlEngineHandle* engine = kcrawl_create_engine(config_handle);
    /// kcrawl_crawl_config_free(config_handle);
    /// KCRAWLScrapeResult* result = kcrawl_scrape(engine, url);
    /// // ... assertions ...
    /// kcrawl_scrape_result_free(result);
    /// kcrawl_crawl_engine_handle_free(engine);
    /// ```
    #[serde(default)]
    pub c_engine_factory: Option<String>,
    /// Fields in a `json_object` arg that must be wrapped in `java.nio.file.Path.of()`
    /// (Java generator only).
    ///
    /// E.g., `["cache_dir"]` wraps the string value of `cache_dir` so the builder
    /// receives `java.nio.file.Path.of("/tmp/dir")` instead of a plain string.
    #[serde(default)]
    pub path_fields: Vec<String>,
    /// Trait name for the visitor pattern (Rust e2e tests only).
    ///
    /// When a fixture declares a `visitor` block, the Rust e2e generator emits
    /// `impl <trait_name> for _TestVisitor { ... }` and imports the trait from
    /// `{module}::visitor`. When unset, no visitor block is emitted and fixtures
    /// that declare a visitor will cause a codegen error.
    ///
    /// E.g., `"HtmlVisitor"` generates:
    /// ```rust,ignore
    /// use html_to_markdown_rs::visitor::{HtmlVisitor, NodeContext, VisitResult};
    /// // ...
    /// impl HtmlVisitor for _TestVisitor { ... }
    /// ```
    #[serde(default)]
    pub visitor_trait: Option<String>,
    /// Maps result field paths to their wasm-bindgen enum class names.
    ///
    /// wasm-bindgen exposes Rust enums as numeric discriminants in JavaScript
    /// (`WasmFinishReason.Stop === 0`), not string variants. When an `equals`
    /// assertion targets a field listed here, the WASM generator emits
    /// `expect(result.choices[0].finishReason).toBe(WasmFinishReason.Stop)`
    /// instead of attempting `(value ?? "").trim()`.
    ///
    /// The fixture's expected string value is converted to PascalCase to look
    /// up the variant (e.g. `"tool_calls"` -> `ToolCalls`).
    ///
    /// Example:
    /// ```toml
    /// [e2e.calls.chat.overrides.wasm]
    /// result_enum_fields = { "choices[0].finish_reason" = "WasmFinishReason", "status" = "WasmBatchStatus" }
    /// ```
    #[serde(default)]
    pub result_enum_fields: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

/// Per-language package reference configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackageRef {
    /// Package/crate/gem/module name.
    #[serde(default)]
    pub name: Option<String>,
    /// Relative path from e2e/{lang}/ to the package.
    #[serde(default)]
    pub path: Option<String>,
    /// Go module path.
    #[serde(default)]
    pub module: Option<String>,
    /// Package version (e.g., for go.mod require directives).
    #[serde(default)]
    pub version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_e2e_with_test_documents(dir: &str) -> E2eConfig {
        E2eConfig {
            test_documents_dir: dir.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_documents_dir_default_is_test_documents() {
        let cfg: E2eConfig = toml::from_str("[call]\nfunction = \"f\"\n").expect("minimal TOML must deserialize");
        assert_eq!(cfg.test_documents_dir, "test_documents");
    }

    #[test]
    fn test_documents_dir_explicit_override_wins() {
        let cfg: E2eConfig = toml::from_str("test_documents_dir = \"fixture_files\"\n[call]\nfunction = \"f\"\n")
            .expect("explicit override must deserialize");
        assert_eq!(cfg.test_documents_dir, "fixture_files");
    }

    #[test]
    fn test_documents_relative_from_at_lang_root_returns_two_dots_up() {
        let cfg = empty_e2e_with_test_documents("test_documents");
        assert_eq!(cfg.test_documents_relative_from(0), "../../test_documents");
    }

    #[test]
    fn test_documents_relative_from_at_spec_depth_returns_three_dots_up() {
        let cfg = empty_e2e_with_test_documents("test_documents");
        assert_eq!(cfg.test_documents_relative_from(1), "../../../test_documents");
    }

    #[test]
    fn test_documents_relative_from_at_two_subdirs_deep_returns_four_dots_up() {
        let cfg = empty_e2e_with_test_documents("test_documents");
        assert_eq!(cfg.test_documents_relative_from(2), "../../../../test_documents");
    }

    #[test]
    fn test_documents_relative_uses_configured_dir_name() {
        let cfg = empty_e2e_with_test_documents("fixture_files");
        assert_eq!(cfg.test_documents_relative_from(0), "../../fixture_files");
        assert_eq!(cfg.test_documents_relative_from(1), "../../../fixture_files");
    }
}
