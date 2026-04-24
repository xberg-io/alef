//! FFI artifact staging — copies built shared libraries into language-specific
//! directories for Go, Java, and C# packages.
//!
//! After `cargo build --release -p {name}-ffi --target {triple}`, the shared
//! library lives in `target/{triple}/release/`. This module copies it to:
//! - Go: `packages/go/lib/` or a platform subdirectory
//! - Java: `packages/java/src/main/resources/natives/{rid}/`
//! - C#: `packages/csharp/{Project}/runtimes/{rid}/native/`

use crate::platform::RustTarget;
use alef_core::config::AlefConfig;
use alef_core::config::extras::Language;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

/// Stage the FFI shared library for a specific language and target.
pub fn stage_ffi(config: &AlefConfig, lang: Language, target: &RustTarget, workspace_root: &Path) -> Result<PathBuf> {
    let lib_name = config.ffi_lib_name();
    let shared_lib = target.shared_lib_name(&lib_name);

    // Locate the built library.
    let lib_path = find_built_library(workspace_root, target, &shared_lib)?;

    // Determine destination directory.
    let dest_dir = staging_dir(config, lang, target, workspace_root)?;
    fs::create_dir_all(&dest_dir).with_context(|| format!("creating {}", dest_dir.display()))?;

    let dest_path = dest_dir.join(&shared_lib);
    fs::copy(&lib_path, &dest_path)
        .with_context(|| format!("copying {} to {}", lib_path.display(), dest_path.display()))?;

    tracing::info!(
        lang = %lang,
        lib = %shared_lib,
        dest = %dest_dir.display(),
        "staged FFI library"
    );

    Ok(dest_path)
}

/// Optionally stage the C header alongside the shared library.
pub fn stage_header(
    config: &AlefConfig,
    lang: Language,
    target: &RustTarget,
    workspace_root: &Path,
) -> Result<Option<PathBuf>> {
    let header_name = config.ffi_header_name();
    let ffi_crate_dir = find_ffi_crate_dir(config, workspace_root);

    let header_src = ffi_crate_dir.join("include").join(&header_name);
    if !header_src.exists() {
        return Ok(None);
    }

    let dest_dir = staging_dir(config, lang, target, workspace_root)?;
    let include_dir = dest_dir.join("include");
    fs::create_dir_all(&include_dir)?;

    let dest_path = include_dir.join(&header_name);
    fs::copy(&header_src, &dest_path)?;

    Ok(Some(dest_path))
}

/// Find the built shared library in the target directory.
fn find_built_library(workspace_root: &Path, target: &RustTarget, shared_lib: &str) -> Result<PathBuf> {
    // Try target/{triple}/release/ first (cross-compilation).
    let cross_path = workspace_root
        .join("target")
        .join(&target.triple)
        .join("release")
        .join(shared_lib);
    if cross_path.exists() {
        return Ok(cross_path);
    }

    // Fall back to target/release/ (native build).
    let native_path = workspace_root.join("target").join("release").join(shared_lib);
    if native_path.exists() {
        return Ok(native_path);
    }

    bail!(
        "FFI library {shared_lib} not found at {} or {}",
        cross_path.display(),
        native_path.display()
    );
}

/// Determine the staging directory for a language + target combination.
fn staging_dir(config: &AlefConfig, lang: Language, target: &RustTarget, workspace_root: &Path) -> Result<PathBuf> {
    let pkg_dir = config.package_dir(lang);
    let platform = target.platform_for(lang);

    let rel = match lang {
        Language::Go => PathBuf::from(&pkg_dir).join("lib"),
        Language::Java => PathBuf::from(&pkg_dir)
            .join("src/main/resources/natives")
            .join(&platform),
        Language::Csharp => {
            let namespace = config.csharp_namespace();
            PathBuf::from(&pkg_dir)
                .join(&namespace)
                .join("runtimes")
                .join(&platform)
                .join("native")
        }
        other => bail!("FFI staging not supported for {other}"),
    };

    Ok(workspace_root.join(rel))
}

/// Find the FFI crate directory (for locating the header file). Public alias for use by packagers.
pub fn find_ffi_crate_dir_pub(config: &AlefConfig, workspace_root: &Path) -> PathBuf {
    find_ffi_crate_dir(config, workspace_root)
}

/// Find the FFI crate directory (for locating the header file).
fn find_ffi_crate_dir(config: &AlefConfig, workspace_root: &Path) -> PathBuf {
    if let Some(ffi_output) = config.output.ffi.as_ref() {
        // ffi output is like "crates/my-lib-ffi/src/" — walk up to find the crate dir.
        let p = Path::new(ffi_output);
        for ancestor in p.ancestors() {
            if ancestor.join("Cargo.toml").exists() || ancestor.join("include").exists() {
                return workspace_root.join(ancestor);
            }
        }
        // Fall back to parent of "src" component.
        if let Some(parent) = p.parent() {
            return workspace_root.join(parent);
        }
    }

    // Default: crates/{name}-ffi
    let crate_name = &config.crate_config.name;
    workspace_root.join(format!("crates/{crate_name}-ffi"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn minimal_config() -> AlefConfig {
        toml::from_str(
            r#"
languages = ["go", "java", "csharp"]

[crate]
name = "my-lib"
sources = ["crates/my-lib/src/lib.rs"]

[ffi]
prefix = "mylib"
lib_name = "my_lib_ffi"
header_name = "my_lib.h"

[csharp]
namespace = "MyLib"
"#,
        )
        .unwrap()
    }

    fn setup_built_ffi(root: &Path, target_triple: &str) {
        let target = RustTarget::parse(target_triple).unwrap();
        let lib_name = target.shared_lib_name("my_lib_ffi");
        let release_dir = root.join("target").join(target_triple).join("release");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(release_dir.join(lib_name), "fake-lib").unwrap();
    }

    fn setup_header(root: &Path) {
        let include_dir = root.join("crates/my-lib-ffi/include");
        fs::create_dir_all(&include_dir).unwrap();
        fs::write(include_dir.join("my_lib.h"), "#pragma once").unwrap();
    }

    #[test]
    fn stage_ffi_go() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi(root, "x86_64-unknown-linux-gnu");
        fs::create_dir_all(root.join("packages/go")).unwrap();

        let result = stage_ffi(&config, Language::Go, &target, root).unwrap();
        assert!(result.exists());
        assert!(result.to_string_lossy().contains("packages/go/lib"));
    }

    #[test]
    fn stage_ffi_java() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi(root, "x86_64-unknown-linux-gnu");
        fs::create_dir_all(root.join("packages/java")).unwrap();

        let result = stage_ffi(&config, Language::Java, &target, root).unwrap();
        assert!(result.exists());
        assert!(result.to_string_lossy().contains("natives/linux-x86_64"));
    }

    #[test]
    fn stage_ffi_csharp() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("aarch64-apple-darwin").unwrap();

        setup_built_ffi(root, "aarch64-apple-darwin");
        fs::create_dir_all(root.join("packages/csharp")).unwrap();

        let result = stage_ffi(&config, Language::Csharp, &target, root).unwrap();
        assert!(result.exists());
        assert!(result.to_string_lossy().contains("runtimes/osx-arm64/native"));
    }

    #[test]
    fn stage_ffi_not_found() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        let result = stage_ffi(&config, Language::Go, &target, root);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn stage_header_present() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi(root, "x86_64-unknown-linux-gnu");
        setup_header(root);
        fs::create_dir_all(root.join("packages/go")).unwrap();

        // Stage the lib first (creates the dir).
        stage_ffi(&config, Language::Go, &target, root).unwrap();

        let result = stage_header(&config, Language::Go, &target, root).unwrap();
        assert!(result.is_some());
        assert!(result.unwrap().exists());
    }

    #[test]
    fn stage_header_missing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        setup_built_ffi(root, "x86_64-unknown-linux-gnu");
        fs::create_dir_all(root.join("packages/go")).unwrap();
        stage_ffi(&config, Language::Go, &target, root).unwrap();

        let result = stage_header(&config, Language::Go, &target, root).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn stage_ffi_native_build_fallback() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let config = minimal_config();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let lib_name = target.shared_lib_name("my_lib_ffi");

        // Place lib in target/release/ instead of target/{triple}/release/.
        let release_dir = root.join("target/release");
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(release_dir.join(&lib_name), "fake-lib").unwrap();
        fs::create_dir_all(root.join("packages/go")).unwrap();

        let result = stage_ffi(&config, Language::Go, &target, root).unwrap();
        assert!(result.exists());
    }
}
