use std::collections::HashSet;
use std::path::Path;
use tracing::debug;

use super::version_core::{patch_cargo_crates_io_version, patch_workspace_dep_versions, write_version_to_cargo_toml};

pub(super) fn sync_workspace_cargo_toml_versions(
    crate_name: &str,
    version: &str,
    updated: &mut Vec<String>,
    any_cargo_toml_modified: &mut bool,
) {
    let Some((cargo_toml_paths, workspace_member_names)) = collect_workspace_cargo_toml_paths() else {
        return;
    };

    for path_str in &cargo_toml_paths {
        if write_version_to_cargo_toml(path_str, version).is_ok() && !updated.contains(path_str) {
            updated.push(path_str.clone());
            *any_cargo_toml_modified = true;
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

fn collect_workspace_cargo_toml_paths() -> Option<(Vec<String>, HashSet<String>)> {
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
            if let Ok(paths) = glob::glob(&format!("{pattern}/Cargo.toml")) {
                for entry in paths.flatten() {
                    cargo_toml_paths.push(entry.to_string_lossy().to_string());
                }
            }
        }
    }

    for entry in glob::glob("packages/*/rust/Cargo.toml").into_iter().flatten().flatten() {
        let path_str = entry.to_string_lossy().to_string();
        if !cargo_toml_paths.contains(&path_str) {
            cargo_toml_paths.push(path_str);
        }
    }

    Some((cargo_toml_paths, workspace_member_names))
}
