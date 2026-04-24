//! C FFI distribution packaging — shared lib + static lib + header + pkg-config + cmake.

use super::PackageArtifact;
use crate::platform::RustTarget;
use alef_core::config::AlefConfig;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Package C FFI artifacts into a distributable tarball.
///
/// Produces: `{name}-ffi-v{version}-{platform}.tar.gz` containing:
/// - `lib/` — shared and static libraries
/// - `include/` — C header
/// - `share/pkgconfig/` — .pc file (if `pkg_config` enabled)
/// - `lib/cmake/` — CMake find module (if `cmake_config` enabled)
/// - `LICENSE`
pub fn package_c_ffi(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let lib_name = config.ffi_lib_name();
    let header_name = config.ffi_header_name();
    let crate_name = &config.crate_config.name;
    let platform = target.platform_for(alef_core::config::extras::Language::Ffi);

    let pkg_name = format!("{crate_name}-ffi-v{version}-{platform}");
    let staging = output_dir.join(&pkg_name);

    // Clean and create staging dirs.
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

    // Copy static library (optional — might not exist).
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

    // Generate pkg-config .pc file.
    let pub_config = publish_lang_config(config);
    if pub_config.pkg_config.unwrap_or(true) {
        let pkgconfig_dir = staging.join("share/pkgconfig");
        fs::create_dir_all(&pkgconfig_dir)?;
        let pc_content = generate_pc_file(crate_name, version, &lib_name, &header_name);
        fs::write(pkgconfig_dir.join(format!("{crate_name}.pc")), pc_content)?;
    }

    // Generate CMake find module.
    if pub_config.cmake_config.unwrap_or(true) {
        let cmake_dir = staging.join("lib/cmake").join(crate_name);
        fs::create_dir_all(&cmake_dir)?;
        let cmake_content = generate_cmake_config(crate_name, &lib_name);
        fs::write(cmake_dir.join(format!("{crate_name}-config.cmake")), cmake_content)?;
        let version_content = generate_cmake_version(version);
        fs::write(
            cmake_dir.join(format!("{crate_name}-config-version.cmake")),
            version_content,
        )?;
    }

    // Copy LICENSE if present.
    for name in &["LICENSE", "LICENSE-MIT", "LICENSE-APACHE"] {
        let license = workspace_root.join(name);
        if license.exists() {
            fs::copy(&license, staging.join(name))?;
            break;
        }
    }

    // Create tarball.
    let ext = target.archive_ext();
    let archive_name = format!("{pkg_name}.{ext}");
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

fn publish_lang_config(config: &AlefConfig) -> alef_core::config::publish::PublishLanguageConfig {
    if let Some(publish) = &config.publish {
        if let Some(cfg) = publish.languages.get("c_ffi").or_else(|| publish.languages.get("ffi")) {
            return cfg.clone();
        }
    }
    alef_core::config::publish::PublishLanguageConfig::default()
}

fn generate_pc_file(name: &str, version: &str, lib_name: &str, _header: &str) -> String {
    format!(
        "prefix=${{pcfiledir}}/../..\n\
         libdir=${{prefix}}/lib\n\
         includedir=${{prefix}}/include\n\n\
         Name: {name}\n\
         Description: {name} C FFI library\n\
         Version: {version}\n\
         Libs: -L${{libdir}} -l{lib_name}\n\
         Cflags: -I${{includedir}}\n"
    )
}

fn generate_cmake_config(name: &str, lib_name: &str) -> String {
    format!(
        "# CMake find module for {name}\n\
         get_filename_component(_dir \"${{CMAKE_CURRENT_LIST_FILE}}\" PATH)\n\
         get_filename_component(_prefix \"${{_dir}}/../..\" ABSOLUTE)\n\n\
         set({name}_INCLUDE_DIR \"${{_prefix}}/include\")\n\
         set({name}_LIBRARY \"${{_prefix}}/lib/lib{lib_name}${{CMAKE_SHARED_LIBRARY_SUFFIX}}\")\n\n\
         if(EXISTS \"${{{name}_LIBRARY}}\")\n\
         \x20\x20set({name}_FOUND TRUE)\n\
         else()\n\
         \x20\x20set({name}_FOUND FALSE)\n\
         endif()\n"
    )
}

fn generate_cmake_version(version: &str) -> String {
    format!(
        "set(PACKAGE_VERSION \"{version}\")\n\n\
         if(PACKAGE_FIND_VERSION VERSION_EQUAL PACKAGE_VERSION)\n\
         \x20\x20set(PACKAGE_VERSION_EXACT TRUE)\n\
         endif()\n\n\
         if(NOT PACKAGE_FIND_VERSION VERSION_GREATER PACKAGE_VERSION)\n\
         \x20\x20set(PACKAGE_VERSION_COMPATIBLE TRUE)\n\
         else()\n\
         \x20\x20set(PACKAGE_VERSION_UNSUITABLE TRUE)\n\
         endif()\n"
    )
}
