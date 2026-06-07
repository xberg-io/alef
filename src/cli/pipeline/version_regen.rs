use crate::core::config::ResolvedCrateConfig;
use anyhow::Context as _;

use super::{extract, readme};

/// Regenerate registry-mode test_apps scaffold files after a version sync so
/// that version pins in generated files (e.g. pyproject.toml, mix.exs,
/// build.zig.zon, Package.swift) reflect the updated workspace version.
///
/// Mirrors the `TestApps::Generate` dispatch in `main.rs` but runs inside the
/// `sync_versions` pipeline so the update is atomic with the alef.toml mutation
/// performed by `sync_registry_package_versions`.
///
/// The config is reloaded from `config_path` (which was just updated by
/// `sync_registry_package_versions`) so that the regenerated scaffold files
/// pick up the new registry package version values, not the stale in-memory
/// values from the config that was loaded before `sync_versions` ran.
///
/// Returns the number of files written (0 when everything was already current).
pub(super) fn regenerate_test_apps_after_sync(
    config: &ResolvedCrateConfig,
    _e2e_config: &crate::core::config::e2e::E2eConfig,
    config_path: &std::path::Path,
) -> anyhow::Result<usize> {
    use crate::core::config::NewAlefConfig;
    use crate::core::config::e2e::DependencyMode;

    // Reload alef.toml from disk so the in-memory config reflects the
    // registry package version that `sync_registry_package_versions` just wrote.
    // The stale in-memory `config.e2e` would produce pyproject.toml / mix.exs /
    // build.zig.zon with the old version pins — exactly the rc.13 bug this
    // function is designed to prevent.
    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {} for test_apps regen", config_path.display()))?;
    let new_alef_cfg: NewAlefConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse {} for test_apps regen", config_path.display()))?;
    let mut resolved_crates = new_alef_cfg
        .resolve()
        .with_context(|| format!("failed to resolve {} for test_apps regen", config_path.display()))?;

    // Find the matching crate by name. Fall back to the first crate with an
    // [e2e] block when the name doesn't match (e.g. single-crate repos).
    let fresh_config = resolved_crates
        .iter()
        .position(|c| c.name == config.name && c.e2e.is_some())
        .or_else(|| resolved_crates.iter().position(|c| c.e2e.is_some()))
        .map(|idx| resolved_crates.swap_remove(idx))
        .ok_or_else(|| anyhow::anyhow!("no crate with [e2e] block found in reloaded config"))?;

    let e2e_config = fresh_config
        .e2e
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("reloaded crate has no [e2e] block"))?;

    // Build a registry-mode clone so `generate_e2e` uses published-package
    // coordinates rather than local path dependencies.
    let mut registry_config = e2e_config.clone();
    registry_config.dep_mode = DependencyMode::Registry;
    let e2e_ref = &registry_config;

    // Extract IR (empty for repos with no sources configured — the scaffold
    // files like pyproject.toml do not require IR content).
    let api = extract(&fresh_config, config_path, false)?;

    // Generate test_apps/ scaffold files for all configured e2e languages.
    let files = crate::e2e::generate_e2e(&fresh_config, e2e_ref, None, &api.types, &api.enums)?;
    if files.is_empty() {
        return Ok(0);
    }

    let base_dir = std::path::PathBuf::from(".");
    let count = super::generate::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;

    let sources_hash = super::super::cache::sources_hash(&fresh_config.sources)?;
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    let path_set: std::collections::HashSet<std::path::PathBuf> =
        files.iter().map(|f| base_dir.join(&f.path)).collect();
    super::generate::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

    Ok(count)
}

/// Regenerate scaffold files (pyproject.toml, package.json, gemspec, pubspec.yaml,
/// Cargo.toml in binding crates, etc.) after a version sync so that version fields
/// embedded at scaffold-generation time reflect the updated workspace version.
///
/// The scaffold generator reads `api.version` from the IR, which in turn reflects
/// the current `Cargo.toml` workspace version. Reloading the config from
/// `config_path` after `sync_versions` has written the bumped version ensures the
/// IR carries the fresh version string.
///
/// Scaffold files with `generated_header: true` are always overwritten (they are
/// fully alef-managed, e.g. `.cargo/config.toml`). Scaffold files with
/// `generated_header: false` (seeds — Cargo.toml templates, gemspec, pubspec.yaml)
/// are also overwritten here so version strings stay in sync atomically with the
/// workspace bump. This mirrors what `alef all --clean` would do.
///
/// Returns the number of scaffold files written (0 when all were already current).
pub(super) fn regenerate_scaffold_after_sync(
    config: &ResolvedCrateConfig,
    config_path: &std::path::Path,
) -> anyhow::Result<usize> {
    use crate::core::config::NewAlefConfig;

    // Reload alef.toml so the in-memory config reflects the bumped version that
    // `sync_versions` just wrote to Cargo.toml (version_from). The stale
    // in-memory `api.version` would produce scaffold files with the old version
    // string — identical to the rc.13 bug for test_apps but on the scaffold side.
    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {} for scaffold regen", config_path.display()))?;
    let new_alef_cfg: NewAlefConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse {} for scaffold regen", config_path.display()))?;
    let mut resolved_crates = new_alef_cfg
        .resolve()
        .with_context(|| format!("failed to resolve {} for scaffold regen", config_path.display()))?;

    // Match by name; fall back to first crate (single-crate repos).
    let fresh_config = resolved_crates
        .iter()
        .position(|c| c.name == config.name)
        .or(Some(0))
        .and_then(|idx| {
            if idx < resolved_crates.len() {
                Some(resolved_crates.swap_remove(idx))
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("no crate found in reloaded config for scaffold regen"))?;

    // Extract IR — scaffold generators use api.version (from Cargo.toml) and
    // api.types/enums. Sources may be empty for pure-scaffold repos; extract
    // tolerates that.
    let api = extract(&fresh_config, config_path, false)?;
    let languages = fresh_config.languages.clone();

    let scaffold_files = super::scaffold(&api, &fresh_config, &languages)?;
    if scaffold_files.is_empty() {
        return Ok(0);
    }

    let base_dir = std::path::PathBuf::from(".");
    // Always overwrite: scaffold seed files (gemspec, pubspec.yaml, Cargo.toml)
    // must reflect the bumped version even when they already exist on disk.
    let count = super::generate::write_scaffold_files_with_overwrite(&scaffold_files, &base_dir, true)?;

    let sources_hash = super::super::cache::sources_hash(&fresh_config.sources)?;
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    let path_set: std::collections::HashSet<std::path::PathBuf> =
        scaffold_files.iter().map(|f| base_dir.join(&f.path)).collect();
    super::generate::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

    Ok(count)
}

/// Internal helper to regenerate READMEs after a version sync.
/// Extracts IR, computes README files, and writes them to disk.
pub(super) fn regenerate_readmes(config: &ResolvedCrateConfig, config_path: &std::path::Path) -> anyhow::Result<usize> {
    let api = extract(config, config_path, false)?;
    let languages = config.languages.clone();
    let readme_files = readme(&api, config, &languages)?;
    let base_dir = std::path::PathBuf::from(".");
    let sources_hash = super::super::cache::sources_hash(&config.sources)?;
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    let count = super::generate::write_scaffold_files_with_overwrite(&readme_files, &base_dir, true)?;
    let paths: std::collections::HashSet<std::path::PathBuf> =
        readme_files.iter().map(|f| base_dir.join(&f.path)).collect();
    super::generate::finalize_hashes(&paths, &sources_hash, &alef_toml_bytes)?;
    Ok(count)
}
