//! Ruby native gem packager.
//!
//! Builds a pre-compiled platform gem from a vendored Ruby package directory.
//! Assumes `alef publish prepare` has already vendored core-only dependencies.
//!
//! Steps:
//! 1. Locate the compiled `.so`/`.bundle`/`.dll` native extension.
//! 2. Stage it under `lib/{gem}/{ruby_abi}/` in the gem directory.
//! 3. Generate a modified gemspec with platform set to the target.
//! 4. Run `gem build` to produce the `.gem` file.
//! 5. Move to `output_dir`.

use super::PackageArtifact;
use crate::platform::RustTarget;
use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Package a Ruby native gem for the given target.
///
/// Produces: `{gem_name}-{version}-{platform}.gem`
pub fn package_ruby(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let gem_name = config.ruby_gem_name();
    let platform = target.platform_for(alef_core::config::extras::Language::Ruby);
    let pkg_dir_str = config.package_dir(alef_core::config::extras::Language::Ruby);
    let pkg_dir = workspace_root.join(&pkg_dir_str);

    if !pkg_dir.exists() {
        anyhow::bail!("Ruby package directory does not exist: {}", pkg_dir.display());
    }

    // Find the compiled native extension.
    let rb_crate = crate::crate_name_from_output(config, alef_core::config::extras::Language::Ruby)
        .unwrap_or_else(|| format!("{}-rb", config.crate_config.name));
    let lib_filename = target.shared_lib_name(&rb_crate.replace('-', "_"));
    let native_lib = find_ruby_native_lib(workspace_root, target, &rb_crate, &lib_filename)?;

    // Determine abi directory name (e.g. "3.2.0", "3.1.0").
    // We use a fixed conventional path: lib/{gem_name}/ for the shared lib.
    let lib_dest_dir = pkg_dir.join("lib").join(&gem_name);
    fs::create_dir_all(&lib_dest_dir).with_context(|| format!("creating {}", lib_dest_dir.display()))?;
    let lib_dest = lib_dest_dir.join(&lib_filename);
    fs::copy(&native_lib, &lib_dest).with_context(|| format!("copying native lib to {}", lib_dest.display()))?;

    // Write a platform-specific gemspec.
    let gemspec_name = format!("{gem_name}-platform.gemspec");
    let gemspec_path = pkg_dir.join(&gemspec_name);
    let platform_gemspec = generate_platform_gemspec(&gem_name, version, &platform, &lib_filename)?;
    fs::write(&gemspec_path, platform_gemspec)?;

    // Run gem build.
    let build_cmd = format!("gem build {gemspec_name}");
    crate::run_shell_command_in(&build_cmd, &pkg_dir)?;

    // Find the produced .gem file.
    let gem_file = find_gem_file(&pkg_dir, &gem_name, version, &platform)
        .with_context(|| format!("gem build did not produce expected .gem in {}", pkg_dir.display()))?;

    let gem_filename = gem_file
        .file_name()
        .context("gem has no filename")?
        .to_string_lossy()
        .to_string();
    let dest = output_dir.join(&gem_filename);
    fs::copy(&gem_file, &dest)?;

    // Cleanup temporary platform gemspec.
    let _ = fs::remove_file(&gemspec_path);
    // Cleanup staged native lib copy.
    let _ = fs::remove_file(&lib_dest);

    Ok(PackageArtifact {
        path: dest,
        name: gem_filename,
        checksum: None,
    })
}

fn find_ruby_native_lib(
    workspace_root: &Path,
    target: &RustTarget,
    rb_crate: &str,
    lib_filename: &str,
) -> Result<PathBuf> {
    // Cross path.
    let cross = workspace_root
        .join("target")
        .join(&target.triple)
        .join("release")
        .join(lib_filename);
    if cross.exists() {
        return Ok(cross);
    }
    // Native path.
    let native = workspace_root.join("target/release").join(lib_filename);
    if native.exists() {
        return Ok(native);
    }
    // rb-sys may also produce it inside the gem crate dir.
    let in_crate = workspace_root
        .join("crates")
        .join(rb_crate)
        .join("target")
        .join("release")
        .join(lib_filename);
    if in_crate.exists() {
        return Ok(in_crate);
    }
    anyhow::bail!(
        "Ruby native lib '{lib_filename}' not found in target dirs for {}",
        target.triple
    )
}

fn generate_platform_gemspec(gem_name: &str, version: &str, platform: &str, lib_file: &str) -> Result<String> {
    // Generate a minimal gemspec that references the pre-compiled native library.
    let lib_path = format!("lib/{gem_name}/{lib_file}");
    Ok(format!(
        r#"# frozen_string_literal: true
Gem::Specification.new do |spec|
  spec.name          = {gem_name:?}
  spec.version       = {version:?}
  spec.platform      = {platform:?}
  spec.summary       = "{gem_name} native extension"
  spec.files         = [{lib_path:?}]
  spec.require_paths = ["lib"]
end
"#
    ))
}

fn find_gem_file(dir: &Path, gem_name: &str, version: &str, platform: &str) -> Result<PathBuf> {
    // gem build produces: {name}-{version}-{platform}.gem in cwd.
    let expected = dir.join(format!("{gem_name}-{version}-{platform}.gem"));
    if expected.exists() {
        return Ok(expected);
    }
    // Fallback: scan for any .gem matching the version.
    let candidates: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "gem")
                && p.file_name().is_some_and(|n| n.to_string_lossy().contains(version))
        })
        .collect();
    candidates
        .into_iter()
        .next()
        .with_context(|| format!("no .gem file for {gem_name}-{version} found in {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_platform_gemspec_valid_ruby() {
        let spec = generate_platform_gemspec("mylib", "1.0.0", "x86_64-linux", "libmylib_rb.so").unwrap();
        assert!(spec.contains("mylib"));
        assert!(spec.contains("1.0.0"));
        assert!(spec.contains("x86_64-linux"));
        assert!(spec.contains("libmylib_rb.so"));
    }

    #[test]
    fn find_gem_file_expected_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let gem_path = tmp.path().join("mygem-1.0.0-x86_64-linux.gem");
        std::fs::write(&gem_path, b"fake").unwrap();

        let result = find_gem_file(tmp.path(), "mygem", "1.0.0", "x86_64-linux").unwrap();
        assert_eq!(result, gem_path);
    }

    #[test]
    fn find_gem_file_missing_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = find_gem_file(tmp.path(), "mygem", "1.0.0", "x86_64-linux");
        assert!(result.is_err());
    }
}
