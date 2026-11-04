//! Version checks for a components-only crate, whose alef-owned manifests are the generated
//! Rust producer crates rather than binding packages.

use super::validate_versions::{VersionCheck, read_cargo_version};
use crate::core::config::component::component_producer_dir;
use crate::core::config::{ComponentProducerLanguage, ResolvedCrateConfig};
use std::path::Path;

const VERSION_CONSTANT_PREFIX: &str = "const COMPONENT_VERSION: &[u8] = b\"";

fn has_rust_producers(config: &ResolvedCrateConfig) -> bool {
    config
        .components
        .iter()
        .any(|component| component.producer_language == ComponentProducerLanguage::Rust)
}

/// A components-only crate whose producers are all non-Rust (Go) has no alef-owned manifest that
/// carries a version, so an empty check set is the correct result, not a gate that examined nothing.
pub(crate) fn nothing_to_examine(config: &ResolvedCrateConfig) -> bool {
    config.is_components_only() && !has_rust_producers(config)
}

fn failing(label: String) -> VersionCheck {
    VersionCheck {
        label,
        matches: false,
        found: None,
        blocked_on_publish: None,
    }
}

fn embedded_version(lib_rs: &str) -> Option<String> {
    let rest = lib_rs
        .lines()
        .find_map(|line| line.trim_start().strip_prefix(VERSION_CONSTANT_PREFIX))?;
    rest.split_once('"').map(|(version, _)| version.to_string())
}

fn check(label: String, found: Option<String>, canonical: &str) -> VersionCheck {
    match found {
        Some(found) => VersionCheck {
            matches: found == canonical,
            label,
            found: Some(found),
            blocked_on_publish: None,
        },
        None => failing(label),
    }
}

/// Checks for every Rust producer: its `Cargo.toml` version and the `COMPONENT_VERSION` constant
/// embedded in its `src/lib.rs`. A manifest or constant that cannot be read is a failing check,
/// never a dropped one. Go producers carry no version-bearing manifest and yield nothing.
pub(super) fn collect(config: &ResolvedCrateConfig, workspace_root: &Path, canonical: &str) -> Vec<VersionCheck> {
    let core_dir = config.core_crate_dir();
    let mut checks = Vec::new();
    for component in config
        .components
        .iter()
        .filter(|component| component.producer_language == ComponentProducerLanguage::Rust)
    {
        let dir = component_producer_dir(&core_dir, &component.name, component.producer_dir.as_deref());
        let label_of = |file: &str| dir.join(file).to_string_lossy().replace('\\', "/");

        let manifest = label_of("Cargo.toml");
        let found = read_cargo_version(&workspace_root.join(&manifest));
        checks.push(check(manifest, found, canonical));

        let lib = label_of("src/lib.rs");
        let found = std::fs::read_to_string(workspace_root.join(&lib))
            .ok()
            .and_then(|content| embedded_version(&content));
        checks.push(check(lib, found, canonical));
    }
    checks
}

#[cfg(test)]
mod tests;
