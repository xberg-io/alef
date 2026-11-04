use std::path::{Path, PathBuf};

use super::{ComponentConfig, ComponentDistributionConfig, RawCrateConfig, ResolveError, ResolvedCrateConfig};

pub(super) fn validate_components(
    krate: &RawCrateConfig,
    component_distribution: Option<&ComponentDistributionConfig>,
) -> Result<(), ResolveError> {
    let mut contract_names = std::collections::HashSet::new();
    for contract in &krate.component_contracts {
        validate_component_name(&krate.name, "contract", &contract.name)?;
        if !contract_names.insert(contract.name.as_str()) {
            return invalid_component_config(&krate.name, format!("duplicate component contract `{}`", contract.name));
        }
        validate_rust_item_path(&krate.name, "component contract trait_path", &contract.trait_path)?;
        if contract.interface_version == 0 {
            return invalid_component_config(
                &krate.name,
                format!(
                    "component contract `{}` interface_version must be greater than zero",
                    contract.name
                ),
            );
        }
    }

    let mut component_names = std::collections::HashSet::new();
    for component in &krate.components {
        validate_component_name(&krate.name, "component", &component.name)?;
        if !component_names.insert(component.name.as_str()) {
            return invalid_component_config(&krate.name, format!("duplicate component `{}`", component.name));
        }
        validate_component_provides(&krate.name, component, &contract_names)?;
        // A component built from the core crate is distinguished by its feature set, so it must name
        // one; a Go producer, or one whose implementation lives in its own `source_manifest` crate,
        // may enable none.
        let may_omit_features = component.producer_language == crate::core::config::ComponentProducerLanguage::Go
            || component.source_manifest.is_some();
        if !may_omit_features || !component.features.is_empty() {
            validate_nonempty_values(&krate.name, &component.name, "features", &component.features)?;
        }
        validate_component_targets(&krate.name, component)?;
        validate_component_dependencies(&krate.name, component)?;
        validate_component_locations(&krate.name, component)?;
    }

    validate_component_distribution(&krate.name, component_distribution)
}

fn validate_component_locations(crate_name: &str, component: &ComponentConfig) -> Result<(), ResolveError> {
    if component.producer_language == crate::core::config::ComponentProducerLanguage::Go
        && component.source_manifest.is_some()
    {
        return invalid_component_config(
            crate_name,
            format!(
                "Go component `{}` cannot declare `source_manifest`; it names a Cargo manifest",
                component.name
            ),
        );
    }
    if let Some(manifest) = &component.source_manifest {
        crate::core::config::component::validate_source_manifest(manifest).or_else(|reason| {
            invalid_component_config(crate_name, format!("component `{}`: {reason}", component.name))
        })?;
    }
    if let Some(dir) = &component.producer_dir {
        crate::core::config::component::validate_producer_dir(dir).or_else(|reason| {
            invalid_component_config(crate_name, format!("component `{}`: {reason}", component.name))
        })?;
    }
    Ok(())
}

/// Split paths into case-insensitive segments for portable overlap checks. Missing paths cannot
/// be canonicalized, and differently cased suffixes can become aliases when first written.
fn dir_segments(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(str::to_lowercase)
        .collect()
}

/// Reject collisions in the public path-independent configuration resolver.
pub(super) fn validate_producer_dir_overlaps(configs: &[ResolvedCrateConfig]) -> Result<(), ResolveError> {
    let lexical = |path: &Path| Ok(dir_segments(&path.to_string_lossy()));
    validate_producer_dirs(configs, &[], &[], lexical)
}

/// Recheck physical directories after explicit component paths have been rebased onto the working directory.
pub(crate) fn validate_physical_producer_paths(
    configs: &[ResolvedCrateConfig],
    cwd: &Path,
    config_dir: &Path,
) -> Result<(), ResolveError> {
    if !configs.iter().any(|config| !config.components.is_empty()) {
        return Ok(());
    }
    let physical = |path: &Path| {
        let absolute = super::super::component::lexically_clean(&cwd.join(path));
        physical_path(&absolute)
            .map(|path| dir_segments(&path.to_string_lossy()))
            .map_err(|error| {
                ResolveError::InvalidConfig(format!(
                    "cannot resolve component directory `{}`: {error}",
                    absolute.display()
                ))
            })
    };
    let working_root = physical(Path::new(""))?;
    let config_root = physical(config_dir)?;
    validate_producer_dirs(configs, &working_root, &config_root, physical)
}

// Resolve existing ancestors and keep the suffix that generation would create. ~keep
fn physical_path(path: &Path) -> std::io::Result<PathBuf> {
    let mut current = path;
    let mut suffix = Vec::new();
    loop {
        match current.canonicalize() {
            Ok(mut resolved) => {
                resolved.extend(suffix.into_iter().rev());
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match current.symlink_metadata() {
                    Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {}
                    Err(other) => return Err(other),
                    Ok(_) => return Err(error),
                }
                let (Some(parent), Some(name)) = (current.parent(), current.file_name()) else {
                    return Err(error);
                };
                suffix.push(name);
                current = parent;
            }
            Err(error) => return Err(error),
        }
    }
}

fn validate_producer_dirs(
    configs: &[ResolvedCrateConfig],
    working_root: &[String],
    config_root: &[String],
    resolve: impl Fn(&Path) -> Result<Vec<String>, ResolveError>,
) -> Result<(), ResolveError> {
    let mut inputs = Vec::new();
    for config in configs {
        inputs.extend(protected_component_inputs(config, working_root, config_root, &resolve)?);
    }
    let mut producers: Vec<(Vec<String>, &str)> = Vec::new();
    for config in configs {
        let core_dir = config.core_crate_dir();
        for component in &config.components {
            let dir = crate::core::config::component::component_producer_dir(
                &core_dir,
                &component.name,
                component.producer_dir.as_deref(),
            );
            let path = resolve(&dir)?;
            validate_producer_path(&config.name, &component.name, &path, &inputs, &producers)?;
            producers.push((path, &component.name));
        }
    }
    Ok(())
}

fn protected_component_inputs(
    config: &ResolvedCrateConfig,
    working_root: &[String],
    config_root: &[String],
    resolve: &impl Fn(&Path) -> Result<Vec<String>, ResolveError>,
) -> Result<Vec<(Vec<String>, String)>, ResolveError> {
    let core_dir = config.core_crate_root();
    let mut core = resolve(&core_dir)?;
    if core == working_root {
        core = resolve(&core_dir.join("src"))?;
    }
    let core_label = format!("core crate directory `{}`", core.join("/"));
    let mut inputs = vec![(core, core_label)];
    for manifest in config
        .components
        .iter()
        .filter_map(|component| component.source_manifest.as_deref())
    {
        let source = Path::new(manifest).parent().unwrap_or_else(|| Path::new(""));
        let mut path = resolve(source)?;
        if path == config_root || path == working_root {
            path = resolve(&source.join("src"))?;
        }
        inputs.push((path, format!("source_manifest directory `{manifest}`")));
    }
    Ok(inputs)
}

fn validate_producer_path(
    crate_name: &str,
    component: &str,
    path: &[String],
    inputs: &[(Vec<String>, String)],
    producers: &[(Vec<String>, &str)],
) -> Result<(), ResolveError> {
    let overlaps = |other: &[String]| path.starts_with(other) || other.starts_with(path);
    if let Some((other, previous)) = producers.iter().find(|(other, _)| overlaps(other)) {
        let relation = if other == path {
            "resolve to the same"
        } else {
            "resolve to overlapping"
        };
        return invalid_component_config(
            crate_name,
            format!(
                "components `{previous}` and `{component}` {relation} producer directory `{}`",
                path.join("/")
            ),
        );
    }
    if let Some((_, input)) = inputs.iter().find(|(input, _)| overlaps(input)) {
        return invalid_component_config(
            crate_name,
            format!(
                "component `{component}` producer_dir `{}` must not overlap {input}",
                path.join("/")
            ),
        );
    }
    Ok(())
}

fn validate_component_dependencies(crate_name: &str, component: &ComponentConfig) -> Result<(), ResolveError> {
    if component.producer_language == crate::core::config::ComponentProducerLanguage::Go
        && !component.extra_dependencies.is_empty()
    {
        return invalid_component_config(
            crate_name,
            format!(
                "Go component `{}` cannot declare Rust extra_dependencies",
                component.name
            ),
        );
    }
    for (name, spec) in &component.extra_dependencies {
        crate::core::config::component::validate_extra_dependency(crate_name, name, spec).or_else(|reason| {
            invalid_component_config(crate_name, format!("component `{}`: {reason}", component.name))
        })?;
    }
    Ok(())
}

fn validate_component_provides(
    crate_name: &str,
    component: &ComponentConfig,
    contract_names: &std::collections::HashSet<&str>,
) -> Result<(), ResolveError> {
    if component.provides.is_empty() {
        return invalid_component_config(
            crate_name,
            format!(
                "component `{}` must declare at least one entry in `provides`",
                component.name
            ),
        );
    }
    let mut provided = std::collections::HashSet::new();
    for entry in &component.provides {
        if !contract_names.contains(entry.contract.as_str()) {
            return invalid_component_config(
                crate_name,
                format!(
                    "component `{}` references unknown contract `{}`",
                    component.name, entry.contract
                ),
            );
        }
        if !provided.insert(entry.contract.as_str()) {
            return invalid_component_config(
                crate_name,
                format!(
                    "component `{}` provides contract `{}` more than once",
                    component.name, entry.contract
                ),
            );
        }
        validate_component_implementation(crate_name, component, &entry.implementation)?;
    }
    Ok(())
}

fn validate_component_implementation(
    crate_name: &str,
    component: &ComponentConfig,
    implementation: &str,
) -> Result<(), ResolveError> {
    if component.producer_language == crate::core::config::ComponentProducerLanguage::Rust {
        return validate_rust_item_path(crate_name, "component implementation", implementation);
    }
    if crate::core::config::component::is_valid_go_constructor_identifier(implementation) {
        return Ok(());
    }
    invalid_component_config(
        crate_name,
        format!(
            "component `{}` implementation `{implementation}` must be a Go constructor identifier",
            component.name
        ),
    )
}

fn validate_component_targets(crate_name: &str, component: &ComponentConfig) -> Result<(), ResolveError> {
    let Some(targets) = component.targets.as_ref() else {
        return Ok(());
    };
    for target in targets {
        if !crate::core::config::component::is_valid_target_triple(target) {
            return invalid_component_config(
                crate_name,
                format!(
                    "component `{}` target `{target}` is not a valid Rust target triple",
                    component.name
                ),
            );
        }
        if component.bundled_on.iter().any(|bundled| bundled == target) {
            return invalid_component_config(
                crate_name,
                format!(
                    "component `{}` target `{target}` is listed in both `targets` and `bundled_on`",
                    component.name
                ),
            );
        }
    }
    Ok(())
}

fn validate_component_name(crate_name: &str, kind: &str, name: &str) -> Result<(), ResolveError> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-');
    if valid {
        return Ok(());
    }
    invalid_component_config(
        crate_name,
        format!("{kind} name `{name}` must contain only ASCII letters, digits, `_`, or `-`"),
    )
}

fn validate_rust_item_path(crate_name: &str, label: &str, path: &str) -> Result<(), ResolveError> {
    let normalized = path.strip_prefix("::").unwrap_or(path);
    let segments: Vec<&str> = normalized.split("::").collect();
    let valid = segments.len() >= 2 && segments.iter().all(|segment| is_valid_rust_path_segment(segment));
    if valid {
        return Ok(());
    }
    invalid_component_config(
        crate_name,
        format!("{label} `{path}` must be a fully-qualified Rust item path"),
    )
}

fn is_valid_rust_path_segment(segment: &str) -> bool {
    let segment = segment.strip_prefix("r#").unwrap_or(segment);
    let mut characters = segment.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn validate_nonempty_values(
    crate_name: &str,
    component_name: &str,
    label: &str,
    values: &[String],
) -> Result<(), ResolveError> {
    if !values.is_empty() && values.iter().all(|value| !value.trim().is_empty()) {
        return Ok(());
    }
    invalid_component_config(
        crate_name,
        format!("component `{component_name}` must declare non-empty {label}"),
    )
}

fn validate_component_distribution(
    crate_name: &str,
    distribution: Option<&ComponentDistributionConfig>,
) -> Result<(), ResolveError> {
    let Some(distribution) = distribution else {
        return Ok(());
    };
    if let Err(message) = ComponentDistributionConfig::validate_url_template(&distribution.url_template) {
        return invalid_component_config(crate_name, message);
    }
    if let Some(template) = distribution.artifact_name_template.as_deref()
        && let Err(message) = ComponentDistributionConfig::validate_artifact_name_template(template)
    {
        return invalid_component_config(crate_name, message);
    }
    for (key_id, encoded_key) in &distribution.public_keys {
        validate_component_name(crate_name, "component distribution public key", key_id)?;
        // Shared with `alef-component-runtime`'s loader and `alef`'s artifact verification
        // so a key that validates here is guaranteed to also decode when the lock is
        // loaded -- see `alef_component_runtime::decode_public_key`.
        if alef_component_runtime::decode_public_key(encoded_key).is_err() {
            return invalid_component_config(
                crate_name,
                format!(
                    "component distribution public key `{key_id}` must be a PEM or base64-encoded \
                     (standard or unpadded) DER/raw 32-byte Ed25519 public key"
                ),
            );
        }
    }
    Ok(())
}

fn invalid_component_config<T>(crate_name: &str, message: impl std::fmt::Display) -> Result<T, ResolveError> {
    Err(ResolveError::InvalidConfig(format!("crate `{crate_name}`: {message}")))
}

/// Validate `[crates.component_host]`: the paths are well formed and the crate has components.
pub(super) fn validate_component_host(krate: &RawCrateConfig) -> Result<(), ResolveError> {
    let Some(host) = &krate.component_host else {
        return Ok(());
    };
    host.validate()
        .or_else(|reason| invalid_component_config(&krate.name, reason))?;
    if krate.components.is_empty() {
        return invalid_component_config(
            &krate.name,
            "component_host requires at least one `[[crates.components]]` entry",
        );
    }
    let mut selected = std::collections::HashSet::new();
    for id in &host.stdio_components {
        if !krate.components.iter().any(|component| &component.name == id) {
            return invalid_component_config(
                &krate.name,
                format!("component_host references unknown component `{id}`"),
            );
        }
        if !selected.insert(id) {
            return invalid_component_config(
                &krate.name,
                format!("component_host has duplicate stdio component `{id}`"),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod transport_tests {
    use crate::core::config::NewAlefConfig;

    fn configuration(producer: &str, implementation: &str, stdio: &[&str]) -> NewAlefConfig {
        serde_json::from_value(serde_json::json!({
            "workspace": { "languages": ["ffi"] },
            "crates": [{
                "name": "sample", "sources": ["src/lib.rs"],
                "component_contracts": [{ "name": "codec", "trait_path": "sample::Codec" }],
                "components": [{
                    "name": "reverse", "producer_language": producer,
                    "provides": [{ "contract": "codec", "implementation": implementation }],
                    "features": ["reverse"]
                }],
                "component_host": { "output": "host.rs", "stdio_components": stdio }
            }]
        }))
        .expect("component transport configuration must deserialize")
    }

    #[test]
    fn component_transport_accepts_go_constructor_and_known_stdio_id() {
        let resolved = configuration("go", "NewCodec", &["reverse"]).resolve().unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].components[0].provides[0].implementation, "NewCodec");
    }

    #[test]
    fn component_transport_rejects_go_expressions_keywords_and_injection() {
        for implementation in [
            "NewCodec()",
            "codec.NewCodec",
            "sample::Codec",
            "func",
            "_",
            "9Codec",
            "NewCodec;panic()",
        ] {
            let error = configuration("go", implementation, &[])
                .resolve()
                .unwrap_err()
                .to_string();
            assert!(error.contains("Go constructor identifier"), "{implementation}: {error}");
        }
    }

    #[test]
    fn component_transport_rejects_rust_dependencies_for_go_producer() {
        let mut config = configuration("go", "NewCodec", &[]);
        config.crates[0].components[0]
            .extra_dependencies
            .insert("serde".into(), serde_json::json!("1"));
        let error = config.resolve().unwrap_err().to_string();
        assert!(error.contains("extra_dependencies") && error.contains("Go"), "{error}");
    }

    #[test]
    fn component_transport_retains_rust_item_path_validation() {
        let error = configuration("rust", "NewCodec", &[])
            .resolve()
            .unwrap_err()
            .to_string();
        assert!(error.contains("fully-qualified Rust item path"), "{error}");
        assert!(configuration("rust", "sample::Codec", &[]).resolve().is_ok());
    }

    #[test]
    fn component_transport_rejects_unknown_or_duplicate_stdio_ids() {
        for (ids, expected) in [
            (vec!["missing"], "unknown component `missing`"),
            (vec!["reverse", "reverse"], "duplicate stdio component `reverse`"),
        ] {
            let error = configuration("rust", "sample::Codec", &ids)
                .resolve()
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
        }
    }
}
