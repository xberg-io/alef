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
use crate::core::config::ResolvedCrateConfig;
use crate::publish::platform::RustTarget;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Package a Ruby native gem for the given target.
///
/// Produces: `{gem_name}-{version}-{platform}.gem`
pub fn package_ruby(
    config: &ResolvedCrateConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let gem_name = config.ruby_gem_name();
    let platform = target.platform_for(crate::core::config::extras::Language::Ruby);
    let pkg_dir_str = config.package_dir(crate::core::config::extras::Language::Ruby);
    let pkg_dir = workspace_root.join(&pkg_dir_str);

    if !pkg_dir.exists() {
        anyhow::bail!("Ruby package directory does not exist: {}", pkg_dir.display());
    }

    // Find the compiled native extension.
    let rb_crate = crate::publish::crate_name_from_output(config, crate::core::config::extras::Language::Ruby)
        .unwrap_or_else(|| format!("{}-rb", config.name));
    let lib_filename = target.shared_lib_name(&rb_crate.replace('-', "_"));
    let native_lib = find_ruby_native_lib(workspace_root, target, &rb_crate, &lib_filename)?;

    // Determine abi directory name (e.g. "3.2.0", "3.1.0").
    // We use a fixed conventional path: lib/{gem_name}/ for the shared lib.
    let lib_dest_dir = pkg_dir.join("lib").join(&gem_name);
    fs::create_dir_all(&lib_dest_dir).with_context(|| format!("creating {}", lib_dest_dir.display()))?;
    let lib_dest = lib_dest_dir.join(&lib_filename);
    fs::copy(&native_lib, &lib_dest).with_context(|| format!("copying native lib to {}", lib_dest.display()))?;

    // Collect all .rb wrapper files already present in lib/ so they are included
    // alongside the native shared object.  Without them `gem push` rejects the gem
    // with "invalid gem structure" because the require paths cannot be satisfied.
    let mut rb_files: Vec<String> = scan_rb_files(&pkg_dir.join("lib"))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| p.strip_prefix(&pkg_dir).ok().map(|r| r.to_string_lossy().into_owned()))
        .collect();
    rb_files.sort();
    // Always include the native lib path even if it was just staged (scan_rb_files
    // skips .so/.bundle files by design).
    let native_lib_path = format!("lib/{gem_name}/{lib_filename}");
    if !rb_files.contains(&native_lib_path) {
        rb_files.push(native_lib_path);
    }

    // Propagate `required_ruby_version` from the source gemspec so platform
    // gems refuse to install on incompatible Ruby ABIs.
    let required_ruby_version = read_required_ruby_version(&pkg_dir);

    // Write a platform-specific gemspec.
    let gemspec_name = format!("{gem_name}-platform.gemspec");
    let gemspec_path = pkg_dir.join(&gemspec_name);
    let platform_gemspec = generate_platform_gemspec(
        &gem_name,
        version,
        &platform,
        &rb_files,
        required_ruby_version.as_deref(),
    )?;
    fs::write(&gemspec_path, platform_gemspec)?;

    // Run gem build.
    let build_cmd = format!("gem build {gemspec_name}");
    crate::publish::run_shell_command_in(&build_cmd, &pkg_dir)?;

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

fn scan_rb_files(lib_dir: &Path) -> Result<Vec<PathBuf>> {
    // Walk lib_dir and collect all .rb files (not native libs).
    let mut found = Vec::new();
    if !lib_dir.exists() {
        return Ok(found);
    }
    for entry in fs::read_dir(lib_dir).with_context(|| format!("reading {}", lib_dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // One level of recursion is sufficient for typical gem layouts:
            // lib/{gem}.rb, lib/{gem}/version.rb, lib/{gem}/native.rb, etc.
            for sub in fs::read_dir(&path).with_context(|| format!("reading {}", path.display()))? {
                let sub = sub?;
                let sub_path = sub.path();
                if sub_path.extension().is_some_and(|e| e == "rb") {
                    found.push(sub_path);
                }
            }
        } else if path.extension().is_some_and(|e| e == "rb") {
            found.push(path);
        }
    }
    Ok(found)
}

fn generate_platform_gemspec(
    gem_name: &str,
    version: &str,
    platform: &str,
    files: &[String],
    required_ruby_version: Option<&str>,
) -> Result<String> {
    // Generate a minimal gemspec that references the pre-compiled native library
    // AND all Ruby wrapper files required to satisfy the gem's require paths.
    let files_ruby = files
        .iter()
        .map(|f| format!("    {f:?}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let required_ruby_line = required_ruby_version
        .map(|v| format!("  spec.required_ruby_version = {v}\n"))
        .unwrap_or_default();
    Ok(format!(
        r#"# frozen_string_literal: true
Gem::Specification.new do |spec|
  spec.name          = {gem_name:?}
  spec.version       = {version:?}
  spec.platform      = {platform:?}
{required_ruby_line}  spec.summary       = "{gem_name} native extension"
  spec.files         = [
{files_ruby}
  ]
  spec.require_paths = ["lib"]
end
"#
    ))
}

/// Scan `pkg_dir` for the source `.gemspec` and extract the raw right-hand-side
/// expression assigned to `required_ruby_version`.
///
/// Captures either form RubyGems accepts:
///   - single string:  `spec.required_ruby_version = ">= 3.2.0"`
///   - array literal:  `spec.required_ruby_version = [">= 3.2.0", "< 4.0"]`
///
/// The returned value is the verbatim RHS (including surrounding quotes or
/// brackets) so the platform gemspec emitter can re-emit it unchanged.
///
/// Returns `None` when no source gemspec exists, no field is set, or the
/// regex fails — callers should treat absence as "no constraint".
fn read_required_ruby_version(pkg_dir: &Path) -> Option<String> {
    let entries = fs::read_dir(pkg_dir).ok()?;
    let re = regex::Regex::new(r#"(?m)^\s*\w+\.required_ruby_version\s*=\s*(\[[^\]]+\]|['"][^'"]+['"])"#).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "gemspec") {
            continue;
        }
        // Skip the platform-specific gemspec we ourselves emit.
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("-platform.gemspec"))
        {
            continue;
        }
        let content = fs::read_to_string(&path).ok()?;
        if let Some(caps) = re.captures(&content) {
            return Some(caps[1].to_string());
        }
    }
    None
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
    fn generate_platform_gemspec_includes_native_and_wrapper_files() {
        let files = vec![
            "lib/mylib.rb".to_string(),
            "lib/mylib/version.rb".to_string(),
            "lib/mylib/native.rb".to_string(),
            "lib/mylib/libmylib_rb.so".to_string(),
        ];
        let spec = generate_platform_gemspec("mylib", "1.0.0", "x86_64-linux", &files, None).unwrap();
        assert!(spec.contains("mylib"), "gem name present");
        assert!(spec.contains("1.0.0"), "version present");
        assert!(spec.contains("x86_64-linux"), "platform present");
        assert!(spec.contains("libmylib_rb.so"), "native lib present");
        assert!(spec.contains("lib/mylib.rb"), "top-level wrapper present");
        assert!(spec.contains("lib/mylib/version.rb"), "version wrapper present");
        assert!(spec.contains("lib/mylib/native.rb"), "native wrapper present");
        assert!(
            !spec.contains("required_ruby_version"),
            "no required_ruby_version emitted when None",
        );
    }

    #[test]
    fn generate_platform_gemspec_includes_required_ruby_version_when_some() {
        let files = vec!["lib/mylib.rb".to_string()];
        let spec = generate_platform_gemspec("mylib", "1.0.0", "x86_64-linux", &files, Some(r#"">= 3.2.0""#)).unwrap();
        assert!(
            spec.contains(r#"spec.required_ruby_version = ">= 3.2.0""#),
            "required_ruby_version line present: {spec}",
        );
    }

    #[test]
    fn generate_platform_gemspec_emits_array_form_verbatim() {
        let files = vec!["lib/mylib.rb".to_string()];
        let spec = generate_platform_gemspec(
            "mylib",
            "1.0.0",
            "x86_64-linux",
            &files,
            Some(r#"[">= 3.2.0", "< 4.0"]"#),
        )
        .unwrap();
        assert!(
            spec.contains(r#"spec.required_ruby_version = [">= 3.2.0", "< 4.0"]"#),
            "array-form required_ruby_version preserved verbatim: {spec}",
        );
    }

    #[test]
    fn read_required_ruby_version_extracts_from_source_gemspec() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Decoy: platform gemspec must be skipped.
        std::fs::write(
            tmp.path().join("mylib-platform.gemspec"),
            r#"spec.required_ruby_version = ">= 99.0""#,
        )
        .unwrap();
        // Real source gemspec.
        std::fs::write(
            tmp.path().join("mylib.gemspec"),
            "# frozen_string_literal: true\nGem::Specification.new do |spec|\n  spec.required_ruby_version = \">= 3.2.0\"\nend\n",
        )
        .unwrap();
        assert_eq!(
            read_required_ruby_version(tmp.path()),
            Some(r#"">= 3.2.0""#.to_string())
        );
    }

    #[test]
    fn read_required_ruby_version_extracts_array_form() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("mylib.gemspec"),
            "Gem::Specification.new do |spec|\n  spec.required_ruby_version = [\">= 3.2.0\", \"< 4.0\"]\nend\n",
        )
        .unwrap();
        assert_eq!(
            read_required_ruby_version(tmp.path()),
            Some(r#"[">= 3.2.0", "< 4.0"]"#.to_string())
        );
    }

    #[test]
    fn read_required_ruby_version_returns_none_when_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("mylib.gemspec"),
            "Gem::Specification.new do |spec|\n  spec.name = \"mylib\"\nend\n",
        )
        .unwrap();
        assert_eq!(read_required_ruby_version(tmp.path()), None);
    }

    #[test]
    fn scan_rb_files_finds_wrappers_and_skips_non_rb() {
        let tmp = tempfile::TempDir::new().unwrap();
        let lib_dir = tmp.path().join("lib");
        let sub_dir = lib_dir.join("mylib");
        std::fs::create_dir_all(&sub_dir).unwrap();
        // Top-level wrapper.
        std::fs::write(lib_dir.join("mylib.rb"), b"").unwrap();
        // Sub-level wrappers.
        std::fs::write(sub_dir.join("version.rb"), b"").unwrap();
        std::fs::write(sub_dir.join("native.rb"), b"").unwrap();
        // Native lib — should NOT appear in scan results.
        std::fs::write(sub_dir.join("libmylib_rb.so"), b"").unwrap();

        let mut found = scan_rb_files(&lib_dir).unwrap();
        found.sort();
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"mylib.rb".to_string()), "top-level wrapper found");
        assert!(names.contains(&"version.rb".to_string()), "version.rb found");
        assert!(names.contains(&"native.rb".to_string()), "native.rb found");
        assert!(!names.contains(&"libmylib_rb.so".to_string()), ".so excluded from scan");
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
