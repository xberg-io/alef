//! Kotlin Maven package — builds and stages the compiled jar artifact.

use super::PackageArtifact;
use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Package Kotlin bindings as a Maven artifact (jar).
///
/// Produces: `{name}-{version}.jar` by running `gradle build` in the Kotlin package directory
/// and collecting the built JAR from `build/libs/`. The Maven coordinate is
/// `{kotlin_package}:{crate_name}:{version}` — the group comes from
/// `config.kotlin_package()` and the artifact id is the crate name.
pub fn package_kotlin(
    config: &AlefConfig,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<PackageArtifact> {
    let crate_name = &config.crate_config.name;
    let pkg_dir = config.package_dir(alef_core::config::extras::Language::Kotlin);
    let pkg_path = workspace_root.join(&pkg_dir);

    if !pkg_path.exists() {
        anyhow::bail!("Kotlin package directory not found: {}", pkg_dir);
    }

    // Build the Kotlin project using gradle.
    let status = std::process::Command::new("gradle")
        .arg("build")
        .current_dir(&pkg_path)
        .status()
        .context("running gradle build for Kotlin")?;

    if !status.success() {
        anyhow::bail!("gradle build failed with exit code {}", status.code().unwrap_or(-1));
    }

    // Locate the built JAR in the Gradle build output.
    // Standard pattern: build/libs/{module-name}-{version}.jar
    let build_libs = pkg_path.join("build/libs");
    if !build_libs.exists() {
        anyhow::bail!("gradle build did not produce build/libs directory");
    }

    // Find a jar matching the expected version or any jar.
    let jar_file = find_jar_in_dir(&build_libs, version)
        .or_else(|| find_any_jar(&build_libs))
        .context("no JAR found in build/libs")?;

    // Copy JAR to output directory.
    let output_jar_name = format!("{crate_name}-{version}.jar");
    let output_jar = output_dir.join(&output_jar_name);
    fs::copy(&jar_file, &output_jar).context("copying JAR to output directory")?;

    Ok(PackageArtifact {
        path: output_jar,
        name: output_jar_name,
        checksum: None,
    })
}

/// Find a JAR file matching the version pattern in a directory.
fn find_jar_in_dir(dir: &Path, version: &str) -> Option<std::path::PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.extension().is_some_and(|ext| ext == "jar")
                && p.to_string_lossy().contains(version)
        })
}

/// Find any JAR file in a directory (fallback).
fn find_any_jar(dir: &Path) -> Option<std::path::PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|ext| ext == "jar"))
}
