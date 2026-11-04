use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail, ensure};

use crate::cli::{dispatch, pipeline};
use crate::codegen::component::{ResolvedComponent, resolve_component_targets, resolve_components};
use crate::codegen::component_host::contract_hash_rows;
use crate::component::artifact::{
    ComponentArtifactRecord, ComponentLock, PackageInput, ProvidedContractInput, SigningKeySource, build_lock,
    build_unsigned_lock, canonical_json, create_manifest, feature_hash, load_signing_key_from_source, read_record,
    sign_manifest_with_key, verify_record_with, write_lock, write_package_recorded,
};
use crate::core::config::{ComponentConfig, ComponentDistributionConfig, Language, ResolvedCrateConfig};

use super::args::{Commands, ComponentAction};
use super::dispatch::DispatchContext;
use super::helpers::load_config;

#[path = "component_build.rs"]
mod build;
#[path = "component_plan.rs"]
mod plan;

use build::{build_component, built_library_path};
use plan::{Fingerprinter, PlanArgs};

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
            target_dir,
            plan,
            extra,
            env,
        } => {
            let plan = plan::load_selection(plan.as_deref(), debug)?;
            let mut fingerprinter = Fingerprinter::new(&extra, &env)?;
            for config in &crates {
                let contracts = extract_contracts(config, &context.config_path)?;
                for profile in plan::plan_profiles(config, &component, plan.as_ref())? {
                    let resolved_component = contracts
                        .get(profile.name.as_str())
                        .with_context(|| format!("component `{}` has no resolved contract", profile.name))?;
                    for provided in &resolved_component.provides {
                        ensure!(
                            provided.contract_hash == provided.contract.hash()?,
                            "component contract hash changed during build planning"
                        );
                    }
                    for rust_target in plan::plan_targets(config, profile, &target, plan.as_ref())? {
                        let planned_hash = match &plan {
                            Some(plan) => {
                                let Some(hash) = plan::gate(
                                    plan,
                                    &mut fingerprinter,
                                    config,
                                    profile,
                                    resolved_component,
                                    &rust_target,
                                    debug,
                                )?
                                else {
                                    continue;
                                };
                                hash
                            }
                            None => None,
                        };
                        let library =
                            built_library_path(config, &profile.name, &rust_target, debug, target_dir.as_deref());
                        if !dry_run {
                            plan::clear_sidecar(&library)?;
                        }
                        build_component(config, profile, &rust_target, debug, dry_run, target_dir.as_deref())?;
                        if let (false, Some(hash)) = (dry_run, planned_hash.as_deref()) {
                            plan::write_sidecar(&library, hash)?;
                        }
                    }
                }
            }
            tracing::info!("Component build complete");
            Ok(None)
        }
        ComponentAction::Plan {
            previous_lock,
            component,
            target,
            extra,
            env,
            debug,
            output,
        } => {
            plan::run(
                &crates,
                &context.config_path,
                PlanArgs {
                    previous_lock,
                    component,
                    target,
                    extra,
                    env,
                    debug,
                    output,
                },
            )?;
            Ok(None)
        }
        ComponentAction::Package {
            component,
            target,
            output,
            version,
            signing_key,
            signing_key_env,
            key_id,
            unsigned,
            library,
            debug,
            target_dir,
            extra,
            env,
            plan,
        } => {
            let plan = plan::load_selection(plan.as_deref(), debug)?;
            let mut fingerprinter = Fingerprinter::new(&extra, &env)?;
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
            let key_source = signing_key_source(signing_key, signing_key_env);
            ensure!(
                unsigned || key_source.is_some(),
                "component packages must be signed; pass --signing-key (or --signing-key-env) with --key-id, or explicitly use --unsigned for a CI intermediate"
            );
            // Load once, up front: stdin can be read only once, and a bad key should fail before
            // any archive is written.
            let loaded_key = match (&key_source, key_id.as_deref()) {
                (Some(source), Some(_)) => Some(load_signing_key_from_source(source)?),
                (Some(_), None) => bail!("--key-id is required with --signing-key/--signing-key-env"),
                (None, Some(_)) => bail!("--key-id requires --signing-key or --signing-key-env"),
                (None, None) => None,
            };

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

                for profile in plan::plan_profiles(config, &component, plan.as_ref())? {
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
                    for rust_target in plan::plan_targets(config, profile, &target, plan.as_ref())? {
                        let library_path = library.clone().unwrap_or_else(|| {
                            built_library_path(config, &profile.name, &rust_target, debug, target_dir.as_deref())
                        });
                        let input_hash = match &plan {
                            Some(plan) => {
                                let Some(hash) = plan::gate(
                                    plan,
                                    &mut fingerprinter,
                                    config,
                                    profile,
                                    resolved_component,
                                    &rust_target,
                                    debug,
                                )?
                                else {
                                    continue;
                                };
                                plan::check_sidecar(&library_path, hash.as_deref())?;
                                hash
                            }
                            None => None,
                        };
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
                        let signature = match (loaded_key.as_ref(), key_id.as_deref()) {
                            (Some(private_key), Some(signing_key_id)) => {
                                Some(sign_manifest_with_key(&manifest, private_key, signing_key_id)?)
                            }
                            _ => None,
                        };
                        let name_template = config
                            .component_distribution
                            .as_ref()
                            .and_then(|distribution| distribution.artifact_name_template.as_deref());
                        let record = write_package_recorded(
                            &library_path,
                            &output,
                            manifest,
                            signature,
                            name_template,
                            input_hash,
                        )?;
                        tracing::info!("Packaged component {}", output.join(&record.archive).display());
                        package_count += 1;
                    }
                }
            }
            tracing::info!("Component package complete: {package_count} artifact(s)");
            Ok(None)
        }
        ComponentAction::Lock {
            input,
            output,
            unsigned,
            url_template,
            target,
            component,
            reuse,
            plan,
        } => {
            validate_lock_targets(&crates, &target)?;
            plan::check_reuse_options(url_template.as_deref(), reuse.is_some())?;
            let reuse = plan::load_reuse(reuse.as_deref(), plan.as_deref())?;
            for config in &crates {
                if !component.is_empty() {
                    select_profiles(config, &component)?;
                }
            }
            let stage_bindings = component.is_empty() && (!unsigned || output.is_none());
            let output = output.unwrap_or_else(|| PathBuf::from("components.lock.json"));
            if let Some(template) = url_template.as_deref() {
                ComponentDistributionConfig::validate_url_template(template).map_err(|message| anyhow!(message))?;
            }
            let records = if reuse.is_some() && !input.exists() {
                Vec::new()
            } else {
                records_in(&input)?
            };
            ensure!(
                reuse.is_some() || !records.is_empty(),
                "no component artifact records found in {}",
                input.display()
            );
            let mut public_keys = BTreeMap::new();
            let mut artifacts = Vec::new();
            for config in &crates {
                if !config.components.iter().any(|profile| {
                    profile_selected(&component, profile)
                        && (target.is_empty()
                            || resolve_component_targets(config, profile)
                                .iter()
                                .any(|item| target.contains(item)))
                }) {
                    continue;
                }
                let distribution = config.component_distribution.as_ref();
                let signed_keys = if unsigned {
                    None
                } else {
                    let distribution = distribution.with_context(|| {
                        format!("crate `{}` has components but no component_distribution", config.name)
                    })?;
                    merge_public_keys(&mut public_keys, &distribution.public_keys)?;
                    Some(&distribution.public_keys)
                };
                let template = url_template
                    .clone()
                    .or_else(|| {
                        distribution
                            .map(|distribution| distribution.url_template.clone())
                            .filter(|template| !template.is_empty())
                    })
                    .with_context(|| {
                        format!(
                            "crate `{}` has no component_distribution.url_template; pass --url-template",
                            config.name
                        )
                    })?;
                let name_template =
                    distribution.and_then(|distribution| distribution.artifact_name_template.as_deref());
                let crate_records = records
                    .iter()
                    .filter(|(_, record)| {
                        record.manifest.identity.crate_name == config.name
                            && (target.is_empty() || target.contains(&record.manifest.identity.target))
                            && (component.is_empty() || component.contains(&record.manifest.identity.component))
                    })
                    .collect::<Vec<_>>();
                for (path, _) in &crate_records {
                    verify_record_with(path, signed_keys, name_template)?;
                }
                let reused =
                    plan::reuse_for_crate(reuse.as_ref(), config, &crate_records, signed_keys, &component, &target)?;
                let reused_keys = reused
                    .iter()
                    .map(|entry| (entry.identity.component.clone(), entry.identity.target.clone()))
                    .collect::<Vec<_>>();
                ensure_configured_matrix_present(config, &crate_records, &target, &component, &reused_keys)?;
                ensure!(
                    !(crate_records.is_empty() && reused.is_empty()),
                    "no artifact records found for crate `{}`",
                    config.name
                );
                let owned = crate_records
                    .iter()
                    .map(|(_, record)| (*record).clone())
                    .collect::<Vec<_>>();
                let lock = match signed_keys {
                    Some(keys) => build_lock(&owned, &template, keys)?,
                    None => build_unsigned_lock(&owned, &template)?,
                };
                artifacts.extend(lock.artifacts);
                artifacts.extend(reused);
            }
            ensure!(
                !artifacts.is_empty(),
                "no artifact records matched the selected targets"
            );
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
            for config in crates.into_iter().filter(|config| {
                stage_bindings
                    && (target.is_empty()
                        || lock
                            .artifacts
                            .iter()
                            .any(|artifact| artifact.identity.crate_name == config.name))
            }) {
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
        ComponentAction::ContractHash => {
            for config in &crates {
                let mut contracts = extract_contracts(config, &context.config_path)?;
                let ordered = config
                    .components
                    .iter()
                    .filter_map(|component| contracts.remove(component.name.as_str()))
                    .collect::<Vec<_>>();
                let mut stdout = std::io::stdout().lock();
                for line in contract_hash_lines(&config.name, &ordered)? {
                    // The hashes are the command's machine-readable result, so they go to stdout.
                    std::io::Write::write_all(&mut stdout, format!("{line}\n").as_bytes())
                        .context("failed to write the contract hashes")?;
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
                verify_record_with(
                    path,
                    Some(&distribution.public_keys),
                    distribution.artifact_name_template.as_deref(),
                )?;
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

/// The `component contract hash` output lines for one crate's resolved components.
fn contract_hash_lines(crate_name: &str, resolved: &[ResolvedComponent]) -> Result<Vec<String>> {
    Ok(contract_hash_rows(resolved)?
        .into_iter()
        .map(|(component, contract, hash)| {
            format!("component contract hash crate={crate_name} component={component} contract={contract} hash={hash}")
        })
        .collect())
}

/// Map the mutually exclusive `--signing-key` / `--signing-key-env` flags to a key source;
/// `--signing-key -` selects stdin.
fn signing_key_source(path: Option<PathBuf>, env_name: Option<String>) -> Option<SigningKeySource> {
    match (path, env_name) {
        (Some(path), _) if path.as_os_str() == "-" => Some(SigningKeySource::Stdin),
        (Some(path), _) => Some(SigningKeySource::File(path)),
        (None, Some(name)) => Some(SigningKeySource::Env(name)),
        (None, None) => None,
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

/// Write an empty, schema-valid `components.lock.json` at every path a binding for `config`
/// expects to `include_str!`, for any that does not exist yet.
///
/// Generated crates embed this file at compile time (see
/// `alef::backends::native_components::generate`), so a crate with `[[crates.components]]`
/// configured but no `alef component lock` run yet would otherwise fail its very first `cargo
/// build` -- before there is even an artifact to lock -- with an unhelpful "file not found"
/// from `include_str!`. `ComponentManager::from_lock`/`ensure_contract` treat this exact empty
/// shape as "no components locked yet" and report a typed error naming the command to run,
/// rather than a build failure with no actionable message.
///
/// Never overwrites an existing lock: a real one from a prior `alef component lock` run is
/// left untouched.
pub(crate) fn bootstrap_missing_component_locks(config: &ResolvedCrateConfig, base_dir: &Path) -> Result<()> {
    if config.components.is_empty() {
        return Ok(());
    }
    let empty_lock = ComponentLock {
        schema_version: crate::component::artifact::COMPONENT_MANIFEST_SCHEMA,
        public_keys: BTreeMap::new(),
        artifacts: Vec::new(),
    };
    let lock_bytes = canonical_json(&empty_lock)?;
    for relative_path in binding_component_lock_paths(config) {
        let path = base_dir.join(&relative_path);
        if path.is_file() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create component lock directory {}", parent.display()))?;
        }
        fs::write(&path, &lock_bytes)
            .with_context(|| format!("failed to bootstrap empty component lock {}", path.display()))?;
        tracing::info!("Bootstrapped empty component lock {}", path.display());
    }
    Ok(())
}

pub(crate) fn binding_component_lock_paths(config: &ResolvedCrateConfig) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();
    if let Some(host) = &config.component_host {
        paths.insert(host.lock_file_path());
    }
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
    // Older runtimes deny unknown lock fields, so the planning-only `input_hash` stays out of the
    // lock that ships inside binding packages.
    let artifacts = lock
        .artifacts
        .iter()
        .filter(|artifact| artifact.identity.crate_name == crate_name)
        .cloned()
        .map(|mut artifact| {
            artifact.input_hash = None;
            artifact
        })
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

fn validate_lock_targets(configs: &[&ResolvedCrateConfig], requested: &[String]) -> Result<()> {
    let configured = configs
        .iter()
        .flat_map(|config| {
            config
                .components
                .iter()
                .flat_map(|profile| resolve_component_targets(config, profile))
        })
        .collect::<HashSet<_>>();
    let missing = requested
        .iter()
        .filter(|target| !configured.contains(*target))
        .map(String::as_str)
        .collect::<Vec<_>>();
    ensure!(
        missing.is_empty(),
        "component lock target(s) not configured: {}",
        missing.join(", ")
    );
    Ok(())
}

fn ensure_configured_matrix_present(
    config: &ResolvedCrateConfig,
    records: &[&(PathBuf, ComponentArtifactRecord)],
    requested: &[String],
    components: &[String],
    reused: &[(String, String)],
) -> Result<()> {
    for profile in config
        .components
        .iter()
        .filter(|profile| profile_selected(components, profile))
    {
        for target in resolve_component_targets(config, profile) {
            if !requested.is_empty() && !requested.contains(&target) {
                continue;
            }
            ensure!(
                records.iter().any(|(_, record)| {
                    record.manifest.identity.component == profile.name && record.manifest.identity.target == target
                }) || reused
                    .iter()
                    .any(|(component, reused_target)| component == &profile.name && reused_target == &target),
                "component lock input is missing `{}` for target `{target}`",
                profile.name
            );
        }
    }
    Ok(())
}

/// Whether `profile` is one of the `requested` component names; an empty request selects every profile.
fn profile_selected(requested: &[String], profile: &ComponentConfig) -> bool {
    requested.is_empty() || requested.iter().any(|name| name == &profile.name)
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
#[path = "component_commands_tests.rs"]
mod tests;
