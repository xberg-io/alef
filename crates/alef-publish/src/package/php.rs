//! PHP PIE package builder — creates a distributable archive with the compiled
//! PHP extension, composer.json, pie.json metadata, and installation instructions.

use super::PackageArtifact;
use crate::platform::RustTarget;
use alef_core::config::AlefConfig;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Package a PHP PIE archive.
///
/// Produces: `{name}-{version}-{platform}.tar.gz` containing:
/// - `ext/` — the compiled `.so`/`.dylib`
/// - `composer.json`
/// - `pie.json` — metadata manifest
/// - `README.md`, `LICENSE`
pub fn package_php(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let crate_name = &config.crate_config.name;
    let ext_name = config.php_extension_name();
    let platform = target.platform_for(alef_core::config::extras::Language::Php);
    let pkg_dir = config.package_dir(alef_core::config::extras::Language::Php);

    let pkg_name = format!("{crate_name}-{version}-{platform}");
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    let ext_dir = staging.join("ext");
    fs::create_dir_all(&ext_dir)?;

    // Find and copy the compiled extension.
    let lib_file = target.shared_lib_name(&ext_name);
    let lib_src = find_php_ext(workspace_root, target, &lib_file)?;
    fs::copy(&lib_src, ext_dir.join(&lib_file))?;

    // Copy composer.json from the PHP package dir.
    let composer_src = workspace_root.join(&pkg_dir).join("composer.json");
    if composer_src.exists() {
        fs::copy(&composer_src, staging.join("composer.json"))?;
    }

    // Copy README.md, LICENSE.
    for name in &["README.md", "LICENSE", "CHANGELOG.md"] {
        let src = workspace_root.join(&pkg_dir).join(name);
        if src.exists() {
            fs::copy(&src, staging.join(name))?;
        }
    }
    // Also try root LICENSE.
    if !staging.join("LICENSE").exists() {
        for name in &["LICENSE", "LICENSE-MIT"] {
            let src = workspace_root.join(name);
            if src.exists() {
                fs::copy(&src, staging.join("LICENSE"))?;
                break;
            }
        }
    }

    // Generate pie.json manifest.
    let (os, arch) = parse_platform(&platform);
    let pie_json = serde_json::json!({
        "name": crate_name,
        "version": version,
        "platform": platform,
        "os": os,
        "arch": arch,
        "php_version": ">=8.1",
        "extension_file": lib_file,
        "built_at": chrono_now_stub(),
    });
    fs::write(staging.join("pie.json"), serde_json::to_string_pretty(&pie_json)?)?;

    // Generate INSTALL.md.
    let install_md = format!(
        "# Installing {crate_name}\n\n\
         ## Using PIE\n\n\
         ```bash\npie install {crate_name}\n```\n\n\
         ## Manual installation\n\n\
         1. Copy `ext/{lib_file}` to your PHP extensions directory\n\
         2. Add `extension={lib_file}` to your `php.ini`\n\
         3. Restart PHP\n"
    );
    fs::write(staging.join("INSTALL.md"), install_md)?;

    // Create tarball.
    let archive_name = format!("{pkg_name}.tar.gz");
    let archive_path = output_dir.join(&archive_name);
    super::create_tar_gz(&staging, &archive_path)?;

    fs::remove_dir_all(&staging).ok();

    Ok(PackageArtifact {
        path: archive_path,
        name: archive_name,
        checksum: None,
    })
}

fn find_php_ext(workspace_root: &Path, target: &RustTarget, lib_file: &str) -> Result<PathBuf> {
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
    anyhow::bail!("PHP extension {lib_file} not found")
}

fn parse_platform(platform: &str) -> (&str, &str) {
    if let Some((os, arch)) = platform.split_once('-') {
        (os, arch)
    } else {
        (platform, "unknown")
    }
}

fn chrono_now_stub() -> String {
    // Simple UTC timestamp without pulling in chrono.
    let output = std::process::Command::new("date")
        .arg("-u")
        .arg("+%Y-%m-%dT%H:%M:%SZ")
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => "unknown".to_string(),
    }
}
