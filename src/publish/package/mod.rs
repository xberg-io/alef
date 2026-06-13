//! Artifact packaging — creates distributable archives for each language.

pub mod c_ffi;
pub mod cli;
pub mod csharp;
pub mod dart;
pub mod elixir;
pub mod gleam;
pub mod go;
pub mod java;
pub mod kotlin;
pub mod node;
pub mod php;
pub mod python;
pub mod ruby;
pub mod swift;
pub mod util;
pub mod wasm;
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
///
/// The staging directory's basename becomes the single top-level entry inside
/// the archive — so callers whose consumers expect that wrapper (CLI tarballs,
/// FFI tarballs, language SDK archives) get the conventional `dirname/...`
/// layout. For consumers that need the staging contents at the archive root
/// (PHP PIE, which probes the extracted-source root for the extension `.so`),
/// use [`create_tar_gz_flat`] instead.
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

/// Create a tar.gz archive whose entries are the contents of `staging_dir`,
/// without the wrapping directory.
///
/// PHP PIE's `UnixBuild` probes the extracted-source root for the extension
/// `.so`; if it sees only a single subdirectory it would `unfoldUnarchivedSourcePaths()`,
/// but only when that subdir contains `config.m4` / `config.w32`. Our PIE
/// archive is a precompiled binary with neither, so PIE never unfolds and the
/// install fails with "extension not found". Archive contents directly so the
/// `.so` lands at the archive root.
///
/// Entries are enumerated explicitly rather than passing `.` to `tar`, because
/// `tar czf out.tgz -C dir .` emits a leading `./` directory entry that PIE's
/// Phar-based `TarDownloader` rejects with `Cannot extract ".", internal error`.
/// Passing each top-level entry by name produces a flat archive with no
/// directory entries at all.
pub fn create_tar_gz_flat(staging_dir: &Path, output_path: &Path) -> Result<()> {
    let mut entries: Vec<String> = std::fs::read_dir(staging_dir)
        .with_context(|| format!("reading staging dir {}", staging_dir.display()))?
        .map(|res| {
            res.map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(anyhow::Error::from)
        })
        .collect::<Result<Vec<_>>>()?;
    if entries.is_empty() {
        anyhow::bail!(
            "staging dir {} is empty; refusing to create empty archive",
            staging_dir.display()
        );
    }
    entries.sort();

    let status = std::process::Command::new("tar")
        .arg("czf")
        .arg(output_path)
        .arg("-C")
        .arg(staging_dir)
        .args(&entries)
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
    target: &crate::publish::platform::RustTarget,
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
