//! Dart pub.dev package — archives the Flutter Rust Bridge source tree for distribution.

use super::util::{copy_dir_recursive, copy_optional_file};
use super::PackageArtifact;
use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Package Dart bindings into a source tarball suitable for pub.dev.
///
/// Produces: `{pubspec_name}-{version}.tar.gz` containing:
/// - `pubspec.yaml` — copied from `packages/dart/pubspec.yaml`
/// - `lib/` — Dart wrappers (including `lib/src/` for FRB bridge code)
/// - `rust/` — Rust-side FRB crate
/// - `README.md`, `CHANGELOG.md`, `LICENSE` if present in workspace root
///
/// Note: this is a source archive for archival and review. Actual pub.dev uploads
/// are performed by `dart pub publish` from the package directory, which enforces
/// the pub.dev package layout itself.
pub fn package_dart(
    config: &AlefConfig,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let pubspec_name = config.dart_pubspec_name();
    let pkg_dir = config.package_dir(alef_core::config::extras::Language::Dart);

    let pkg_name = format!("{pubspec_name}-{version}");
    let staging = output_dir.join(&pkg_name);

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;

    // Copy the full dart package directory into staging.
    let pkg_src = workspace_root.join(&pkg_dir);
    if !pkg_src.exists() {
        anyhow::bail!("Dart package directory not found: {}", pkg_dir);
    }
    copy_dir_recursive(&pkg_src, &staging).context("copying Dart package directory")?;

    // Copy optional top-level docs into the staging root.
    for filename in ["README.md", "CHANGELOG.md", "LICENSE"] {
        copy_optional_file(workspace_root, filename, &staging)
            .with_context(|| format!("staging {filename} for Dart package"))?;
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use alef_core::config::AlefConfig;
    use std::fs;

    fn minimal_config(name: &str) -> AlefConfig {
        let toml = format!(
            r#"
languages = ["dart"]

[crate]
name = "{name}"
version_from = "Cargo.toml"
sources = []
"#
        );
        toml::from_str(&toml).expect("valid config")
    }

    #[test]
    fn package_dart_errors_when_pkg_dir_missing() {
        let config = minimal_config("my-lib");
        let tmp = tempfile::tempdir().expect("tempdir");
        let output = tmp.path().join("out");
        fs::create_dir_all(&output).unwrap();

        let err = package_dart(&config, tmp.path(), &output, "0.1.0").unwrap_err();
        assert!(
            err.to_string().contains("Dart package directory not found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn package_dart_produces_tarball() {
        let config = minimal_config("my-lib");
        let tmp = tempfile::tempdir().expect("tempdir");

        // Create a minimal packages/dart/ tree.
        let dart_pkg = tmp.path().join("packages/dart");
        fs::create_dir_all(dart_pkg.join("lib/src")).unwrap();
        fs::write(dart_pkg.join("pubspec.yaml"), "name: my_lib\nversion: 0.1.0\n").unwrap();
        fs::write(dart_pkg.join("lib/my_lib.dart"), "// generated\n").unwrap();
        fs::write(dart_pkg.join("lib/src/bridge.dart"), "// bridge\n").unwrap();

        let output = tmp.path().join("out");
        fs::create_dir_all(&output).unwrap();

        let artifact = package_dart(&config, tmp.path(), &output, "0.1.0").unwrap();
        assert!(artifact.path.exists(), "tarball should exist");
        assert_eq!(artifact.name, "my_lib-0.1.0.tar.gz");
    }
}
