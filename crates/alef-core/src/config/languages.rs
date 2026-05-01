use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::extras::Language;

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
    /// Map of type name -> PyCapsule name for raw pointer wrapping.
    /// When a function returns one of these types, alef generates PyCapsule_New instead of Arc wrapping.
    // TODO: Wire into gen_bindings.rs to emit PyCapsule_New / PyCapsule_GetPointer at call sites.
    #[serde(default)]
    pub capsule_types: HashMap<String, String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StubsConfig {
    pub output: PathBuf,
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
    /// Example: with `core_crate_override = "spikard-http"`, setting
    /// `source_crate_remaps = ["spikard-core", "spikard"]` rewrites
    /// `spikard_core::Method` and `spikard::Method` references to
    /// `spikard_http::Method` (assumes `spikard-http` re-exports them via
    /// `pub use spikard_core::*`).
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
/// - `"multiplatform"`: reserved for the KMP stage (Phase 3 follow-up).
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
    /// the cbindgen C FFI library. `"multiplatform"` is reserved for the KMP stage.
    #[serde(default)]
    pub target: KotlinTarget,
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
            Language::Kotlin | Language::Swift | Language::Dart | Language::Gleam | Language::Zig => &[],
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
