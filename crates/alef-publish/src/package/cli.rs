//! CLI binary packaging — archives the compiled binary with LICENSE and README.

use super::PackageArtifact;
use crate::platform::RustTarget;
use alef_core::config::AlefConfig;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Package CLI binary into a distributable archive.
///
/// Produces: `{name}-v{version}-{target}.{tar.gz|zip}` containing:
/// - the binary
/// - `LICENSE`
/// - `README.md`
pub fn package_cli(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let crate_name = &config.crate_config.name;
    let binary_name = format!("{crate_name}{}", target.binary_ext());

    let pkg_name = format!("{crate_name}-v{version}-{}", target.triple);
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;

    // Find and copy the CLI binary.
    let bin_src = find_binary(workspace_root, target, &binary_name)?;
    fs::copy(&bin_src, staging.join(&binary_name))?;

    // Copy LICENSE.
    for name in &["LICENSE", "LICENSE-MIT", "LICENSE-APACHE"] {
        let src = workspace_root.join(name);
        if src.exists() {
            fs::copy(&src, staging.join(name))?;
            break;
        }
    }

    // Copy README.
    let readme = workspace_root.join("README.md");
    if readme.exists() {
        fs::copy(&readme, staging.join("README.md"))?;
    }

    // Create archive.
    let ext = target.archive_ext();
    let archive_name = format!("{pkg_name}.{ext}");
    let archive_path = output_dir.join(&archive_name);
    super::create_tar_gz(&staging, &archive_path)?;

    fs::remove_dir_all(&staging).ok();

    Ok(PackageArtifact {
        path: archive_path,
        name: archive_name,
        checksum: None,
    })
}

fn find_binary(workspace_root: &Path, target: &RustTarget, binary_name: &str) -> Result<PathBuf> {
    let cross = workspace_root
        .join("target")
        .join(&target.triple)
        .join("release")
        .join(binary_name);
    if cross.exists() {
        return Ok(cross);
    }
    let native = workspace_root.join("target/release").join(binary_name);
    if native.exists() {
        return Ok(native);
    }
    anyhow::bail!("CLI binary {binary_name} not found")
}
