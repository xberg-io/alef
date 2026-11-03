//! Cargo license metadata for generated native component producers.

use crate::core::config::ResolvedCrateConfig;
use anyhow::{Context, Result, bail};
use std::path::Path;

fn read_producer_license_manifest(path: &Path) -> Result<Option<toml::Value>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("reading producer license metadata from {}", path.display()));
        }
    };
    toml::from_str(&contents)
        .map(Some)
        .with_context(|| format!("parsing producer license metadata from {}", path.display()))
}

fn workspace_producer_license(manifest: &toml::Value, path: &Path) -> Result<String> {
    manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("license"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .with_context(|| {
            format!(
                "missing workspace.package.license for producer license inheritance in {}",
                path.display()
            )
        })
}

fn inherited_producer_license(core_root: &Path, manifest: &toml::Value) -> Result<String> {
    let core_manifest = core_root.join("Cargo.toml");
    if manifest.get("workspace").is_some() {
        return workspace_producer_license(manifest, &core_manifest);
    }
    if let Some(workspace) = manifest.get("package").and_then(|package| package.get("workspace")) {
        let workspace = workspace
            .as_str()
            .with_context(|| format!("package.workspace must be a path in {}", core_manifest.display()))?;
        let path = core_root.join(workspace).join("Cargo.toml");
        let owner = read_producer_license_manifest(&path)?.with_context(|| {
            format!(
                "missing explicit producer license workspace manifest {}",
                path.display()
            )
        })?;
        return workspace_producer_license(&owner, &path);
    }
    for ancestor in core_root.ancestors().skip(1) {
        let path = ancestor.join("Cargo.toml");
        if let Some(owner) = read_producer_license_manifest(&path)?
            && owner.get("workspace").is_some()
        {
            return workspace_producer_license(&owner, &path);
        }
    }
    bail!(
        "no owning workspace for producer license inheritance in {}",
        core_manifest.display()
    )
}

pub(super) fn producer_license(config: &ResolvedCrateConfig) -> Result<Option<String>> {
    if let Some(license) = crate::scaffold::scaffold_meta(config).license {
        return Ok(Some(license));
    }
    let Some(root) = &config.workspace_root else {
        return Ok(None);
    };
    let core_root = root.join(config.core_crate_root());
    let Some(manifest) = read_producer_license_manifest(&core_root.join("Cargo.toml"))? else {
        return Ok(None);
    };
    let Some(license) = manifest.get("package").and_then(|package| package.get("license")) else {
        return Ok(None);
    };
    if let Some(license) = license.as_str() {
        return Ok(Some(license.to_owned()));
    }
    if license.get("workspace").and_then(toml::Value::as_bool) != Some(true) {
        return Ok(None);
    }
    inherited_producer_license(&core_root, &manifest).map(Some)
}
