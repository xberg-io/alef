use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

use crate::cli::{dispatch, pipeline};
use crate::codegen::component::{ResolvedComponent, resolve_component_targets, resolve_components};
use crate::component::artifact::{
    ComponentArtifactRecord, ComponentLock, PackageInput, ProvidedContractInput, build_lock, canonical_json,
    create_manifest, dynamic_library_name, feature_hash, read_record, sign_manifest, verify_record, write_lock,
    write_package,
};
use crate::core::config::{ComponentConfig, Language, ResolvedCrateConfig};

use super::args::{Commands, ComponentAction};
use super::dispatch::DispatchContext;
use super::helpers::load_config;

pub(crate) fn handle(command: Commands, context: &DispatchContext) -> Result<Option<Commands>> {
    let Commands::Component { action } = command else {
        return Ok(Some(command));
    };
    let (_workspace, resolved) = load_config(&context.config_path)?;
    let crates = dispatch::select_crates(&resolved, &context.crate_filter)?;
    ensure!(!crates.is_empty(), "no crates selected");

    match action {
        ComponentAction::Build {
            component,
            target,
            debug,
            dry_run,
        } => {
            for config in &crates {
                let contracts = extract_contracts(config, &context.config_path)?;
                for profile in select_profiles(config, &component)? {
                    let resolved_component = contracts
                        .get(profile.name.as_str())
                        .with_context(|| format!("component `{}` has no resolved contract", profile.name))?;
                    for provided in &resolved_component.provides {
                        ensure!(
                            provided.contract_hash == provided.contract.hash()?,
                            "component contract hash changed during build planning"
                        );
                    }
                    for rust_target in select_targets(config, profile, &target)? {
                        build_component(config, profile, &rust_target, debug, dry_run)?;
                    }
                }
            }
            tracing::info!("Component build complete");
            Ok(None)
        }
        ComponentAction::Package {
            component,
            target,
            output,
            version,
            signing_key,
            key_id,
            unsigned,
            library,
        } => {
            let mut package_count = 0_usize;
            let requested_count = crates
                .iter()
                .map(|config| {
                    select_profiles(config, &component)
                        .map(|profiles| {
                            profiles
                                .into_iter()
                                .map(|profile| select_targets(config, profile, &target).map(|targets| targets.len()))
                                .collect::<Result<Vec<_>>>()
                                .map(|counts| counts.into_iter().sum::<usize>())
                        })
                        .and_then(|result| result)
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .sum::<usize>();
            if library.is_some() {
                ensure!(
                    requested_count == 1,
                    "--library can only be used when packaging exactly one profile/target"
                );
            }
            ensure!(
                unsigned || signing_key.is_some(),
                "component packages must be signed; pass --signing-key/--key-id or explicitly use --unsigned for a CI intermediate"
            );

            for config in crates {
                let contracts = extract_contracts(config, &context.config_path)?;
                let resolved_version = version
                    .clone()
                    .or_else(|| config.resolved_version())
                    .with_context(|| format!("could not determine component version for `{}`", config.name))?;
                if let Some(signing_key_id) = key_id.as_deref() {
                    let distribution = config
                        .component_distribution
                        .as_ref()
                        .context("signed component packaging requires component_distribution")?;
                    ensure!(
                        distribution.public_keys.contains_key(signing_key_id),
                        "signing key ID `{signing_key_id}` is not configured in component_distribution.public_keys"
                    );
                }

                for profile in select_profiles(config, &component)? {
                    let resolved_component = contracts
                        .get(profile.name.as_str())
                        .with_context(|| format!("component `{}` has no resolved contract", profile.name))?;
                    let contract_hashes = resolved_component
                        .provides
                        .iter()
                        .map(|provided| provided.contract.hash_hex())
                        .collect::<Result<Vec<_>>>()?;
                    let provides = resolved_component
                        .provides
                        .iter()
                        .zip(&contract_hashes)
                        .map(|(provided, contract_hash)| ProvidedContractInput {
                            contract: &provided.contract.name,
                            interface_version: provided.contract.interface_version,
                            contract_hash,
                            implementation: &provided.implementation,
                        })
                        .collect::<Vec<_>>();
                    let profile_feature_hash = feature_hash(&profile.features, profile.default_features);
                    for rust_target in select_targets(config, profile, &target)? {
                        let library_path = library
                            .clone()
                            .unwrap_or_else(|| built_library_path(config, &profile.name, &rust_target, false));
                        let manifest = create_manifest(
                            &library_path,
                            PackageInput {
                                crate_name: &config.name,
                                component: &profile.name,
                                version: &resolved_version,
                                target: &rust_target,
                                provides: &provides,
                                features: &profile.features,
                                default_features: profile.default_features,
                                feature_hash: &profile_feature_hash,
                            },
                        )?;
                        let signature = match (signing_key.as_deref(), key_id.as_deref()) {
                            (Some(private_key), Some(signing_key_id)) => {
                                Some(sign_manifest(&manifest, private_key, signing_key_id)?)
                            }
                            (None, None) if unsigned => None,
                            _ => bail!("both --signing-key and --key-id are required for signed component packages"),
                        };
                        let record = write_package(&library_path, &output, manifest, signature)?;
                        tracing::info!("Packaged component {}", output.join(&record.archive).display());
                        package_count += 1;
                    }
                }
            }
            tracing::info!("Component package complete: {package_count} artifact(s)");
            Ok(None)
        }
        ComponentAction::Lock { input, output } => {
            let records = records_in(&input)?;
            ensure!(
                !records.is_empty(),
                "no component artifact records found in {}",
                input.display()
            );
            let mut public_keys = BTreeMap::new();
            let mut artifacts = Vec::new();
            for config in &crates {
                let distribution = config
                    .component_distribution
                    .as_ref()
                    .with_context(|| format!("crate `{}` has components but no component_distribution", config.name))?;
                merge_public_keys(&mut public_keys, &distribution.public_keys)?;
                let crate_records = records
                    .iter()
                    .filter(|(_, record)| record.manifest.identity.crate_name == config.name)
                    .collect::<Vec<_>>();
                ensure!(
                    !crate_records.is_empty(),
                    "no artifact records found for crate `{}`",
                    config.name
                );
                for (path, _) in &crate_records {
                    verify_record(path, &distribution.public_keys)?;
                }
                ensure_configured_matrix_present(config, &crate_records)?;
                let owned = crate_records
                    .iter()
                    .map(|(_, record)| (*record).clone())
                    .collect::<Vec<_>>();
                artifacts.extend(build_lock(&owned, &distribution.url_template, &distribution.public_keys)?.artifacts);
            }
            artifacts.sort_by(|left, right| {
                (
                    &left.identity.crate_name,
                    &left.identity.component,
                    &left.identity.version,
                    &left.identity.target,
                    &left.identity.feature_hash,
                )
                    .cmp(&(
                        &right.identity.crate_name,
                        &right.identity.component,
                        &right.identity.version,
                        &right.identity.target,
                        &right.identity.feature_hash,
                    ))
            });
            ensure_unique_lock_entries(&artifacts)?;
            let lock = ComponentLock {
                schema_version: crate::component::artifact::COMPONENT_MANIFEST_SCHEMA,
                public_keys,
                artifacts,
            };
            write_lock(&output, &lock)?;
            tracing::info!("Wrote component lock {}", output.display());
            for config in crates {
                let binding_lock = binding_lock_for_crate(&lock, &config.name);
                let lock_bytes = canonical_json(&binding_lock)?;
                for embedded_path in binding_component_lock_paths(config) {
                    if let Some(parent) = embedded_path.parent() {
                        fs::create_dir_all(parent)
                            .with_context(|| format!("failed to create binding directory {}", parent.display()))?;
                    }
                    fs::write(&embedded_path, &lock_bytes)
                        .with_context(|| format!("failed to stage component lock {}", embedded_path.display()))?;
                    tracing::info!("Staged component lock {}", embedded_path.display());
                }
            }
            Ok(None)
        }
        ComponentAction::Verify { input } => {
            let records = records_in(&input)?;
            ensure!(
                !records.is_empty(),
                "no component artifact records found in {}",
                input.display()
            );
            let configs = crates
                .iter()
                .map(|config| (config.name.as_str(), *config))
                .collect::<HashMap<_, _>>();
            let mut contract_cache = HashMap::new();
            let mut verified = 0_usize;
            for (path, record) in &records {
                let Some(config) = configs.get(record.manifest.identity.crate_name.as_str()).copied() else {
                    continue;
                };
                let distribution = config
                    .component_distribution
                    .as_ref()
                    .with_context(|| format!("crate `{}` has no component_distribution", config.name))?;
                verify_record(path, &distribution.public_keys)?;
                if !contract_cache.contains_key(&config.name) {
                    contract_cache.insert(config.name.clone(), extract_contracts(config, &context.config_path)?);
                }
                verify_against_config(record, config, &contract_cache[&config.name])?;
                tracing::info!("Verified component {}", path.display());
                verified += 1;
            }
            ensure!(verified > 0, "no artifact records matched the selected crates");
            tracing::info!("Component verification complete: {verified} artifact(s)");
            Ok(None)
        }
    }
}

fn extract_contracts(config: &ResolvedCrateConfig, config_path: &Path) -> Result<HashMap<String, ResolvedComponent>> {
    ensure!(
        !config.components.is_empty(),
        "crate `{}` has no configured components",
        config.name
    );
    let extraction_config = component_extraction_config(config);
    let api = pipeline::extract(&extraction_config, config_path, false)?;
    Ok(resolve_components(&api, config)?
        .into_iter()
        .map(|component| (component.component_name.clone(), component))
        .collect())
}

fn component_extraction_config(config: &ResolvedCrateConfig) -> ResolvedCrateConfig {
    let mut extraction_config = config.clone();
    for component in &config.components {
        extraction_config.features.extend(component.features.iter().cloned());
    }
    extraction_config.features.sort();
    extraction_config.features.dedup();
    extraction_config
}

fn select_profiles<'a>(config: &'a ResolvedCrateConfig, requested: &[String]) -> Result<Vec<&'a ComponentConfig>> {
    if requested.is_empty() {
        return Ok(config.components.iter().collect());
    }
    let requested = requested.iter().map(String::as_str).collect::<HashSet<_>>();
    let selected = config
        .components
        .iter()
        .filter(|profile| requested.contains(profile.name.as_str()))
        .collect::<Vec<_>>();
    let found = selected
        .iter()
        .map(|profile| profile.name.as_str())
        .collect::<HashSet<_>>();
    let missing = requested.difference(&found).copied().collect::<Vec<_>>();
    ensure!(
        missing.is_empty(),
        "unknown component profile(s) for crate `{}`: {}",
        config.name,
        missing.join(", ")
    );
    Ok(selected)
}

fn select_targets(
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    requested: &[String],
) -> Result<Vec<String>> {
    let configured = resolve_component_targets(config, profile);
    if requested.is_empty() {
        return Ok(configured);
    }
    let configured_set = configured.iter().map(String::as_str).collect::<HashSet<_>>();
    let selected = requested
        .iter()
        .filter(|target| configured_set.contains(target.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let found = selected.iter().map(String::as_str).collect::<HashSet<_>>();
    let missing = requested
        .iter()
        .map(String::as_str)
        .filter(|target| !found.contains(target))
        .collect::<Vec<_>>();
    ensure!(
        missing.is_empty(),
        "component `{}` is not configured for target(s): {}",
        profile.name,
        missing.join(", ")
    );
    Ok(selected)
}

fn build_component(
    config: &ResolvedCrateConfig,
    profile: &ComponentConfig,
    rust_target: &str,
    debug: bool,
    dry_run: bool,
) -> Result<()> {
    let target_dir = component_target_dir(config, &profile.name);
    let producer_manifest = producer_manifest_path(config, &profile.name);
    ensure!(
        producer_manifest.is_file(),
        "component producer manifest does not exist: {}; run `alef scaffold` first",
        producer_manifest.display()
    );
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(&producer_manifest)
        .args(["--target", rust_target, "--target-dir"])
        .arg(&target_dir);
    if !debug {
        command.arg("--release");
    }
    tracing::info!(
        "Building component `{}` for `{}` (features: {})",
        profile.name,
        rust_target,
        if profile.features.is_empty() {
            "<none>".to_string()
        } else {
            profile.features.join(",")
        }
    );
    tracing::debug!("component build command: {command:?}");
    if dry_run {
        return Ok(());
    }
    let status = command
        .status()
        .context("failed to execute cargo for component build")?;
    ensure!(
        status.success(),
        "component `{}` build failed for target `{rust_target}`",
        profile.name
    );
    let library = built_library_path(config, &profile.name, rust_target, debug);
    ensure!(
        library.is_file(),
        "component build succeeded but did not produce {}",
        library.display()
    );
    Ok(())
}

fn component_target_dir(config: &ResolvedCrateConfig, component: &str) -> PathBuf {
    Path::new(".alef")
        .join("components")
        .join("build")
        .join(&config.name)
        .join(component)
}

fn built_library_path(config: &ResolvedCrateConfig, component: &str, target: &str, debug: bool) -> PathBuf {
    component_target_dir(config, component)
        .join(target)
        .join(if debug { "debug" } else { "release" })
        .join(dynamic_library_name(&producer_package_name(config, component), target))
}

fn producer_package_name(config: &ResolvedCrateConfig, component: &str) -> String {
    format!("{}-{}-component", config.core_crate_dir(), component.replace('_', "-"))
}

fn producer_manifest_path(config: &ResolvedCrateConfig, component: &str) -> PathBuf {
    Path::new("crates")
        .join(producer_package_name(config, component))
        .join("Cargo.toml")
}

fn python_component_lock_path(config: &ResolvedCrateConfig) -> PathBuf {
    let explicit = config
        .explicit_output
        .python
        .as_deref()
        .or_else(|| config.output_for("python"));
    if let Some(output) = explicit {
        for ancestor in output.ancestors() {
            if ancestor.as_os_str().is_empty() || ancestor == Path::new(".") {
                continue;
            }
            if ancestor.join("Cargo.toml").is_file() {
                return ancestor.join("components.lock.json");
            }
        }
    }
    Path::new("crates")
        .join(format!("{}-py", config.core_crate_dir()))
        .join("components.lock.json")
}

fn binding_component_lock_paths(config: &ResolvedCrateConfig) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();
    for language in &config.languages {
        match language {
            Language::Python => {
                paths.insert(python_component_lock_path(config));
            }
            Language::Node => {
                paths.insert(node_component_lock_path(config));
            }
            Language::Ruby => {
                paths.insert(Path::new(&config.package_dir(Language::Ruby)).join("components.lock.json"));
            }
            Language::Php => {
                paths.insert(
                    Path::new("crates")
                        .join(format!("{}-php", config.core_crate_dir()))
                        .join("components.lock.json"),
                );
            }
            Language::Elixir => {
                paths.insert(Path::new(&config.package_dir(Language::Elixir)).join("components.lock.json"));
            }
            Language::R => {
                paths.insert(Path::new(&config.package_dir(Language::R)).join("components.lock.json"));
            }
            Language::Ffi | Language::Go | Language::Java | Language::Csharp | Language::Kotlin | Language::Zig => {
                paths.insert(ffi_component_lock_path(config));
            }
            Language::Dart => {
                if config
                    .dart
                    .as_ref()
                    .is_some_and(|dart| dart.style == crate::core::config::DartStyle::Ffi)
                {
                    paths.insert(ffi_component_lock_path(config));
                } else {
                    paths.insert(
                        Path::new(&config.package_dir(Language::Dart))
                            .join("rust")
                            .join("components.lock.json"),
                    );
                }
            }
            Language::Swift => {
                paths.insert(
                    Path::new(&config.package_dir(Language::Swift))
                        .join("rust")
                        .join("components.lock.json"),
                );
            }
            Language::Gleam => {
                paths.insert(Path::new(&config.package_dir(Language::Elixir)).join("components.lock.json"));
            }
            Language::Wasm | Language::KotlinAndroid | Language::Jni | Language::Rust | Language::C => {}
        }
    }
    paths
}

fn binding_lock_for_crate(lock: &ComponentLock, crate_name: &str) -> ComponentLock {
    let artifacts = lock
        .artifacts
        .iter()
        .filter(|artifact| artifact.identity.crate_name == crate_name)
        .cloned()
        .collect::<Vec<_>>();
    let key_ids = artifacts
        .iter()
        .map(|artifact| artifact.key_id.as_str())
        .filter(|key_id| !key_id.is_empty())
        .collect::<HashSet<_>>();
    let public_keys = lock
        .public_keys
        .iter()
        .filter(|(key_id, _)| key_ids.contains(key_id.as_str()))
        .map(|(key_id, key)| (key_id.clone(), key.clone()))
        .collect();
    ComponentLock {
        schema_version: lock.schema_version,
        public_keys,
        artifacts,
    }
}

fn ffi_component_lock_path(config: &ResolvedCrateConfig) -> PathBuf {
    let explicit = config
        .explicit_output
        .ffi
        .as_deref()
        .or_else(|| config.output_for("ffi"));
    if let Some(output) = explicit {
        for ancestor in output.ancestors() {
            if ancestor.as_os_str().is_empty() || ancestor == Path::new(".") {
                continue;
            }
            if ancestor.join("Cargo.toml").is_file() {
                return ancestor.join("components.lock.json");
            }
        }
    }
    Path::new("crates")
        .join(format!("{}-ffi", config.core_crate_dir()))
        .join("components.lock.json")
}

fn node_component_lock_path(config: &ResolvedCrateConfig) -> PathBuf {
    let explicit = config
        .explicit_output
        .node
        .as_deref()
        .or_else(|| config.output_for("node"));
    if let Some(output) = explicit {
        for ancestor in output.ancestors() {
            if ancestor.as_os_str().is_empty() || ancestor == Path::new(".") {
                continue;
            }
            if ancestor.join("Cargo.toml").is_file() {
                return ancestor.join("components.lock.json");
            }
        }
    }
    Path::new("crates")
        .join(format!("{}-node", config.core_crate_dir()))
        .join("components.lock.json")
}

fn records_in(path: &Path) -> Result<Vec<(PathBuf, ComponentArtifactRecord)>> {
    if path.is_file() {
        return Ok(vec![(path.to_path_buf(), read_record(path)?)]);
    }
    ensure!(
        path.is_dir(),
        "component record input does not exist: {}",
        path.display()
    );
    let mut paths = fs::read_dir(path)
        .with_context(|| format!("failed to read component record directory {}", path.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|entry| {
            entry
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".record.json"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| read_record(&path).map(|record| (path, record)))
        .collect()
}

fn merge_public_keys(destination: &mut BTreeMap<String, String>, source: &BTreeMap<String, String>) -> Result<()> {
    for (key_id, key) in source {
        if let Some(existing) = destination.get(key_id) {
            ensure!(
                existing == key,
                "public key ID `{key_id}` has conflicting values across crates"
            );
        } else {
            destination.insert(key_id.clone(), key.clone());
        }
    }
    Ok(())
}

fn ensure_configured_matrix_present(
    config: &ResolvedCrateConfig,
    records: &[&(PathBuf, ComponentArtifactRecord)],
) -> Result<()> {
    for profile in &config.components {
        for target in resolve_component_targets(config, profile) {
            ensure!(
                records.iter().any(|(_, record)| {
                    record.manifest.identity.component == profile.name && record.manifest.identity.target == target
                }),
                "component lock input is missing `{}` for target `{target}`",
                profile.name
            );
        }
    }
    Ok(())
}

fn ensure_unique_lock_entries(entries: &[crate::component::artifact::ComponentLockEntry]) -> Result<()> {
    for pair in entries.windows(2) {
        ensure!(
            pair[0].identity != pair[1].identity,
            "duplicate component artifact identity in lock input"
        );
    }
    Ok(())
}

fn verify_against_config(
    record: &ComponentArtifactRecord,
    config: &ResolvedCrateConfig,
    contracts: &HashMap<String, ResolvedComponent>,
) -> Result<()> {
    let profile = config
        .components
        .iter()
        .find(|profile| profile.name == record.manifest.identity.component)
        .with_context(|| {
            format!(
                "artifact references unknown component `{}`",
                record.manifest.identity.component
            )
        })?;
    ensure!(
        resolve_component_targets(config, profile).contains(&record.manifest.identity.target),
        "artifact target is not configured for component"
    );
    ensure!(
        record.manifest.features == sorted_features(&profile.features),
        "artifact feature set differs from config"
    );
    ensure!(
        record.manifest.default_features == profile.default_features,
        "artifact default-feature policy differs from config"
    );
    ensure!(
        record.manifest.identity.feature_hash == feature_hash(&profile.features, profile.default_features),
        "artifact feature hash differs from config"
    );
    let resolved_component = contracts
        .get(&profile.name)
        .with_context(|| format!("component `{}` has no resolved contract", profile.name))?;
    verify_provides_against_config(record, profile, resolved_component)
}

fn verify_provides_against_config(
    record: &ComponentArtifactRecord,
    profile: &ComponentConfig,
    resolved_component: &ResolvedComponent,
) -> Result<()> {
    ensure!(
        record.manifest.provides.len() == profile.provides.len(),
        "artifact provides a different number of contracts than configured"
    );
    for provided_config in &profile.provides {
        let recorded = record
            .manifest
            .provides
            .iter()
            .find(|provided| provided.contract == provided_config.contract)
            .with_context(|| {
                format!(
                    "artifact does not provide configured contract `{}`",
                    provided_config.contract
                )
            })?;
        ensure!(
            recorded.implementation == provided_config.implementation,
            "artifact implementation for contract `{}` differs from config",
            provided_config.contract
        );
        let resolved = resolved_component
            .provides
            .iter()
            .find(|provided| provided.contract.name == provided_config.contract)
            .with_context(|| format!("component has no resolved contract `{}`", provided_config.contract))?;
        ensure!(
            recorded.contract_hash == resolved.contract.hash_hex()?,
            "artifact contract hash for `{}` differs from source",
            provided_config.contract
        );
        ensure!(
            recorded.interface_version == resolved.contract.interface_version,
            "artifact contract version for `{}` differs from source",
            provided_config.contract
        );
    }
    Ok(())
}

fn sorted_features(features: &[String]) -> Vec<String> {
    let mut sorted = features.to_vec();
    sorted.sort();
    sorted.dedup();
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::artifact::{ComponentDeliveryMode, ComponentIdentity, ComponentLockEntry};
    use crate::core::config::{ComponentConfig, ComponentProvidesConfig};

    fn provides(contract: &str, implementation: &str) -> Vec<ComponentProvidesConfig> {
        vec![ComponentProvidesConfig {
            contract: contract.into(),
            implementation: implementation.into(),
        }]
    }

    #[test]
    fn target_filter_must_be_declared_by_profile() {
        let config = ResolvedCrateConfig::default();
        let profile = ComponentConfig {
            name: "fast".into(),
            provides: provides("engine", "demo::Fast"),
            features: vec!["fast".into()],
            default_features: false,
            targets: Some(vec!["aarch64-apple-darwin".into()]),
            bundled_on: Vec::new(),
        };
        let requested = vec!["x86_64-unknown-linux-gnu".to_string()];
        assert!(select_targets(&config, &profile, &requested).is_err());
    }

    #[test]
    fn component_extraction_uses_feature_union_without_changing_binding_features() {
        let config = ResolvedCrateConfig {
            features: vec!["base".into()],
            components: vec![
                ComponentConfig {
                    name: "fast".into(),
                    provides: provides("engine", "demo::Fast"),
                    features: vec!["simd".into(), "base".into()],
                    default_features: false,
                    targets: Some(vec!["aarch64-apple-darwin".into()]),
                    bundled_on: Vec::new(),
                },
                ComponentConfig {
                    name: "gpu".into(),
                    provides: provides("engine", "demo::Gpu"),
                    features: vec!["cuda".into()],
                    default_features: false,
                    targets: Some(vec!["x86_64-unknown-linux-gnu".into()]),
                    bundled_on: Vec::new(),
                },
            ],
            ..ResolvedCrateConfig::default()
        };

        let extraction = component_extraction_config(&config);
        assert_eq!(extraction.features, vec!["base", "cuda", "simd"]);
        assert_eq!(config.features, vec!["base"]);
    }

    #[test]
    fn default_python_lock_path_does_not_select_workspace_manifest() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            output_paths: HashMap::from([("python".into(), PathBuf::from("packages/python/demo_core"))]),
            ..ResolvedCrateConfig::default()
        };
        assert_eq!(
            python_component_lock_path(&config),
            PathBuf::from("crates/demo-core-py/components.lock.json")
        );
    }

    #[test]
    fn component_lock_paths_cover_each_native_binding_once() {
        let config = ResolvedCrateConfig {
            name: "demo-core".into(),
            languages: vec![
                Language::Python,
                Language::Node,
                Language::Ruby,
                Language::Php,
                Language::Elixir,
                Language::R,
                Language::Ffi,
                Language::Go,
                Language::Java,
                Language::Csharp,
                Language::Kotlin,
                Language::Swift,
                Language::Dart,
                Language::Gleam,
                Language::Zig,
                Language::Jni,
                Language::Wasm,
                Language::KotlinAndroid,
            ],
            ..ResolvedCrateConfig::default()
        };

        assert_eq!(
            binding_component_lock_paths(&config),
            BTreeSet::from([
                PathBuf::from("crates/demo-core-ffi/components.lock.json"),
                PathBuf::from("crates/demo-core-node/components.lock.json"),
                PathBuf::from("crates/demo-core-php/components.lock.json"),
                PathBuf::from("crates/demo-core-py/components.lock.json"),
                PathBuf::from("packages/elixir/components.lock.json"),
                PathBuf::from("packages/dart/rust/components.lock.json"),
                PathBuf::from("packages/r/components.lock.json"),
                PathBuf::from("packages/ruby/components.lock.json"),
                PathBuf::from("packages/swift/rust/components.lock.json"),
            ])
        );
    }

    #[test]
    fn staged_binding_lock_is_scoped_to_one_crate_and_its_keys() {
        let entry = |crate_name: &str, component: &str, key_id: &str| ComponentLockEntry {
            identity: ComponentIdentity {
                crate_name: crate_name.into(),
                component: component.into(),
                version: "1.0.0".into(),
                target: "aarch64-apple-darwin".into(),
                feature_hash: "01".repeat(32),
                contract_hash: "02".repeat(32),
            },
            provides: Vec::new(),
            mode: ComponentDeliveryMode::Download,
            url: "https://example.invalid/component.tar.gz".into(),
            sha256: "03".repeat(32),
            size: 10,
            manifest_sha256: "04".repeat(32),
            key_id: key_id.into(),
        };
        let lock = ComponentLock {
            schema_version: 1,
            public_keys: BTreeMap::from([
                ("alpha-key".into(), "alpha-public-key".into()),
                ("beta-key".into(), "beta-public-key".into()),
            ]),
            artifacts: vec![
                entry("alpha-core", "fast", "alpha-key"),
                entry("beta-core", "fast", "beta-key"),
            ],
        };

        let scoped = binding_lock_for_crate(&lock, "alpha-core");
        assert_eq!(scoped.artifacts.len(), 1);
        assert_eq!(scoped.artifacts[0].identity.crate_name, "alpha-core");
        assert_eq!(
            scoped.public_keys,
            BTreeMap::from([("alpha-key".into(), "alpha-public-key".into())])
        );
    }
}
