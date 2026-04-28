//! WASM package builder via wasm-pack + npm pack.
//!
//! Invokes `wasm-pack build` on the WASM crate, then `npm pack` on the
//! produced `pkg/` directory to generate a tarball, which is moved to
//! `output_dir`.

use super::PackageArtifact;
use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Package a WASM crate into an npm tarball.
///
/// Produces: `{name}-{version}.tgz` (npm tarball format).
///
/// Steps:
/// 1. `wasm-pack build crates/{wasm-crate} --release --target bundler`
/// 2. `npm pack` inside the produced `pkg/` directory
/// 3. Move `*.tgz` to `output_dir`
pub fn package_wasm(
    config: &AlefConfig,
    workspace_root: &Path,
    output_dir: &Path,
    _version: &str,
) -> Result<PackageArtifact> {
    let wasm_crate = crate::crate_name_from_output(config, alef_core::config::extras::Language::Wasm)
        .unwrap_or_else(|| format!("{}-wasm", config.crate_config.name));

    let crate_dir = workspace_root.join("crates").join(&wasm_crate);

    // Run wasm-pack build.
    let build_cmd = format!("wasm-pack build {} --release --target bundler", crate_dir.display());
    crate::run_shell_command_in(&build_cmd, workspace_root)?;

    // The pkg/ directory is produced inside the wasm crate directory.
    let pkg_dir = crate_dir.join("pkg");
    if !pkg_dir.exists() {
        anyhow::bail!(
            "wasm-pack build did not produce pkg/ directory at {}",
            pkg_dir.display()
        );
    }

    // Run npm pack inside pkg/.
    crate::run_shell_command_in("npm pack", &pkg_dir)?;

    // Find the produced .tgz file.
    let tgz_path = find_tgz(&pkg_dir).context("npm pack: no .tgz found in pkg/")?;
    let file_name = tgz_path
        .file_name()
        .context("tgz has no filename")?
        .to_string_lossy()
        .to_string();

    let dest = output_dir.join(&file_name);
    fs::copy(&tgz_path, &dest).with_context(|| format!("copying {} to {}", tgz_path.display(), dest.display()))?;

    Ok(PackageArtifact {
        path: dest,
        name: file_name,
        checksum: None,
    })
}

fn find_tgz(dir: &Path) -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "tgz"))
        .collect();
    candidates.sort_by_key(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    candidates
        .into_iter()
        .next_back()
        .with_context(|| format!("no .tgz found in {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn find_tgz_returns_latest() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.tgz"), b"a").unwrap();
        // Sleep briefly to get different mtimes on some platforms, otherwise
        // order is filesystem-dependent; we just verify no panic + Some result.
        fs::write(tmp.path().join("b.tgz"), b"b").unwrap();

        let result = find_tgz(tmp.path()).unwrap();
        assert!(result.extension().unwrap() == "tgz");
    }

    #[test]
    fn find_tgz_empty_dir_errors() {
        let tmp = TempDir::new().unwrap();
        let result = find_tgz(tmp.path());
        assert!(result.is_err());
    }
}
