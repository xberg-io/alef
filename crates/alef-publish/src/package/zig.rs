//! Zig package — archives the source code + FFI shared library for distribution.

use super::util::copy_dir_recursive;
use super::PackageArtifact;
use crate::platform::RustTarget;
use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Package Zig bindings as a source distribution with bundled FFI library.
///
/// Produces: `{name}-v{version}.tar.gz` containing:
/// - `src/` — Zig source code
/// - `lib/` — FFI shared library (.so/.dylib)
/// - `include/` — C header
/// - `build.zig`, `build.zig.zon` — Zig build files
pub fn package_zig(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let lib_name = config.ffi_lib_name();
    let header_name = config.ffi_header_name();
    let crate_name = &config.crate_config.name;
    let pkg_dir = config.package_dir(alef_core::config::extras::Language::Zig);

    let pkg_name = format!("{crate_name}-v{version}");
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;

    // Copy Zig package source files.
    let pkg_src = workspace_root.join(&pkg_dir);
    if !pkg_src.exists() {
        anyhow::bail!("Zig package directory not found: {}", pkg_dir);
    }

    copy_dir_recursive(&pkg_src, &staging).context("copying Zig package")?;

    // Create lib/ and include/ directories if needed.
    let lib_dir = staging.join("lib");
    let include_dir = staging.join("include");
    fs::create_dir_all(&lib_dir)?;
    fs::create_dir_all(&include_dir)?;

    // Copy FFI shared library — required for the Zig package to be usable.
    let shared_lib = target.shared_lib_name(&lib_name);
    let shared_src = super::find_built_artifact(workspace_root, target, &shared_lib)
        .with_context(|| format!("locating built FFI artifact `{shared_lib}` for Zig package"))?;
    fs::copy(&shared_src, lib_dir.join(&shared_lib)).context("copying FFI .so into Zig package")?;

    // Copy C header — required so downstream consumers can @cInclude it.
    let ffi_crate_dir = crate::ffi_stage::find_ffi_crate_dir_pub(config, workspace_root);
    let header_src = ffi_crate_dir.join("include").join(&header_name);
    if !header_src.exists() {
        anyhow::bail!(
            "FFI C header not found at {} — run `alef build --lang=ffi` first",
            header_src.display()
        );
    }
    fs::copy(&header_src, include_dir.join(&header_name)).context("copying FFI header into Zig package")?;

    // Create tarball.
    let archive_name = format!("{pkg_name}.tar.gz");
    let archive_path = output_dir.join(&archive_name);
    super::create_tar_gz(&staging, &archive_path)?;

    // Clean up staging.
    fs::remove_dir_all(&staging).ok();

    Ok(PackageArtifact {
        path: archive_path,
        name: archive_name,
        checksum: None,
    })
}

