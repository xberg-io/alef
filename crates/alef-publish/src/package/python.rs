//! Python wheel and sdist packaging via maturin.
//!
//! Locates maturin-produced wheels in `target/wheels/` and copies them to
//! `output_dir`. For sdist, invokes `maturin sdist` and moves the resulting
//! tarball to `output_dir`.

use super::PackageArtifact;
use crate::platform::RustTarget;
use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Package Python wheels and/or sdist.
///
/// Produces one or more artifacts:
/// - `{name}-{version}-*.whl` — platform wheel(s) from `target/wheels/`
/// - `{name}-{version}.tar.gz` — sdist (when `sdist = true`)
///
/// The `wheel` and `sdist` flags default to `true` when `None`.
pub fn package_python(
    config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    version: &str,
) -> Result<Vec<PackageArtifact>> {
    let lang_cfg = publish_lang_config(config);
    let do_wheel = lang_cfg.wheel.unwrap_or(true);
    let do_sdist = lang_cfg.sdist.unwrap_or(true);

    let mut artifacts = Vec::new();

    if do_wheel {
        let wheel =
            package_wheel(config, target, workspace_root, output_dir, version).context("packaging Python wheel")?;
        artifacts.push(wheel);
    }

    if do_sdist {
        let sdist = package_sdist(config, workspace_root, output_dir).context("packaging Python sdist")?;
        artifacts.push(sdist);
    }

    Ok(artifacts)
}

fn package_wheel(
    _config: &AlefConfig,
    target: &RustTarget,
    workspace_root: &Path,
    output_dir: &Path,
    _version: &str,
) -> Result<PackageArtifact> {
    // maturin places wheels in target/wheels/ by default.
    let wheels_dir = workspace_root.join("target/wheels");
    let platform = target.platform_for(alef_core::config::extras::Language::Python);

    // Find a wheel that matches the current target platform fragment.
    let wheel_path = find_wheel(&wheels_dir, &platform)?;
    let file_name = wheel_path
        .file_name()
        .context("wheel path has no filename")?
        .to_string_lossy()
        .to_string();

    let dest = output_dir.join(&file_name);
    fs::copy(&wheel_path, &dest)
        .with_context(|| format!("copying wheel {} to {}", wheel_path.display(), dest.display()))?;

    Ok(PackageArtifact {
        path: dest,
        name: file_name,
        checksum: None,
    })
}

fn package_sdist(config: &AlefConfig, workspace_root: &Path, output_dir: &Path) -> Result<PackageArtifact> {
    let py_crate = crate::crate_name_from_output(config, alef_core::config::extras::Language::Python)
        .unwrap_or_else(|| format!("{}-py", config.crate_config.name));

    // Run `maturin sdist --manifest-path crates/{py_crate}/Cargo.toml -o {output_dir}`
    let manifest = workspace_root.join("crates").join(&py_crate).join("Cargo.toml");
    let cmd = format!(
        "maturin sdist --manifest-path {} -o {}",
        manifest.display(),
        output_dir.display()
    );
    crate::run_shell_command_in(&cmd, workspace_root)?;

    // Find the produced sdist tarball.
    let sdist_path =
        find_latest_file(output_dir, ".tar.gz").context("maturin sdist: no .tar.gz found in output dir")?;
    let name = sdist_path
        .file_name()
        .context("sdist has no filename")?
        .to_string_lossy()
        .to_string();

    Ok(PackageArtifact {
        path: sdist_path,
        name,
        checksum: None,
    })
}

fn find_wheel(wheels_dir: &Path, platform_fragment: &str) -> Result<PathBuf> {
    if !wheels_dir.exists() {
        anyhow::bail!("wheels directory does not exist: {}", wheels_dir.display());
    }
    // Maturin encodes the platform in the wheel filename with underscores
    // replacing hyphens (e.g. linux_x86_64).
    let fragment_underscore = platform_fragment.replace('-', "_");

    let mut candidates: Vec<PathBuf> = fs::read_dir(wheels_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "whl")
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().contains(&fragment_underscore))
        })
        .collect();

    // Sort by modification time descending to pick the newest.
    candidates.sort_by_key(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    candidates.into_iter().next_back().with_context(|| {
        format!(
            "no wheel matching platform '{platform_fragment}' in {}",
            wheels_dir.display()
        )
    })
}

fn find_latest_file(dir: &Path, suffix: &str) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(suffix)))
        .collect();
    candidates.sort_by_key(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    candidates
        .into_iter()
        .next_back()
        .with_context(|| format!("no file ending with '{suffix}' in {}", dir.display()))
}

fn publish_lang_config(config: &AlefConfig) -> alef_core::config::publish::PublishLanguageConfig {
    if let Some(publish) = &config.publish {
        if let Some(cfg) = publish.languages.get("python") {
            return cfg.clone();
        }
    }
    alef_core::config::publish::PublishLanguageConfig::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn minimal_config() -> AlefConfig {
        toml::from_str(
            r#"
languages = ["python"]
[crate]
name = "my-lib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn find_wheel_matches_platform_fragment() {
        let tmp = TempDir::new().unwrap();
        let wheels_dir = tmp.path().join("target/wheels");
        fs::create_dir_all(&wheels_dir).unwrap();
        fs::write(wheels_dir.join("my_lib-0.1.0-cp310-cp310-linux_x86_64.whl"), b"fake").unwrap();

        let result = find_wheel(&wheels_dir, "linux-x86_64").unwrap();
        assert!(result.file_name().unwrap().to_str().unwrap().contains("linux_x86_64"));
    }

    #[test]
    fn find_wheel_no_match_errors() {
        let tmp = TempDir::new().unwrap();
        let wheels_dir = tmp.path().join("target/wheels");
        fs::create_dir_all(&wheels_dir).unwrap();
        // Write a wheel that doesn't match.
        fs::write(wheels_dir.join("my_lib-0.1.0-cp310-cp310-linux_aarch64.whl"), b"fake").unwrap();

        let result = find_wheel(&wheels_dir, "linux-x86_64");
        assert!(result.is_err());
    }

    #[test]
    fn publish_lang_config_defaults_wheel_sdist_to_none() {
        let config = minimal_config();
        let cfg = publish_lang_config(&config);
        // Defaults: None means "use default = true" at call site.
        assert!(cfg.wheel.is_none());
        assert!(cfg.sdist.is_none());
    }

    #[test]
    fn publish_lang_config_wheel_sdist_flags() {
        let config: AlefConfig = toml::from_str(
            r#"
languages = ["python"]
[crate]
name = "my-lib"
sources = ["src/lib.rs"]
[publish.languages.python]
wheel = false
sdist = true
"#,
        )
        .unwrap();
        let cfg = publish_lang_config(&config);
        assert_eq!(cfg.wheel, Some(false));
        assert_eq!(cfg.sdist, Some(true));
    }
}
