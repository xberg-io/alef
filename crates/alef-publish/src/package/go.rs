//! Go FFI package — archives the shared library + header for GitHub Release upload.

use super::PackageArtifact;
use crate::platform::RustTarget;
use alef_core::config::AlefConfig;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Package Go FFI artifacts into a distributable tarball.
///
/// Produces: `{name}-ffi-v{version}-{platform}.tar.gz` containing:
/// - `lib/` — shared library (and optionally static library)
/// - `include/` — C header
pub fn package_go_ffi(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let lib_name = config.ffi_lib_name();
    let header_name = config.ffi_header_name();
    let crate_name = &config.crate_config.name;
    let platform = target.platform_for(alef_core::config::extras::Language::Go);

    let pkg_name = format!("{crate_name}-ffi-v{version}-{platform}");
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    let lib_dir = staging.join("lib");
    let include_dir = staging.join("include");
    fs::create_dir_all(&lib_dir)?;
    fs::create_dir_all(&include_dir)?;

    // Copy shared library.
    let shared_lib = target.shared_lib_name(&lib_name);
    let shared_src = find_lib(workspace_root, target, &shared_lib)?;
    fs::copy(&shared_src, lib_dir.join(&shared_lib))?;

    // Copy static library (optional).
    let static_lib = target.static_lib_name(&lib_name);
    if let Ok(static_src) = find_lib(workspace_root, target, &static_lib) {
        fs::copy(&static_src, lib_dir.join(&static_lib))?;
    }

    // Copy header.
    let ffi_crate_dir = crate::ffi_stage::find_ffi_crate_dir_pub(config, workspace_root);
    let header_src = ffi_crate_dir.join("include").join(&header_name);
    if header_src.exists() {
        fs::copy(&header_src, include_dir.join(&header_name))?;
    }

    // Create tarball.
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

fn find_lib(workspace_root: &Path, target: &RustTarget, lib_file: &str) -> Result<PathBuf> {
    let cross = workspace_root
        .join("target")
        .join(&target.triple)
        .join("release")
        .join(lib_file);
    if cross.exists() {
        return Ok(cross);
    }
    let native = workspace_root.join("target/release").join(lib_file);
    if native.exists() {
        return Ok(native);
    }
    anyhow::bail!("{lib_file} not found")
}
