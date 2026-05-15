use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::extras::Language;

/// Configuration for a single capsule type entry in `PythonConfig::capsule_types`.
///
/// Supports two TOML forms via `#[serde(untagged)]`:
///
/// - String: `Language = "tree_sitter.Language"` → capsule round-trip via `into_raw()`
/// - Struct: `Parser = { python_type = "tree_sitter.Parser", construct_from = "Language" }` → Python-side construction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum CapsuleTypeConfig {
    /// Capsule round-trip: the Rust type exposes `into_raw()` returning a raw pointer.
    /// The generated code calls `PyCapsule_New(value.into_raw(), capsule_name, None)` on return,
    /// and `PyCapsule_GetPointer` + `from_raw()` on input.
    ///
    /// Value is the fully-qualified Python capsule name (e.g. `"tree_sitter.Language"`).
    Capsule(String),
    /// Python-side construction: the type does not have a direct `into_raw()`.
    /// Instead, the generated code constructs the Python type by calling a Python factory
    /// (e.g. `tree_sitter.Parser(language)`) where `language` is a bound capsule argument.
    ConstructFrom {
        /// The fully-qualified Python type to import and call (e.g. `"tree_sitter.Parser"`).
        python_type: String,
        /// The capsule-type argument name to pass to the Python constructor.
        /// Must be one of the other capsule-type entries (e.g. `"Language"`).
        construct_from: String,
    },
}

impl CapsuleTypeConfig {
    /// Returns the Python type string (dotted path) for this config entry.
    pub fn python_type(&self) -> &str {
        match self {
            Self::Capsule(name) => name,
            Self::ConstructFrom { python_type, .. } => python_type,
        }
    }

    /// Returns the `construct_from` dependency type name, if this is a `ConstructFrom` entry.
    pub fn construct_from(&self) -> Option<&str> {
        match self {
            Self::ConstructFrom { construct_from, .. } => Some(construct_from.as_str()),
            Self::Capsule(_) => None,
        }
    }

    /// Returns true when this entry represents a raw capsule round-trip (not Python-side construction).
    pub fn is_capsule_roundtrip(&self) -> bool {
        matches!(self, Self::Capsule(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonConfig {
    pub module_name: Option<String>,
    pub async_runtime: Option<String>,
    pub stubs: Option<StubsConfig>,
    /// PyPI package name (e.g. `"html-to-markdown"`). Used as the `[project] name` in
    /// `pyproject.toml` and to derive the `python-packages` list for maturin.
    /// Defaults to the crate name.
    #[serde(default)]
    pub pip_name: Option<String>,
    /// Per-language feature override. When set, these features are used instead of
    /// `[crate] features` for this language's binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Map of type name -> capsule config for PyCapsule pass-through.
    /// Types listed here are emitted as PyCapsule_New / PyCapsule_GetPointer instead of
    /// opaque `#[pyclass]` wrappers. Use `CapsuleTypeConfig::Capsule` for raw capsule
    /// round-trips and `CapsuleTypeConfig::ConstructFrom` for Python-side construction.
    #[serde(default)]
    pub capsule_types: HashMap<String, CapsuleTypeConfig>,
    /// When true, wrap blocking function bodies in py.allow_threads() to release the GIL.
    // TODO: Wire into gen_bindings.rs to emit py.allow_threads(|| { ... }) for non-async functions.
    #[serde(default)]
    pub release_gil: bool,
    /// Functions to exclude from Python binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Python binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Additional Cargo dependencies for this language's binding crate only.
    #[serde(default)]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Override the scaffold output directory for this language's Cargo.toml and package files.
    #[serde(default)]
    pub scaffold_output: Option<PathBuf>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name` (e.g.
    /// `"LayoutDetection.class"`), value is the desired binding field name. Applied after
    /// automatic keyword escaping, so an explicit entry takes priority.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    /// E.g., `run_wrapper = "uv run --no-sync"` turns `ruff format packages/python` into
    /// `uv run --no-sync ruff format packages/python`.
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    /// Space-separated paths are appended to the command.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Additional `from <module> import <symbol>` lines to emit in the generated `__init__.py`.
    /// Key is the relative or absolute Python module path (e.g. `"._supported_languages"`),
    /// value is the list of symbols to import. The symbols are also added to `__all__`.
    ///
    /// Use this to re-export hand-written sibling modules (e.g. generated by a project's own
    /// build script) without alef's cleanup culling them. The hand-written file must NOT contain
    /// the substrings `"DO NOT EDIT"`, `"auto-generated by alef"`, or `"AUTO-GENERATED by alef"`
    /// in its first 5 lines, or alef's cleanup pipeline will treat it as a stale alef artifact.
    #[serde(default)]
    pub extra_init_imports: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StubsConfig {
    pub output: PathBuf,
}

/// Configuration for a single capsule type entry in `NodeConfig::capsule_types`.
///
/// When set, the named Rust type is NOT emitted as a `#[napi]` opaque wrapper.
/// Instead, functions returning this type produce a `JsObject` carrying the raw
/// pointer in a configurable `Napi::External<T>` property — the layout consumed
/// by the `tree-sitter` npm package's `Parser.setLanguage()`.
///
/// TOML form:
/// ```toml
/// [crates.node.capsule_types.Language]
/// type = "Language"
/// from_module = "tree-sitter"
/// property_name = "language"
/// type_tag = { lower = "0x8AF2E5212AD58ABF", upper = "0xD5006CAD83ABBA16" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeCapsuleTypeConfig {
    /// User-facing class name in the ecosystem library (e.g. `"Language"`).
    /// Emitted as the return-type annotation in the generated `index.d.ts`.
    #[serde(rename = "type")]
    pub type_name: String,
    /// npm package to import the type from (e.g. `"tree-sitter"`).
    /// Emitted as the `from` clause in the generated `import type` line.
    pub from_module: String,
    /// Codegen strategy. Currently only `"external_pointer"` is supported.
    /// Defaults to `"external_pointer"`.
    #[serde(default = "default_node_capsule_construct")]
    pub construct: String,
    /// JS property name to set on the returned object. `node-tree-sitter`
    /// reads `value["language"]`; other consumers may use different names.
    /// Defaults to `"__parser"` for back-compat with existing configs.
    #[serde(default = "default_node_capsule_property_name")]
    pub property_name: String,
    /// Optional N-API type tag to apply via `napi_type_tag_object`. Required
    /// when the consumer library (e.g. `node-tree-sitter`) calls
    /// `napi_check_object_type_tag` to validate the External before using it.
    #[serde(default)]
    pub type_tag: Option<NapiTypeTagConfig>,
}

/// An N-API `napi_type_tag` value, expressed as two 64-bit hex strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NapiTypeTagConfig {
    /// Lower 64 bits of the tag, hex (e.g. `"0x8AF2E5212AD58ABF"`).
    pub lower: String,
    /// Upper 64 bits of the tag, hex (e.g. `"0xD5006CAD83ABBA16"`).
    pub upper: String,
}

fn default_node_capsule_construct() -> String {
    "external_pointer".to_string()
}

fn default_node_capsule_property_name() -> String {
    "__parser".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub package_name: Option<String>,
    /// Per-language feature override. When set, these features are used instead of
    /// `[crate] features` for this language's binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Prefix for generated type names (e.g. "Js" produces `JsConversionOptions`).
    /// Defaults to `"Js"`.
    #[serde(default)]
    pub type_prefix: Option<String>,
    /// Map of Rust type name -> capsule config for raw-pointer passthrough.
    /// Types listed here skip the default `#[napi]` opaque-wrapper emission;
    /// functions returning them produce a `JsObject` with a `__parser`
    /// `Napi::External<T>` property instead. See [`NodeCapsuleTypeConfig`].
    #[serde(default)]
    pub capsule_types: HashMap<String, NodeCapsuleTypeConfig>,
    /// Functions to exclude from Node binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Node binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Additional Cargo dependencies for this language's binding crate only.
    #[serde(default)]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Override the scaffold output directory for this language's Cargo.toml and package files.
    #[serde(default)]
    pub scaffold_output: Option<PathBuf>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RubyConfig {
    pub gem_name: Option<String>,
    pub stubs: Option<StubsConfig>,
    /// Per-language feature override. When set, these features are used instead of
    /// `[crate] features` for this language's binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Functions to exclude from Ruby binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Ruby binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Additional Cargo dependencies for this language's binding crate only.
    #[serde(default)]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Override the scaffold output directory for this language's Cargo.toml and package files.
    #[serde(default)]
    pub scaffold_output: Option<PathBuf>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhpConfig {
    pub extension_name: Option<String>,
    /// Cargo crate name for the PHP binding (e.g. `"ts-pack-core-php"`).
    /// Used to derive the shared library filename in the e2e test runner.
    /// When absent, the lib name is derived from `extension_name` by appending `_php`.
    #[serde(default)]
    pub cargo_crate_name: Option<String>,
    /// Override the PHP namespace used for class registration and PSR-4 autoloading.
    ///
    /// When set, this value is used verbatim as the PHP namespace (e.g. `"HtmlToMarkdown"`).
    /// When absent, the namespace is derived from `extension_name` by splitting on `_` and
    /// converting each segment to PascalCase (e.g. `html_to_markdown` → `Html\To\Markdown`).
    #[serde(default)]
    pub namespace: Option<String>,
    /// Feature gate for ext-php-rs (default: "extension-module").
    /// All generated code is wrapped in `#[cfg(feature = "...")]`.
    #[serde(default)]
    pub feature_gate: Option<String>,
    /// Output directory for generated PHP facade / stubs (e.g., `packages/php/src/`).
    #[serde(default)]
    pub stubs: Option<StubsConfig>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Functions to exclude from PHP binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from PHP binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Additional Cargo dependencies for this language's binding crate only.
    #[serde(default)]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Override the scaffold output directory for this language's Cargo.toml and package files.
    #[serde(default)]
    pub scaffold_output: Option<PathBuf>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElixirConfig {
    pub app_name: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Functions to exclude from Elixir NIF generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Elixir NIF generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Additional Cargo dependencies for this language's binding crate only.
    #[serde(default)]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Override the scaffold output directory for this language's Cargo.toml and package files.
    #[serde(default)]
    pub scaffold_output: Option<PathBuf>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Functions that should be scheduled on the dirty CPU scheduler.
    /// HTML parsing and other CPU-intensive NIFs should be listed here to avoid
    /// blocking BEAM scheduler threads.
    #[serde(default)]
    pub cpu_bound_functions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmConfig {
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    #[serde(default)]
    pub exclude_types: Vec<String>,
    #[serde(default)]
    pub type_overrides: HashMap<String, String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Prefix for generated type names (e.g. "Wasm" produces `WasmConversionOptions`).
    /// Defaults to `"Wasm"`.
    #[serde(default)]
    pub type_prefix: Option<String>,
    /// Functions to exclude from the public TypeScript re-export (index.ts) while still
    /// generating the Rust binding. Use this when a custom module provides a wrapper.
    #[serde(default)]
    pub exclude_reexports: Vec<String>,
    /// Wide-character C functions to shim for WASM external scanner interop.
    #[serde(default)]
    pub env_shims: Vec<String>,
    /// Additional Cargo dependencies for the WASM binding crate only.
    #[serde(default)]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Override the core Cargo dependency name and path for the WASM binding crate.
    /// When set, the binding `Cargo.toml` depends on this crate (resolved as
    /// `../<override>`) instead of the umbrella `[crate.name]`. Use this to point
    /// the WASM binding at a wasm-safe sub-crate while other languages keep the
    /// facade. Defaults to unset.
    #[serde(default)]
    pub core_crate_override: Option<String>,
    /// Keys to subtract from the merged `extra_dependencies` set for this
    /// language only. Useful when `[crate.extra_dependencies]` lists sibling
    /// crates that the WASM target cannot link.
    #[serde(default)]
    pub exclude_extra_dependencies: Vec<String>,
    /// Hand-written Rust modules to declare in the generated lib.rs with `pub mod <name>;`
    /// and re-export with `pub use <name>::*;`. Separate from `[custom_modules].wasm` which
    /// only adds TypeScript `export *` re-exports. Use this for Rust-side dispatch/glue modules.
    #[serde(default)]
    pub custom_rust_modules: Vec<String>,
    /// Per-type field exclusions for the generated From impls and binding struct.
    /// Key is the type name (e.g. "ServerConfig"), value is a list of field names to skip.
    /// Use when source fields are gated behind `#[cfg(not(target_arch = "wasm32"))]` and
    /// therefore don't exist in the wasm32 compilation environment.
    #[serde(default)]
    pub exclude_fields: HashMap<String, Vec<String>>,
    /// Source crate names whose types are re-exported by the `core_crate_override`
    /// crate. References to `<original_crate>::TypeName` in generated code are
    /// rewritten to `<override_crate>::TypeName`. Only meaningful when
    /// `core_crate_override` is set.
    /// Example: with `core_crate_override = "mylib-http"`, setting
    /// `source_crate_remaps = ["mylib-core", "mylib"]` rewrites
    /// `mylib_core::Method` and `mylib::Method` references to
    /// `mylib_http::Method` (assumes `mylib-http` re-exports them via
    /// `pub use mylib_core::*`).
    #[serde(default)]
    pub source_crate_remaps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FfiConfig {
    pub prefix: Option<String>,
    #[serde(default = "default_error_style")]
    pub error_style: String,
    pub header_name: Option<String>,
    /// Native library name for Go cgo/Java Panama/C# P/Invoke (e.g., "ts_pack_ffi").
    /// Defaults to `{prefix}_ffi`.
    #[serde(default)]
    pub lib_name: Option<String>,
    /// If true, generate visitor/callback FFI support.
    #[serde(default)]
    pub visitor_callbacks: bool,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Functions to exclude from FFI binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from FFI binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Rust expression used to construct an error value of this crate's
    /// `error_type` from a runtime `String` message inside generated FFI
    /// trait-bridge plugin shims (`plugin_impl_initialize`, `plugin_impl_shutdown`).
    ///
    /// The expression has access to a local variable `msg: String` containing
    /// the underlying error message and is interpolated verbatim. Example
    /// values:
    ///
    /// ```toml
    /// # downstream whose error type has a struct variant with two fields:
    /// plugin_error_constructor = """
    /// kreuzberg::KreuzbergError::Plugin { message: msg, plugin_name: String::new() }
    /// """
    ///
    /// # downstream whose error type implements `From<String>`:
    /// plugin_error_constructor = "MyError::from(msg)"
    /// ```
    ///
    /// Defaults to `None`. When unset, the plugin shim still emits — backends
    /// fall back to a `format!("{}: {}", prefix, msg)`-style construction via
    /// the configured `error_constructor`. Downstreams that don't expose
    /// trait-bridged plugins can ignore this knob entirely.
    #[serde(default)]
    pub plugin_error_constructor: Option<String>,
}

fn default_error_style() -> String {
    "last_error".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoConfig {
    pub module: Option<String>,
    /// Override the Go package name (default: derived from module path)
    pub package_name: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JavaConfig {
    pub package: Option<String>,
    /// Override the Maven `<groupId>` emitted by alef-scaffold and alef-e2e. When unset,
    /// `java_group_id()` falls back to the Java `package` value. Set this when the
    /// published Maven coords differ from the Java package path (e.g. group
    /// `dev.kreuzberg`, package `dev.kreuzberg.htmltomarkdown`).
    #[serde(default)]
    pub group_id: Option<String>,
    /// Override the Maven `<artifactId>` emitted by alef-scaffold and alef-e2e. When
    /// unset, defaults to the crate name (the `[[crates]] name = "..."`). Set this when
    /// the published artifactId differs from the source crate name (e.g. crate
    /// `html-to-markdown-rs` published as `html-to-markdown`).
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default = "default_java_ffi_style")]
    pub ffi_style: String,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    /// Ignored when project_file is set.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Project file for Maven/Gradle (e.g., "pom.xml", "build.gradle"). When set, default
    /// lint/build/test commands target this file instead of the output directory.
    #[serde(default)]
    pub project_file: Option<String>,
}

fn default_java_ffi_style() -> String {
    "panama".to_string()
}

/// Target platform for Kotlin code generation.
///
/// - `"jvm"` (default): emits source consuming the Java/Panama FFM facade.
/// - `"native"`: emits Kotlin/Native source consuming the cbindgen C FFI library.
/// - `"multiplatform"`: emits Kotlin Multiplatform project scaffolding.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KotlinTarget {
    #[default]
    Jvm,
    Native,
    // Multiplatform — Phase 3 KMP stage; placeholder so the enum is forward-compatible.
    Multiplatform,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KotlinConfig {
    pub package: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Functions to exclude from Kotlin binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Kotlin binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Target platform for Kotlin output. `"jvm"` (default) emits source consuming
    /// the Java/Panama FFM facade; `"native"` emits Kotlin/Native source consuming
    /// the cbindgen C FFI library. `"multiplatform"` emits KMP scaffolding.
    #[serde(default)]
    pub target: KotlinTarget,
    /// Emission mode controlling which Kotlin project layout is generated.
    ///
    /// Accepted values:
    /// - `"jvm"` (default) — standard JVM-only project under `packages/kotlin/`
    /// - `"kmp"` — Kotlin Multiplatform project under `packages/kotlin-mpp/`
    /// - `"android"` — Android library project under `packages/kotlin-android/`
    ///
    /// When `None`, defaults to `"jvm"`.
    #[serde(default)]
    pub mode: Option<String>,
}

/// Configuration for the dedicated Kotlin/Android backend (`alef-backend-kotlin-android`).
///
/// Distinct from [`KotlinConfig`] (Kotlin/JVM). When a crate targets the
/// `kotlin_android` language slug, this struct controls the emitted
/// `build.gradle.kts`, `AndroidManifest.xml`, namespace, Maven publish
/// coordinates, ABI list, and the bundled Java facade emitted into
/// `src/main/java/` so the AAR is self-contained.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KotlinAndroidConfig {
    /// JVM-style package for Kotlin bindings (e.g. `dev.kreuzberg`).
    /// Defaults to the crate name.
    #[serde(default)]
    pub package: Option<String>,
    /// Android library manifest `namespace`. Defaults to `package`.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Maven `artifactId` for the generated AAR. Defaults to `{crate}-android`.
    #[serde(default)]
    pub artifact_id: Option<String>,
    /// Maven `groupId` for the generated AAR. No default — when unset the
    /// emitter falls back to `package`.
    #[serde(default)]
    pub group_id: Option<String>,
    /// Android compile SDK level. Defaults to `template_versions::toolchain::ANDROID_COMPILE_SDK`.
    #[serde(default)]
    pub compile_sdk: Option<u32>,
    /// Android min SDK level. Defaults to `template_versions::toolchain::ANDROID_MIN_SDK`.
    #[serde(default)]
    pub min_sdk: Option<u32>,
    /// JVM bytecode target for Kotlin and Java compilation
    /// (e.g. `"17"`). Defaults to `template_versions::toolchain::ANDROID_JVM_TARGET`.
    #[serde(default)]
    pub jvm_target: Option<String>,
    /// ABIs to scaffold under `src/main/jniLibs/<abi>/`. Defaults to
    /// `["arm64-v8a", "x86_64"]`.
    #[serde(default)]
    pub abis: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Functions to exclude from generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Prefix wrapper for default tool invocations.
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Per-language feature override. When set, these features are used instead of
    /// `[crate] features` for this language's binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
}

/// Dart bridging style: FRB (default) or raw `dart:ffi`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DartStyle {
    /// flutter_rust_bridge — emits a Rust crate plus Dart wrappers using
    /// FRB-generated bridge symbols. Default.
    #[default]
    Frb,
    /// Raw `dart:ffi` over the cbindgen C ABI — emits Dart-only source that
    /// loads the shared library at runtime. Cheaper to ship; loses FRB's
    /// async ergonomics and freezed-style data classes.
    Ffi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GleamConfig {
    pub app_name: Option<String>,
    /// Erlang atom name for @external(erlang, "<nif>", ...) lookups (e.g., "my_app_nif").
    /// Defaults to the app_name.
    #[serde(default)]
    pub nif_module: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Functions to exclude from Gleam binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Gleam binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Prefix wrapper for default tool invocations.
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Per-`element_type` Gleam record-constructor recipes used by the e2e
    /// generator when emitting `json_object` arg literals. Each entry maps a
    /// fixture-side `element_type` string (e.g. `"BatchFileItem"`) to a
    /// structured constructor description that the codegen interpolates per
    /// JSON-array item. Without an entry the codegen falls back to the
    /// `json_object_wrapper` (or a plain `json_to_gleam`).
    ///
    /// Example:
    ///
    /// ```toml
    /// [[crates.gleam.element_constructors]]
    /// element_type = "BatchFileItem"
    /// constructor = "kreuzberg.BatchFileItem"
    /// [[crates.gleam.element_constructors.fields]]
    /// gleam_field = "path"
    /// kind = "file_path"
    /// json_field = "path"
    /// [[crates.gleam.element_constructors.fields]]
    /// gleam_field = "config"
    /// kind = "literal"
    /// value = "option.None"
    /// ```
    #[serde(default)]
    pub element_constructors: Vec<GleamElementConstructor>,
    /// Optional Gleam expression template used to wrap `json_object` arg
    /// values when no `element_type` recipe matches. The placeholder
    /// `{json}` is replaced with a Gleam string literal containing the JSON
    /// form of the arg value, allowing the downstream's Gleam binding to do
    /// its own parsing.
    ///
    /// Example:
    ///
    /// ```toml
    /// [crates.gleam]
    /// json_object_wrapper = "kreuzberg.config_from_json_string({json})"
    /// ```
    ///
    /// When `None`, the codegen emits `{json}` verbatim (a plain Gleam
    /// string), matching the iter15 default.
    #[serde(default)]
    pub json_object_wrapper: Option<String>,
}

/// One per-`element_type` Gleam record-constructor recipe. Keyed by the
/// fixture-side `element_type` string and consumed by the e2e Gleam codegen
/// when building `json_object` arg literals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GleamElementConstructor {
    /// Fixture-side `element_type` value this recipe applies to (e.g.
    /// `"BatchFileItem"`).
    pub element_type: String,
    /// Fully-qualified Gleam constructor identifier (e.g.
    /// `"kreuzberg.BatchFileItem"`). Emitted verbatim before the `(...)` field
    /// list.
    pub constructor: String,
    /// Ordered list of fields to emit inside the constructor's `(...)` block,
    /// in argument-position order. Each field describes how its value is
    /// derived from the per-item JSON object.
    pub fields: Vec<GleamElementField>,
}

/// One field inside a [`GleamElementConstructor`]'s argument list.
///
/// `kind` selects the source/encoding strategy:
/// * `"file_path"` — read `json_field` from the JSON object as a string,
///   prefix with the configured `test_documents_dir` when the value does not
///   start with `/`, and emit as a Gleam string literal.
/// * `"byte_array"` — read `json_field` from the JSON object as a JSON
///   `Array(Number)` and emit as a Gleam BitArray literal `<<n1, n2, …>>`.
/// * `"string"` — read `json_field` as a string, emit as a Gleam string
///   literal; falls back to `default` (or empty) if missing.
/// * `"literal"` — emit `value` verbatim (no JSON lookup). Use for
///   constant fields like `config: option.None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GleamElementField {
    /// Gleam record field name (e.g. `"path"`, `"config"`).
    pub gleam_field: String,
    /// Source/encoding strategy. See struct doc.
    pub kind: String,
    /// JSON object key to read, when `kind` is one of the JSON-driven
    /// strategies. Required for `"file_path"`, `"byte_array"`, `"string"`;
    /// ignored for `"literal"`.
    #[serde(default)]
    pub json_field: Option<String>,
    /// Default Gleam expression when `json_field` is missing/null. Only
    /// honoured by the `"string"` strategy today.
    #[serde(default)]
    pub default: Option<String>,
    /// Verbatim Gleam expression to emit when `kind = "literal"`.
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DartConfig {
    /// Dart pub.dev package name (e.g. `"my_package"`). Used as the `name` in
    /// `pubspec.yaml`. Defaults to a snake_case derivation of the crate name.
    #[serde(default)]
    pub pubspec_name: Option<String>,
    /// Dart library name (the `library` declaration). Defaults to the pubspec name.
    #[serde(default)]
    pub lib_name: Option<String>,
    /// Dart package name override (e.g. for pub.dev scoped packages).
    #[serde(default)]
    pub package_name: Option<String>,
    /// Bridging style. `"frb"` (default) uses flutter_rust_bridge; `"ffi"` emits
    /// raw `dart:ffi` source over the cbindgen C library.
    #[serde(default)]
    pub style: DartStyle,
    /// flutter_rust_bridge version to pin in generated pubspec.yaml.
    /// Defaults to `template_versions::cargo::FLUTTER_RUST_BRIDGE` when unset.
    #[serde(default)]
    pub frb_version: Option<String>,
    /// Cargo features to enable on the binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Additional Cargo dependencies for the generated Dart Rust bridge crate.
    #[serde(default)]
    pub extra_dependencies: HashMap<String, toml::Value>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Functions to exclude from Dart binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Dart binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Prefix wrapper for default tool invocations.
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Override the core Cargo dependency name and path for the Dart binding crate.
    /// When set, the binding `Cargo.toml` depends on this crate (resolved as
    /// `../../../crates/<override>`) instead of the umbrella `[crate.name]`.
    /// Defaults to unset.
    #[serde(default)]
    pub core_crate_override: Option<String>,
    /// Keys to subtract from the merged `extra_dependencies` set for this
    /// language only.
    #[serde(default)]
    pub exclude_extra_dependencies: Vec<String>,
    /// Method names whose Rust bridge body should be emitted as `unimplemented!()`.
    ///
    /// Use this when a function's FFI signature (e.g. nested tuples containing
    /// `Vec<u8>`) cannot be represented across the FRB bridge at all. Consumers must
    /// list the method names explicitly — this field has no built-in defaults so the
    /// knob is library-agnostic.
    ///
    /// Example (`alef.toml`):
    /// ```toml
    /// [crates.dart]
    /// stub_methods = ["batch_extract_bytes", "batch_extract_bytes_sync"]
    /// ```
    #[serde(default)]
    pub stub_methods: Vec<String>,
    /// Per-target Cargo dependency overrides for the binding crate.
    ///
    /// When set, the emitted `Cargo.toml` wraps the base core dependency in a
    /// `[target.'cfg(not(<cfg>))'.dependencies]` section and adds a matching
    /// `[target.'cfg(<cfg>)'.dependencies]` block using `override_features`
    /// (and `default_features = false` when `override_default_features = false`).
    /// Required when the binding has to swap the feature set on a specific
    /// target triple, e.g. Android x86_64 dropping ORT-dependent features.
    ///
    /// Example (`alef.toml`):
    /// ```toml
    /// [[crates.dart.target_dep_overrides]]
    /// cfg = "all(target_os = \"android\", target_arch = \"x86_64\")"
    /// features = ["android-target"]
    /// default_features = false
    /// ```
    #[serde(default)]
    pub target_dep_overrides: Vec<DartTargetDepOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DartTargetDepOverride {
    /// Cargo `cfg(...)` predicate (without the `cfg(...)` wrapper). Example:
    /// `all(target_os = "android", target_arch = "x86_64")`.
    pub cfg: String,
    /// Features to enable on the core dependency for this target.
    #[serde(default)]
    pub features: Vec<String>,
    /// When false (default), emit `default-features = false` for this target.
    /// When true, allow the core dep's default features through.
    #[serde(default)]
    pub default_features: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SwiftConfig {
    /// Swift module name (e.g. `"MyLibrary"`). Defaults to PascalCase of the crate name.
    #[serde(default)]
    pub module_name: Option<String>,
    /// Swift package name. Defaults to the module name.
    #[serde(default)]
    pub package_name: Option<String>,
    /// swift-bridge version. Defaults to `template_versions::cargo::SWIFT_BRIDGE` when unset.
    #[serde(default)]
    pub swift_bridge_version: Option<String>,
    /// Minimum macOS deployment target. Defaults to `template_versions::toolchain::SWIFT_MIN_MACOS` when unset.
    #[serde(default)]
    pub min_macos_version: Option<String>,
    /// Minimum iOS deployment target. Defaults to `template_versions::toolchain::SWIFT_MIN_IOS` when unset.
    #[serde(default)]
    pub min_ios_version: Option<String>,
    /// Cargo features to enable on the binding crate.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Functions to exclude from Swift binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Swift binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Fields to exclude from Swift binding generation.
    /// Format: `"TypeName.field_name"`.
    #[serde(default)]
    pub exclude_fields: Vec<String>,
    /// Prefix wrapper for default tool invocations.
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Override the core Cargo dependency name and path for the Swift binding crate.
    /// When set, the binding `Cargo.toml` depends on this crate (resolved as
    /// `../../../crates/<override>`) instead of the umbrella `[crate.name]`.
    /// Defaults to unset.
    #[serde(default)]
    pub core_crate_override: Option<String>,
    /// Keys to subtract from the merged `extra_dependencies` set for this
    /// language only.
    #[serde(default)]
    pub exclude_extra_dependencies: Vec<String>,
    /// Override the auto-generated `create_<type>(api_key, base_url)` constructor
    /// body for opaque client types that expose methods. When set, the swift backend
    /// emits this snippet verbatim as the function body (no implicit `Ok(...)`).
    ///
    /// Use this when the source crate's constructor signature differs from the
    /// default `Type::new(api_key, base_url)` shape — e.g. liter-llm uses
    /// `DefaultClient::new(ClientConfig, Option<&str>)` and needs to build a
    /// `ClientConfig` from the bridge inputs first.
    ///
    /// The snippet is parameterised by `{type_name}` (the wrapper newtype name)
    /// and runs in a function body with `api_key: String` and `base_url: Option<String>`
    /// already in scope. It must return `Result<{type_name}, String>`.
    #[serde(default)]
    pub client_constructor_body: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZigConfig {
    pub module_name: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Functions to exclude from Zig binding generation.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
    /// Types to exclude from Zig binding generation.
    #[serde(default)]
    pub exclude_types: Vec<String>,
    /// Prefix wrapper for default tool invocations.
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSharpConfig {
    pub namespace: Option<String>,
    /// NuGet `<PackageId>` to publish under. When unset, falls back to `namespace`.
    /// Use this when the published artifact id must differ from the C# `RootNamespace` —
    /// e.g. when the unprefixed name is owned by a third party on nuget.org and
    /// you publish under a vendor-prefixed id like `KreuzbergDev.<Lib>`.
    #[serde(default)]
    pub package_id: Option<String>,
    pub target_framework: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    /// Ignored when project_file is set.
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
    /// Project file for C# (e.g., "MyProject.csproj", "MySolution.sln"). When set, default
    /// lint/build/test commands target this file instead of the output directory.
    #[serde(default)]
    pub project_file: Option<String>,
    /// Functions to exclude from C# binding generation (e.g., functions not present in the
    /// C FFI layer). Excluded functions are omitted from both NativeMethods.cs and the
    /// wrapper class.
    #[serde(default)]
    pub exclude_functions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RConfig {
    pub package_name: Option<String>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Override the serde rename_all strategy for JSON field names (e.g. "camelCase", "snake_case").
    /// When set, this takes priority over the IR type-level serde_rename_all.
    #[serde(default)]
    pub serde_rename_all: Option<String>,
    /// Per-field name remapping for this language. Key is `TypeName.field_name`, value is the
    /// desired binding field name. Applied after automatic keyword escaping.
    #[serde(default)]
    pub rename_fields: HashMap<String, String>,
    /// Prefix wrapper for default tool invocations. When set, prepends this string to default
    /// commands across all pipelines (lint, test, build, etc.).
    #[serde(default)]
    pub run_wrapper: Option<String>,
    /// Extra paths to append to default lint commands (format, check, typecheck).
    #[serde(default)]
    pub extra_lint_paths: Vec<String>,
}

/// Custom modules that alef should declare (mod X;) but not generate.
/// These are hand-written modules imported by the generated lib.rs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomModulesConfig {
    #[serde(default)]
    pub python: Vec<String>,
    #[serde(default)]
    pub node: Vec<String>,
    #[serde(default)]
    pub ruby: Vec<String>,
    #[serde(default)]
    pub php: Vec<String>,
    #[serde(default)]
    pub elixir: Vec<String>,
    #[serde(default)]
    pub wasm: Vec<String>,
    #[serde(default)]
    pub ffi: Vec<String>,
    #[serde(default)]
    pub go: Vec<String>,
    #[serde(default)]
    pub java: Vec<String>,
    #[serde(default)]
    pub csharp: Vec<String>,
    #[serde(default)]
    pub r: Vec<String>,
}

impl CustomModulesConfig {
    pub fn for_language(&self, lang: Language) -> &[String] {
        match lang {
            Language::Python => &self.python,
            Language::Node => &self.node,
            Language::Ruby => &self.ruby,
            Language::Php => &self.php,
            Language::Elixir => &self.elixir,
            Language::Wasm => &self.wasm,
            Language::Ffi => &self.ffi,
            Language::Go => &self.go,
            Language::Java => &self.java,
            Language::Csharp => &self.csharp,
            Language::R => &self.r,
            Language::Rust => &[], // Rust doesn't need custom modules (no binding crate)
            Language::Kotlin
            | Language::KotlinAndroid
            | Language::Swift
            | Language::Dart
            | Language::Gleam
            | Language::Zig
            | Language::C => &[],
        }
    }
}

/// Custom classes/functions from hand-written modules to register in module init.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomRegistration {
    #[serde(default)]
    pub classes: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
    #[serde(default)]
    pub init_calls: Vec<String>,
}

/// Per-language custom registrations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CustomRegistrationsConfig {
    #[serde(default)]
    pub python: Option<CustomRegistration>,
    #[serde(default)]
    pub node: Option<CustomRegistration>,
    #[serde(default)]
    pub ruby: Option<CustomRegistration>,
    #[serde(default)]
    pub php: Option<CustomRegistration>,
    #[serde(default)]
    pub elixir: Option<CustomRegistration>,
    #[serde(default)]
    pub wasm: Option<CustomRegistration>,
}

impl CustomRegistrationsConfig {
    pub fn for_language(&self, lang: Language) -> Option<&CustomRegistration> {
        match lang {
            Language::Python => self.python.as_ref(),
            Language::Node => self.node.as_ref(),
            Language::Ruby => self.ruby.as_ref(),
            Language::Php => self.php.as_ref(),
            Language::Elixir => self.elixir.as_ref(),
            Language::Wasm => self.wasm.as_ref(),
            _ => None,
        }
    }
}
