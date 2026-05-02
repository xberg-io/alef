//! Enforce and synchronise the `[workspace] alef_version` pin in `alef.toml`.
//!
//! Every alef.toml may carry a `[workspace] alef_version = "X.Y.Z"` field that
//! pins the alef CLI version a project expects. Two invariants:
//!
//! 1. The pinned version must never be greater than the running CLI version.
//!    Trying to regenerate with an older binary against a config that already
//!    moved forward is a recipe for partial output and missing features.
//! 2. After a successful generate, the pin is rewritten to the CLI version so
//!    install-alef and downstream consumers know exactly which alef produced
//!    the on-disk output.
//!
//! The two functions in this module are the only entry points used by the
//! `Generate`, `All`, and `Init` command handlers.

use alef_core::config::WorkspaceConfig;
use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{DocumentMut, Item, value};

/// CLI version baked in at compile time.
pub fn cli_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Error if `workspace.alef_version` is set and > CLI version.
///
/// A missing pin is treated as compatible (no constraint).
pub fn check_alef_toml_version(workspace: &WorkspaceConfig) -> Result<()> {
    let Some(pin) = workspace.alef_version.as_deref() else {
        return Ok(());
    };
    let cli = cli_version();
    let pin_v = semver::Version::parse(pin).with_context(|| {
        format!(
            "alef.toml `[workspace] alef_version = \"{pin}\"` is not a valid semver — expected MAJOR.MINOR.PATCH[-prerelease]"
        )
    })?;
    let cli_v =
        semver::Version::parse(cli).with_context(|| format!("CLI version {cli} is not a valid semver (impossible)"))?;

    if pin_v > cli_v {
        anyhow::bail!(
            "alef.toml pins `[workspace] alef_version = \"{pin}\"` but installed alef CLI is {cli}. \
             Upgrade alef (cargo install alef-cli --version {pin}) before re-running."
        );
    }
    Ok(())
}

/// Rewrite (or insert) `[workspace] alef_version = "..."` in alef.toml so it
/// matches the running CLI. No-op if the field already holds the CLI version.
///
/// The new-schema field lives under `[workspace]`. A top-level `version = "..."`
/// would be flagged as legacy by [`alef_core::config::detect_legacy_keys`].
pub fn write_alef_toml_version(config_path: &Path) -> Result<()> {
    let cli = cli_version();
    let content =
        std::fs::read_to_string(config_path).with_context(|| format!("failed to read {}", config_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse {} as TOML", config_path.display()))?;

    // Ensure `[workspace]` exists as a Table (not an inline table).
    if !doc.contains_key("workspace") {
        let mut tbl = toml_edit::Table::new();
        tbl.set_implicit(false);
        doc.insert("workspace", Item::Table(tbl));
    }
    let workspace = doc["workspace"]
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[workspace] in {} is not a table", config_path.display()))?;

    let already_current = workspace
        .get("alef_version")
        .and_then(|v| v.as_str())
        .map(|s| s == cli)
        .unwrap_or(false);
    if already_current {
        return Ok(());
    }

    workspace["alef_version"] = value(cli);

    let new_content = doc.to_string();
    std::fs::write(config_path, &new_content).with_context(|| format!("failed to write {}", config_path.display()))?;
    tracing::info!("Updated {} `[workspace] alef_version` to {cli}", config_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn workspace_with_version(v: Option<&str>) -> WorkspaceConfig {
        let mut toml = String::new();
        if let Some(version) = v {
            toml.push_str(&format!("alef_version = \"{version}\"\n"));
        }
        toml::from_str(&toml).expect("valid workspace config")
    }

    #[test]
    fn missing_pin_is_compatible() {
        let ws = workspace_with_version(None);
        assert!(check_alef_toml_version(&ws).is_ok());
    }

    #[test]
    fn pin_equal_to_cli_passes() {
        let ws = workspace_with_version(Some(cli_version()));
        assert!(check_alef_toml_version(&ws).is_ok());
    }

    #[test]
    fn pin_lower_than_cli_passes() {
        let ws = workspace_with_version(Some("0.0.1"));
        assert!(check_alef_toml_version(&ws).is_ok());
    }

    #[test]
    fn pin_higher_than_cli_errors() {
        // Bump the major past anything the CLI could plausibly be.
        let ws = workspace_with_version(Some("999.0.0"));
        let err = check_alef_toml_version(&ws).expect_err("higher pin must reject");
        let msg = format!("{err}");
        assert!(msg.contains("999.0.0"), "error must mention the offending pin: {msg}");
        assert!(msg.contains(cli_version()), "error must mention the CLI version: {msg}");
    }

    #[test]
    fn pin_invalid_semver_errors() {
        let ws = workspace_with_version(Some("not-a-version"));
        assert!(check_alef_toml_version(&ws).is_err());
    }

    #[test]
    fn write_replaces_existing_workspace_alef_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        fs::write(
            &path,
            "[workspace]\nalef_version = \"0.0.1\"\nlanguages = []\n\n[[crates]]\nname = \"x\"\nsources = []\n",
        )
        .unwrap();

        write_alef_toml_version(&path).expect("write ok");
        let updated = fs::read_to_string(&path).unwrap();
        assert!(
            updated.contains(&format!("alef_version = \"{}\"", cli_version())),
            "alef.toml must contain CLI version after write: {updated}"
        );
        assert!(!updated.contains("0.0.1"), "old version must be gone: {updated}");
    }

    #[test]
    fn write_inserts_pin_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        fs::write(
            &path,
            "[workspace]\nlanguages = []\n\n[[crates]]\nname = \"x\"\nsources = []\n",
        )
        .unwrap();

        write_alef_toml_version(&path).expect("write ok");
        let updated = fs::read_to_string(&path).unwrap();
        assert!(
            updated.contains(&format!("alef_version = \"{}\"", cli_version())),
            "pin must appear in [workspace]: {updated}"
        );
    }

    #[test]
    fn write_creates_workspace_section_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        fs::write(&path, "[[crates]]\nname = \"x\"\nsources = []\n").unwrap();

        write_alef_toml_version(&path).expect("write ok");
        let updated = fs::read_to_string(&path).unwrap();
        assert!(
            updated.contains("[workspace]"),
            "[workspace] must be inserted: {updated}"
        );
        assert!(
            updated.contains(&format!("alef_version = \"{}\"", cli_version())),
            "alef_version must be set under [workspace]: {updated}"
        );
    }

    #[test]
    fn write_does_not_clobber_dependency_version_specs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        fs::write(
            &path,
            "[workspace]\nalef_version = \"0.0.1\"\nlanguages = []\n\n[[crates]]\nname = \"x\"\nsources = []\n\n[crates.extra_dependencies.something]\nversion = \"1.2.3\"\n",
        )
        .unwrap();

        write_alef_toml_version(&path).expect("write ok");
        let updated = fs::read_to_string(&path).unwrap();
        assert!(
            updated.contains("version = \"1.2.3\""),
            "dependency version under [crates.extra_dependencies.something] must be untouched: {updated}"
        );
        assert!(
            !updated.contains("alef_version = \"0.0.1\""),
            "old alef_version must be replaced: {updated}"
        );
    }
}
