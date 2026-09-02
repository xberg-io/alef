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
use crate::publish::package::BuildProfile;
use crate::publish::platform::RustTarget;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

    let rb_crate = crate::publish::crate_name_from_output(config, crate::core::config::extras::Language::Ruby)
        .unwrap_or_else(|| format!("{}-rb", config.name));
    let lib_filename = target.shared_lib_name(&rb_crate.replace('-', "_"));
    let native_lib = find_ruby_native_lib(workspace_root, target, &rb_crate, &lib_filename)?;

    let ruby_abi = ruby_abi_for_packaging()?;
    let ext_name = rb_crate.replace('-', "_");

    let lib_dest_dir = pkg_dir.join("lib").join(&ext_name).join(&ruby_abi);
    fs::create_dir_all(&lib_dest_dir).with_context(|| format!("creating {}", lib_dest_dir.display()))?;
    let lib_dest = lib_dest_dir.join(&lib_filename);
    fs::copy(&native_lib, &lib_dest).with_context(|| format!("copying native lib to {}", lib_dest.display()))?;

    let lib_dir = pkg_dir.join("lib");
    let mut rb_files: Vec<String> = scan_rb_files(&lib_dir)
        .with_context(|| format!("scanning Ruby wrapper sources under {}", lib_dir.display()))?
        .into_iter()
        .filter_map(|p| p.strip_prefix(&pkg_dir).ok().map(|r| r.to_string_lossy().into_owned()))
        .collect();
    rb_files.sort();
    let native_lib_path = format!("lib/{ext_name}/{ruby_abi}/{lib_filename}");
    if !rb_files.contains(&native_lib_path) {
        rb_files.push(native_lib_path);
    }

    let source_gemspec_name = format!("{gem_name}.gemspec");
    anyhow::ensure!(
        pkg_dir.join(&source_gemspec_name).is_file(),
        "ruby: missing source gemspec {}",
        pkg_dir.join(&source_gemspec_name).display()
    );
    let gemspec_name = format!("{gem_name}-platform.gemspec");
    let gemspec_path = pkg_dir.join(&gemspec_name);
    let platform_gemspec = generate_platform_gemspec(&source_gemspec_name, version, &platform, &rb_files)?;
    fs::write(&gemspec_path, platform_gemspec)?;

    run_gem_build(&pkg_dir, &gemspec_name)?;

    let gem_file = find_gem_file(&pkg_dir, &gem_name, version, &platform)
        .with_context(|| format!("gem build did not produce expected .gem in {}", pkg_dir.display()))?;

    let gem_filename = gem_file
        .file_name()
        .context("gem has no filename")?
        .to_string_lossy()
        .to_string();
    let dest = output_dir.join(&gem_filename);
    fs::copy(&gem_file, &dest)?;

    let _ = fs::remove_file(&gemspec_path);
    let _ = fs::remove_file(&lib_dest);

    Ok(PackageArtifact {
        path: dest,
        name: gem_filename,
        checksum: None,
    })
}

fn gem_build_command(gemspec_name: &str) -> Command {
    let mut command = Command::new("ruby");
    command.args(["-S", "gem", "build", gemspec_name]);
    command
}

fn run_gem_build(pkg_dir: &Path, gemspec_name: &str) -> Result<()> {
    let output = gem_build_command(gemspec_name)
        .current_dir(pkg_dir)
        .output()
        .context("failed to execute `ruby -S gem build`")?;
    anyhow::ensure!(
        output.status.success(),
        "`ruby -S gem build {gemspec_name}` failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

/// Locate the compiled Ruby native extension. Delegates to
/// [`crate::publish::package::find_built_artifact_with_extra_dirs`] for the two canonical
/// `target/{triple}/release/` / `target/release/` locations, plus one Ruby-specific extra: a
/// `rb_sys`/rake-driven build sometimes runs `cargo build` from inside the extension crate's own
/// directory rather than the workspace root, leaving its output under
/// `crates/{rb_crate}/target/release/` instead of being uplifted to the workspace `target/`.
fn find_ruby_native_lib(
    workspace_root: &Path,
    target: &RustTarget,
    rb_crate: &str,
    lib_filename: &str,
) -> Result<PathBuf> {
    let in_crate_dir = workspace_root
        .join("crates")
        .join(rb_crate)
        .join("target")
        .join("release");
    crate::publish::package::find_built_artifact_with_extra_dirs(
        workspace_root,
        target,
        lib_filename,
        BuildProfile::Release,
        std::slice::from_ref(&in_crate_dir),
    )
}

fn scan_rb_files(lib_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    if !lib_dir.exists() {
        return Ok(found);
    }
    for entry in fs::read_dir(lib_dir).with_context(|| format!("reading {}", lib_dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
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
    source_gemspec_name: &str,
    version: &str,
    platform: &str,
    files: &[String],
) -> Result<String> {
    let files_ruby = files
        .iter()
        .map(|f| format!("    {f:?}"))
        .collect::<Vec<_>>()
        .join(",\n");
    Ok(format!(
        r#"# frozen_string_literal: true
source_gemspec = File.expand_path({source_gemspec_name:?}, __dir__)
spec = Gem::Specification.load(source_gemspec)
raise "could not load source gemspec: #{{source_gemspec}}" unless spec

spec.version = {version:?}
spec.platform = {platform:?}
spec.files = (spec.files + [
{files_ruby}
  ]).uniq.sort
spec.extensions = []
spec
"#
    ))
}

/// Locate the `.gem` `gem build` just produced.
///
/// The exact `{gem_name}-{version}-{platform}.gem` wins; the fallback exists only because
/// RubyGems normalizes some platform strings (`arm64-darwin` -> `arm64-darwin-23`), so it is
/// anchored on the `{gem_name}-{version}-` prefix rather than a bare "name contains the
/// version" test, and it refuses to choose between several matches — a stale gem from an
/// earlier run would otherwise be published under this run's name. ~keep
fn find_gem_file(dir: &Path, gem_name: &str, version: &str, platform: &str) -> Result<PathBuf> {
    let expected = dir.join(format!("{gem_name}-{version}-{platform}.gem"));
    if expected.exists() {
        return Ok(expected);
    }
    let prefix = format!("{gem_name}-{version}-");
    let mut candidates: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "gem"))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        })
        .collect();
    candidates.sort();

    match candidates.as_slice() {
        [] => anyhow::bail!("no .gem file for {gem_name}-{version} found in {}", dir.display()),
        [only] => Ok(only.clone()),
        many => anyhow::bail!(
            "ambiguous .gem for {gem_name}-{version} in {}: expected {}, found {}: {}",
            dir.display(),
            expected.display(),
            many.len(),
            many.iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn ruby_abi_for_packaging() -> Result<String> {
    if let Some(abi) = ruby_abi_override(std::env::var("RUBY_ABI").ok()) {
        return Ok(abi);
    }

    let output = Command::new("ruby")
        .arg("-rrbconfig")
        .arg("-e")
        .arg("print RbConfig::CONFIG.fetch(\"ruby_version\")")
        .output()
        .context("failed to execute `ruby` to read RbConfig['ruby_version']")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "`ruby -rrbconfig -e 'print RbConfig::CONFIG.fetch(\"ruby_version\")' failed with {}: {}",
            output.status,
            stderr.trim()
        );
    }

    let abi = String::from_utf8(output.stdout)
        .context("ruby output for RbConfig['ruby_version'] was not valid UTF-8")?
        .trim()
        .to_string();

    if abi.is_empty() {
        anyhow::bail!("ruby ABI is empty from `RbConfig['ruby_version']`")
    }

    Ok(abi)
}

/// Normalize a `RUBY_ABI` override value: trim surrounding whitespace (common in CI env
/// injection) and treat an unset or blank value as "no override".
fn ruby_abi_override(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn write_executable(path: &Path, content: &str) {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::write(path, content).expect("write executable");
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod executable");
    }

    #[cfg(unix)]
    #[test]
    fn gem_build_uses_active_ruby_despite_foreign_gem_shebang() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ruby = temp.path().join("ruby");
        let gem = temp.path().join("gem");
        let marker = temp.path().join("marker");
        write_executable(
            &ruby,
            "#!/bin/sh\n[ \"$1\" = -S ] || exit 91\nshift\nscript=$(command -v \"$1\") || exit 92\nshift\nexec /bin/sh \"$script\" \"$@\"\n",
        );
        write_executable(
            &gem,
            "#!/missing/foreign/ruby\nprintf '%s\\n' \"$*\" > \"$ABI_PROBE\"\n",
        );
        let path = format!(
            "{}:{}",
            temp.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let direct = Command::new(&gem).arg("build").arg("package.gemspec").status();
        assert!(direct.is_err() || direct.is_ok_and(|status| !status.success()));
        let status = gem_build_command("package.gemspec")
            .env("PATH", path)
            .env("ABI_PROBE", &marker)
            .status()
            .expect("run gem build command");
        assert!(status.success());
        assert_eq!(
            std::fs::read_to_string(marker).expect("read marker"),
            "build package.gemspec\n"
        );
    }

    #[test]
    fn ruby_abi_override_trims_and_rejects_blank() {
        assert_eq!(ruby_abi_override(Some("3.4.0".to_string())), Some("3.4.0".to_string()));
        assert_eq!(
            ruby_abi_override(Some("  3.4.0 \n".to_string())),
            Some("3.4.0".to_string())
        );
        assert_eq!(ruby_abi_override(Some("   ".to_string())), None);
        assert_eq!(ruby_abi_override(Some(String::new())), None);
        assert_eq!(ruby_abi_override(None), None);
    }

    #[test]
    fn generate_platform_gemspec_reuses_source_metadata_and_replaces_build_fields() {
        let files = vec![
            "lib/mylib.rb".to_string(),
            "lib/mylib/version.rb".to_string(),
            "lib/mylib/native.rb".to_string(),
            "lib/mylib/libmylib_rb.so".to_string(),
        ];
        let spec = generate_platform_gemspec("mylib.gemspec", "1.0.0", "x86_64-linux", &files).unwrap();
        assert!(
            spec.contains(r#"Gem::Specification.load(source_gemspec)"#),
            "source gemspec supplies authors, dependencies, licenses, and package metadata: {spec}"
        );
        assert!(spec.contains(r#"File.expand_path("mylib.gemspec", __dir__)"#));
        assert!(spec.contains("1.0.0"), "version present");
        assert!(spec.contains("x86_64-linux"), "platform present");
        assert!(spec.contains("libmylib_rb.so"), "native lib present");
        assert!(spec.contains("lib/mylib.rb"), "top-level wrapper present");
        assert!(spec.contains("lib/mylib/version.rb"), "version wrapper present");
        assert!(spec.contains("lib/mylib/native.rb"), "native wrapper present");
        assert!(
            spec.contains("spec.files + ["),
            "all source package files remain in the platform gem: {spec}"
        );
        assert!(
            spec.contains("spec.extensions = []"),
            "precompiled gems must not invoke the source extension build: {spec}"
        );
        assert!(
            !spec.contains("spec.authors =") && !spec.contains("spec.add_dependency"),
            "metadata must have one implementation in the source gemspec: {spec}"
        );
    }

    /// Runs through RubyGems itself because a syntactically plausible generated gemspec can
    /// still build an invalid platform gem, as happened when Alef omitted required authors.
    #[test]
    #[ignore = "requires Ruby and RubyGems"]
    fn generated_platform_gemspec_builds_with_source_metadata_and_native_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let lib_dir = tmp.path().join("lib/mylib/3.2.0");
        let ext_dir = tmp.path().join("ext/mylib");
        std::fs::create_dir_all(&lib_dir).expect("create lib directory");
        std::fs::create_dir_all(&ext_dir).expect("create extension directory");
        std::fs::write(tmp.path().join("README.md"), "# mylib\n").expect("write README");
        std::fs::write(tmp.path().join("lib/mylib.rb"), "# frozen_string_literal: true\n").expect("write wrapper");
        std::fs::write(lib_dir.join("libmylib_rb.so"), "native-placeholder").expect("write native placeholder");
        std::fs::write(ext_dir.join("extconf.rb"), "# source build placeholder\n")
            .expect("write extension placeholder");
        std::fs::write(
            tmp.path().join("mylib.gemspec"),
            r#"Gem::Specification.new do |spec|
  spec.name = "mylib"
  spec.version = "0.1.0"
  spec.authors = ["Alef Test"]
  spec.summary = "metadata regression test"
  spec.license = "MIT"
  spec.required_ruby_version = ">= 3.2"
  spec.files = ["README.md", "lib/mylib.rb"]
  spec.require_paths = ["lib"]
  spec.extensions = ["ext/mylib/extconf.rb"]
  spec.add_dependency "rake", ">= 13"
end
"#,
        )
        .expect("write source gemspec");

        let files = vec!["lib/mylib.rb".to_string(), "lib/mylib/3.2.0/libmylib_rb.so".to_string()];
        let platform_gemspec = generate_platform_gemspec("mylib.gemspec", "1.2.3", "x86_64-linux", &files)
            .expect("generate platform gemspec");
        std::fs::write(tmp.path().join("mylib-platform.gemspec"), platform_gemspec).expect("write platform gemspec");

        run_gem_build(tmp.path(), "mylib-platform.gemspec").expect("build platform gem");
        let gem = tmp.path().join("mylib-1.2.3-x86_64-linux.gem");
        assert!(gem.is_file(), "expected {}", gem.display());

        let assertion = r#"
require "rubygems/package"
spec = Gem::Package.new(ARGV.fetch(0)).spec
abort "authors" unless spec.authors == ["Alef Test"]
abort "license" unless spec.licenses == ["MIT"]
abort "dependency" unless spec.runtime_dependencies.any? { |dep| dep.name == "rake" }
abort "ruby version" unless spec.required_ruby_version.satisfied_by?(Gem::Version.new("3.2"))
abort "extensions" unless spec.extensions.empty?
abort "README" unless spec.files.include?("README.md")
abort "native" unless spec.files.include?("lib/mylib/3.2.0/libmylib_rb.so")
"#;
        let status = Command::new("ruby")
            .args(["-e", assertion, gem.to_str().expect("UTF-8 gem path")])
            .status()
            .expect("inspect built gem");
        assert!(status.success(), "built gem did not preserve source metadata");
    }

    #[test]
    fn scan_rb_files_finds_wrappers_and_skips_non_rb() {
        let tmp = tempfile::TempDir::new().unwrap();
        let lib_dir = tmp.path().join("lib");
        let sub_dir = lib_dir.join("mylib");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::write(lib_dir.join("mylib.rb"), b"").unwrap();
        std::fs::write(sub_dir.join("version.rb"), b"").unwrap();
        std::fs::write(sub_dir.join("native.rb"), b"").unwrap();
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

    /// `scan_rb_files` failing means the wrapper sources could not be enumerated -- the caller
    /// propagates it rather than emitting a gemspec whose `files` list is silently empty.
    #[test]
    fn scan_rb_files_errors_when_lib_dir_is_not_a_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let lib = tmp.path().join("lib");
        std::fs::write(&lib, b"not a directory").unwrap();

        assert!(
            scan_rb_files(&lib).is_err(),
            "a non-directory lib path must be an error"
        );
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

    /// The fallback exists for RubyGems' platform-string normalization only; an unrelated gem
    /// that merely contains the version string must never be published under this gem's name.
    #[test]
    fn find_gem_file_ignores_unrelated_gem_with_matching_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("othergem-1.0.0-x86_64-linux.gem"), b"fake").unwrap();

        let error = find_gem_file(tmp.path(), "mygem", "1.0.0", "x86_64-linux")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no .gem file for mygem-1.0.0"),
            "unexpected error: {error}"
        );
    }

    /// The normalization the fallback exists for: RubyGems rewrites `arm64-darwin` to
    /// `arm64-darwin-23`, and a single such match is still unambiguous.
    #[test]
    fn find_gem_file_accepts_single_platform_normalized_variant() {
        let tmp = tempfile::TempDir::new().unwrap();
        let normalized = tmp.path().join("mygem-1.0.0-arm64-darwin-23.gem");
        std::fs::write(&normalized, b"fake").unwrap();

        let found = find_gem_file(tmp.path(), "mygem", "1.0.0", "arm64-darwin").unwrap();
        assert_eq!(found, normalized);
    }

    /// Several same-version gems (a stale one from an earlier run) must error naming all of
    /// them, not resolve by directory-iteration order.
    #[test]
    fn find_gem_file_errors_when_several_same_version_gems_exist() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("mygem-1.0.0-arm64-darwin-23.gem"), b"fake").unwrap();
        std::fs::write(tmp.path().join("mygem-1.0.0-x86_64-linux.gem"), b"fake").unwrap();

        let error = find_gem_file(tmp.path(), "mygem", "1.0.0", "arm64-darwin")
            .unwrap_err()
            .to_string();
        assert!(error.contains("ambiguous .gem"), "unexpected error: {error}");
        assert!(
            error.contains("arm64-darwin-23.gem"),
            "must name every candidate: {error}"
        );
        assert!(error.contains("x86_64-linux.gem"), "must name every candidate: {error}");
    }

    /// `find_ruby_native_lib` now delegates to
    /// `crate::publish::package::find_built_artifact_with_extra_dirs` for the two canonical
    /// locations -- this proves that delegation actually finds a normal workspace-uplifted
    /// artifact, the same as before the rewrite.
    #[test]
    fn find_ruby_native_lib_finds_canonical_workspace_uplift() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let release_dir = tmp.path().join("target").join(&target.triple).join("release");
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::write(release_dir.join("libmylib_rb.so"), b"canonical").unwrap();

        let found = find_ruby_native_lib(tmp.path(), &target, "mylib-rb", "libmylib_rb.so").unwrap();
        assert_eq!(found, release_dir.join("libmylib_rb.so"));
    }

    /// The Ruby-specific extra fallback this rewrite preserves: `rb_sys`/rake-driven builds can
    /// run `cargo build` from inside the extension crate's own directory, leaving output under
    /// `crates/{rb_crate}/target/release/` rather than uplifted to the workspace `target/`. This
    /// is the exact case `find_built_artifact` alone (no `extra_dirs`) would miss.
    #[test]
    fn find_ruby_native_lib_falls_back_to_in_crate_build_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();
        let in_crate_dir = tmp.path().join("crates/mylib-rb/target/release");
        std::fs::create_dir_all(&in_crate_dir).unwrap();
        std::fs::write(in_crate_dir.join("libmylib_rb.so"), b"in-crate").unwrap();

        let found = find_ruby_native_lib(tmp.path(), &target, "mylib-rb", "libmylib_rb.so").unwrap();
        assert_eq!(
            found,
            in_crate_dir.join("libmylib_rb.so"),
            "expected the in-crate fallback at {}, got {}",
            in_crate_dir.join("libmylib_rb.so").display(),
            found.display()
        );
    }

    /// The canonical workspace-uplifted location must still win over the in-crate fallback when
    /// both exist -- the fallback is for tools that skip the uplift entirely, never a preference
    /// over cargo's own uplifted output.
    #[test]
    fn find_ruby_native_lib_prefers_canonical_uplift_over_in_crate_fallback() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = RustTarget::parse("x86_64-unknown-linux-gnu").unwrap();

        let release_dir = tmp.path().join("target").join(&target.triple).join("release");
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::write(release_dir.join("libmylib_rb.so"), b"canonical").unwrap();

        let in_crate_dir = tmp.path().join("crates/mylib-rb/target/release");
        std::fs::create_dir_all(&in_crate_dir).unwrap();
        std::fs::write(in_crate_dir.join("libmylib_rb.so"), b"in-crate").unwrap();

        let found = find_ruby_native_lib(tmp.path(), &target, "mylib-rb", "libmylib_rb.so").unwrap();
        assert_eq!(found, release_dir.join("libmylib_rb.so"));
    }
}
