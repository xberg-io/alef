use anyhow::Context as _;
use std::collections::HashSet;
use std::path::Path;
use tracing::debug;

use crate::cli::git::IgnoreFilter;
use crate::core::config::ResolvedCrateConfig;

use super::version_core::{patch_workspace_dep_versions, write_version_to_cargo_toml};

pub(super) fn sync_extra_cargo_manifest_version(
    path: &str,
    content: &str,
    version: &str,
    crate_name: &str,
) -> anyhow::Result<bool> {
    let document: toml::Value = toml::from_str(content).with_context(|| format!("failed to parse {path}"))?;
    let package_version = document.get("package").and_then(|package| package.get("version"));
    let workspace_version = document
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"));
    let has_literal_version = [package_version, workspace_version]
        .into_iter()
        .flatten()
        .any(toml::Value::is_str);
    let package_changed = has_literal_version && write_version_to_cargo_toml(path, version)?;
    let core_member = std::iter::once(crate_name.to_owned()).collect();
    let dependency_changed = patch_workspace_dep_versions(path, version, &core_member)?;
    Ok(package_changed || dependency_changed)
}

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
    config: &ResolvedCrateConfig,
    version: &str,
    writable: &IgnoreFilter,
    updated: &mut Vec<String>,
    any_cargo_toml_modified: &mut bool,
) {
    let Some((cargo_toml_paths, selected_core)) = collect_workspace_cargo_toml_paths(config, writable) else {
        return;
    };

    let version_family = selected_version_family(config, selected_core.as_ref(), &cargo_toml_paths);
    let components_only = config.is_components_only();
    for path_str in &cargo_toml_paths {
        // A components-only crate owns just the producer crates it generated; every other
        // workspace member (the core crate included) keeps its own version and dependency
        // literals. ~keep
        if components_only && !manifest_carries_alef_marker(path_str) {
            debug!("Skipping version sync for a workspace member alef does not own: {path_str}");
            continue;
        }
        if Path::new(path_str)
            .canonicalize()
            .is_ok_and(|path| version_family.manifests.contains(&path))
            // Producer crates are `publish = false` yet alef-owned, so they are stamped regardless. ~keep
            && (components_only || crate::publish::workspace::manifest_is_publishable(Path::new(path_str)))
        {
            if write_version_to_cargo_toml(path_str, version).unwrap_or(false) && !updated.contains(path_str) {
                updated.push(path_str.clone());
                *any_cargo_toml_modified = true;
            }
        } else {
            debug!("Skipping package version stamp outside the publishable selected family: {path_str}");
        }

        match patch_family_dependency_versions(path_str, version, &version_family) {
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

    match patch_family_dependency_versions("Cargo.toml", version, &version_family) {
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

fn manifest_carries_alef_marker(path: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|content| crate::core::hash::content_has_alef_marker(&content))
}

fn collect_workspace_cargo_toml_paths(
    config: &ResolvedCrateConfig,
    writable: &IgnoreFilter,
) -> Option<(Vec<String>, Option<WorkspaceManifest>)> {
    let root_content = std::fs::read_to_string("Cargo.toml").ok()?;
    let root_toml = root_content.parse::<toml::Table>().ok()?;
    let workspace_root = std::env::current_dir().ok()?;
    let empty_vec = vec![];
    let members = root_toml
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .unwrap_or(&empty_vec);

    let mut cargo_toml_paths: Vec<String> = vec![];
    for pattern_val in members {
        if let Some(pattern) = pattern_val.as_str() {
            for entry in writable.glob(&format!("{pattern}/Cargo.toml")) {
                if !workspace_manifest_is_excluded(&entry, &workspace_root, &root_toml) {
                    cargo_toml_paths.push(entry.to_string_lossy().to_string());
                }
            }
        }
    }

    let selected_core = selected_core_manifest(config, &cargo_toml_paths);
    for entry in writable.glob("packages/*/rust/Cargo.toml") {
        let path_str = entry.to_string_lossy().to_string();
        if !cargo_toml_paths.contains(&path_str) {
            cargo_toml_paths.push(path_str);
        }
    }

    if let Some(core) = selected_core.as_ref()
        && !cargo_toml_paths
            .iter()
            .any(|path| Path::new(path).canonicalize().ok().as_ref() == Some(&core.path))
    {
        cargo_toml_paths.push(core.path.to_string_lossy().into_owned());
    }
    Some((cargo_toml_paths, selected_core))
}

fn workspace_manifest_is_excluded(path: &Path, root: &Path, document: &toml::Table) -> bool {
    let candidate = normalize_workspace_path(&root.join(path));
    document
        .get("workspace")
        .and_then(|workspace| workspace.get("exclude"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .any(|exclude| candidate.starts_with(normalize_workspace_path(&root.join(exclude))))
}

fn normalize_workspace_path(path: &Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

struct VersionFamily {
    manifests: HashSet<std::path::PathBuf>,
    names: HashSet<String>,
}

struct WorkspaceManifest {
    path: std::path::PathBuf,
    name: String,
    document: toml::Table,
    managed: bool,
}

fn read_workspace_manifest(path: &str) -> Option<WorkspaceManifest> {
    let content = std::fs::read_to_string(path).ok()?;
    let document: toml::Table = content.parse().ok()?;
    let name = document.get("package")?.get("name")?.as_str()?.to_owned();
    Some(WorkspaceManifest {
        path: Path::new(path).canonicalize().ok()?,
        name,
        document,
        managed: crate::core::hash::content_has_alef_marker(&content),
    })
}

fn selected_core_manifest(config: &ResolvedCrateConfig, explicit_members: &[String]) -> Option<WorkspaceManifest> {
    let configured = [
        config.core_crate_root().join("Cargo.toml"),
        config.version_from.clone().into(),
    ]
    .iter()
    .filter_map(|path| read_workspace_manifest(&path.to_string_lossy()))
    .find(|manifest| manifest.name == config.name);
    if configured.is_some() {
        return configured;
    }
    let mut candidates: Vec<_> = explicit_members
        .iter()
        .filter_map(|path| read_workspace_manifest(path))
        .filter(|manifest| manifest.name == config.name)
        .collect();
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    candidates.dedup_by(|left, right| left.path == right.path);
    if candidates.len() == 1 {
        return candidates.pop();
    }
    if candidates.len() > 1 {
        tracing::warn!(crate_name = %config.name, "cannot select a unique explicit workspace core manifest");
    }
    None
}

fn workspace_dependencies() -> toml::Table {
    std::fs::read_to_string("Cargo.toml")
        .ok()
        .and_then(|content| content.parse::<toml::Table>().ok())
        .and_then(|root| root.get("workspace")?.get("dependencies")?.as_table().cloned())
        .unwrap_or_default()
}

// Provenance alone does not associate a generated SDK with this core: an explicit
// dependency source must resolve to a manifest already in the selected family. ~keep
fn selected_version_family(
    config: &ResolvedCrateConfig,
    selected_core: Option<&WorkspaceManifest>,
    paths: &[String],
) -> VersionFamily {
    let workspace_dependencies = workspace_dependencies();
    let candidates: Vec<_> = paths
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("Cargo.toml"))
        .filter_map(read_workspace_manifest)
        .collect();
    let mut family = VersionFamily {
        manifests: selected_core
            .map(|manifest| manifest.path.clone())
            .into_iter()
            .collect(),
        names: HashSet::from([config.name.clone()]),
    };
    loop {
        let previous_count = family.manifests.len();
        for manifest in &candidates {
            if manifest.managed
                && depends_on_version_family(&manifest.document, &manifest.path, &workspace_dependencies, &family)
            {
                family.manifests.insert(manifest.path.clone());
                family.names.insert(manifest.name.clone());
            }
        }
        if family.manifests.len() == previous_count {
            return family;
        }
    }
}

fn dependency_belongs_to_family(
    key: &str,
    dependency: &toml::Value,
    manifest: &Path,
    workspace_dependencies: &toml::Table,
    family: &VersionFamily,
) -> bool {
    let inherited = dependency.get("workspace").and_then(toml::Value::as_bool) == Some(true);
    let dependency = if inherited {
        let Some(dependency) = workspace_dependencies.get(key) else {
            return false;
        };
        dependency
    } else {
        dependency
    };
    if dependency.get("git").is_some() {
        return false;
    }
    if let Some(path) = dependency.get("path").and_then(toml::Value::as_str) {
        let base = if inherited {
            Path::new(".")
        } else {
            manifest.parent().unwrap_or(Path::new("."))
        };
        return base
            .join(path)
            .join("Cargo.toml")
            .canonicalize()
            .is_ok_and(|path| family.manifests.contains(&path));
    }
    let package = dependency.get("package").and_then(toml::Value::as_str).unwrap_or(key);
    family.names.contains(package)
}

fn depends_on_version_family(
    document: &toml::Table,
    manifest: &Path,
    workspace_dependencies: &toml::Table,
    family: &VersionFamily,
) -> bool {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if document
            .get(section)
            .and_then(toml::Value::as_table)
            .is_some_and(|dependencies| {
                dependencies.iter().any(|(key, dependency)| {
                    dependency_belongs_to_family(key, dependency, manifest, workspace_dependencies, family)
                })
            })
        {
            return true;
        }
    }
    document
        .get("target")
        .and_then(toml::Value::as_table)
        .is_some_and(|targets| {
            targets
                .values()
                .filter_map(toml::Value::as_table)
                .any(|target| depends_on_version_family(target, manifest, workspace_dependencies, family))
        })
}

fn patch_family_dependency_versions(path: &str, version: &str, family: &VersionFamily) -> anyhow::Result<bool> {
    let content = std::fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?;
    let source: toml::Table = content.parse().with_context(|| format!("failed to parse {path}"))?;
    let mut document: toml_edit::DocumentMut = content.parse().with_context(|| format!("failed to parse {path}"))?;
    let workspace_dependencies = workspace_dependencies();
    let mut changed = patch_family_sections(
        document.as_table_mut(),
        &source,
        Path::new(path),
        &workspace_dependencies,
        version,
        family,
    );
    if let Some(workspace) = document
        .get_mut("workspace")
        .and_then(toml_edit::Item::as_table_like_mut)
        && let Some(dependencies) = workspace
            .get_mut("dependencies")
            .and_then(toml_edit::Item::as_table_like_mut)
    {
        changed |= patch_family_dependency_table(
            dependencies,
            &workspace_dependencies,
            Path::new("Cargo.toml"),
            &workspace_dependencies,
            version,
            family,
        );
    }
    changed |= patch_family_registry_overrides(&mut document, &source, Path::new(path), version, family);
    if changed {
        std::fs::write(path, document.to_string()).with_context(|| format!("failed to write {path}"))?;
    }
    Ok(changed)
}

fn patch_family_dependency_table(
    table: &mut dyn toml_edit::TableLike,
    source: &toml::Table,
    manifest: &Path,
    workspace_dependencies: &toml::Table,
    version: &str,
    family: &VersionFamily,
) -> bool {
    let mut changed = false;
    for (key, item) in table.iter_mut() {
        if source.get(key.get()).is_some_and(|dependency| {
            dependency_belongs_to_family(key.get(), dependency, manifest, workspace_dependencies, family)
        }) && let Some(value) = item
            .as_table_like_mut()
            .and_then(|dependency| dependency.get_mut("version"))
            && value.as_str() != Some(version)
        {
            *value = toml_edit::value(version);
            changed = true;
        }
    }
    changed
}

fn patch_family_sections(
    table: &mut dyn toml_edit::TableLike,
    source: &toml::Table,
    manifest: &Path,
    workspace_dependencies: &toml::Table,
    version: &str,
    family: &VersionFamily,
) -> bool {
    let mut changed = false;
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependencies) = table.get_mut(section).and_then(toml_edit::Item::as_table_like_mut)
            && let Some(original) = source.get(section).and_then(toml::Value::as_table)
        {
            changed |= patch_family_dependency_table(
                dependencies,
                original,
                manifest,
                workspace_dependencies,
                version,
                family,
            );
        }
    }
    if let Some(targets) = table.get_mut("target").and_then(toml_edit::Item::as_table_like_mut)
        && let Some(original_targets) = source.get("target").and_then(toml::Value::as_table)
    {
        for (key, item) in targets.iter_mut() {
            if let Some(target) = item.as_table_like_mut()
                && let Some(original) = original_targets.get(key.get()).and_then(toml::Value::as_table)
            {
                changed |= patch_family_sections(target, original, manifest, workspace_dependencies, version, family);
            }
        }
    }
    changed
}

fn patch_family_registry_overrides(
    document: &mut toml_edit::DocumentMut,
    source: &toml::Table,
    manifest: &Path,
    version: &str,
    family: &VersionFamily,
) -> bool {
    let Some(table) = document
        .get_mut("patch")
        .and_then(|patch| patch.get_mut("crates-io"))
        .and_then(toml_edit::Item::as_table_like_mut)
    else {
        return false;
    };
    let Some(original) = source
        .get("patch")
        .and_then(|patch| patch.get("crates-io"))
        .and_then(toml::Value::as_table)
    else {
        return false;
    };
    patch_family_dependency_table(table, original, manifest, &toml::Table::new(), version, family)
}
