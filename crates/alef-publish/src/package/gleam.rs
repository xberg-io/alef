//! Gleam Hex package — archives the source tree for distribution.

use super::util::copy_dir_recursive;
use super::PackageArtifact;
use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Package Gleam bindings into a source tarball.
///
/// Produces: `{name}-{version}.tar` containing the Gleam source tree.
///
/// Note: this is NOT a directly-uploadable Hex tarball — Hex requires the
/// nested `metadata.config` + `contents.tar.gz` + `CHECKSUM` layout that
/// `gleam publish` produces internally. Use this artifact for archival and
/// run `gleam publish` from the package directory for actual Hex uploads.
pub fn package_gleam(
    config: &AlefConfig,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let crate_name = &config.crate_config.name;
    let pkg_dir = config.package_dir(alef_core::config::extras::Language::Gleam);

    let pkg_name = format!("{crate_name}-{version}");
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;

    // Copy the entire gleam package directory into staging.
    let pkg_src = workspace_root.join(&pkg_dir);
    if !pkg_src.exists() {
        anyhow::bail!("Gleam package directory not found: {}", pkg_dir);
    }

    // Copy all files from the Gleam package.
    copy_dir_recursive(&pkg_src, &staging).context("copying Gleam package directory")?;

    // Create tarball (Hex expects .tar, not .tar.gz).
    let archive_name = format!("{pkg_name}.tar");
    let archive_path = output_dir.join(&archive_name);

    let status = std::process::Command::new("tar")
        .arg("cf")
        .arg(&archive_path)
        .arg("-C")
        .arg(output_dir)
        .arg(&pkg_name)
        .status()
        .context("creating Gleam tarball")?;

    if !status.success() {
        anyhow::bail!("tar failed with exit code {}", status.code().unwrap_or(-1));
    }

    // Clean up staging.
    fs::remove_dir_all(&staging).ok();

    Ok(PackageArtifact {
        path: archive_path,
        name: archive_name,
        checksum: None,
    })
}

