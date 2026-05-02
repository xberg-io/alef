use serde::{Deserialize, Serialize};

pub mod build_defaults;
pub mod clean_defaults;
pub mod derive;
pub mod dto;
pub mod e2e;
pub mod extras;
pub mod languages;
pub mod legacy;
pub mod lint_defaults;
pub mod new_config;
pub mod output;
pub mod publish;
pub mod raw_crate;
pub mod resolve_helpers;
pub mod resolved;
pub mod setup_defaults;
pub mod test_defaults;
pub mod tools;
pub mod trait_bridge;
pub mod update_defaults;
pub mod validation;
pub mod workspace;

// Re-exports for backward compatibility — all types were previously flat in config.rs.
pub use derive::{derive_go_module_from_repo, derive_repo_org, derive_reverse_dns_package};
pub use dto::{
    CsharpDtoStyle, DtoConfig, ElixirDtoStyle, GoDtoStyle, JavaDtoStyle, NodeDtoStyle, PhpDtoStyle, PythonDtoStyle,
    RDtoStyle, RubyDtoStyle,
};
pub use e2e::E2eConfig;
pub use extras::{AdapterConfig, AdapterParam, AdapterPattern, Language};
pub use languages::{
    CSharpConfig, CustomModulesConfig, CustomRegistration, CustomRegistrationsConfig, DartConfig, DartStyle,
    ElixirConfig, FfiConfig, GleamConfig, GoConfig, JavaConfig, KotlinConfig, KotlinTarget, NodeConfig, PhpConfig,
    PythonConfig, RConfig, RubyConfig, StubsConfig, SwiftConfig, WasmConfig, ZigConfig,
};
pub use legacy::{LegacyConfigError, LegacyKey, detect_legacy_keys};
pub use new_config::{NewAlefConfig, ResolveError};
pub use output::{
    BuildCommandConfig, CleanConfig, ExcludeConfig, IncludeConfig, LintConfig, OutputConfig, OutputTemplate,
    ReadmeConfig, ScaffoldCargo, ScaffoldCargoEnvValue, ScaffoldCargoTargets, ScaffoldConfig, SetupConfig, SyncConfig,
    TestConfig, TextReplacement, UpdateConfig,
};
pub use publish::{PublishConfig, PublishLanguageConfig, VendorMode};
pub use raw_crate::RawCrateConfig;
pub use resolve_helpers::{detect_serde_available, resolve_output_dir};
pub use resolved::ResolvedCrateConfig;
pub use tools::{DEFAULT_RUST_DEV_TOOLS, LangContext, ToolsConfig, require_tool, require_tools};
pub use trait_bridge::{BridgeBinding, TraitBridgeConfig};
pub use workspace::WorkspaceConfig;

/// A source crate group for multi-crate extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCrate {
    /// Crate name (hyphens converted to underscores for rust_path).
    pub name: String,
    /// Source files belonging to this crate.
    pub sources: Vec<std::path::PathBuf>,
}

fn default_true() -> bool {
    true
}

/// Controls which generation passes alef runs.
/// All flags default to `true`; set to `false` to skip a pass.
/// Can be overridden per-language via `[generate_overrides.<lang>]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateConfig {
    /// Generate low-level struct wrappers, From impls, module init (default: true)
    #[serde(default = "default_true")]
    pub bindings: bool,
    /// Generate error type hierarchies from thiserror enums (default: true)
    #[serde(default = "default_true")]
    pub errors: bool,
    /// Generate config builder constructors from Default types (default: true)
    #[serde(default = "default_true")]
    pub configs: bool,
    /// Generate async/sync function pairs with runtime management (default: true)
    #[serde(default = "default_true")]
    pub async_wrappers: bool,
    /// Generate recursive type marshaling helpers (default: true)
    #[serde(default = "default_true")]
    pub type_conversions: bool,
    /// Generate package manifests (pyproject.toml, package.json, etc.) (default: true)
    #[serde(default = "default_true")]
    pub package_metadata: bool,
    /// Generate idiomatic public API wrappers (default: true)
    #[serde(default = "default_true")]
    pub public_api: bool,
    /// Generate `From<BindingType> for CoreType` reverse conversions (default: true).
    /// Set to false when the binding layer only returns core types and never accepts them.
    #[serde(default = "default_true")]
    pub reverse_conversions: bool,
}

impl Default for GenerateConfig {
    fn default() -> Self {
        Self {
            bindings: true,
            errors: true,
            configs: true,
            async_wrappers: true,
            type_conversions: true,
            package_metadata: true,
            public_api: true,
            reverse_conversions: true,
        }
    }
}

/// Post-generation formatting configuration.
/// After code generation, alef can automatically run language-native formatters
/// on the emitted package directories to ensure CI formatter checks pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatConfig {
    /// Enable post-generation formatting (default: true).
    /// Set to false to skip formatting for all languages, or use per-language
    /// overrides in `[format.<lang>]` to disable specific formatters.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional custom command override. If set, this command is run instead
    /// of the language's default formatter. Must be a shell command string
    /// (e.g., "prettier --write .").
    #[serde(default)]
    pub command: Option<String>,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            command: None,
        }
    }
}
