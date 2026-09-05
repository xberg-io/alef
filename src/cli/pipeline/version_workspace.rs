use std::collections::HashSet;
use std::path::Path;
use tracing::debug;

use crate::cli::git::IgnoreFilter;
use crate::core::config::ResolvedCrateConfig;

use super::version_core::{patch_cargo_crates_io_version, patch_workspace_dep_versions, write_version_to_cargo_toml};

/// Sync `[package] version` AND the self-referential dependency pin (`<crate> = {
/// version = "...", ... }`, with or without a `package = "..."` rename key) in every
/// Rust e2e harness manifest alef owns.
///
/// `alef e2e generate` writes the local/path-mode harness under `<e2e.output>/rust`
/// (default `"e2e"`) and, separately, the registry-mode harness under
/// `<e2e.registry.output>/rust` (default `"test_apps"`) — two distinct directories
/// that can both exist for the same consumer. Both were previously collapsed into
/// one lookup that only ever checked `e2e.registry.output`, so a consumer using the
/// (default) local/path e2e harness had its `e2e/rust/Cargo.toml` silently skipped
/// on every version bump. ~keep
///
/// `alef sync-versions` (without `--regen`) never runs `alef e2e generate`'s full
/// `render_cargo_toml` pass — regenerating code is opt-in and deliberately gated
/// behind `--regen` (see `SyncVersions::regen`'s doc). Without a second, explicit
/// patch here, a plain `sync-versions` run bumped `[package] version` in this exact
/// manifest but left its `[dependencies]` pin on the crate itself untouched — the
/// same shape as the `packages/ruby/ext/*/native/Cargo.toml` gap below, which already
/// pairs `write_version_to_cargo_toml` with `patch_workspace_dep_versions` for exactly
/// this reason. `patch_workspace_dep_versions`'s membership check already covers both
/// the plain form (`samplewidget = { version = "...", ... }`, dependency key == crate
/// name, no `package = "..."` needed) and the renamed form (`sample_widget_lib =
/// { package = "sample-widget-lib", version = "...", ... }`); this call just needed to
/// exist. Confirmed by reproducing against a consumer's actual alef.toml/source tree: a
/// plain `alef sync-versions --bump patch` left `samplewidget = { version = "1.3.1", ... }`
/// pinned to the pre-bump version while `[package] version` moved to 1.3.2, exactly the bug
/// alef #152 reported — reachable regardless of dependency-key spelling, because
/// nothing here ever called `patch_workspace_dep_versions` at all, in either form.
/// ~keep
pub(super) fn sync_rust_test_app_version(
    config: &ResolvedCrateConfig,
    version: &str,
    updated: &mut Vec<String>,
    any_cargo_toml_modified: &mut bool,
) {
    let Some(e2e_config) = config.e2e.as_ref() else {
        return;
    };
    let core_member: HashSet<String> = std::iter::once(config.name.clone()).collect();
    for output_dir in [e2e_config.output.as_str(), e2e_config.registry.output.as_str()] {
        sync_rust_harness_cargo_toml(output_dir, version, &core_member, updated, any_cargo_toml_modified);
    }
}

fn sync_rust_harness_cargo_toml(
    output_dir: &str,
    version: &str,
    core_member: &HashSet<String>,
    updated: &mut Vec<String>,
    any_cargo_toml_modified: &mut bool,
) {
    let path = Path::new(output_dir).join("rust/Cargo.toml");
    if !path.exists() {
        return;
    }
    let path_string = path.to_string_lossy().to_string();
    let mut changed = write_version_to_cargo_toml(&path_string, version).unwrap_or(false);
    match patch_workspace_dep_versions(&path_string, version, core_member) {
        Ok(true) => changed = true,
        Ok(false) => {}
        Err(e) => debug!("Could not patch self dep pin in {path_string}: {e}"),
    }
    if changed && !updated.contains(&path_string) {
        updated.push(path_string);
        *any_cargo_toml_modified = true;
    }
}

pub(super) fn sync_workspace_cargo_toml_versions(
    crate_name: &str,
    version: &str,
    writable: &IgnoreFilter,
    updated: &mut Vec<String>,
    any_cargo_toml_modified: &mut bool,
) {
    let Some((cargo_toml_paths, workspace_member_names)) = collect_workspace_cargo_toml_paths(writable) else {
        return;
    };

    for path_str in &cargo_toml_paths {
        if crate::publish::workspace::manifest_is_publishable(Path::new(path_str)) {
            if write_version_to_cargo_toml(path_str, version).unwrap_or(false) && !updated.contains(path_str) {
                updated.push(path_str.clone());
                *any_cargo_toml_modified = true;
            }
        } else {
            debug!("Skipping version stamp for {path_str}: publish = false (local-only compatibility shim)");
        }

        if workspace_member_names.is_empty() {
            continue;
        }

        match patch_workspace_dep_versions(path_str, version, &workspace_member_names) {
            Ok(true) => {
                if !updated.contains(path_str) {
                    updated.push(path_str.clone());
                    *any_cargo_toml_modified = true;
                }
            }
            Ok(false) => {}
            Err(e) => {
                debug!("Could not patch dep versions in {path_str}: {e}");
            }
        }
    }

    if !workspace_member_names.is_empty() {
        match patch_workspace_dep_versions("Cargo.toml", version, &workspace_member_names) {
            Ok(true) => {
                if !updated.contains(&"Cargo.toml".to_string()) {
                    updated.push("Cargo.toml".to_string());
                    *any_cargo_toml_modified = true;
                }
            }
            Ok(false) => {}
            Err(e) => {
                debug!("Could not patch workspace dep versions in root Cargo.toml: {e}");
            }
        }
    }

    match patch_cargo_crates_io_version("Cargo.toml", crate_name, version) {
        Ok(true) => {
            if !updated.contains(&"Cargo.toml".to_string()) {
                updated.push("Cargo.toml".to_string());
                *any_cargo_toml_modified = true;
            }
        }
        Ok(false) => {}
        Err(e) => {
            debug!("Could not patch [patch.crates-io] version in root Cargo.toml: {e}");
        }
    }
}

fn collect_workspace_cargo_toml_paths(writable: &IgnoreFilter) -> Option<(Vec<String>, HashSet<String>)> {
    let root_content = std::fs::read_to_string("Cargo.toml").ok()?;
    let root_toml = root_content.parse::<toml::Table>().ok()?;
    let empty_vec = vec![];
    let members = root_toml
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .unwrap_or(&empty_vec);
    let excludes = root_toml
        .get("workspace")
        .and_then(|w| w.get("exclude"))
        .and_then(|m| m.as_array())
        .unwrap_or(&empty_vec);

    let workspace_member_names = crate::publish::workspace::workspace_member_crates(Path::new("."))
        .map(|m| m.names.into_iter().collect())
        .unwrap_or_default();

    let mut cargo_toml_paths: Vec<String> = vec![];
    for pattern_val in members.iter().chain(excludes.iter()) {
        if let Some(pattern) = pattern_val.as_str() {
            for entry in writable.glob(&format!("{pattern}/Cargo.toml")) {
                cargo_toml_paths.push(entry.to_string_lossy().to_string());
            }
        }
    }

    for entry in writable.glob("packages/*/rust/Cargo.toml") {
        let path_str = entry.to_string_lossy().to_string();
        if !cargo_toml_paths.contains(&path_str) {
            cargo_toml_paths.push(path_str);
        }
    }

    Some((cargo_toml_paths, workspace_member_names))
}
