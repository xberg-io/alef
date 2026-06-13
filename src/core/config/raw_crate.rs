//! `[[crates]]` entries — the raw per-crate config as written in `alef.toml`.
//!
//! A `RawCrateConfig` is what the user types in their TOML; a
//! [`crate::core::config::resolved::ResolvedCrateConfig`] is what every backend
//! consumes after workspace defaults have been merged in.
//!
//! Each entry produces an independent set of polyglot binding packages.
//! Crates may share workspace defaults (tooling, DTO style, default
//! pipelines), but they do not share crate-shaped state (sources,
//! per-language module names, publish settings, e2e fixtures).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::SourceCrate;
use super::e2e::E2eConfig;
use super::extras::{AdapterConfig, Language};
use super::languages::{
    CSharpConfig, CustomModulesConfig, CustomRegistrationsConfig, DartConfig, ElixirConfig, FfiConfig, GleamConfig,
    GoConfig, JavaConfig, JniConfig, KotlinAndroidConfig, KotlinConfig, NodeConfig, PhpConfig, PythonConfig, RConfig,
    RubyConfig, SwiftConfig, WasmConfig, ZigConfig,
};
use super::output::{
    BuildCommandConfig, CleanConfig, ExcludeConfig, IncludeConfig, LintConfig, OutputConfig, ReadmeConfig,
    ScaffoldConfig, SetupConfig, TestConfig, UpdateConfig,
};
use super::package_metadata::PackageMetadataConfig;
use super::publish::PublishConfig;
use super::service::{ErrorTypeConfig, HandlerContractConfig, LifecycleHookConfig, ServiceConfig, SseRouteConfig, WebSocketRouteConfig};
use super::trait_bridge::TraitBridgeConfig;

/// One `[[crates]]` entry — an independently published Rust facade plus its
/// per-crate language settings, pipelines, and packaging metadata.
///
/// Every field except `name` is optional. Fields left unset inherit from
/// the workspace defaults during resolution; required fields with no
/// workspace default fall back to Alef's built-in defaults or trigger a
/// validation error.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RawCrateConfig {
    // -----------------------------------------------------------------
    // Identity and source crate
    // -----------------------------------------------------------------
    /// Crate name (e.g. `"sample_project"`). Must be unique within the workspace.
    pub name: String,

    /// Rust source files this crate's bindings extract from.
    /// May be empty when [`source_crates`](Self::source_crates) is non-empty.
    #[serde(default)]
    pub sources: Vec<PathBuf>,

    /// File whose `[package] version` is the source of truth for this crate.
    /// Defaults to `"Cargo.toml"` (resolved relative to the alef.toml).
    #[serde(default)]
    pub version_from: Option<String>,

    /// Rust import path for the core crate referenced from generated bindings.
    /// Defaults to the crate name with hyphens replaced by underscores.
    #[serde(default)]
    pub core_import: Option<String>,

    /// Optional workspace root path for resolving `pub use` re-exports from
    /// sibling crates.
    #[serde(default)]
    pub workspace_root: Option<PathBuf>,

    /// When true, skip adding `use {core_import};` to generated bindings.
    #[serde(default)]
    pub skip_core_import: bool,

    /// Crate error type name (e.g. `"SampleCrateError"`). Used by trait-bridge
    /// generation. Defaults to `"Error"`.
    #[serde(default)]
    pub error_type: Option<String>,

    /// Pattern for constructing error values from a String in trait bridges.
    /// `{msg}` is replaced with the format!(...) expression.
    #[serde(default)]
    pub error_constructor: Option<String>,

    /// Cargo features enabled in binding crates. Fields gated by
    /// `#[cfg(feature = "...")]` matching these features are treated as
    /// always present (cfg stripped from the IR).
    #[serde(default)]
    pub features: Vec<String>,

    /// Maps extracted rust_path prefixes to actual import paths in binding crates.
    #[serde(default)]
    pub path_mappings: HashMap<String, String>,

    /// Additional Cargo dependencies merged into every binding crate Cargo.toml
    /// for this crate (per-language extra_dependencies still override these).
    #[serde(default)]
    #[schemars(with = "HashMap<String, serde_json::Value>")]
    pub extra_dependencies: HashMap<String, toml::Value>,

    /// Automatically derive path mappings from source file locations.
    /// Default: `true`.
    #[serde(default)]
    pub auto_path_mappings: Option<bool>,

    /// Multi-crate source groups for workspaces with types spread across
    /// sibling crates. When non-empty, the top-level `sources` field is ignored.
    #[serde(default)]
    pub source_crates: Vec<SourceCrate>,

    // -----------------------------------------------------------------
    // Language selection
    // -----------------------------------------------------------------
    /// Override of the workspace `languages` list for this crate.
    /// When `None`, this crate inherits the workspace default.
    #[serde(default)]
    pub languages: Option<Vec<Language>>,

    // -----------------------------------------------------------------
    // Per-language settings (was top-level [python], [node], …)
    // -----------------------------------------------------------------
    #[serde(default)]
    pub python: Option<PythonConfig>,
    #[serde(default)]
    pub node: Option<NodeConfig>,
    #[serde(default)]
    pub ruby: Option<RubyConfig>,
    #[serde(default)]
    pub php: Option<PhpConfig>,
    #[serde(default)]
    pub elixir: Option<ElixirConfig>,
    #[serde(default)]
    pub wasm: Option<WasmConfig>,
    #[serde(default)]
    pub ffi: Option<FfiConfig>,
    #[serde(default)]
    pub go: Option<GoConfig>,
    #[serde(default)]
    pub java: Option<JavaConfig>,
    #[serde(default)]
    pub dart: Option<DartConfig>,
    #[serde(default)]
    pub kotlin: Option<KotlinConfig>,
    #[serde(default)]
    pub kotlin_android: Option<KotlinAndroidConfig>,
    #[serde(default)]
    pub jni: Option<JniConfig>,
    #[serde(default)]
    pub swift: Option<SwiftConfig>,
    #[serde(default)]
    pub gleam: Option<GleamConfig>,
    #[serde(default)]
    pub csharp: Option<CSharpConfig>,
    #[serde(default)]
    pub r: Option<RConfig>,
    #[serde(default)]
    pub zig: Option<ZigConfig>,

    // -----------------------------------------------------------------
    // Filters and output paths
    // -----------------------------------------------------------------
    #[serde(default)]
    pub exclude: ExcludeConfig,
    #[serde(default)]
    pub include: IncludeConfig,

    /// Per-crate explicit output paths. Wins over the workspace
    /// `output_template` for any language with an entry here.
    #[serde(default)]
    pub output: OutputConfig,

    // -----------------------------------------------------------------
    // Per-crate generation, formatting, and DTO overrides.
    // -----------------------------------------------------------------
    /// Override the workspace default `generate` flags. When `Some`, replaces the
    /// workspace value wholesale. When `None`, the crate inherits `workspace.generate`.
    #[serde(default)]
    pub generate: Option<super::GenerateConfig>,

    /// Override the workspace default `format` flags. When `Some`, replaces the
    /// workspace value wholesale.
    #[serde(default)]
    pub format: Option<super::FormatConfig>,

    /// Override the workspace default DTO styles. When `Some`, replaces the
    /// workspace value wholesale (no field-level merge — a partial DTO override
    /// would silently drop unspecified language entries).
    #[serde(default)]
    pub dto: Option<super::DtoConfig>,

    /// Per-language per-crate formatting overrides keyed by language code.
    /// Merged with workspace `format_overrides`: per-crate keys win wholesale,
    /// missing keys fall through to the workspace map.
    #[serde(default)]
    pub format_overrides: HashMap<String, super::FormatConfig>,

    /// Per-language per-crate generation flag overrides keyed by language code.
    /// Merged with workspace `generate_overrides`: per-crate keys win wholesale,
    /// missing keys fall through to the workspace map.
    #[serde(default)]
    pub generate_overrides: HashMap<String, super::GenerateConfig>,

    // -----------------------------------------------------------------
    // Per-crate pipeline overrides — merged field-wise with workspace defaults.
    // -----------------------------------------------------------------
    #[serde(default)]
    pub lint: HashMap<String, LintConfig>,
    #[serde(default)]
    pub test: HashMap<String, TestConfig>,
    #[serde(default)]
    pub setup: HashMap<String, SetupConfig>,
    #[serde(default)]
    pub update: HashMap<String, UpdateConfig>,
    #[serde(default)]
    pub clean: HashMap<String, CleanConfig>,
    #[serde(default)]
    pub build_commands: HashMap<String, BuildCommandConfig>,

    // -----------------------------------------------------------------
    // Packaging, e2e, extensibility
    // -----------------------------------------------------------------
    #[serde(default)]
    pub publish: Option<PublishConfig>,
    #[serde(default)]
    pub e2e: Option<E2eConfig>,
    #[serde(default)]
    pub adapters: Vec<AdapterConfig>,
    #[serde(default)]
    pub trait_bridges: Vec<TraitBridgeConfig>,
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
    #[serde(default)]
    pub handler_contracts: Vec<HandlerContractConfig>,
    /// Lifecycle hook contracts registered on the service owner.
    ///
    /// Each entry declares one named callback slot (e.g. `on_request`, `pre_handler`,
    /// `on_response`, `on_error`) that backends emit as `app.on_<name>(fn)` methods.
    ///
    /// ```toml
    /// [[crates.lifecycle_hooks]]
    /// name = "on_request"
    /// callback_contract = "RequestHook"
    /// ```
    #[serde(default)]
    pub lifecycle_hooks: Vec<LifecycleHookConfig>,
    /// WebSocket route registration contracts.
    ///
    /// Each entry causes backends to emit `app.websocket(path, handler_fn)`.
    ///
    /// ```toml
    /// [[crates.websocket_routes]]
    /// handler_wrapper_type = "WebSocketHandlerWrapper"
    /// socket_type = "WebSocketConnection"
    /// ```
    #[serde(default)]
    pub websocket_routes: Vec<WebSocketRouteConfig>,
    /// SSE route registration contracts.
    ///
    /// Each entry causes backends to emit `app.sse(path, producer_fn)`.
    ///
    /// ```toml
    /// [[crates.sse_routes]]
    /// producer_wrapper_type = "SseProducerWrapper"
    /// event_type = "SseEvent"
    /// ```
    #[serde(default)]
    pub sse_routes: Vec<SseRouteConfig>,
    /// Cross-binding error types emitted as native exception classes.
    ///
    /// Each entry causes backends to emit an exception/error class that maps to the
    /// specified HTTP status and serializes as RFC 9457 ProblemDetails JSON.
    ///
    /// ```toml
    /// [[crates.error_types]]
    /// name = "NotFoundError"
    /// http_status = 404
    /// ```
    #[serde(default)]
    pub error_types: Vec<ErrorTypeConfig>,
    #[serde(default)]
    pub scaffold: Option<ScaffoldConfig>,
    #[serde(default)]
    pub package_metadata: Option<PackageMetadataConfig>,
    #[serde(default)]
    pub readme: Option<ReadmeConfig>,
    #[serde(default)]
    pub custom_files: HashMap<String, Vec<PathBuf>>,
    #[serde(default)]
    pub custom_modules: CustomModulesConfig,
    #[serde(default)]
    pub custom_registrations: CustomRegistrationsConfig,
    /// Validation diagnostic codes that are downgraded from errors to warnings
    /// for this crate, allowing generation to proceed despite pre-existing surface
    /// issues. Each entry is the string form of a [`crate::core::validation::ValidationCode`] (e.g.
    /// `"lossy_sanitized_surface"`, `"unknown_named_type"`).
    ///
    /// Use this to opt out of specific validation gates while you work toward full
    /// compliance; remove entries once the underlying surface issues are fixed.
    #[serde(default)]
    pub suppress_validation_codes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_crate_config_minimal_deserializes() {
        let toml_str = r#"
name = "sample_router"
sources = ["src/lib.rs"]
"#;
        let cfg: RawCrateConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.name, "sample_router");
        assert_eq!(cfg.sources, vec![PathBuf::from("src/lib.rs")]);
        assert!(cfg.python.is_none());
        assert!(cfg.publish.is_none());
        assert!(cfg.e2e.is_none());
    }

    #[test]
    fn raw_crate_config_with_source_crates() {
        let toml_str = r#"
name = "sample_router"
sources = []
features = ["di"]
core_import = "sample_router"

[[source_crates]]
name = "sample_router-core"
sources = ["crates/sample_router-core/src/http.rs"]

[[source_crates]]
name = "sample_router-http"
sources = ["crates/sample_router-http/src/lib.rs"]
"#;
        let cfg: RawCrateConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.source_crates.len(), 2);
        assert_eq!(cfg.source_crates[0].name, "sample_router-core");
        assert_eq!(cfg.features, vec!["di"]);
    }

    #[test]
    fn raw_crate_config_with_per_crate_python_and_lint_override() {
        let toml_str = r#"
name = "sample_router"
sources = []

[python]
module_name = "_sample_router"
pip_name    = "sample_router"
release_gil = true

[lint.python]
check = "ruff check crates/sample_router-py/"
"#;
        let cfg: RawCrateConfig = toml::from_str(toml_str).unwrap();
        let py = cfg.python.expect("python section present");
        assert_eq!(py.module_name.as_deref(), Some("_sample_router"));
        assert_eq!(py.pip_name.as_deref(), Some("sample_router"));
        assert!(py.release_gil);

        let lint_py = cfg.lint.get("python").expect("lint.python override present");
        assert_eq!(
            lint_py.check.as_ref().unwrap().commands(),
            vec!["ruff check crates/sample_router-py/"]
        );
    }
}
