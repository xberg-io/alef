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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct E2eConfig {
    /// Directory containing fixture JSON files (default: "fixtures").
    #[serde(default = "default_fixtures_dir")]
    pub fixtures: String,
    /// Output directory for generated e2e test projects (default: "e2e").
    #[serde(default = "default_output_dir")]
    pub output: String,
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
    /// Known top-level fields on the result type.
    ///
    /// When non-empty, assertions whose resolved field path starts with a
    /// segment that is NOT in this set are emitted as comments (skipped)
    /// instead of executable assertions.  This prevents broken assertions
    /// when fixtures reference fields from a different operation (e.g.,
    /// `batch.completed_count` on a `ScrapeResult`).
    #[serde(default)]
    pub result_fields: HashSet<String>,
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
}

fn default_fixtures_dir() -> String {
    "fixtures".to_string()
}

fn default_output_dir() -> String {
    "e2e".to_string()
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
}

fn default_result_var() -> String {
    "result".to_string()
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
    /// How to pass json_object args: "kwargs" (default), "dict", or "json".
    ///
    /// - `"kwargs"`: construct `OptionsType(key=val, ...)` (requires `options_type`).
    /// - `"dict"`: pass as a plain dict/object literal `{"key": "val"}`.
    /// - `"json"`: pass via `json.loads('...')` / `JSON.parse('...')`.
    #[serde(default)]
    pub options_via: Option<String>,
    /// Maps fixture option field names to their enum type names.
    /// E.g., `{"headingStyle": "HeadingStyle", "codeBlockStyle": "CodeBlockStyle"}`.
    /// The generator imports these types and maps string values to enum constants.
    #[serde(default)]
    pub enum_fields: HashMap<String, String>,
    /// Module to import enum types from (if different from the main module).
    /// E.g., "html_to_markdown._html_to_markdown" for PyO3 native enums.
    #[serde(default)]
    pub enum_module: Option<String>,
    /// When `true`, the function returns a simple type (e.g., `String`) rather
    /// than a struct.  Generators that would normally emit `result.content`
    /// (or equivalent field access) will use the result variable directly.
    #[serde(default)]
    pub result_is_simple: bool,
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
