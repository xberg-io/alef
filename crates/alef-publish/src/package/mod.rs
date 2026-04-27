//! Artifact packaging — creates distributable archives for each language.

pub mod c_ffi;
pub mod cli;
pub mod dart;
pub mod gleam;
pub mod go;
pub mod kotlin;
pub mod php;
pub mod swift;
pub mod util;
pub mod zig;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A produced package artifact.
#[derive(Debug)]
pub struct PackageArtifact {
    /// Path to the artifact file.
    pub path: PathBuf,
    /// Human-readable artifact name.
    pub name: String,
    /// SHA256 hex digest (if computed).
    pub checksum: Option<String>,
}

/// Create a tar.gz archive from a staging directory.
pub fn create_tar_gz(staging_dir: &Path, output_path: &Path) -> Result<()> {
    let file_name = staging_dir
        .file_name()
        .context("staging dir has no file name")?
        .to_string_lossy();

    let status = std::process::Command::new("tar")
        .arg("czf")
        .arg(output_path)
        .arg("-C")
        .arg(staging_dir.parent().unwrap_or(Path::new(".")))
        .arg(file_name.as_ref())
        .status()?;

    if !status.success() {
        anyhow::bail!("tar failed with exit code {}", status.code().unwrap_or(-1));
    }
    Ok(())
}

/// Find a built artifact in the target directory.
///
/// Searches `target/{triple}/release/` then `target/release/` for the given filename.
pub fn find_built_artifact(
    workspace_root: &Path,
    target: &crate::platform::RustTarget,
    filename: &str,
) -> Result<PathBuf> {
    let cross = workspace_root
        .join("target")
        .join(&target.triple)
        .join("release")
        .join(filename);
    if cross.exists() {
        return Ok(cross);
    }
    let native = workspace_root.join("target/release").join(filename);
    if native.exists() {
        return Ok(native);
    }
    anyhow::bail!(
        "{filename} not found in target/{}/release/ or target/release/",
        target.triple
    )
}
