//! `NewAlefConfig` and `ResolveError` — the multi-crate config schema.

use std::collections::HashMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::extras::{Language, is_known_language};
use super::output::{BuildCommandConfig, GeneratedHeaderConfig, ScaffoldConfig};
use super::package_metadata::PackageMetadataConfig;
use super::raw_crate::RawCrateConfig;
use super::resolve_helpers::{merge_map, resolve_output_paths};
use super::resolved::ResolvedCrateConfig;
use super::workspace::WorkspaceConfig;

/// Error variants produced when resolving a [`NewAlefConfig`] into per-crate views.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// Two `[[crates]]` entries share the same `name`.
    #[error("duplicate crate name `{0}` — every [[crates]] entry must have a unique name")]
    DuplicateCrateName(String),

    /// A crate has no target languages after merging workspace and per-crate config.
    #[error("crate `{0}` has no target languages — set `languages` on the crate or in `[workspace]`")]
    EmptyLanguages(String),

    /// Two or more crates would write to the same output path for the same language.
    #[error(
        "overlapping output path for language `{lang}`: `{path}` is claimed by crates: {crates}",
        path = path.display(),
        crates = crates.join(", ")
    )]
    OverlappingOutputPath {
        lang: String,
        path: PathBuf,
        crates: Vec<String>,
    },

    /// A crate has an invalid or incompatible configuration.
    #[error("{0}")]
    InvalidConfig(String),
}

/// Top-level multi-crate configuration (new schema).
///
/// Deserializes from an `alef.toml` that has a `[workspace]` section and one
/// or more `[[crates]]` entries.  Call [`NewAlefConfig::resolve`] to produce
/// the per-crate [`ResolvedCrateConfig`] list that backends consume.
///
/// ```toml
/// [workspace]
/// languages = ["python", "node"]
///
/// [[crates]]
/// name = "sample_project"
/// sources = ["src/lib.rs"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewAlefConfig {
    /// Workspace-level shared defaults.
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// One entry per independently published binding package.
    pub crates: Vec<RawCrateConfig>,
    /// Opaque per-extension configuration tables. alef does not interpret these;
    /// each registered [`crate::core::extension::Extension`] reads its own
    /// `[extensions.<name>]` table via
    /// [`crate::core::extension::read_extension_config`]. Declaring the field
    /// keeps `deny_unknown_fields` typo protection while admitting extension
    /// sections inline in `alef.toml`.
    #[serde(default)]
    #[schemars(skip)]
    pub extensions: std::collections::BTreeMap<String, toml::Value>,
}

impl NewAlefConfig {
    /// Merge workspace defaults into each crate and validate the result.
    ///
    /// Returns a `Vec<ResolvedCrateConfig>` in the same order as `self.crates`.
    ///
    /// # Errors
    ///
    /// - [`ResolveError::DuplicateCrateName`] when two crates share a name.
    /// - [`ResolveError::EmptyLanguages`] when a crate has no target languages.
    /// - [`ResolveError::OverlappingOutputPath`] when two crates resolve to the
    ///   same output directory for the same language.
    pub fn resolve(&self) -> Result<Vec<ResolvedCrateConfig>, ResolveError> {
        // --- Uniqueness check ---------------------------------------------------
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (idx, krate) in self.crates.iter().enumerate() {
            if seen.insert(krate.name.as_str(), idx).is_some() {
                return Err(ResolveError::DuplicateCrateName(krate.name.clone()));
            }
        }

        let multi_crate = self.crates.len() > 1;
        let mut resolved: Vec<ResolvedCrateConfig> = Vec::with_capacity(self.crates.len());

        for krate in &self.crates {
            resolved.push(self.resolve_one(krate, multi_crate)?);
        }

        // --- Overlapping output path check --------------------------------------
        // For each language, build a map path → crate names; error on any dup.
        let mut path_owners: HashMap<String, HashMap<PathBuf, Vec<String>>> = HashMap::new();
        for cfg in &resolved {
            for (lang, path) in &cfg.output_paths {
                path_owners
                    .entry(lang.clone())
                    .or_default()
                    .entry(path.clone())
                    .or_default()
                    .push(cfg.name.clone());
            }
        }
        for (lang, path_map) in path_owners {
            for (path, crates) in path_map {
                if crates.len() > 1 {
                    return Err(ResolveError::OverlappingOutputPath { lang, path, crates });
                }
            }
        }

        Ok(resolved)
    }

    fn resolve_one(&self, krate: &RawCrateConfig, multi_crate: bool) -> Result<ResolvedCrateConfig, ResolveError> {
        let ws = &self.workspace;

        // --- Languages ----------------------------------------------------------
        let languages: Vec<Language> = match krate.languages.as_deref() {
            Some(langs) if !langs.is_empty() => langs.to_vec(),
            Some(_) => {
                // Explicitly empty per-crate list: treat as "no override" and use workspace.
                if ws.languages.is_empty() {
                    return Err(ResolveError::EmptyLanguages(krate.name.clone()));
                }
                ws.languages.clone()
            }
            None => {
                if ws.languages.is_empty() {
                    return Err(ResolveError::EmptyLanguages(krate.name.clone()));
                }
                ws.languages.clone()
            }
        };

        // --- Output paths -------------------------------------------------------
        let output_paths = resolve_output_paths(krate, &ws.output_template, &languages, multi_crate);

        // --- HashMap pipelines -------------------------------------------------
        // Most per-language pipeline maps use per-key wholesale overlay. Build
        // commands are intentionally field-wise so workspace defaults and crate
        // overrides can compose without restating preconditions/release commands.
        // `path_mappings` and `extra_dependencies` are intentionally not merged
        // here: they remain strictly per-crate and are taken verbatim below.
        let lint = merge_map(&ws.lint, &krate.lint);
        let test = merge_map(&ws.test, &krate.test);
        let setup = merge_map(&ws.setup, &krate.setup);
        let update = merge_map(&ws.update, &krate.update);
        let clean = merge_map(&ws.clean, &krate.clean);
        let build_commands = merge_build_command_maps(&ws.build_commands, &krate.build_commands);
        let format_overrides = merge_map(&ws.format_overrides, &krate.format_overrides);
        let generate_overrides = merge_map(&ws.generate_overrides, &krate.generate_overrides);

        // --- Cross-language validation ------------------------------------------
        // alef-backend-jni is paired with kotlin-android: the JNI backend derives
        // the package, bridge class name, and feature list from kotlin_android
        // config. Without it the backend has no symbol prefix to emit.
        if languages.contains(&Language::Jni) && !languages.contains(&Language::KotlinAndroid) {
            return Err(ResolveError::InvalidConfig(format!(
                "crate `{}`: language `jni` requires `kotlin_android` to also be enabled in languages",
                krate.name
            )));
        }

        // --- Adapter skip_languages validation ----------------------------------
        // Reject unknown language names in adapter skip_languages early so
        // typos like "wasm32" instead of "wasm" fail loudly at config-resolve time.
        for adapter in &krate.adapters {
            for lang in &adapter.skip_languages {
                if !is_known_language(lang.as_str()) {
                    return Err(ResolveError::InvalidConfig(format!(
                        "crate `{}`: adapter `{}` has unknown language `{}` in skip_languages; \
                         valid names are: python, node, ruby, php, elixir, wasm, ffi, go, java, \
                         csharp, r, rust, kotlin, kotlin_android, swift, dart, gleam, zig, c, jni",
                        krate.name, adapter.name, lang
                    )));
                }
            }
        }

        // --- Service skip_languages validation ----------------------------------
        for service in &krate.services {
            for lang in &service.skip_languages {
                if !is_known_language(lang.as_str()) {
                    return Err(ResolveError::InvalidConfig(format!(
                        "crate `{}`: service `{}` has unknown language `{}` in skip_languages; \
                         valid names are: python, node, ruby, php, elixir, wasm, ffi, go, java, \
                         csharp, r, rust, kotlin, kotlin_android, swift, dart, gleam, zig, c, jni",
                        krate.name, service.owner_type, lang
                    )));
                }
            }
        }

        // --- Service/handler-contract cross-reference validation ----------------
        // Every registration's callback_contract must match a handler_contract trait_name.
        let contract_names: std::collections::HashSet<&str> = krate
            .handler_contracts
            .iter()
            .map(|hc| hc.trait_name.as_str())
            .collect();
        for service in &krate.services {
            for reg in &service.registrations {
                if !contract_names.contains(reg.callback_contract.as_str()) {
                    return Err(ResolveError::InvalidConfig(format!(
                        "crate `{}`: service `{}` registration `{}` references \
                         callback_contract `{}` which is not declared in [[crates.handler_contracts]]",
                        krate.name, service.owner_type, reg.method, reg.callback_contract
                    )));
                }
            }
            // Validate entrypoint kinds are recognised
            for ep in &service.entrypoints {
                if ep.kind != "run" && ep.kind != "finalize" {
                    return Err(ResolveError::InvalidConfig(format!(
                        "crate `{}`: service `{}` entrypoint `{}` has unknown kind `{}`; \
                         valid values are: `run`, `finalize`",
                        krate.name, service.owner_type, ep.method, ep.kind
                    )));
                }
            }
        }

        Ok(ResolvedCrateConfig {
            name: krate.name.clone(),
            sources: krate.sources.clone(),
            source_crates: krate.source_crates.clone(),
            version_from: krate.version_from.clone().unwrap_or_else(|| "Cargo.toml".to_string()),
            core_import: krate.core_import.clone(),
            workspace_root: krate.workspace_root.clone(),
            skip_core_import: krate.skip_core_import,
            error_type: krate.error_type.clone(),
            error_constructor: krate.error_constructor.clone(),
            features: krate.features.clone(),
            path_mappings: krate.path_mappings.clone(),
            extra_dependencies: krate.extra_dependencies.clone(),
            auto_path_mappings: krate.auto_path_mappings.unwrap_or(true),
            languages,
            python: krate.python.clone().or_else(|| ws.python.clone()),
            node: krate.node.clone().or_else(|| ws.node.clone()),
            ruby: krate.ruby.clone().or_else(|| ws.ruby.clone()),
            php: krate.php.clone().or_else(|| ws.php.clone()),
            elixir: krate.elixir.clone().or_else(|| ws.elixir.clone()),
            wasm: krate.wasm.clone().or_else(|| ws.wasm.clone()),
            ffi: krate.ffi.clone().or_else(|| ws.ffi.clone()),
            go: krate.go.clone().or_else(|| ws.go.clone()),
            java: krate.java.clone().or_else(|| ws.java.clone()),
            dart: krate.dart.clone().or_else(|| ws.dart.clone()),
            kotlin: krate.kotlin.clone().or_else(|| ws.kotlin.clone()),
            kotlin_android: krate.kotlin_android.clone().or_else(|| ws.kotlin_android.clone()),
            jni: krate.jni.clone().or_else(|| ws.jni.clone()),
            swift: krate.swift.clone().or_else(|| ws.swift.clone()),
            gleam: krate.gleam.clone().or_else(|| ws.gleam.clone()),
            csharp: krate.csharp.clone().or_else(|| ws.csharp.clone()),
            r: krate.r.clone().or_else(|| ws.r.clone()),
            zig: krate.zig.clone().or_else(|| ws.zig.clone()),
            exclude: krate.exclude.clone(),
            include: krate.include.clone(),
            output_paths,
            explicit_output: krate.output.clone(),
            lint,
            test,
            setup,
            update,
            clean,
            build_commands,
            // Per-crate generate/format/dto override the workspace value when set.
            // None inherits the workspace default. tools and opaque_types are
            // workspace-only by design (see WorkspaceConfig docs).
            generate: krate.generate.clone().unwrap_or_else(|| ws.generate.clone()),
            generate_overrides,
            format: krate.format.clone().unwrap_or_else(|| ws.format.clone()),
            format_overrides,
            dto: krate.dto.clone().unwrap_or_else(|| ws.dto.clone()),
            tools: ws.tools.clone(),
            opaque_types: ws.opaque_types.clone(),
            client_constructors: ws.client_constructors.clone(),
            sync: ws.sync.clone(),
            citation: ws.citation.clone(),
            publish: krate.publish.clone(),
            e2e: krate.e2e.clone(),
            adapters: krate.adapters.clone(),
            trait_bridges: krate.trait_bridges.clone(),
            services: krate.services.clone(),
            handler_contracts: krate.handler_contracts.clone(),
            scaffold: merge_scaffold(
                ws.scaffold.as_ref(),
                krate.scaffold.as_ref(),
                ws.generated_header.as_ref(),
            ),
            package_metadata: PackageMetadataConfig::merge(
                ws.package_metadata.as_ref(),
                krate.package_metadata.as_ref(),
            ),
            readme: krate.readme.clone(),
            docs: super::output::DocsConfig::merge(ws.docs.as_ref(), krate.docs.as_ref()),
            custom_files: krate.custom_files.clone(),
            custom_modules: krate.custom_modules.clone(),
            custom_registrations: krate.custom_registrations.clone(),
            suppress_validation_codes: krate.suppress_validation_codes.clone(),
            untagged_union_text_types: krate.untagged_union_text_types.clone(),
            poly: ws.poly.clone(),
            extra_clippy_allows: ws.extra_clippy_allows.clone(),
        })
    }
}

fn merge_scaffold(
    workspace: Option<&ScaffoldConfig>,
    krate: Option<&ScaffoldConfig>,
    workspace_header: Option<&GeneratedHeaderConfig>,
) -> Option<ScaffoldConfig> {
    if workspace.is_none() && krate.is_none() && workspace_header.is_none() {
        return None;
    }

    let generated_header = merge_generated_header(
        workspace.and_then(|s| s.generated_header.as_ref()).or(workspace_header),
        krate.and_then(|s| s.generated_header.as_ref()),
    );

    Some(ScaffoldConfig {
        description: krate
            .and_then(|s| s.description.clone())
            .or_else(|| workspace.and_then(|s| s.description.clone())),
        license: krate
            .and_then(|s| s.license.clone())
            .or_else(|| workspace.and_then(|s| s.license.clone())),
        repository: krate
            .and_then(|s| s.repository.clone())
            .or_else(|| workspace.and_then(|s| s.repository.clone())),
        homepage: krate
            .and_then(|s| s.homepage.clone())
            .or_else(|| workspace.and_then(|s| s.homepage.clone())),
        authors: krate
            .filter(|s| !s.authors.is_empty())
            .map(|s| s.authors.clone())
            .or_else(|| workspace.map(|s| s.authors.clone()))
            .unwrap_or_default(),
        keywords: krate
            .filter(|s| !s.keywords.is_empty())
            .map(|s| s.keywords.clone())
            .or_else(|| workspace.map(|s| s.keywords.clone()))
            .unwrap_or_default(),
        generated_header,
        cargo: krate
            .and_then(|s| s.cargo.clone())
            .or_else(|| workspace.and_then(|s| s.cargo.clone())),
    })
}

fn merge_generated_header(
    workspace: Option<&GeneratedHeaderConfig>,
    krate: Option<&GeneratedHeaderConfig>,
) -> Option<GeneratedHeaderConfig> {
    if workspace.is_none() && krate.is_none() {
        return None;
    }
    Some(GeneratedHeaderConfig {
        issues_url: krate
            .and_then(|h| h.issues_url.clone())
            .or_else(|| workspace.and_then(|h| h.issues_url.clone())),
        regenerate_command: krate
            .and_then(|h| h.regenerate_command.clone())
            .or_else(|| workspace.and_then(|h| h.regenerate_command.clone())),
        verify_command: krate
            .and_then(|h| h.verify_command.clone())
            .or_else(|| workspace.and_then(|h| h.verify_command.clone())),
    })
}

fn merge_build_command_maps(
    workspace: &HashMap<String, BuildCommandConfig>,
    krate: &HashMap<String, BuildCommandConfig>,
) -> HashMap<String, BuildCommandConfig> {
    let mut merged = workspace.clone();
    for (lang, override_cfg) in krate {
        let next = merged
            .remove(lang)
            .map(|base| base.merge_overlay(override_cfg))
            .unwrap_or_else(|| override_cfg.clone());
        merged.insert(lang.clone(), next);
    }
    merged
}

#[cfg(test)]
mod tests;
