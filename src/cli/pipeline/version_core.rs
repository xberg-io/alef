use crate::core::config::ResolvedCrateConfig;
use anyhow::Context as _;
use std::path::Path;
use tracing::info;

/// Read the version from a Cargo.toml file (workspace or regular package).
pub(crate) fn read_version(version_from: &str) -> anyhow::Result<String> {
    let content =
        std::fs::read_to_string(version_from).with_context(|| format!("failed to read version file {version_from}"))?;
    let value: toml::Value =
        toml::from_str(&content).with_context(|| format!("failed to parse TOML in {version_from}"))?;
    if let Some(v) = value
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(v.to_string());
    }
    if let Some(v) = value
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(v.to_string());
    }
    anyhow::bail!("Could not find version in {version_from}")
}

/// Bump a semver version string by the given component (major, minor, patch).
pub(super) fn bump_version(version: &str, component: &str) -> anyhow::Result<String> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        anyhow::bail!("Invalid semver version: {version}");
    }
    let mut major: u64 = parts[0]
        .parse()
        .with_context(|| format!("Invalid major version component: {}", parts[0]))?;
    let mut minor: u64 = parts[1]
        .parse()
        .with_context(|| format!("Invalid minor version component: {}", parts[1]))?;
    let mut patch: u64 = parts[2]
        .parse()
        .with_context(|| format!("Invalid patch version component: {}", parts[2]))?;

    match component {
        "major" => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        "minor" => {
            minor += 1;
            patch = 0;
        }
        "patch" => {
            patch += 1;
        }
        other => anyhow::bail!("Unknown bump component '{other}': expected major, minor, or patch"),
    }

    Ok(format!("{major}.{minor}.{patch}"))
}

/// Write a bumped version back into a Cargo.toml (workspace or regular package).
///
/// Returns `Ok(true)` when a version field existed and its value actually changed
/// (the file was rewritten), `Ok(false)` when a version field existed but already
/// held `new_version` (a genuine no-op — nothing written), and `Err` only when
/// neither `[package].version` nor `[workspace.package].version` could be found at
/// all.
///
/// The three states matter because `--set <current-version>` re-running a bump
/// after a partial failure is a normal, idempotent operation, not a malformed
/// manifest: before this split, "found the field but it already matches" and
/// "never found the field" both collapsed into the same bail, so a release
/// engineer re-running `sync-versions --set X` when Cargo.toml was already at X
/// was told their `[package]`/`[workspace.package]` version field could not be
/// found, even though it was sitting right there. See `set_version` below for the
/// caller this fixes. ~keep
pub(super) fn write_version_to_cargo_toml(cargo_toml_path: &str, new_version: &str) -> anyhow::Result<bool> {
    use toml_edit::DocumentMut;

    let content =
        std::fs::read_to_string(cargo_toml_path).with_context(|| format!("Failed to read {cargo_toml_path}"))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("Failed to parse TOML in {cargo_toml_path}"))?;

    // `found` tracks whether a real `[package]`/`[workspace.package]` version literal
    // exists at all, independent of whether it already matches `new_version`. This is
    // what actually distinguishes "malformed manifest" from "already at this version" —
    // `changed` alone can't, because both leave it `false`.
    let mut found = false;
    let mut changed = false;

    if let Some(ws_version) = doc
        .get_mut("workspace")
        .and_then(|w| w.as_table_like_mut())
        .and_then(|t| t.get_mut("package"))
        .and_then(|p| p.as_table_like_mut())
        .and_then(|t| t.get_mut("version"))
        && ws_version.is_str()
    {
        found = true;
        if ws_version.as_str() != Some(new_version) {
            *ws_version = toml_edit::value(new_version);
            changed = true;
        }
    }

    if let Some(pkg_version) = doc
        .get_mut("package")
        .and_then(|p| p.as_table_like_mut())
        .and_then(|t| t.get_mut("version"))
        && pkg_version.is_str()
    {
        found = true;
        if pkg_version.as_str() != Some(new_version) {
            *pkg_version = toml_edit::value(new_version);
            changed = true;
        }
    }

    if !found {
        anyhow::bail!(
            "Could not find a `[package]`/`[workspace.package]` version field to update in {cargo_toml_path}"
        );
    }

    if !changed {
        return Ok(false);
    }

    std::fs::write(cargo_toml_path, doc.to_string())
        .with_context(|| format!("Failed to write updated version to {cargo_toml_path}"))?;

    Ok(true)
}

/// Determine whether a `package.json`'s `"private": true` field marks it as
/// a local-only package that must never be published — npm's equivalent of
/// Cargo's `publish = false` for compatibility shims kept only to satisfy a
/// workspace/path dependency.
///
/// Malformed or unparseable JSON is treated as **not** private (fail open),
/// mirroring `manifest_is_publishable`'s fail-open behavior for Cargo.toml.
pub(super) fn package_json_is_private(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| value.get("private").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// Convert a semver pre-release version to PEP 440 format for Python/PyPI.
/// e.g., "0.1.0-rc.1" → "0.1.0rc1", "0.1.0-alpha.2" → "0.1.0a2", "0.1.0-beta.3" → "0.1.0b3"
/// Non-pre-release versions are returned unchanged.
///
/// Single-pass implementation: builds the result into one pre-allocated
/// `String` instead of chaining five `.replace()` calls (each of which
/// allocates a new intermediate `String`).
pub(crate) fn to_pep440(version: &str) -> String {
    let Some((base, pre)) = version.split_once('-') else {
        return version.to_string();
    };
    let mut out = String::with_capacity(base.len() + pre.len());
    out.push_str(base);
    let pre_norm = if let Some(rest) = pre.strip_prefix("alpha.").or_else(|| pre.strip_prefix("alpha")) {
        out.push('a');
        rest
    } else if let Some(rest) = pre.strip_prefix("beta.").or_else(|| pre.strip_prefix("beta")) {
        out.push('b');
        rest
    } else if let Some(rest) = pre.strip_prefix("rc.").or_else(|| pre.strip_prefix("rc")) {
        out.push_str("rc");
        rest
    } else {
        pre
    };
    for c in pre_norm.chars() {
        if c != '.' {
            out.push(c);
        }
    }
    out
}

/// Patch intra-workspace `version = "..."` pins inside a Cargo.toml dep table,
/// preserving all formatting and comments via `toml_edit`.
///
/// Only dep entries whose key is in `workspace_members` are touched. External
/// crates (e.g. `serde`, `tokio`) are left intact.
///
/// Handles these dep-table shapes:
/// - `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`
/// - `[target.'cfg(...)'.dependencies]` and the dev/build variants
/// - `[workspace.dependencies]` (root manifest only, included when present)
///
/// Returns `true` when at least one version pin was updated.
pub(crate) fn patch_workspace_dep_versions(
    cargo_toml_path: &str,
    new_version: &str,
    workspace_members: &std::collections::HashSet<String>,
) -> anyhow::Result<bool> {
    use toml_edit::{DocumentMut, Item};

    let content =
        std::fs::read_to_string(cargo_toml_path).with_context(|| format!("failed to read {cargo_toml_path}"))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse TOML in {cargo_toml_path}"))?;

    let mut changed = false;

    fn patch_dep_table(
        dep_table: &mut Item,
        new_version: &str,
        workspace_members: &std::collections::HashSet<String>,
    ) -> bool {
        let Some(table) = dep_table.as_table_like_mut() else {
            return false;
        };
        let mut any = false;
        for (key, item) in table.iter_mut() {
            let is_member = workspace_members.contains(key.get())
                || item
                    .as_table_like()
                    .and_then(|t| t.get("package"))
                    .and_then(|v| v.as_str())
                    .is_some_and(|pkg| workspace_members.contains(pkg));
            if !is_member {
                continue;
            }
            if let Some(inline) = item.as_table_like_mut()
                && let Some(ver_item) = inline.get_mut("version")
                && ver_item.as_str() != Some(new_version)
            {
                *ver_item = toml_edit::value(new_version);
                any = true;
            }
        }
        any
    }

    for table_key in &["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(item) = doc.get_mut(table_key)
            && patch_dep_table(item, new_version, workspace_members)
        {
            changed = true;
        }
    }

    if let Some(workspace) = doc.get_mut("workspace")
        && let Some(ws_table) = workspace.as_table_like_mut()
        && let Some(deps) = ws_table.get_mut("dependencies")
        && patch_dep_table(deps, new_version, workspace_members)
    {
        changed = true;
    }

    // Walk [target.'cfg(...)'.{dependencies,dev-dependencies,build-dependencies}].
    if let Some(target_item) = doc.get_mut("target")
        && let Some(target_table) = target_item.as_table_like_mut()
    {
        let cfg_keys: Vec<String> = target_table.iter().map(|(k, _)| k.to_string()).collect();
        for cfg_key in cfg_keys {
            if let Some(cfg_item) = target_table.get_mut(&cfg_key)
                && let Some(cfg_table) = cfg_item.as_table_like_mut()
            {
                for dep_key in &["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(dep_item) = cfg_table.get_mut(dep_key)
                        && patch_dep_table(dep_item, new_version, workspace_members)
                    {
                        changed = true;
                    }
                }
            }
        }
    }

    if changed {
        std::fs::write(cargo_toml_path, doc.to_string())
            .with_context(|| format!("failed to write updated dep versions to {cargo_toml_path}"))?;
    }

    Ok(changed)
}

/// Patch the `version = "..."` field inside a `[patch.crates-io]` entry in a
/// root `Cargo.toml`, when the entry belongs to the named crate.
///
/// Only entries that already carry a `version =` key are touched — path-only
/// entries (e.g. `sample_lib = { path = "crates/sample-lib" }`) are left intact.
///
/// Returns `true` when the version was updated, `false` when it was already
/// correct or no matching entry was found.
pub(crate) fn patch_cargo_crates_io_version(
    cargo_toml_path: &str,
    crate_name: &str,
    new_version: &str,
) -> anyhow::Result<bool> {
    use toml_edit::DocumentMut;

    let content =
        std::fs::read_to_string(cargo_toml_path).with_context(|| format!("failed to read {cargo_toml_path}"))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse TOML in {cargo_toml_path}"))?;

    let Some(patch) = doc.get_mut("patch") else {
        return Ok(false);
    };
    let Some(patch_table) = patch.as_table_like_mut() else {
        return Ok(false);
    };
    let Some(crates_io) = patch_table.get_mut("crates-io") else {
        return Ok(false);
    };
    let Some(crates_io_table) = crates_io.as_table_like_mut() else {
        return Ok(false);
    };
    let Some(entry) = crates_io_table.get_mut(crate_name) else {
        return Ok(false);
    };
    let Some(entry_table) = entry.as_table_like_mut() else {
        return Ok(false);
    };
    let Some(ver_item) = entry_table.get_mut("version") else {
        return Ok(false);
    };
    if ver_item.as_str() == Some(new_version) {
        return Ok(false);
    }
    *ver_item = toml_edit::value(new_version);
    std::fs::write(cargo_toml_path, doc.to_string())
        .with_context(|| format!("failed to write updated patch version to {cargo_toml_path}"))?;
    Ok(true)
}

/// Verify that all package manifest versions match the Cargo.toml source of truth.
///
/// Enumeration is config-driven: it delegates to
/// `commands::validate_versions::collect_checks` -- the same discovery `alef validate versions`
/// uses, which itself folds in `commands::version_manifests::collect` for `Cargo.lock`,
/// `.csproj`, Dart, and Zig checks. This replaced a hardcoded literal path list
/// (`packages/python/pyproject.toml`, `packages/node/package.json`, ...) that only ever checked
/// where alef's own packages happened to live, silently passing on any repo whose manifests live
/// elsewhere, and never read a lockfile at all.
///
/// Returns every check performed, including passing ones -- not just a mismatch list -- so a
/// caller can assert on the COUNT examined rather than trust a verdict alone: an enumerator that
/// silently matches nothing must not read the same as "all consistent". A crate for which this
/// enumeration finds nothing to compare against the canonical version is treated as a failure,
/// not a vacuous pass, mirroring `commands::validate_versions::run`'s identical guard.
pub fn verify_versions(
    config: &ResolvedCrateConfig,
    workspace_root: &Path,
) -> anyhow::Result<Vec<crate::cli::commands::validate_versions::VersionCheck>> {
    let canonical = config
        .resolved_version()
        .context("Cannot read canonical version from Cargo.toml (version_from)")?;
    let checks = crate::cli::commands::validate_versions::collect_checks(config, workspace_root, &canonical);
    if checks.is_empty() {
        anyhow::bail!(
            "version verification examined 0 manifests for crate `{}` (canonical {canonical}); \
             check that the configured package directories exist and are readable",
            config.name
        );
    }
    Ok(checks)
}

/// Set an explicit version in the Cargo.toml (supports pre-release versions like 0.1.0-rc.1).
pub fn set_version(config: &ResolvedCrateConfig, version: &str) -> anyhow::Result<()> {
    let changed = write_version_to_cargo_toml(&config.version_from, version)
        .with_context(|| format!("failed to set version to {version}"))?;
    if changed {
        info!("Set version to {version} in {}", config.version_from);
    } else {
        info!(
            "Version already set to {version} in {}; nothing to do",
            config.version_from
        );
    }
    Ok(())
}

#[cfg(test)]
mod verify_versions_tests {
    use super::*;
    use crate::cli::commands::validate_versions::checks_pass;
    use std::fs;
    use tempfile::TempDir;

    /// A minimal workspace with a canonical version at the root and a Python manifest that
    /// starts in sync with it -- neutral fixture name per `project-agnostic-codegen`.
    fn make_workspace(canonical: &str) -> TempDir {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            format!("[package]\nname = \"demo_lib\"\nversion = \"{canonical}\"\nedition = \"2024\"\n"),
        )
        .unwrap();
        fs::create_dir_all(root.join("packages/python")).unwrap();
        fs::write(
            root.join("packages/python/pyproject.toml"),
            format!("[project]\nname = \"demo_lib\"\nversion = \"{canonical}\"\n"),
        )
        .unwrap();
        tmp
    }

    fn minimal_config(root: &std::path::Path) -> ResolvedCrateConfig {
        let root_str = root.display().to_string().replace('\\', "/");
        let content = format!(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "demo_lib"
sources = ["src/lib.rs"]
version_from = "{root_str}/Cargo.toml"
"#,
        );
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(&content).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    /// THE prove-the-check-fired test for Phase 5: `alef verify`'s version-sync gate must
    /// actually be capable of failing on a real desync, not merely report "ok" because the
    /// config-driven enumerator silently examined nothing. Asserts on the COUNT of checks
    /// performed at every step, not only the pass/fail verdict -- a `0 checks` run and a
    /// genuine clean pass would otherwise be indistinguishable.
    #[test]
    fn verify_versions_fails_on_a_deliberate_desync_and_passes_once_resynced() {
        let tmp = make_workspace("1.0.0");
        let config = minimal_config(tmp.path());

        let synced = verify_versions(&config, tmp.path()).expect("a synced workspace must produce checks");
        assert!(
            !synced.is_empty(),
            "the config-driven enumerator must find at least one manifest to check"
        );
        let examined_count = synced.len();
        assert!(
            checks_pass(&synced),
            "a freshly-synced workspace must pass with every check matching: {synced:?}"
        );

        // Deliberately desync the Python manifest and prove the gate actually fails.
        fs::write(
            tmp.path().join("packages/python/pyproject.toml"),
            "[project]\nname = \"demo_lib\"\nversion = \"9.9.9\"\n",
        )
        .unwrap();
        let desynced = verify_versions(&config, tmp.path()).expect("a desynced manifest is still a set of checks");
        assert_eq!(
            desynced.len(),
            examined_count,
            "desyncing one manifest's version string must not change how many are examined"
        );
        assert!(
            !checks_pass(&desynced),
            "a deliberately desynced pyproject.toml must fail the version-sync gate: {desynced:?}"
        );
        let mismatch = desynced
            .iter()
            .find(|check| check.label.contains("pyproject.toml"))
            .expect("pyproject.toml must be among the examined checks");
        assert!(!mismatch.matches, "pyproject.toml must be reported as a mismatch");
        assert_eq!(mismatch.found.as_deref(), Some("9.9.9"));

        // Resync and prove the gate passes again, with the same check count.
        fs::write(
            tmp.path().join("packages/python/pyproject.toml"),
            "[project]\nname = \"demo_lib\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let resynced = verify_versions(&config, tmp.path()).expect("a re-synced workspace must produce checks");
        assert_eq!(
            resynced.len(),
            examined_count,
            "resyncing must not change the examined count"
        );
        assert!(
            checks_pass(&resynced),
            "re-syncing the manifest must make the gate pass again"
        );
    }

    /// The exact regression this whole phase exists to prevent: a config-driven enumerator
    /// that finds nothing to compare against the canonical version must be a hard failure, not
    /// a silent "all consistent" the way the hardcoded path list used to produce for any repo
    /// whose manifests it didn't happen to name.
    #[test]
    fn verify_versions_fails_when_the_enumerator_examines_nothing() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo_lib\"\nversion = \"1.0.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        let config = minimal_config(root);
        let error = verify_versions(&config, root).expect_err("zero manifests examined must error, not vacuously pass");
        assert!(
            error.to_string().contains("0 manifests"),
            "the error must name the empty examination: {error}"
        );
    }

    /// The hardcoded list this replaced never read `Cargo.lock` at all (see the module-level
    /// defect this phase fixes). Proves the config-driven enumerator now includes it, and that
    /// a lockfile pinned at the wrong version fails the gate exactly like any other manifest.
    #[test]
    fn verify_versions_covers_cargo_lock_which_the_old_hardcoded_list_never_read() {
        let tmp = make_workspace("1.0.0");
        let root = tmp.path();
        fs::write(
            root.join("Cargo.lock"),
            "# This file is automatically @generated by Cargo.\nversion = 4\n\n[[package]]\nname = \"demo_lib\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        let config = minimal_config(root);

        let checks = verify_versions(&config, root).expect("checks must be produced");
        assert!(
            checks.iter().any(|check| check.label.contains("Cargo.lock")),
            "Cargo.lock must be part of the enumerated checks: {checks:?}"
        );
        assert!(
            checks_pass(&checks),
            "a Cargo.lock pinned at the canonical version must pass: {checks:?}"
        );

        fs::write(
            root.join("Cargo.lock"),
            "# This file is automatically @generated by Cargo.\nversion = 4\n\n[[package]]\nname = \"demo_lib\"\nversion = \"0.0.1\"\n",
        )
        .unwrap();
        let desynced = verify_versions(&config, root).expect("checks must still be produced");
        assert!(
            !checks_pass(&desynced),
            "a Cargo.lock pinned at the wrong version must fail the gate: {desynced:?}"
        );
    }
}
