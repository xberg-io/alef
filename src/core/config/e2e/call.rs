use super::SelectWhen;
use super::defaults::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Configuration for the function call in each test.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct CallConfig {
    /// Per-call override for `result_fields`.
    ///
    /// When non-empty, this set replaces the global `[e2e].result_fields` for
    /// fixtures routed to this call.  Use this when different API functions return
    /// differently-shaped structs so each call can gate its own field set.
    ///
    /// Example:
    /// ```toml
    /// [e2e.calls.crawl]
    /// result_fields = ["pages", "final_url", "stayed_on_domain"]
    /// ```
    #[serde(default)]
    pub result_fields: HashSet<String>,
    /// Per-call override for `[e2e].fields` alias map.
    ///
    /// When non-empty, replaces (not merges with) the global `fields` map for
    /// fixtures routed to this call.
    #[serde(default)]
    pub fields: HashMap<String, String>,
    /// Per-call override for `[e2e].fields_optional`.
    #[serde(default)]
    pub fields_optional: HashSet<String>,
    /// Per-call override for `[e2e].fields_array`.
    #[serde(default)]
    pub fields_array: HashSet<String>,
    /// Per-call override for `[e2e].fields_method_calls`.
    #[serde(default)]
    pub fields_method_calls: HashSet<String>,
    /// Per-call override for `[e2e].fields_enum`.
    #[serde(default)]
    pub fields_enum: HashSet<String>,
    /// Per-call override for `[e2e].fields_display_as_text`.
    #[serde(default)]
    pub fields_display_as_text: HashSet<String>,
    /// Per-call override for `[e2e].fields_c_types`.
    #[serde(default)]
    pub fields_c_types: HashMap<String, String>,
    /// Assertion recipes enabled for all fixtures routed to this call.
    ///
    /// Recipes intentionally gate domain-shaped assertion shortcuts such as
    /// `embeddings_*`, `keywords`, tree parser helpers, and streaming
    /// pseudo-fields. Generic recursive field/text/JSON assertions do not
    /// require a recipe.
    #[serde(default)]
    pub assertion_recipes: HashSet<String>,
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
    /// Per-backend exclusion: backends listed here will emit a skip comment instead of
    /// a failing test, with the documented reason that the call is unsupported on that
    /// backend (e.g., "brew: interact requires complex JSON serialization of PageAction enums").
    ///
    /// Use this when a backend structurally cannot support a call (e.g., CLI-based
    /// backends that lack certain features). Unlike `skip_languages`, unsupported calls
    /// are documented in the generated test files with rationale comments.
    ///
    /// Example:
    /// ```toml
    /// [e2e.calls.interact]
    /// unsupported_in = { brew = "interact requires serializing Vec<PageAction> enums to JSON CLI args" }
    /// ```
    #[serde(default)]
    pub unsupported_in: HashMap<String, String>,
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
    ///   neutral stream fields like `stream.items` / `stream.items.length` are
    ///   treated as plain field accessors on the result, not streaming adapters.
    /// - `None` (default): auto-detect — treat as streaming when either the
    ///   fixture provides a streaming `mock_response` or any assertion references
    ///   a neutral streaming-virtual-field name.
    #[serde(default)]
    pub streaming: Option<StreamingConfig>,
    /// When `true`, the function returns `Option<T>`.
    #[serde(default)]
    pub result_is_option: bool,
    /// When `true` (combined with `result_is_simple` + `result_is_array`),
    /// signals that the result is `Vec<String>` returned to the host as a
    /// native string array (e.g., Swift `[String]`) rather than an opaque
    /// `RustVec<RustString>` requiring `.asStr().toString()` per element.
    ///
    /// Generators that emit per-element coercion for opaque RustVec types
    /// (currently Swift) drop the coercion and operate on the elements as
    /// native strings when this flag is set.
    #[serde(default)]
    pub result_element_is_string: bool,
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
    /// Call-level constructor type for `json_object` config args.
    ///
    /// This is the type of the function's config parameter (e.g. `EmbeddingConfig`
    /// vs `ExtractionConfig`) and is therefore identical across every binding — set
    /// it on the call, not in per-language overrides. Per-language overrides
    /// (`[e2e.calls.<name>.overrides.<lang>].options_type`) still take precedence
    /// when a binding exposes a language-specific wrapper type (e.g. `JsExtractionConfig`).
    #[serde(default)]
    pub options_type: Option<String>,
}

impl CallConfig {
    /// Effective streaming opt-in/out flag, preserving the legacy
    /// `streaming = true/false` behavior while allowing
    /// `streaming = { item_type = "Event" }` recipes.
    pub fn streaming_enabled(&self) -> Option<bool> {
        self.streaming.as_ref().and_then(StreamingConfig::enabled)
    }

    /// Explicit stream item type configured on the call recipe, if any.
    pub fn streaming_item_type(&self) -> Option<&str> {
        self.streaming.as_ref().and_then(StreamingConfig::item_type)
    }
}

/// E2E streaming call recipe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(untagged)]
pub enum StreamingConfig {
    /// Legacy boolean form: `streaming = true` / `streaming = false`.
    Enabled(bool),
    /// Recipe form: `streaming = { item_type = "Event" }` or
    /// `[crates.e2e.call.streaming] item_type = "Event"`.
    Recipe(StreamingRecipe),
}

impl StreamingConfig {
    fn enabled(&self) -> Option<bool> {
        match self {
            Self::Enabled(value) => Some(*value),
            Self::Recipe(recipe) => recipe.enabled,
        }
    }

    fn item_type(&self) -> Option<&str> {
        match self {
            Self::Enabled(_) => None,
            Self::Recipe(recipe) => recipe.item_type.as_deref().filter(|value| !value.is_empty()),
        }
    }
}

/// Structured streaming recipe options.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct StreamingRecipe {
    /// Optional opt-in/out equivalent to the legacy boolean form.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Concrete tagged-union stream item type used by event assertions.
    #[serde(default)]
    pub item_type: Option<String>,
}

/// Maps a fixture input field to a function argument.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArgMapping {
    /// Argument name in the function signature.
    pub name: String,
    /// JSON field path in the fixture's `input` object.
    pub field: String,
    /// Type hint for code generation.
    #[serde(rename = "type", alias = "arg_type", default = "default_arg_type")]
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
    /// prefix where needed, e.g. `"sample_core.BatchBytesItem"` or `"string"`.
    #[serde(default)]
    pub go_type: Option<String>,
    /// When `true`, the Rust e2e codegen converts `Vec<String>` to `Vec<&str>` before the
    /// call, enabling the slice to coerce to `&[&str]` as required by the Rust core.
    ///
    /// Use when `arg_type = "json_object"` and `element_type = "String"` and the target
    /// Rust function parameter is `&[&str]` (not `&[String]`).
    #[serde(default)]
    pub vec_inner_is_ref: bool,
    /// Trait name for `test_backend` arg type (e.g., `"DocumentExtractor"`, `"OcrBackend"`).
    ///
    /// When `arg_type = "test_backend"`, this field specifies which trait's bridge
    /// codegen should be used to create the test stub instance. Only used when
    /// `arg_type = "test_backend"`.
    #[serde(default, alias = "trait")]
    pub trait_name: Option<String>,
}

/// Per-language override for function call configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct CallOverride {
    /// Override the module/import path.
    #[serde(default)]
    pub module: Option<String>,
    /// Override the function name.
    #[serde(default)]
    pub function: Option<String>,
    /// Assertion recipes enabled only for this language.
    #[serde(default)]
    pub assertion_recipes: HashSet<String>,
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
    /// Emit configured arguments as a single Elixir keyword-list call.
    ///
    /// Use this for Elixir public facades that collapse all NIF parameters into
    /// `opts \\ []`, such as `extract(input: ..., config: ...)`.
    #[serde(default)]
    pub keyword_args: bool,
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
    /// E.g., `"sample_llm._internal_bindings"` for PyO3 native types.
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
    /// E.g., "sample_markdown._sample_markdown" for PyO3 native enums.
    #[serde(default)]
    pub enum_module: Option<String>,
    /// Maps nested fixture object field names to their C# type names.
    /// Used to generate `JsonSerializer.Deserialize<NestedType>(...)` for nested objects.
    /// E.g., `{"preprocessing": "PreprocessingOptions"}`.
    #[serde(default)]
    pub nested_types: HashMap<String, String>,
    /// When `false`, nested config builder results are passed directly to builder methods
    /// without wrapping in `Optional.of(...)`. Set to `false` for bindings where nested
    /// option types are non-optional (e.g., sample-markdown Java).
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
    /// E.g., `"CrawlConfig"` generates `%SampleCrawler.CrawlConfig{download_assets: true}`.
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
    /// $client = SampleLlm::createClient('test-key');
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
    /// E.g., `"ChatCompletionResponse"` for `SamplellmChatCompletionResponse*`.
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
    /// use sample_markdown_rs::visitor::{HtmlVisitor, NodeContext, VisitResult};
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
    /// When `true`, indicates that the result is a pointer type (e.g., `*string` in Go,
    /// `*T` in Rust). The Go codegen will dereference it. When `false` (Go only), the
    /// result is a value type and should not be dereferenced.
    ///
    /// Used to distinguish between functions that return `(value, error)` where value
    /// is a scalar (string, uint, bool) as-is vs. those that return pointers.
    /// Defaults to `true` for backward compatibility with existing fixtures.
    #[serde(default = "default_true")]
    pub result_is_pointer: bool,
    /// Per-language override mirroring `CallConfig.result_element_is_string`.
    ///
    /// Set this on a per-language override when only one host's binding exposes
    /// the result as a native string array; otherwise prefer the call-level flag.
    #[serde(default)]
    pub result_element_is_string: bool,
    /// Maps array-typed result fields to the method name on each element that
    /// yields a string used in `contains` / `contains_all` assertions.
    ///
    /// Used when the array element is an opaque struct (e.g., a swift-bridge
    /// `type X;` declaration) and the element's "name" accessor is not the
    /// default `as_str` — for instance, `StructureItem` exposes `kind() -> String`
    /// instead of `as_str()`. The codegen consults this map when emitting
    /// `.map { $0.<accessor>().toString() }` so the closure compiles.
    ///
    /// Example:
    /// ```toml
    /// [e2e.call.overrides.swift]
    /// result_field_accessor = { "structure" = "kind" }
    /// ```
    #[serde(default)]
    pub result_field_accessor: HashMap<String, String>,
    /// Argument indices (0-based) that should be passed without labels in Swift
    /// (i.e., using `(_` parameter syntax instead of `name:`).
    ///
    /// Swift allows unnamed first parameters: `func f(_ x: Int)` vs `func f(x: Int)`.
    /// When the generated test call should match this signature, list the indices here.
    ///
    /// E.g., `[0]` for a single unnamed first parameter:
    /// ```toml
    /// [e2e.call.overrides.swift]
    /// unnamed_arg_indices = [0]
    /// ```
    /// generates `f(contentVec)` instead of `f(content: contentVec)`.
    #[serde(default)]
    pub unnamed_arg_indices: Vec<usize>,
}
