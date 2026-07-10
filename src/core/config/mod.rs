use schemars::JsonSchema;
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
pub mod manifest_extras;
pub mod new_config;
pub mod output;
pub mod package_metadata;
pub mod poly;
pub mod publish;
pub mod raw_crate;
pub mod registry;
pub mod resolve_helpers;
pub mod resolved;
pub mod schema;
pub mod service;
pub mod setup_defaults;
pub mod test_apps_run_defaults;
pub mod test_defaults;
pub mod tools;
pub mod trait_bridge;
pub mod update_defaults;
pub mod validation;
pub mod workspace;

pub use derive::{derive_go_module_from_repo, derive_repo_org, derive_reverse_dns_package};
pub use dto::{
    CsharpDtoStyle, DtoConfig, ElixirDtoStyle, GoDtoStyle, JavaBuilderMode, JavaDtoConfig, JavaDtoStyle, NodeDtoStyle,
    PhpDtoStyle, PythonDtoStyle, RDtoStyle, RubyDtoStyle,
};
pub use e2e::E2eConfig;
pub use extras::{AdapterConfig, AdapterParam, AdapterPattern, Language, is_known_language};
pub use languages::{
    CSharpConfig, CapsuleTypeConfig, CustomModulesConfig, CustomRegistration, CustomRegistrationsConfig, DartConfig,
    DartStyle, ElixirConfig, FfiCapsuleTypeConfig, FfiConfig, FfiTargetDepOverride, GleamConfig,
    GleamElementConstructor, GleamElementField, GoConfig, HostCapsuleTypeConfig, JavaConfig, JniConfig,
    KotlinAndroidConfig, KotlinConfig, KotlinFfiStyle, KotlinTarget, NapiTypeTagConfig, NodeCapsuleTypeConfig,
    NodeConfig, PhpConfig, PythonConfig, RConfig, RubyConfig, StubsConfig, SwiftConfig, WasmConfig, ZigConfig,
};
pub use legacy::{LegacyConfigError, LegacyKey, detect_legacy_keys};
pub use new_config::{NewAlefConfig, ResolveError};
pub use output::{
    BuildCommandConfig, CitationAuthor, CitationConfig, CleanConfig, DocsConfig, DocsLlmsConfig,
    DocsSkillTemplateConfig, DocsSkillsConfig, DocsSnippetsConfig, DocsSourceConfig, ExcludeConfig,
    GeneratedHeaderConfig, IncludeConfig, LintConfig, OutputConfig, OutputTemplate, ReadmeConfig, ScaffoldCargo,
    ScaffoldCargoEnvValue, ScaffoldCargoTargets, ScaffoldConfig, SetupConfig, SyncConfig, TestConfig, TextReplacement,
    UpdateConfig,
};
pub use package_metadata::PackageMetadataConfig;
pub use poly::{PolyConfig, TyposConfig};
pub use publish::{PublishConfig, PublishLanguageConfig, VendorMode};
pub use raw_crate::RawCrateConfig;
pub use resolve_helpers::{detect_serde_available, resolve_output_dir};
pub use resolved::ResolvedCrateConfig;
pub use schema::{
    DEFAULT_SCHEMA_PATH, alef_config_schema, check_alef_config_schema, render_alef_config_schema,
    write_alef_config_schema,
};
pub use service::{EntrypointSpec, HandlerContractConfig, RegistrationSpec, ServiceConfig};
pub use tools::{DEFAULT_RUST_DEV_TOOLS, LangContext, ToolsConfig, require_tool, require_tools};
pub use trait_bridge::{BridgeBinding, TraitBridgeConfig};
pub use workspace::WorkspaceConfig;

/// A source crate group for multi-crate extraction.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceCrate {
    /// Crate name (hyphens converted to underscores for rust_path).
    pub name: String,
    /// Source files belonging to this crate.
    ///
    /// When [`from_registry`](Self::from_registry) is `false` (the default), these paths are
    /// resolved relative to the consumer workspace root — a sibling checkout must be present.
    ///
    /// When `from_registry = true`, each path is treated as **relative to the crate's
    /// source directory in the cargo registry** (e.g. `~/.cargo/registry/src/…`). Alef
    /// locates that directory via `cargo metadata` so no sibling checkout is required.
    pub sources: Vec<std::path::PathBuf>,
    /// Type roots to import from this crate as external DTOs.
    ///
    /// When empty, this entry behaves as a normal multi-crate source group.
    /// When non-empty, Alef extracts only the transitive binding-safe type graph
    /// reachable from these roots and merges those DTOs into the host crate's
    /// binding surface without importing functions or services.
    #[serde(default)]
    pub roots: Vec<String>,
    /// Resolve sources from the cargo registry instead of a sibling workspace checkout.
    ///
    /// When `true`, Alef runs `cargo metadata` against the consumer workspace and
    /// rebases each entry in [`sources`](Self::sources) against the registry source
    /// directory of the crate named [`name`](Self::name). This makes regeneration
    /// hermetic: CI, worktrees, and fresh clones do not need a sibling checkout of
    /// the dependency.
    ///
    /// Defaults to `false` for full backward compatibility.
    #[serde(default)]
    pub from_registry: bool,
}

fn default_true() -> bool {
    true
}

/// Controls which generation passes alef runs.
/// All flags default to `true`; set to `false` to skip a pass.
/// Can be overridden per-language via `[generate_overrides.<lang>]`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
