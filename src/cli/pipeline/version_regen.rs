use crate::core::backend::GeneratedFile;
use crate::core::config::ResolvedCrateConfig;
use anyhow::Context as _;

use super::{extract, format_generated, readme};

/// Run the post-generation formatters over a freshly written set of scaffold /
/// e2e files, mirroring exactly what the `alef all` generate path does between
/// writing files and finalizing hashes (see `bin_cli/all_commands.rs`).
///
/// The bug this fixes: `sync-versions` regenerated manifests
/// (`package.json`, `composer.json`, `Package.swift`, …) by writing the raw
/// scaffold serializer output (2-space JSON, scaffold key order, trailing comma
/// on single-element Swift arrays) and then finalizing hashes — but it never ran
/// the formatter the generate path applies afterwards (oxfmt, php-cs-fixer,
/// swift-format, …). The committed files are the *formatted* form, so
/// `sync-versions` rewrote already-committed bytes into a different shape and the
/// downstream `alef verify` → `sync-versions` → `git diff --exit-code` freshness
/// gate failed. Running the same formatter here makes the two paths produce
/// byte-identical output.
///
/// `format_generated`'s `None` argument selects its whole-tree convergence pass, which formats
/// every generated package under `base_dir` regardless of which files this call actually wrote
/// -- so nothing here needs to repackage `files` by language the way earlier versions of this
/// function did. `files`/`config.languages` remain solely to decide whether there is anything to
/// format at all: an empty write, or a crate with no configured languages, has nothing under
/// `base_dir` this pass would usefully touch. Formatter failures are best-effort warnings inside
/// `format_generated` and never abort the sync.
fn format_regenerated_files(config: &ResolvedCrateConfig, files: &[GeneratedFile], base_dir: &std::path::Path) {
    if files.is_empty() || config.languages.is_empty() {
        return;
    }
    format_generated(config, base_dir, None);
}

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

    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {} for test_apps regen", config_path.display()))?;
    let new_alef_cfg: NewAlefConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse {} for test_apps regen", config_path.display()))?;
    let mut resolved_crates = new_alef_cfg
        .resolve()
        .with_context(|| format!("failed to resolve {} for test_apps regen", config_path.display()))?;

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

    let mut registry_config = e2e_config.clone();
    registry_config.dep_mode = DependencyMode::Registry;
    let e2e_ref = &registry_config;

    let api = extract(&fresh_config, config_path, false)?;

    // `generate_e2e` now returns a per-backend generator failure alongside the files it did
    // produce instead of folding it into this `Result`. Nothing here caches a stage hash or
    // sweeps orphans against a previous run (unlike the `alef all` / `alef e2e generate`
    // callers), so there is no premature-success or stale-cache hazard to gate -- writing
    // what succeeded and re-raising the failure afterward is sufficient. ~keep
    let (generated, generator_error) = crate::e2e::generate_e2e(
        &fresh_config,
        e2e_ref,
        None,
        &api.types,
        &api.enums,
        &api.functions,
        &api.errors,
    )?;
    if generated.is_empty() {
        if let Some(error) = generator_error {
            return Err(error);
        }
        return Ok(0);
    }

    let base_dir = std::path::PathBuf::from(".");
    let files = generated;
    if files.is_empty() {
        return Ok(0);
    }
    let count = super::generate::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;

    let managed_files = super::managed_generated_files(&files);
    format_regenerated_files(&fresh_config, &managed_files, &base_dir);

    let sources_hash = super::super::cache::sources_hash(&fresh_config.source_hash_paths()?)?;
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    let path_set: std::collections::HashSet<std::path::PathBuf> =
        managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
    super::generate::finalize_hashes_after_tree_format(&path_set, &base_dir, &sources_hash, &alef_toml_bytes)?;

    if let Some(error) = generator_error {
        return Err(error);
    }
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
/// Scaffold files with `generated_header: true` are overwritten here as long as
/// [`super::generate::write_scaffold_files_report`]'s ownership guard can prove alef
/// authored the pre-existing content (they are otherwise fully alef-managed, e.g.
/// `.cargo/config.toml`). Scaffold files with `generated_header: false` (seeds —
/// Cargo.toml templates, gemspec, pubspec.yaml) are create-once by design: passing
/// `overwrite: true` here does **not** force their version fields back in sync once
/// they already exist on disk, since the ownership guard has no marker to check for a
/// seed and refuses rather than risk clobbering a hand-edit. A seed whose version
/// string genuinely needs to track the workspace bump on every sync belongs on the
/// `generated_header: true` rail instead, not on a widened overwrite here.
///
/// Returns the number of scaffold files written (0 when all were already current).
pub(super) fn regenerate_scaffold_after_sync(
    config: &ResolvedCrateConfig,
    config_path: &std::path::Path,
) -> anyhow::Result<usize> {
    use crate::core::config::NewAlefConfig;

    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {} for scaffold regen", config_path.display()))?;
    let new_alef_cfg: NewAlefConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse {} for scaffold regen", config_path.display()))?;
    let mut resolved_crates = new_alef_cfg
        .resolve()
        .with_context(|| format!("failed to resolve {} for scaffold regen", config_path.display()))?;

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

    let api = extract(&fresh_config, config_path, false)?;
    let languages = fresh_config.languages.clone();

    let generated_scaffold = super::scaffold(&api, &fresh_config, &languages, config_path)?;
    if generated_scaffold.is_empty() {
        return Ok(0);
    }

    let base_dir = std::path::PathBuf::from(".");
    let scaffold_files = generated_scaffold;
    if scaffold_files.is_empty() {
        return Ok(0);
    }
    let count = super::generate::write_scaffold_files_with_overwrite(&scaffold_files, &base_dir, true)?;

    let managed_files = super::managed_generated_files(&scaffold_files);
    format_regenerated_files(&fresh_config, &managed_files, &base_dir);

    let sources_hash = super::super::cache::sources_hash(&fresh_config.source_hash_paths()?)?;
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    let path_set: std::collections::HashSet<std::path::PathBuf> =
        managed_files.iter().map(|f| base_dir.join(&f.path)).collect();
    super::generate::finalize_hashes_after_tree_format(&path_set, &base_dir, &sources_hash, &alef_toml_bytes)?;

    Ok(count)
}

/// Internal helper to regenerate READMEs after a version sync.
/// Extracts IR, computes README files, and writes them to disk.
pub(super) fn regenerate_readmes(config: &ResolvedCrateConfig, config_path: &std::path::Path) -> anyhow::Result<usize> {
    let api = extract(config, config_path, false)?;
    let languages = crate::readme::expand_configured_readme_languages(config, &config.languages);
    let readme_files = readme(&api, config, &languages)?;
    let base_dir = std::path::PathBuf::from(".");
    let sources_hash = super::super::cache::sources_hash(&config.source_hash_paths()?)?;
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    let count = super::generate::write_scaffold_files_with_overwrite(&readme_files, &base_dir, true)?;
    let paths = super::generate::stampable_output_paths(&readme_files, &base_dir);
    super::generate::finalize_hashes(&paths, &sources_hash, &alef_toml_bytes)?;
    Ok(count)
}
