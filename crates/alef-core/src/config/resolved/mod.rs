//! `ResolvedCrateConfig` — the per-crate view that backends consume.
//!
//! Every backend in `alef-backend-*`, every codegen pass, scaffold step,
//! e2e generator, and publish step takes a `&ResolvedCrateConfig`. The
//! resolved view is what you get after merging a [`crate::config::raw_crate::RawCrateConfig`]
//! with the workspace [`crate::config::workspace::WorkspaceConfig`] defaults.
//!
//! Resolution merges values *into* a per-crate value. Workspace defaults
//! that the crate did not override are folded in. Output paths are
//! resolved through the workspace [`crate::config::output::OutputTemplate`]
//! unless the crate set an explicit path in its `[crates.output]` table.

pub mod ffi;
pub mod fields;
pub mod identifiers;
pub mod imports;
pub mod lookups;
pub mod naming;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::SourceCrate;
use crate::config::dto::DtoConfig;
use crate::config::e2e::E2eConfig;
use crate::config::extras::{AdapterConfig, Language};
use crate::config::languages::{
    CSharpConfig, CustomModulesConfig, CustomRegistrationsConfig, DartConfig, ElixirConfig, FfiConfig, GleamConfig,
    GoConfig, JavaConfig, KotlinAndroidConfig, KotlinConfig, NodeConfig, PhpConfig, PythonConfig, RConfig, RubyConfig,
    SwiftConfig, WasmConfig, ZigConfig,
};
use crate::config::output::{
    BuildCommandConfig, CleanConfig, ExcludeConfig, IncludeConfig, LintConfig, OutputConfig, ReadmeConfig,
    ScaffoldConfig, SetupConfig, SyncConfig, TestConfig, UpdateConfig,
};
use crate::config::publish::PublishConfig;
use crate::config::tools::ToolsConfig;
use crate::config::trait_bridge::TraitBridgeConfig;
use crate::config::{FormatConfig, GenerateConfig};

/// Fully-resolved configuration for one crate.
///
/// Backends consume `&ResolvedCrateConfig`; they should not need to look at
/// the workspace defaults directly. Anything a backend reads has already been
/// merged in by [`crate::config::NewAlefConfig::resolve`].
///
/// `output_paths` is precomputed: for every language this crate targets, the
/// map holds the resolved output directory (with `{crate}` and `{lang}`
/// placeholders substituted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedCrateConfig {
    // -----------------------------------------------------------------
    // Identity
    // -----------------------------------------------------------------
    pub name: String,
    pub sources: Vec<PathBuf>,
    pub source_crates: Vec<SourceCrate>,
    pub version_from: String,
    pub core_import: Option<String>,
    pub workspace_root: Option<PathBuf>,
    pub skip_core_import: bool,
    pub error_type: Option<String>,
    pub error_constructor: Option<String>,
    pub features: Vec<String>,
    pub path_mappings: HashMap<String, String>,
    pub extra_dependencies: HashMap<String, toml::Value>,
    pub auto_path_mappings: bool,

    // -----------------------------------------------------------------
    // Languages targeted by this crate
    // -----------------------------------------------------------------
    pub languages: Vec<Language>,

    // -----------------------------------------------------------------
    // Per-language settings (workspace defaults already merged)
    // -----------------------------------------------------------------
    pub python: Option<PythonConfig>,
    pub node: Option<NodeConfig>,
    pub ruby: Option<RubyConfig>,
    pub php: Option<PhpConfig>,
    pub elixir: Option<ElixirConfig>,
    pub wasm: Option<WasmConfig>,
    pub ffi: Option<FfiConfig>,
    pub go: Option<GoConfig>,
    pub java: Option<JavaConfig>,
    pub dart: Option<DartConfig>,
    pub kotlin: Option<KotlinConfig>,
    pub kotlin_android: Option<KotlinAndroidConfig>,
    pub swift: Option<SwiftConfig>,
    pub gleam: Option<GleamConfig>,
    pub csharp: Option<CSharpConfig>,
    pub r: Option<RConfig>,
    pub zig: Option<ZigConfig>,

    // -----------------------------------------------------------------
    // Filters
    // -----------------------------------------------------------------
    pub exclude: ExcludeConfig,
    pub include: IncludeConfig,

    /// Resolved output directory per language code (`"python"` → `packages/python/spikard/`).
    /// Only contains entries for languages this crate actually targets.
    pub output_paths: HashMap<String, PathBuf>,

    /// Raw user-supplied per-language output paths from `[crates.output]`.
    ///
    /// Distinct from [`Self::output_paths`]: this preserves the original (possibly
    /// `None`) value so methods that need to distinguish "user explicitly set this
    /// path" from "template-derived" can do so. Used by [`Self::ffi_lib_name`] and
    /// any other consumer that derives identifiers from the user-supplied path.
    pub explicit_output: OutputConfig,

    // -----------------------------------------------------------------
    // Pipelines (workspace defaults merged with per-crate overrides)
    // -----------------------------------------------------------------
    pub lint: HashMap<String, LintConfig>,
    pub test: HashMap<String, TestConfig>,
    pub setup: HashMap<String, SetupConfig>,
    pub update: HashMap<String, UpdateConfig>,
    pub clean: HashMap<String, CleanConfig>,
    pub build_commands: HashMap<String, BuildCommandConfig>,

    // -----------------------------------------------------------------
    // Generation flags
    // -----------------------------------------------------------------
    pub generate: GenerateConfig,
    pub generate_overrides: HashMap<String, GenerateConfig>,
    pub format: FormatConfig,
    pub format_overrides: HashMap<String, FormatConfig>,
    pub dto: DtoConfig,

    // -----------------------------------------------------------------
    // Workspace concerns surfaced to the crate (read-only inheritance)
    // -----------------------------------------------------------------
    pub tools: ToolsConfig,
    pub opaque_types: HashMap<String, String>,
    pub sync: Option<SyncConfig>,

    // -----------------------------------------------------------------
    // Packaging, e2e, extensibility
    // -----------------------------------------------------------------
    pub publish: Option<PublishConfig>,
    pub e2e: Option<E2eConfig>,
    pub adapters: Vec<AdapterConfig>,
    pub trait_bridges: Vec<TraitBridgeConfig>,
    pub scaffold: Option<ScaffoldConfig>,
    pub readme: Option<ReadmeConfig>,
    pub custom_files: HashMap<String, Vec<PathBuf>>,
    pub custom_modules: CustomModulesConfig,
    pub custom_registrations: CustomRegistrationsConfig,
}

impl ResolvedCrateConfig {
    /// Convenience accessor: the resolved output directory for a language.
    /// Returns `None` if this crate does not target the language.
    pub fn output_for(&self, lang: &str) -> Option<&std::path::Path> {
        self.output_paths.get(lang).map(|p| p.as_path())
    }

    /// Whether this crate targets the given language.
    pub fn targets(&self, lang: Language) -> bool {
        self.languages.contains(&lang)
    }
}
