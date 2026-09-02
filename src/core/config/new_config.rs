//! `NewAlefConfig` and `ResolveError` — the multi-crate config schema.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::abi_grammar;
use super::extras::{Language, is_known_language};
use super::languages::FfiConfig;
use super::output::{
    BuildCommandConfig, GeneratedHeaderConfig, ScaffoldConfig, validate_output_path, validate_output_segment,
};
use super::package_metadata::PackageMetadataConfig;
use super::raw_crate::RawCrateConfig;
use super::resolve_helpers::{merge_map, resolve_output_paths};
use super::resolved::ResolvedCrateConfig;
use super::workspace::WorkspaceConfig;

/// Error variants produced when resolving a [`NewAlefConfig`] into per-crate views.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// The config declares no `[[crates]]` entries at all, so every downstream
    /// per-crate command (build, publish, tagging, version checks) would silently
    /// process zero crates and report success.
    #[error(
        "no [[crates]] entries defined — alef.toml must declare at least one crate to generate \
         bindings for; add a [[crates]] table or remove this config"
    )]
    NoCratesConfigured,

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

    /// Registry resolution for a `from_registry = true` source crate failed.
    #[error("registry resolution failed for source crate: {0}")]
    RegistryResolution(String),
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
    /// - [`ResolveError::NoCratesConfigured`] when `[[crates]]` is empty.
    /// - [`ResolveError::DuplicateCrateName`] when two crates share a name.
    /// - [`ResolveError::EmptyLanguages`] when a crate has no target languages.
    /// - [`ResolveError::OverlappingOutputPath`] when two crates resolve to the
    ///   same output directory for the same language.
    pub fn resolve(&self) -> Result<Vec<ResolvedCrateConfig>, ResolveError> {
        if self.crates.is_empty() {
            return Err(ResolveError::NoCratesConfigured);
        }

        let mut seen: HashMap<&str, usize> = HashMap::new();
        for (idx, krate) in self.crates.iter().enumerate() {
            if seen.insert(krate.name.as_str(), idx).is_some() {
                return Err(ResolveError::DuplicateCrateName(krate.name.clone()));
            }
            validate_crate_name_path_safety(&krate.name)?;
        }

        let multi_crate = self.crates.len() > 1;
        let mut resolved: Vec<ResolvedCrateConfig> = Vec::with_capacity(self.crates.len());

        for krate in &self.crates {
            resolved.push(self.resolve_one(krate, multi_crate)?);
        }

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

        validate_no_nuget_package_id_collisions(&resolved)?;

        Ok(resolved)
    }

    fn resolve_one(&self, krate: &RawCrateConfig, multi_crate: bool) -> Result<ResolvedCrateConfig, ResolveError> {
        let ws = &self.workspace;

        let languages: Vec<Language> = match krate.languages.as_deref() {
            Some(langs) if !langs.is_empty() => langs.to_vec(),
            Some(_) => {
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

        let output_paths = resolve_output_paths(krate, &ws.output_template, &languages, multi_crate)?;

        // Per-language config, merged crate-over-workspace, computed once here so the path-safety
        // checks below and the struct literal at the end of this function see the same values —
        // duplicating the `.clone().or_else(...)` merge in both places would let them drift.
        let python = krate.python.clone().or_else(|| ws.python.clone());
        let node = krate.node.clone().or_else(|| ws.node.clone());
        let ruby = krate.ruby.clone().or_else(|| ws.ruby.clone());
        let php = krate.php.clone().or_else(|| ws.php.clone());
        let elixir = krate.elixir.clone().or_else(|| ws.elixir.clone());
        let wasm = krate.wasm.clone().or_else(|| ws.wasm.clone());
        let jni = krate.jni.clone().or_else(|| ws.jni.clone());
        let java = krate.java.clone().or_else(|| ws.java.clone());
        let kotlin = krate.kotlin.clone().or_else(|| ws.kotlin.clone());
        let kotlin_android = krate.kotlin_android.clone().or_else(|| ws.kotlin_android.clone());
        let csharp = krate.csharp.clone().or_else(|| ws.csharp.clone());
        let dart = krate.dart.clone().or_else(|| ws.dart.clone());
        let swift = krate.swift.clone().or_else(|| ws.swift.clone());
        let gleam = krate.gleam.clone().or_else(|| ws.gleam.clone());
        let zig = krate.zig.clone().or_else(|| ws.zig.clone());
        let r = krate.r.clone().or_else(|| ws.r.clone());

        validate_language_specific_path_fields(
            &krate.name,
            PathSafetyFields {
                jni_crate_dir: jni.as_ref().and_then(|c| c.crate_dir.as_deref()),
                node_crate_dir: node.as_ref().and_then(|c| c.crate_dir.as_deref()),
                wasm_crate_dir: wasm.as_ref().and_then(|c| c.crate_dir.as_deref()),
                dart_lib_name: dart.as_ref().and_then(|c| c.lib_name.as_deref()),
                java_package: java.as_ref().and_then(|c| c.package.as_deref()),
                kotlin_package: kotlin.as_ref().and_then(|c| c.package.as_deref()),
                kotlin_android_package: kotlin_android.as_ref().and_then(|c| c.package.as_deref()),
                csharp_namespace: csharp.as_ref().and_then(|c| c.namespace.as_deref()),
                python_module_name: python.as_ref().and_then(|c| c.module_name.as_deref()),
                elixir_app_name: elixir.as_ref().and_then(|c| c.app_name.as_deref()),
                gleam_app_name: gleam.as_ref().and_then(|c| c.app_name.as_deref()),
                swift_module_name: swift.as_ref().and_then(|c| c.module_name.as_deref()),
                zig_module_name: zig.as_ref().and_then(|c| c.module_name.as_deref()),
                ruby_gem_name: ruby.as_ref().and_then(|c| c.gem_name.as_deref()),
                php_extension_name: php.as_ref().and_then(|c| c.extension_name.as_deref()),
                r_package_name: r.as_ref().and_then(|c| c.package_name.as_deref()),
                python_scaffold_output: python.as_ref().and_then(|c| c.scaffold_output.as_deref()),
                node_scaffold_output: node.as_ref().and_then(|c| c.scaffold_output.as_deref()),
                ruby_scaffold_output: ruby.as_ref().and_then(|c| c.scaffold_output.as_deref()),
                php_scaffold_output: php.as_ref().and_then(|c| c.scaffold_output.as_deref()),
                elixir_scaffold_output: elixir.as_ref().and_then(|c| c.scaffold_output.as_deref()),
            },
        )?;

        let lint = merge_map(&ws.lint, &krate.lint);
        let test = merge_map(&ws.test, &krate.test);
        let setup = merge_map(&ws.setup, &krate.setup);
        let update = merge_map(&ws.update, &krate.update);
        let clean = merge_map(&ws.clean, &krate.clean);
        let build_commands = merge_build_command_maps(&ws.build_commands, &krate.build_commands);
        let generate_overrides = merge_map(&ws.generate_overrides, &krate.generate_overrides);

        if languages.contains(&Language::Jni) && !languages.contains(&Language::KotlinAndroid) {
            return Err(ResolveError::InvalidConfig(format!(
                "crate `{}`: language `jni` requires `kotlin_android` to also be enabled in languages",
                krate.name
            )));
        }

        for adapter in &krate.adapters {
            for lang in &adapter.skip_languages {
                if !is_known_language(lang.as_str()) {
                    return Err(ResolveError::InvalidConfig(format!(
                        "crate `{}`: adapter `{}` has unknown language `{}` in skip_languages; \
                         valid names are: {}",
                        krate.name,
                        adapter.name,
                        lang,
                        Language::all_names_joined()
                    )));
                }
            }
        }

        for service in &krate.services {
            for lang in &service.skip_languages {
                if !is_known_language(lang.as_str()) {
                    return Err(ResolveError::InvalidConfig(format!(
                        "crate `{}`: service `{}` has unknown language `{}` in skip_languages; \
                         valid names are: {}",
                        krate.name,
                        service.owner_type,
                        lang,
                        Language::all_names_joined()
                    )));
                }
            }
            for registration in &service.registrations {
                for variant in &registration.variants {
                    for lang in variant.languages.keys() {
                        if !is_known_language(lang.as_str()) {
                            return Err(ResolveError::InvalidConfig(format!(
                                "crate `{}`: service `{}` registration `{}` variant `{}` has \
                                 unknown language `{}` in languages; valid names are: python, \
                                 node, ruby, php, elixir, wasm, ffi, go, java, csharp, r, rust, \
                                 kotlin, kotlin_android, swift, dart, gleam, zig, c, jni",
                                krate.name, service.owner_type, registration.method, variant.name, lang
                            )));
                        }
                    }
                }
            }
        }

        for trait_bridge in &krate.trait_bridges {
            for lang in &trait_bridge.exclude_languages {
                if !is_known_language(lang.as_str()) {
                    return Err(ResolveError::InvalidConfig(format!(
                        "crate `{}`: trait bridge `{}` has unknown language `{}` in \
                         exclude_languages; valid names are: python, node, ruby, php, elixir, \
                         wasm, ffi, go, java, csharp, r, rust, kotlin, kotlin_android, swift, \
                         dart, gleam, zig, c, jni",
                        krate.name, trait_bridge.trait_name, lang
                    )));
                }
            }
        }

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

        let crate_attributes = krate
            .crate_attributes
            .iter()
            .map(|raw| validate_crate_attribute(&krate.name, raw))
            .collect::<Result<Vec<String>, ResolveError>>()?;

        let source_crates = resolve_source_crates(&krate.source_crates, krate.workspace_root.as_deref())?;

        // Per-target toggles: workspace defaults, overridden per key by the crate. ~keep
        let mut targets = ws.targets.clone();
        targets.extend(krate.targets.iter().map(|(k, v)| (k.clone(), *v)));
        for key in targets.keys() {
            if !crate::publish::platform::CANONICAL_TARGET_KEYS.contains(&key.as_str()) {
                return Err(ResolveError::InvalidConfig(format!(
                    "crate `{}`: unknown target key `{}` in `[targets]`; valid keys are: {}",
                    krate.name,
                    key,
                    crate::publish::platform::CANONICAL_TARGET_KEYS.join(", ")
                )));
            }
        }

        let effective_ffi = krate.ffi.clone().or_else(|| ws.ffi.clone());
        if let Some(ffi) = effective_ffi.as_ref() {
            validate_ffi_config(&krate.name, ffi)?;
        }

        let resolved = ResolvedCrateConfig {
            name: krate.name.clone(),
            sources: krate.sources.clone(),
            source_crates,
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
            targets,
            python,
            node,
            ruby,
            php,
            elixir,
            wasm,
            ffi: effective_ffi,
            go: krate.go.clone().or_else(|| ws.go.clone()),
            java,
            dart,
            kotlin,
            kotlin_android,
            jni,
            swift,
            gleam,
            csharp,
            r,
            zig,
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
            generate: krate.generate.clone().unwrap_or_else(|| ws.generate.clone()),
            generate_overrides,
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
            crate_attributes,
            cargo_lints: krate.cargo_lints.clone(),
            verify: krate.verify.clone(),
        };
        validate_all_effective_ffi_configs(&resolved)?;
        validate_package_coordinates(&resolved)?;
        Ok(resolved)
    }
}

fn invalid_coordinate(resolved: &ResolvedCrateConfig, field: &str, value: &str, reason: String) -> ResolveError {
    ResolveError::InvalidConfig(format!(
        "crate `{}`: {field} value `{value}` is not a valid coordinate: {reason}",
        resolved.name
    ))
}

/// Reject out-of-grammar package coordinates before any generator can splice them into a
/// manifest.
///
/// ~keep This runs inside `resolve_one`, so every command that loads an `alef.toml` is covered by
/// construction. Validating at the point of *use* instead would leave each backend free to skip
/// the check, which is how these coordinates reached `pom.xml` and `.csproj` unvalidated: a
/// `package` of `dev"; System.exit(1); //` was emitted verbatim as a `<groupId>`, and a
/// `namespace` of `My.$(Evil)` became a live MSBuild property expansion in a generated project
/// file.
fn validate_package_coordinates(resolved: &ResolvedCrateConfig) -> Result<(), ResolveError> {
    let languages = resolved.effective_languages();
    validate_jvm_coordinates(resolved, &languages)?;
    validate_kotlin_android_coordinates(resolved, &languages)?;
    validate_dotnet_coordinates(resolved, &languages)?;
    validate_swift_coordinates(resolved, &languages)?;
    validate_dart_coordinates(resolved, &languages)?;
    Ok(())
}

/// Reject two `[[crates]]` whose NuGet package IDs collide once case is folded the way NuGet.org
/// itself folds it.
///
/// [`crate::codegen::coordinates::validate_nuget_package_id`]'s own doc comment promises this:
/// "Callers must also check case-insensitive collisions across a workspace." `validate_dotnet_coordinates`
/// (run per crate, inside `resolve_one`) cannot see sibling crates, so this runs once over every
/// resolved crate, the same shape as the `OverlappingOutputPath` check directly above it in
/// [`NewAlefConfig::resolve`] -- two crates publishing `MyLib` and `mylib` would collide on the
/// real registry even though `resolve_one` validates each in isolation and sees no conflict.
fn validate_no_nuget_package_id_collisions(resolved: &[ResolvedCrateConfig]) -> Result<(), ResolveError> {
    use crate::codegen::coordinates::nuget_ordinal_fold;

    let mut owners: HashMap<String, Vec<String>> = HashMap::new();
    for cfg in resolved {
        if !cfg.effective_languages().contains(&Language::Csharp) {
            continue;
        }
        let package_id = cfg.nuget_package_id();
        owners
            .entry(nuget_ordinal_fold(&package_id))
            .or_default()
            .push(cfg.name.clone());
    }
    for (fold, crates) in owners {
        if crates.len() > 1 {
            return Err(ResolveError::InvalidConfig(format!(
                "NuGet package ID collision (case-insensitive, fold key `{fold}`): crates {} would publish \
                 indistinguishable package IDs on nuget.org — give each crate a distinct `[crates.csharp] \
                 package_id`",
                crates.join(", ")
            )));
        }
    }
    Ok(())
}

/// `[crates.java].package` is consumed by more than the `java` backend: the plain `kotlin`
/// backend (target `jvm`/`multiplatform`, i.e. *not* `kotlin_android`) reads
/// `config.java_package()` too, and splices it verbatim into emitted `.kt` source --
/// `import {java_package}.{Type}` and `{java_package}.{ClassName}` fully-qualified names in
/// `backends::kotlin::gen_bindings::{emit_client_type_file, generate_jvm}`,
/// `backends::kotlin::gen_mpp::emit_jvm_actual`, and
/// `backends::kotlin::gen_bindings::service_api::generate`. A `languages = ["kotlin"]` crate
/// with no `java`/`kotlin_android` in `languages` reaches every one of those, so the package
/// must be validated even when neither of the other two JVM languages is selected. ~keep
fn java_package_is_consumed(languages: &[Language]) -> bool {
    languages.contains(&Language::Java)
        || languages.contains(&Language::Kotlin)
        || languages.contains(&Language::KotlinAndroid)
}

fn validate_jvm_coordinates(resolved: &ResolvedCrateConfig, languages: &[Language]) -> Result<(), ResolveError> {
    use crate::codegen::coordinates::{validate_java_package, validate_kotlin_package, validate_maven_coordinate};

    let invalid = |field: &str, value: &str, reason: String| invalid_coordinate(resolved, field, value, reason);
    if java_package_is_consumed(languages) {
        let package = resolved.java_package();
        validate_java_package(&package).map_err(|error| invalid("[crates.java].package", &package, error))?;
        // The plain `kotlin` backend splices this same value into `.kt` source (see the
        // `java_package_is_consumed` doc comment), so it must also satisfy the stricter Kotlin
        // grammar whenever that backend is enabled -- Java accepts `dev.fun` (`fun` is not a
        // Java keyword) but Kotlin does not, and `dev.fun` reaching `import dev.fun.Type` in
        // generated Kotlin source fails to compile. ~keep
        if languages.contains(&Language::Kotlin) {
            validate_kotlin_package(&package).map_err(|error| invalid("[crates.java].package", &package, error))?;
        }
    }
    if languages.contains(&Language::Java) || languages.contains(&Language::KotlinAndroid) {
        let group = resolved.java_group_id();
        validate_maven_coordinate("groupId", &group)
            .map_err(|error| invalid("[crates.java].group_id", &group, error))?;
        let artifact = resolved.java_artifact_id();
        validate_maven_coordinate("artifactId", &artifact)
            .map_err(|error| invalid("[crates.java].artifact_id", &artifact, error))?;
    }
    if languages.contains(&Language::Kotlin) {
        let package = resolved.kotlin_package();
        validate_kotlin_package(&package).map_err(|error| invalid("[crates.kotlin].package", &package, error))?;
    }
    Ok(())
}

/// `[crates.kotlin_android]` carries its own `package`/`namespace`/`group_id`/`artifact_id` --
/// distinct config fields from `[crates.java]` and `[crates.kotlin]`, each with its own default
/// (see `backends::kotlin_android::naming`) -- so validating those two tables does not cover
/// this one. All four are genuinely spliced into generated output: `naming::kotlin_package` and
/// `naming::namespace` reach the bundled Kotlin facade's `package` declarations
/// (`backends::kotlin_android::gen_bindings`, `gen_proguard`, `gen_seed_test`) and the AAR's
/// Gradle `namespace = "..."` line, while `naming::aar_group_id`/`naming::aar_artifact_id` reach
/// the Maven publish coordinates in `build.gradle.kts`
/// (`backends::kotlin_android::gen_build_gradle::emit`) and `settings.gradle.kts`
/// (`backends::kotlin_android::gen_settings_gradle`).
fn validate_kotlin_android_coordinates(
    resolved: &ResolvedCrateConfig,
    languages: &[Language],
) -> Result<(), ResolveError> {
    use crate::backends::kotlin_android::naming::{aar_artifact_id, aar_group_id, kotlin_package, namespace};
    use crate::codegen::coordinates::{validate_kotlin_package, validate_maven_coordinate};

    if !languages.contains(&Language::KotlinAndroid) {
        return Ok(());
    }
    let invalid = |field: &str, value: &str, reason: String| invalid_coordinate(resolved, field, value, reason);

    let package = kotlin_package(resolved);
    validate_kotlin_package(&package).map_err(|error| invalid("[crates.kotlin_android].package", &package, error))?;

    let android_namespace = namespace(resolved);
    validate_kotlin_package(&android_namespace)
        .map_err(|error| invalid("[crates.kotlin_android].namespace", &android_namespace, error))?;

    let group = aar_group_id(resolved);
    validate_maven_coordinate("groupId", &group)
        .map_err(|error| invalid("[crates.kotlin_android].group_id", &group, error))?;

    let artifact = aar_artifact_id(resolved);
    validate_maven_coordinate("artifactId", &artifact)
        .map_err(|error| invalid("[crates.kotlin_android].artifact_id", &artifact, error))?;

    Ok(())
}

fn validate_dotnet_coordinates(resolved: &ResolvedCrateConfig, languages: &[Language]) -> Result<(), ResolveError> {
    use crate::codegen::coordinates::{validate_csharp_namespace, validate_nuget_package_id};

    if !languages.contains(&Language::Csharp) {
        return Ok(());
    }
    let invalid = |field: &str, value: &str, reason: String| invalid_coordinate(resolved, field, value, reason);
    let namespace = resolved.csharp_namespace();
    validate_csharp_namespace(&namespace).map_err(|error| invalid("[crates.csharp].namespace", &namespace, error))?;
    let package_id = resolved.nuget_package_id();
    validate_nuget_package_id(&package_id).map_err(|error| invalid("[crates.csharp].package_id", &package_id, error))
}

/// `Package.swift` is executable Swift, and its two coordinates have different grammars.
/// `module_name` becomes a compiled Swift identifier (`import <name>`); `package_name` is a
/// free-form manifest label that published packages routinely write in kebab-case, so it only
/// gets the narrower "cannot break out of the string literal" check.
fn validate_swift_coordinates(resolved: &ResolvedCrateConfig, languages: &[Language]) -> Result<(), ResolveError> {
    use crate::codegen::coordinates::{validate_swift_module_name, validate_swift_package_name};

    if !languages.contains(&Language::Swift) {
        return Ok(());
    }
    let invalid = |field: &str, value: &str, reason: String| invalid_coordinate(resolved, field, value, reason);
    let module_name = resolved.swift_module();
    validate_swift_module_name(&module_name)
        .map_err(|error| invalid("[crates.swift].module_name", &module_name, error))?;
    let package_name = resolved.swift_package_name();
    validate_swift_package_name(&package_name)
        .map_err(|error| invalid("[crates.swift].package_name", &package_name, error))
}

/// `[crates.dart].pubspec_name` and `[crates.dart].lib_name` are two different grammars:
/// `pubspec_name` is pub.dev's own *package* name, restricted to `lowercase_with_underscores`;
/// `lib_name` is a single Dart import-URI path segment / file basename
/// (`import 'package:{pubspec_name}/{lib_name}.dart'`) that `dart_bridge_class_name` already
/// documents as accepting hyphens (e.g. `"sample-widget"` -> `"SampleWidgetBridge"`), so it gets
/// the looser [`validate_dart_library_name`] rather than the package-name grammar. See that
/// function's doc comment for why reusing the stricter check here would be a backward-
/// incompatible regression, not a tightening.
fn validate_dart_coordinates(resolved: &ResolvedCrateConfig, languages: &[Language]) -> Result<(), ResolveError> {
    use crate::codegen::coordinates::{validate_dart_library_name, validate_dart_package_name};

    if !languages.contains(&Language::Dart) {
        return Ok(());
    }
    let invalid = |field: &str, value: &str, reason: String| invalid_coordinate(resolved, field, value, reason);
    let pubspec_name = resolved.dart_pubspec_name();
    validate_dart_package_name(&pubspec_name)
        .map_err(|error| invalid("[crates.dart].pubspec_name", &pubspec_name, error))?;
    let library_name = resolved.dart_library_name();
    validate_dart_library_name(&library_name).map_err(|error| invalid("[crates.dart].lib_name", &library_name, error))
}

/// Validate that a crate's own `name` cannot itself carry a path-traversal or absolute-path
/// takeover, called once per crate from [`NewAlefConfig::resolve`] before any per-crate,
/// per-language resolution runs.
///
/// `name` is the fallback value behind several language-specific defaults — `jni_crate_base()`
/// (used whenever `[crates.jni] crate_dir` is unset), the Dart pubspec-derived library name, and
/// `csharp_namespace()`'s pascal-case default — and every one of today's consuming call sites
/// happens to compose it safely (a fixed literal suffix, a `to_pascal_case()` transform that
/// strips non-alphanumeric characters entirely, or a literal prefix ahead of it). That safety is
/// an accident of the current call sites, not a guarantee, and relying on it silently drops
/// coverage the moment a language is absent from `OutputTemplate` (`jni` has no entry there) or
/// every configured language happens to carry an explicit `[crates.output]` override — either
/// way, `OutputTemplate::resolve`'s own crate-name check never runs for that crate. Checking
/// `name` here instead is unconditional: it runs for every crate regardless of which languages
/// or output overrides are configured, and it runs early enough (before
/// [`resolve_output_paths`](super::resolve_helpers::resolve_output_paths) can reach
/// `OutputTemplate::resolve`'s panicking equivalent) to surface a bad name as a graceful
/// [`ResolveError::InvalidConfig`] instead of a process panic. Reuses
/// [`validate_package_like_field`]'s combined segment-and-dot-replaced-path check — the stricter
/// of the two field-validation shapes — since a crate name should never legitimately contain a
/// path separator or start with `.` (unlike `node`/`wasm` `crate_dir`, which need the narrower
/// [`validate_relative_path_field`] because `/` is legitimate structure for those two fields). ~keep
fn validate_crate_name_path_safety(crate_name: &str) -> Result<(), ResolveError> {
    validate_package_like_field(crate_name, Some(crate_name), "name")
}

/// Validate every explicit per-language config override that becomes part of a generated
/// output path, called once from `resolve_one` with the already-merged (crate-over-workspace)
/// value for each field.
///
/// `jni.crate_dir` and `dart.lib_name` are documented single flat names (`crates/<jni
/// crate_dir>-jni/`; the Dart `library` declaration), so any `/` in them is already invalid
/// input and [`validate_path_segment_field`] rejects it outright. `node.crate_dir` and
/// `wasm.crate_dir` are, by contrast, documented and tested (see
/// `package_dir_node_crate_dir_override_takes_precedence` in `resolved::lookups`) to hold a
/// full relative path such as `"crates/sample-markdown-node"` — rejecting `/` there would break
/// that legitimate, already-shipped shape, so they get the narrower
/// [`validate_relative_path_field`], which only rejects an absolute value or a `..` component.
/// The four dotted package/namespace fields go through [`validate_package_like_field`], which
/// accounts for the `.replace('.', "/")` every one of those backends applies before joining the
/// value onto an output directory. Module, app, gem, extension, and package names that become
/// generated filenames or directories use the flat-segment validator; even where a current
/// backend normalizes one of them, validating the merged source value keeps later sinks from
/// silently weakening containment. ~keep
fn validate_language_specific_path_fields(crate_name: &str, fields: PathSafetyFields<'_>) -> Result<(), ResolveError> {
    validate_path_segment_field(crate_name, fields.jni_crate_dir, "jni.crate_dir")?;
    validate_relative_path_field(crate_name, fields.node_crate_dir, "node.crate_dir")?;
    validate_relative_path_field(crate_name, fields.wasm_crate_dir, "wasm.crate_dir")?;
    validate_path_segment_field(crate_name, fields.dart_lib_name, "dart.lib_name")?;
    validate_package_like_field(crate_name, fields.java_package, "java.package")?;
    validate_package_like_field(crate_name, fields.kotlin_package, "kotlin.package")?;
    validate_package_like_field(crate_name, fields.kotlin_android_package, "kotlin_android.package")?;
    validate_package_like_field(crate_name, fields.csharp_namespace, "csharp.namespace")?;
    validate_path_segment_field(crate_name, fields.python_module_name, "python.module_name")?;
    validate_path_segment_field(crate_name, fields.elixir_app_name, "elixir.app_name")?;
    validate_path_segment_field(crate_name, fields.gleam_app_name, "gleam.app_name")?;
    validate_path_segment_field(crate_name, fields.swift_module_name, "swift.module_name")?;
    validate_path_segment_field(crate_name, fields.zig_module_name, "zig.module_name")?;
    validate_path_segment_field(crate_name, fields.ruby_gem_name, "ruby.gem_name")?;
    validate_path_segment_field(crate_name, fields.php_extension_name, "php.extension_name")?;
    validate_path_segment_field(crate_name, fields.r_package_name, "r.package_name")?;
    validate_path_field(crate_name, fields.python_scaffold_output, "python.scaffold_output")?;
    validate_path_field(crate_name, fields.node_scaffold_output, "node.scaffold_output")?;
    validate_path_field(crate_name, fields.ruby_scaffold_output, "ruby.scaffold_output")?;
    validate_path_field(crate_name, fields.php_scaffold_output, "php.scaffold_output")?;
    validate_path_field(crate_name, fields.elixir_scaffold_output, "elixir.scaffold_output")?;
    Ok(())
}

/// The already-merged (crate-over-workspace) values [`validate_language_specific_path_fields`]
/// checks, bundled into one struct so the call site takes two arguments instead of nine.
struct PathSafetyFields<'a> {
    jni_crate_dir: Option<&'a str>,
    node_crate_dir: Option<&'a str>,
    wasm_crate_dir: Option<&'a str>,
    dart_lib_name: Option<&'a str>,
    java_package: Option<&'a str>,
    kotlin_package: Option<&'a str>,
    kotlin_android_package: Option<&'a str>,
    csharp_namespace: Option<&'a str>,
    python_module_name: Option<&'a str>,
    elixir_app_name: Option<&'a str>,
    gleam_app_name: Option<&'a str>,
    swift_module_name: Option<&'a str>,
    zig_module_name: Option<&'a str>,
    ruby_gem_name: Option<&'a str>,
    php_extension_name: Option<&'a str>,
    r_package_name: Option<&'a str>,
    python_scaffold_output: Option<&'a Path>,
    node_scaffold_output: Option<&'a Path>,
    ruby_scaffold_output: Option<&'a Path>,
    php_scaffold_output: Option<&'a Path>,
    elixir_scaffold_output: Option<&'a Path>,
}

fn validate_path_field(crate_name: &str, value: Option<&Path>, label: &str) -> Result<(), ResolveError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_output_path(value)
        .map_err(|detail| ResolveError::InvalidConfig(format!("crate `{crate_name}`: invalid {label}: {detail}")))
}

/// Validate an explicit config override documented as a single flat name (e.g. `[crates.jni]
/// crate_dir`, which becomes the `<crate_dir>` in `crates/<crate_dir>-jni/`; `[crates.dart]
/// lib_name`, the Dart `library` declaration) rather than a multi-segment path.
///
/// Rejects a NUL byte or path separator (`validate_output_segment`) — a legitimate value for
/// either field never contains `/` — and, once that passes, rejects the value resolving to a
/// bare `..` on its own (`validate_output_path`): `jni_output_path` and the Dart barrel-file
/// path both format the value in with a fixed literal suffix (`-jni`, `.dart`), which defeats a
/// bare `..` becoming a real `ParentDir` component there, but this check does not rely on that
/// incidental protection — a bare `..` is never a legitimate value for either field regardless.
/// A value of `None` (the field left unset) is not validated: for `jni.crate_dir` the default
/// (`config.name`) is covered unconditionally by [`validate_crate_name_path_safety`] instead, run
/// once per crate regardless of language configuration. `dart.lib_name`'s fallback additionally
/// goes through `dart_pubspec_name()`'s own hyphen-to-underscore transform before it reaches a
/// path, so `validation::validate_dart_library_name` checks that resolved, post-default value
/// separately rather than relying on the crate-name check alone.
fn validate_path_segment_field(crate_name: &str, value: Option<&str>, label: &str) -> Result<(), ResolveError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_output_segment(value, label)
        .map_err(|detail| ResolveError::InvalidConfig(format!("crate `{crate_name}`: {detail}")))?;
    validate_output_path(Path::new(value))
        .map_err(|detail| ResolveError::InvalidConfig(format!("crate `{crate_name}`: invalid {label}: {detail}")))
}

/// Validate an explicit config override documented and tested to hold a full relative path
/// rather than a single flat segment — `[crates.node] crate_dir` and `[crates.wasm]
/// crate_dir`, which may legitimately be e.g. `"crates/sample-markdown-node"` (see
/// `package_dir_node_crate_dir_override_takes_precedence` in `resolved::lookups`).
///
/// Unlike [`validate_path_segment_field`], this does not reject `/` — that is expected,
/// legitimate structure here. It only rejects the value resolving to an absolute path or
/// containing a `..` component (`validate_output_path`), which is what would let it escape the
/// output tree once `ResolvedCrateConfig::package_dir`'s Node/Wasm arms return it verbatim and a
/// caller (`cli::pipeline::format::poly_paths`, `cli::pipeline::generate::orphans`) does
/// `base_dir.join(package_dir)`. A value of `None` is not validated; see
/// [`validate_path_segment_field`] for why the unset case is safe by construction.
fn validate_relative_path_field(crate_name: &str, value: Option<&str>, label: &str) -> Result<(), ResolveError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_output_path(Path::new(value))
        .map_err(|detail| ResolveError::InvalidConfig(format!("crate `{crate_name}`: invalid {label}: {detail}")))
}

/// Validate an explicit dotted package/namespace override (`[java] package`, `[kotlin]
/// package`, `[kotlin_android] package`, `[csharp] namespace`) that every one of those
/// backends turns into nested path segments via `value.replace('.', "/")` before joining it
/// onto an output directory.
///
/// The raw value itself must not contain a path separator or NUL
/// (`validate_output_segment`); separately, the slash-converted form must not collapse to an
/// absolute path or a `..` component (`validate_output_path`) — a value that *starts* with a
/// `.` (e.g. `".foo"`) turns into a leading `/` after the replace, which `PathBuf::join`
/// treats as a full override of whatever output directory it's joined onto, discarding it
/// entirely. A value of `None` is not validated: the derived default for each of these fields
/// (repo-URL reverse-DNS derivation, or a literal placeholder) can never contain a `.`-led
/// component or a raw separator, so it is safe by construction.
fn validate_package_like_field(crate_name: &str, value: Option<&str>, label: &str) -> Result<(), ResolveError> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_output_segment(value, label)
        .map_err(|detail| ResolveError::InvalidConfig(format!("crate `{crate_name}`: {detail}")))?;
    validate_output_path(Path::new(&value.replace('.', "/")))
        .map_err(|detail| ResolveError::InvalidConfig(format!("crate `{crate_name}`: invalid {label}: {detail}")))
}

/// Validate a single `crate_attributes` entry.
///
/// Entries are raw Rust attribute *bodies* (the content between `#![` and `]`), not
/// full attribute syntax — e.g. `recursion_limit = "256"`, not
/// `#![recursion_limit = "256"]`. This mirrors `extra_clippy_allows`, which likewise
/// takes bare lint names rather than a full `#![allow(...)]` attribute.
///
/// This performs a shallow syntactic check (non-empty, single line, not already
/// wrapped in `#![...]`, and a valid leading attribute path) — not a full Rust
/// attribute-grammar parse. Malformed entries are rejected here, at config-resolve
/// time, rather than being spliced into generated output where they would only fail
/// much later at `rustc`/`clippy`.
fn validate_crate_attribute(crate_name: &str, raw: &str) -> Result<String, ResolveError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ResolveError::InvalidConfig(format!(
            "crate `{crate_name}`: crate_attributes entry is empty or whitespace-only"
        )));
    }
    if trimmed.contains('\n') {
        return Err(ResolveError::InvalidConfig(format!(
            "crate `{crate_name}`: crate_attributes entry `{trimmed}` must not contain a \
             newline; each entry is a single inner attribute body, e.g. `recursion_limit = \"256\"`"
        )));
    }
    if trimmed.starts_with('#') || trimmed.starts_with('[') {
        return Err(ResolveError::InvalidConfig(format!(
            "crate `{crate_name}`: crate_attributes entry `{trimmed}` must not include the \
             `#![...]` wrapper — pass only the attribute body, e.g. `recursion_limit = \"256\"` \
             not `#![recursion_limit = \"256\"]`"
        )));
    }

    let path_len = trimmed
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != ':')
        .unwrap_or(trimmed.len());
    let path = &trimmed[..path_len];
    let valid_path = !path.is_empty()
        && path.split("::").all(|segment| {
            let mut chars = segment.chars();
            matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    if !valid_path {
        return Err(ResolveError::InvalidConfig(format!(
            "crate `{crate_name}`: crate_attributes entry `{trimmed}` must start with a valid \
             attribute path (an identifier such as `recursion_limit`)"
        )));
    }

    let rest = trimmed[path_len..].trim_start();
    let well_formed_rest = rest.is_empty() || rest.starts_with('=') || rest.starts_with('(');
    if !well_formed_rest {
        return Err(ResolveError::InvalidConfig(format!(
            "crate `{crate_name}`: crate_attributes entry `{trimmed}` is malformed — expected \
             `path`, `path = value`, or `path(...)`"
        )));
    }

    Ok(trimmed.to_string())
}

/// Validate every user-supplied C-ABI override under `[crates.ffi]`.
///
/// This only validates fields the consumer actually set (`Option::Some`/non-empty
/// `Vec`/non-empty `HashMap` entries) — never the derived defaults (`{prefix}.h`,
/// `{prefix}_ffi`, the crate-name-derived prefix itself), which are guaranteed
/// safe by construction and out of scope here. See `core::config::abi_grammar`
/// for the grammar each check enforces and where that grammar comes from.
fn validate_ffi_config(crate_name: &str, ffi: &FfiConfig) -> Result<(), ResolveError> {
    let invalid = |field: &str, value: &str, error: String| {
        ResolveError::InvalidConfig(format!(
            "crate `{crate_name}`: `[ffi] {field}` value `{value}` is invalid: {error}"
        ))
    };

    if let Some(header_name) = ffi.header_name.as_ref() {
        abi_grammar::validate_c_header_filename(header_name).map_err(|e| invalid("header_name", header_name, e))?;
    }
    if let Some(lib_name) = ffi.lib_name.as_ref() {
        abi_grammar::validate_native_artifact_basename(lib_name).map_err(|e| invalid("lib_name", lib_name, e))?;
    }
    if let Some(prefix) = ffi.prefix.as_ref() {
        abi_grammar::validate_ascii_abi_prefix(prefix).map_err(|e| invalid("prefix", prefix, e))?;
    }
    for feature in ffi
        .features
        .iter()
        .flatten()
        .chain(&ffi.extra_features)
        .chain(&ffi.excluded_default_features)
    {
        abi_grammar::validate_cargo_feature_name(feature).map_err(|e| invalid("features", feature, e))?;
    }

    let mut seen_c_return_types: HashMap<&str, &str> = HashMap::new();
    for (rust_type_name, capsule) in &ffi.capsule_types {
        abi_grammar::validate_rust_pointee_type_path(&capsule.into_raw_type)
            .map_err(|e| invalid("capsule_types.into_raw_type", &capsule.into_raw_type, e))?;
        abi_grammar::validate_ascii_abi_identifier(&capsule.c_return_type)
            .map_err(|e| invalid("capsule_types.c_return_type", &capsule.c_return_type, e))?;
        if let Some(package) = capsule.package.as_ref() {
            abi_grammar::validate_cargo_package_name(package)
                .map_err(|e| invalid("capsule_types.package", package, e))?;
        }
        if let Some(version) = capsule.package_version.as_ref() {
            abi_grammar::validate_cargo_version_req(version)
                .map_err(|e| invalid("capsule_types.package_version", version, e))?;
        }
        if let Some(other_type) = seen_c_return_types.insert(capsule.c_return_type.as_str(), rust_type_name.as_str()) {
            return Err(ResolveError::InvalidConfig(format!(
                "crate `{crate_name}`: capsule types `{other_type}` and `{rust_type_name}` both declare \
                 `c_return_type = \"{}\"` — cbindgen would forward-declare one typedef for two different \
                 pointee types; give each capsule type a distinct `c_return_type`",
                capsule.c_return_type
            )));
        }
    }

    for override_ in &ffi.target_dep_overrides {
        abi_grammar::validate_cfg_expression(&override_.cfg)
            .map_err(|e| invalid("target_dep_overrides.cfg", &override_.cfg, e))?;
        for feature in &override_.features {
            abi_grammar::validate_cargo_feature_name(feature)
                .map_err(|e| invalid("target_dep_overrides.features", feature, e))?;
        }
    }

    Ok(())
}

fn validate_effective_ffi_config(config: &ResolvedCrateConfig) -> Result<(), ResolveError> {
    let c_e2e_enabled = config.c_e2e_enabled();
    let uses_c_abi = config.ffi.is_some()
        || c_e2e_enabled
        || config.languages.iter().any(|language| {
            matches!(
                language,
                Language::Ffi
                    | Language::C
                    | Language::Go
                    | Language::Java
                    | Language::Csharp
                    | Language::Dart
                    | Language::Kotlin
                    | Language::Swift
                    | Language::Zig
                    | Language::KotlinAndroid
                    | Language::Jni
            )
        });
    if !uses_c_abi {
        return Ok(());
    }

    let invalid = |field: &str, value: &str, error: String| {
        ResolveError::InvalidConfig(format!(
            "crate `{}`: effective C-ABI {field} value `{value}` is invalid: {error}",
            config.name
        ))
    };
    let prefix = config.ffi_prefix();
    abi_grammar::validate_ascii_abi_prefix(&prefix).map_err(|error| invalid("prefix", &prefix, error))?;
    let header = config.ffi_header_name();
    abi_grammar::validate_c_header_filename(&header).map_err(|error| invalid("header_name", &header, error))?;
    let lib = config.ffi_lib_name();
    abi_grammar::validate_native_artifact_basename(&lib).map_err(|error| invalid("lib_name", &lib, error))?;

    if c_e2e_enabled {
        validate_effective_c_e2e_config(config, &invalid)?;
    }
    Ok(())
}

fn validate_all_effective_ffi_configs(config: &ResolvedCrateConfig) -> Result<(), ResolveError> {
    validate_effective_ffi_config(config)?;
    if config.e2e.is_none() {
        return Ok(());
    }
    let mut registry_config = config.clone();
    if let Some(e2e) = registry_config.e2e.as_mut() {
        e2e.dep_mode = super::e2e::DependencyMode::Registry;
    }
    validate_effective_ffi_config(&registry_config)
}

fn validate_effective_c_e2e_config(
    config: &ResolvedCrateConfig,
    invalid: &impl Fn(&str, &str, String) -> ResolveError,
) -> Result<(), ResolveError> {
    let Some(e2e) = config.e2e.as_ref() else {
        return Ok(());
    };
    let package = e2e.resolve_package("c");
    validate_c_call_override("e2e.call", &e2e.call, invalid)?;
    let mut named_calls: Vec<_> = e2e.calls.iter().collect();
    named_calls.sort_unstable_by_key(|(name, _)| *name);
    for (name, call) in named_calls {
        validate_c_call_override(&format!("e2e.calls.{name}"), call, invalid)?;
    }
    let registry_mode = e2e.dep_mode == super::e2e::DependencyMode::Registry;
    let package_field = if registry_mode {
        "e2e.registry.packages.c"
    } else {
        "e2e.packages.c"
    };
    if let Some(name) = package.as_ref().and_then(|package| package.name.as_deref()) {
        abi_grammar::validate_native_artifact_basename(name)
            .map_err(|error| invalid(&format!("{package_field}.name"), name, error))?;
    }
    let output = e2e.effective_output();
    let output_field = if registry_mode {
        "e2e.registry.output"
    } else {
        "e2e.output"
    };
    abi_grammar::validate_c_output_base(output).map_err(|error| invalid(output_field, output, error))?;
    let explicit_path = package
        .as_ref()
        .and_then(|package| package.path.as_deref())
        .map(str::to_string);
    let path = match explicit_path {
        Some(path) => path,
        None => config
            .ffi_crate_path_from(&format!("{output}/c"))
            .map_err(|error| invalid(&format!("{package_field}.path"), output, error))?,
    };
    abi_grammar::validate_c_make_path(&path, output)
        .map_err(|error| invalid(&format!("{package_field}.path"), &path, error))
}

fn validate_c_call_override(
    field: &str,
    call: &super::e2e::CallConfig,
    invalid: &impl Fn(&str, &str, String) -> ResolveError,
) -> Result<(), ResolveError> {
    let Some(overrides) = call.overrides.get("c") else {
        return Ok(());
    };
    if let Some(prefix) = overrides.prefix.as_deref() {
        abi_grammar::validate_ascii_abi_prefix(prefix)
            .map_err(|error| invalid(&format!("{field}.overrides.c.prefix"), prefix, error))?;
    }
    if let Some(header) = overrides.header.as_deref() {
        abi_grammar::validate_c_header_filename(header)
            .map_err(|error| invalid(&format!("{field}.overrides.c.header"), header, error))?;
    }
    Ok(())
}

/// Resolve a list of `SourceCrate` entries, rebasing sources for any entry with
/// `from_registry = true` against the cargo registry path of that crate.
///
/// Entries with `from_registry = false` are returned unchanged.
fn resolve_source_crates(
    source_crates: &[super::SourceCrate],
    workspace_root: Option<&Path>,
) -> Result<Vec<super::SourceCrate>, ResolveError> {
    source_crates
        .iter()
        .map(|sc| {
            if !sc.from_registry {
                return Ok(sc.clone());
            }

            let root = workspace_root
                .map(|p| p.to_path_buf())
                .or_else(|| std::env::current_dir().ok())
                .ok_or_else(|| {
                    ResolveError::RegistryResolution(format!(
                        "source_crate `{}` has `from_registry = true` but `workspace_root` is not \
                         set and `std::env::current_dir()` failed",
                        sc.name
                    ))
                })?;

            let crate_dir =
                super::registry::resolve_crate_source_dir(&root, &sc.name).map_err(ResolveError::RegistryResolution)?;

            let rebased_sources = sc.sources.iter().map(|rel| crate_dir.join(rel)).collect();

            Ok(super::SourceCrate {
                name: sc.name.clone(),
                sources: rebased_sources,
                roots: sc.roots.clone(),
                from_registry: sc.from_registry,
            })
        })
        .collect()
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
mod c_abi_tests;
#[cfg(test)]
mod path_safety_review_tests;
#[cfg(test)]
mod path_safety_tests;
#[cfg(test)]
mod tests;
