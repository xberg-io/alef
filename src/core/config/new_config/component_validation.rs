use super::{ComponentConfig, ComponentDistributionConfig, RawCrateConfig, ResolveError};

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
        validate_nonempty_values(&krate.name, &component.name, "features", &component.features)?;
        validate_component_targets(&krate.name, component)?;
    }

    validate_component_distribution(&krate.name, component_distribution)
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
        validate_rust_item_path(crate_name, "component implementation", &entry.implementation)?;
    }
    Ok(())
}

fn validate_component_targets(crate_name: &str, component: &ComponentConfig) -> Result<(), ResolveError> {
    let Some(targets) = component.targets.as_ref() else {
        return Ok(());
    };
    for target in targets {
        if !crate::core::config::component::SUPPORTED_COMPONENT_TARGETS.contains(&target.as_str()) {
            return invalid_component_config(
                crate_name,
                format!("component `{}` uses unsupported v1 target `{target}`", component.name),
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
    let url_template = distribution.url_template.trim();
    let authority = url_template
        .strip_prefix("https://")
        .and_then(|remainder| remainder.split('/').next());
    if authority.is_none_or(|value| value.is_empty() || value.chars().any(char::is_whitespace)) {
        return invalid_component_config(crate_name, "component distribution url_template must use HTTPS");
    }
    for placeholder in ["component", "version", "target", "artifact"] {
        if !url_template.contains(&format!("{{{placeholder}}}")) {
            return invalid_component_config(
                crate_name,
                format!("component distribution url_template must contain `{{{placeholder}}}`"),
            );
        }
    }
    if distribution.public_keys.is_empty() {
        return invalid_component_config(
            crate_name,
            "component distribution must declare at least one public key",
        );
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
