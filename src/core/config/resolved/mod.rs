//! `ResolvedCrateConfig` — the per-crate view that backends consume.
//!
//! Every backend in `alef-backend-*`, every codegen pass, scaffold step,
//! e2e generator, and publish step takes a `&ResolvedCrateConfig`. The
//! resolved view is what you get after merging a [`crate::core::config::raw_crate::RawCrateConfig`]
//! with the workspace [`crate::core::config::workspace::WorkspaceConfig`] defaults.
//!
//! Resolution merges values *into* a per-crate value. Workspace defaults
//! that the crate did not override are folded in. Output paths are
//! resolved through the workspace [`crate::core::config::output::OutputTemplate`]
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

use crate::core::config::SourceCrate;
use crate::core::config::dto::DtoConfig;
use crate::core::config::e2e::E2eConfig;
use crate::core::config::extras::{AdapterConfig, Language};
use crate::core::config::languages::{
    CSharpConfig, CustomModulesConfig, CustomRegistrationsConfig, DartConfig, ElixirConfig, FfiConfig, GleamConfig,
    GoConfig, JavaConfig, JniConfig, KotlinAndroidConfig, KotlinConfig, NodeConfig, PhpConfig, PythonConfig, RConfig,
    RubyConfig, SwiftConfig, WasmConfig, ZigConfig,
};
use crate::core::config::output::{
    BuildCommandConfig, CitationConfig, CleanConfig, DocsConfig, ExcludeConfig, IncludeConfig, LintConfig,
    OutputConfig, ReadmeConfig, ScaffoldConfig, SetupConfig, SyncConfig, TestConfig, UpdateConfig,
};
use crate::core::config::package_metadata::PackageMetadataConfig;
use crate::core::config::poly::PolyConfig;
use crate::core::config::publish::PublishConfig;
use crate::core::config::service::{HandlerContractConfig, ServiceConfig};
use crate::core::config::tools::ToolsConfig;
use crate::core::config::trait_bridge::TraitBridgeConfig;
use crate::core::config::workspace::ClientConstructorConfig;
use crate::core::config::{FormatConfig, GenerateConfig};

/// Fully-resolved configuration for one crate.
///
/// Backends consume `&ResolvedCrateConfig`; they should not need to look at
/// the workspace defaults directly. Anything a backend reads has already been
/// merged in by [`crate::core::config::NewAlefConfig::resolve`].
///
/// `output_paths` is precomputed: for every language this crate targets, the
/// map holds the resolved output directory (with `{crate}` and `{lang}`
/// placeholders substituted).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    pub jni: Option<JniConfig>,
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

    /// Resolved output directory per language code (`"python"` → `packages/python/sample_project/`).
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
    pub client_constructors: HashMap<String, ClientConstructorConfig>,
    pub sync: Option<SyncConfig>,
    pub citation: Option<CitationConfig>,

    // -----------------------------------------------------------------
    // Packaging, e2e, extensibility
    // -----------------------------------------------------------------
    pub publish: Option<PublishConfig>,
    pub e2e: Option<E2eConfig>,
    pub adapters: Vec<AdapterConfig>,
    pub trait_bridges: Vec<TraitBridgeConfig>,
    pub services: Vec<ServiceConfig>,
    pub handler_contracts: Vec<HandlerContractConfig>,
    pub scaffold: Option<ScaffoldConfig>,
    pub package_metadata: Option<PackageMetadataConfig>,
    pub readme: Option<ReadmeConfig>,
    pub docs: Option<DocsConfig>,
    pub custom_files: HashMap<String, Vec<PathBuf>>,
    pub custom_modules: CustomModulesConfig,
    pub custom_registrations: CustomRegistrationsConfig,
    /// Validation diagnostic codes downgraded from errors to warnings for this
    /// crate. Set via `suppress_validation_codes` in `[[crates]]`. Generation
    /// proceeds when every error matches a suppressed code; unmatched errors
    /// still fail.
    pub suppress_validation_codes: Vec<String>,

    /// Resolved from [`crate::core::config::raw_crate::RawCrateConfig::untagged_union_text_types`].
    ///
    /// Untagged-union type names whose generated binding wrappers (Go / Java / C#)
    /// should receive an additional `Text()` / `text()` display-text accessor.
    /// Empty by default — no accessors are emitted.
    pub untagged_union_text_types: Vec<String>,

    /// Repository-level `poly.toml` customisations inherited from the workspace
    /// config.  Every resolved crate receives the same repo-wide poly settings.
    /// The poly emitter merges these into its generated output.
    pub poly: PolyConfig,

    /// Extra clippy lints to allow in generated Rust binding files, inherited
    /// from `[workspace] extra_clippy_allows`. Empty by default.
    pub extra_clippy_allows: Vec<String>,
}

impl ResolvedCrateConfig {
    /// Rust source paths that affect extraction and generated output hashes.
    #[must_use]
    pub fn source_hash_paths(&self) -> Vec<PathBuf> {
        let mut sources = self.sources.clone();
        for source_crate in &self.source_crates {
            sources.extend(source_crate.sources.iter().cloned());
        }
        sources.sort();
        sources.dedup();
        sources
    }

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
