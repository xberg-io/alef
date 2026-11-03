use anyhow::{Context, bail};
use std::path::{Path, PathBuf};

use crate::core::config::ResolvedCrateConfig;

pub(super) fn r_rust_min_version(config: &ResolvedCrateConfig) -> anyhow::Result<String> {
    let floor = crate::core::template_versions::toolchain::RUST_MIN_VERSION;
    let Some(path) = super::core_crate_manifest_path(config) else {
        return Ok(floor.to_string());
    };
    let Some(manifest) = read_manifest(&path)? else {
        return Ok(floor.to_string());
    };
    let Some(required) = manifest.get("package").and_then(|package| package.get("rust-version")) else {
        return Ok(floor.to_string());
    };
    let version = if let Some(version) = required.as_str() {
        version.to_string()
    } else if required
        .as_table()
        .is_some_and(|table| table.len() == 1 && table.get("workspace").and_then(toml::Value::as_bool) == Some(true))
    {
        inherited_version(&path, &manifest)?
    } else {
        bail!(
            "invalid package.rust-version in {}: expected a version or workspace = true",
            path.display()
        );
    };
    if parse_version(&version, &path)? > parse_version(floor, &path)? {
        Ok(version)
    } else {
        Ok(floor.to_string())
    }
}

fn read_manifest(path: &Path) -> anyhow::Result<Option<toml::Value>> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read Rust MSRV from {}", path.display())),
    };
    toml::from_str(&content)
        .with_context(|| format!("parse Rust MSRV manifest {}", path.display()))
        .map(Some)
}

fn parse_version(version: &str, path: &Path) -> anyhow::Result<semver::Version> {
    let normalized = if version.split('.').count() == 2 {
        format!("{version}.0")
    } else {
        version.to_string()
    };
    let parsed = semver::Version::parse(&normalized)
        .with_context(|| format!("invalid rust-version {version:?} in {}", path.display()))?;
    if !parsed.pre.is_empty() || !parsed.build.is_empty() {
        bail!(
            "invalid rust-version {version:?} in {}: expected a release version",
            path.display()
        );
    }
    Ok(parsed)
}

fn inherited_version(path: &Path, manifest: &toml::Value) -> anyhow::Result<String> {
    let workspace_path = workspace_manifest_path(path, manifest)?;
    let workspace = read_manifest(&workspace_path)?.with_context(|| {
        format!(
            "missing workspace manifest {} inherited by {}",
            workspace_path.display(),
            path.display()
        )
    })?;
    let version = workspace
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("rust-version"))
        .and_then(toml::Value::as_str)
        .with_context(|| {
            format!(
                "missing or invalid workspace.package.rust-version in {} inherited by {}",
                workspace_path.display(),
                path.display()
            )
        })?;
    parse_version(version, &workspace_path)?;
    Ok(version.to_string())
}

fn workspace_manifest_path(path: &Path, manifest: &toml::Value) -> anyhow::Result<PathBuf> {
    let directory = path.parent().context("bound crate manifest has no parent directory")?;
    if let Some(explicit) = manifest.get("package").and_then(|package| package.get("workspace")) {
        let relative = explicit
            .as_str()
            .with_context(|| format!("invalid package.workspace in {}", path.display()))?;
        return Ok(directory.join(relative).join("Cargo.toml"));
    }
    if manifest.get("workspace").is_some() {
        return Ok(path.to_path_buf());
    }
    for ancestor in directory.ancestors().skip(1) {
        let candidate = ancestor.join("Cargo.toml");
        if read_manifest(&candidate)?.is_some_and(|manifest| manifest.get("workspace").is_some()) {
            return Ok(candidate);
        }
    }
    bail!("cannot resolve workspace rust-version inherited by {}", path.display())
}
