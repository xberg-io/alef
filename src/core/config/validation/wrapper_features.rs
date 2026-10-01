use super::super::abi_grammar;
use super::super::resolved::ResolvedCrateConfig;
use std::path::{Path, PathBuf};

pub(super) fn validate(config: &ResolvedCrateConfig) -> Result<(), String> {
    if config.wrapper_default_features.is_empty() {
        return Ok(());
    }

    for feature in &config.wrapper_default_features {
        validate_name(feature).map_err(|error| {
            format!(
                "crate `{}`: wrapper_default_features value `{feature}` is invalid: {error}",
                config.name
            )
        })?;
    }

    for (target, language, excluded) in native_exclusions(config) {
        if !config.targets(target) {
            continue;
        }
        for feature in &config.wrapper_default_features {
            if excluded.iter().any(|name| name == feature) {
                return Err(format!(
                    "crate `{}`: wrapper_default_features feature `{feature}` conflicts with \
                     `[crates.{language}].excluded_default_features`; a feature cannot be both \
                     required as a native wrapper default and excluded from that wrapper's defaults",
                    config.name
                ));
            }
        }
    }

    let manifest_path = core_manifest_path(config)?;
    let declared = crate::codegen::cfg::read_declared_cargo_features(&manifest_path).ok_or_else(|| {
        format!(
            "crate `{}`: cannot validate wrapper_default_features because the core crate manifest \
             `{}` cannot be read or parsed",
            config.name,
            manifest_path.display()
        )
    })?;
    for feature in &config.wrapper_default_features {
        if !declared.contains(feature) {
            return Err(format!(
                "crate `{}`: wrapper_default_features feature `{feature}` is not declared in the \
                 core crate's `[features]` table at `{}`",
                config.name,
                manifest_path.display()
            ));
        }
    }

    Ok(())
}

pub(super) fn validate_name(feature: &str) -> Result<(), String> {
    abi_grammar::validate_cargo_feature_name(feature)?;
    if matches!(feature, "default" | "extension-module") {
        return Err(format!(
            "feature `{feature}` is reserved by generated native wrapper manifests"
        ));
    }
    Ok(())
}

fn native_exclusions(config: &ResolvedCrateConfig) -> [(super::super::extras::Language, &'static str, &[String]); 4] {
    use super::super::extras::Language;

    [
        (
            Language::Node,
            "node",
            config
                .node
                .as_ref()
                .map(|value| value.excluded_default_features.as_slice())
                .unwrap_or_default(),
        ),
        (
            Language::Ruby,
            "ruby",
            config
                .ruby
                .as_ref()
                .map(|value| value.excluded_default_features.as_slice())
                .unwrap_or_default(),
        ),
        (
            Language::Php,
            "php",
            config
                .php
                .as_ref()
                .map(|value| value.excluded_default_features.as_slice())
                .unwrap_or_default(),
        ),
        (
            Language::Elixir,
            "elixir",
            config
                .elixir
                .as_ref()
                .map(|value| value.excluded_default_features.as_slice())
                .unwrap_or_default(),
        ),
    ]
}

fn core_manifest_path(config: &ResolvedCrateConfig) -> Result<PathBuf, String> {
    let source = config.sources.first().ok_or_else(|| {
        format!(
            "crate `{}`: cannot validate wrapper_default_features because no core source path is configured",
            config.name
        )
    })?;
    let crate_dir = source
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "src"))
        .and_then(Path::parent)
        .ok_or_else(|| {
            format!(
                "crate `{}`: cannot validate wrapper_default_features because source path `{}` \
                 is not inside a crate `src` directory",
                config.name,
                source.display()
            )
        })?;
    let root = match &config.workspace_root {
        Some(root) => root.clone(),
        None => std::env::current_dir().map_err(|error| {
            format!(
                "crate `{}`: cannot resolve the workspace root for wrapper_default_features: {error}",
                config.name
            )
        })?,
    };
    Ok(root.join(crate_dir).join("Cargo.toml"))
}
