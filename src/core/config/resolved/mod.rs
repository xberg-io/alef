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

mod effective_languages;
pub mod ffi;
pub mod fields;
pub mod identifiers;
pub mod imports;
pub mod lookups;
pub mod naming;
#[cfg(test)]
mod package_dir_trailing_slash_tests;
#[cfg(test)]
mod removed_command_config_tests;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::core::config::GenerateConfig;
use crate::core::config::SourceCrate;
use crate::core::config::cargo_lints::CargoLintsConfig;
use crate::core::config::dto::DtoConfig;
use crate::core::config::e2e::E2eConfig;
use crate::core::config::extras::{AdapterConfig, Language};
use crate::core::config::languages::{
    CSharpConfig, CustomModulesConfig, CustomRegistrationsConfig, DartConfig, ElixirConfig, FfiConfig, GleamConfig,
    GoConfig, JavaConfig, JniConfig, KotlinAndroidConfig, KotlinConfig, NodeConfig, PhpConfig, PythonConfig, RConfig,
    RubyConfig, SwiftConfig, WasmConfig, ZigConfig,
};
#[cfg(test)]
use crate::core::config::output::BuildCommandConfig;
use crate::core::config::output::{
    CitationConfig, DocsConfig, ExcludeConfig, IncludeConfig, OutputConfig, ReadmeConfig, ScaffoldConfig, SyncConfig,
    TestConfig,
};
use crate::core::config::package_metadata::PackageMetadataConfig;
use crate::core::config::poly::PolyConfig;
use crate::core::config::publish::PublishConfig;
use crate::core::config::service::{HandlerContractConfig, ServiceConfig};
use crate::core::config::tools::ToolsConfig;
use crate::core::config::trait_bridge::TraitBridgeConfig;
use crate::core::config::verify::VerifyConfig;
use crate::core::config::workspace::ClientConstructorConfig;

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
    pub name: String,
    pub sources: Vec<PathBuf>,
    /// Unresolved `[[crates.source_crates]]` entries, exactly as declared in config. A
    /// `from_registry = true` entry's `sources` are still relative to the registry crate's own
    /// source directory, NOT yet rebased against it -- rebasing shells out to `cargo metadata`
    /// (see `crate::core::config::registry::resolve_crate_source_dir`), which is a cost (and a
    /// failure mode) every subcommand should not pay just to build this config. Use
    /// [`Self::resolved_source_crates`] to get the rebased view; it resolves lazily on first
    /// access and caches the result. ~keep
    pub source_crates: Vec<SourceCrate>,
    /// Cache for [`Self::resolved_source_crates`]. Not serialized: a config loaded from a
    /// snapshot re-resolves on first access rather than trusting a stale rebased path. `pub`
    /// (like every other field here) only so `ResolvedCrateConfig { .., ..Default::default() }`
    /// struct-update syntax keeps compiling from outside this crate (e.g. `tests/*.rs`) --
    /// prefer [`Self::resolved_source_crates`] over touching this directly. ~keep
    #[serde(skip)]
    pub resolved_source_crates: OnceLock<Vec<SourceCrate>>,
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

    pub languages: Vec<Language>,

    /// Resolved per-target opt-out toggles: workspace `[targets]` defaults with
    /// per-crate `[[crates]] targets` overrides merged in. Empty means every
    /// target is enabled (the default). Consumed via [`Self::target_enabled`].
    pub targets: std::collections::BTreeMap<String, bool>,

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

    pub test: HashMap<String, TestConfig>,

    /// Test-only: `#[cfg(test)]`-gated build-command overrides a build-orchestration test set
    /// directly on an in-memory `ResolvedCrateConfig`, never through `alef.toml` -- 0.82.0
    /// removed `[build_commands.<lang>]` / `[workspace.build_commands.<lang>]` /
    /// `[crates.build_commands.<lang>]` from the schema entirely (`RawCrateConfig` and
    /// `WorkspaceConfig` no longer have a `build_commands` field for `resolve()` to populate
    /// this from), so in a real build this map is always empty and
    /// `build_command_config_for_language` returns alef's own built-in default unconditionally.
    /// It survives only so `cli::pipeline::commands::build`'s hermetic regression tests
    /// (`build_orchestration_tests`, `complete_generated_artifacts_staging_order_tests`) can keep
    /// substituting a deterministic `touch`/`true`/`false`/`exit N` for a real npm/go/php/cargo
    /// invocation, exactly as they did when this was a real config table. ~keep
    #[cfg(test)]
    #[serde(skip)]
    pub build_commands: HashMap<String, BuildCommandConfig>,

    pub generate: GenerateConfig,
    pub generate_overrides: HashMap<String, GenerateConfig>,
    pub dto: DtoConfig,

    pub tools: ToolsConfig,
    pub opaque_types: BTreeMap<String, String>,
    pub client_constructors: HashMap<String, ClientConstructorConfig>,
    pub sync: Option<SyncConfig>,
    pub citation: Option<CitationConfig>,

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

    /// Resolved from [`crate::core::config::raw_crate::RawCrateConfig::crate_attributes`].
    ///
    /// Custom inner attribute bodies (e.g. `recursion_limit = "256"`) injected into
    /// every generated Rust `lib.rs` for this crate, across every Rust-emitting
    /// backend. Entries are already validated for well-formedness — see
    /// [`crate::core::config::new_config::NewAlefConfig::resolve`]. Empty by default.
    pub crate_attributes: Vec<String>,

    /// Resolved from [`crate::core::config::raw_crate::RawCrateConfig::cargo_lints`].
    ///
    /// Raw `[lints.rust]` / `[lints.clippy]` tables spliced into every generated Rust
    /// binding-crate `Cargo.toml` for this crate. Empty by default — no `[lints]`
    /// table is emitted and output is byte-identical to a crate that does not set
    /// this field.
    pub cargo_lints: CargoLintsConfig,

    /// Resolved from [`crate::core::config::raw_crate::RawCrateConfig::verify`].
    ///
    /// `alef verify` opt-outs for this crate -- currently just `ignore_ephemeral`, glob
    /// patterns naming generated output the consumer deliberately never commits. Empty by
    /// default, so every check runs at full scope for a crate that never configures it.
    pub verify: VerifyConfig,
}

impl ResolvedCrateConfig {
    /// The rebased view of [`Self::source_crates`]: for each entry with `from_registry = true`,
    /// `sources` rebased against that crate's actual location in the cargo registry (everything
    /// else is returned unchanged). Resolved on first call and cached for the lifetime of this
    /// config -- only codegen backends and the sources hash need this, so a command that never
    /// calls it (e.g. `alef publish package`, a pure archive-the-artifact operation) never pays
    /// for the underlying `cargo metadata` shell-out, and never fails because of it either.
    ///
    /// # Errors
    ///
    /// Returns [`crate::core::config::ResolveError::RegistryResolution`] when a
    /// `from_registry = true` entry's crate cannot be located (e.g. `cargo metadata` fails, or
    /// the crate is not in the resolved dependency graph). The failure is not cached, so a
    /// caller may retry after fixing the underlying cause (e.g. `workspace_root`). ~keep
    pub fn resolved_source_crates(&self) -> Result<&[SourceCrate], crate::core::config::ResolveError> {
        if let Some(resolved) = self.resolved_source_crates.get() {
            return Ok(resolved);
        }
        let resolved = crate::core::config::new_config::resolve_source_crates(
            &self.source_crates,
            self.workspace_root.as_deref(),
        )?;
        // Another caller may have raced this one to `set` -- either value is a valid resolution
        // of the same immutable `self.source_crates`/`workspace_root`, so losing the race and
        // reading back whichever one won is correct, not a bug. ~keep
        let _ = self.resolved_source_crates.set(resolved);
        Ok(self
            .resolved_source_crates
            .get()
            .expect("just set or set by a racing caller"))
    }

    /// Rust source paths that affect extraction and generated output hashes.
    ///
    /// # Errors
    ///
    /// Propagates [`Self::resolved_source_crates`]'s error when a `from_registry = true` source
    /// crate cannot be resolved.
    pub fn source_hash_paths(&self) -> Result<Vec<PathBuf>, crate::core::config::ResolveError> {
        let mut sources = self.sources.clone();
        for source_crate in self.resolved_source_crates()? {
            sources.extend(source_crate.sources.iter().cloned());
        }
        sources.sort();
        sources.dedup();
        Ok(sources)
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

    /// Whether the given Rust target triple is enabled for this crate's
    /// generated target lists, per the resolved `[targets]` opt-out table.
    ///
    /// Returns `true` unless the triple's canonical target key is present with
    /// an explicit `false`. Triples with no canonical key (arm/wasm32) are
    /// always enabled.
    #[must_use]
    pub fn target_enabled(&self, triple: &str) -> bool {
        crate::publish::platform::target_triple_enabled(&self.targets, triple)
    }
}
