//! Enforce and synchronise the `version` pin in `alef.toml`.
//!
//! Every alef.toml may carry a top-level `version = "X.Y.Z"` line that pins the
//! alef CLI version a project expects. Two invariants:
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

use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::LazyLock;

/// CLI version baked in at compile time.
pub fn cli_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Error if `config.version` is set and > CLI version.
///
/// A missing pin is treated as compatible (no constraint).
pub fn check_alef_toml_version(config: &AlefConfig) -> Result<()> {
    let Some(pin) = config.version.as_deref() else {
        return Ok(());
    };
    let cli = cli_version();
    let pin_v = semver::Version::parse(pin).with_context(|| {
        format!("alef.toml `version = \"{pin}\"` is not a valid semver — expected MAJOR.MINOR.PATCH[-prerelease]")
    })?;
    let cli_v =
        semver::Version::parse(cli).with_context(|| format!("CLI version {cli} is not a valid semver (impossible)"))?;

    if pin_v > cli_v {
        anyhow::bail!(
            "alef.toml pins version = \"{pin}\" but installed alef CLI is {cli}. \
             Upgrade alef (cargo install alef-cli --version {pin}) before re-running."
        );
    }
    Ok(())
}

static VERSION_LINE_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#"(?m)^version\s*=\s*"[^"]*""#).expect("valid regex"));

/// Rewrite (or insert) the top-level `version = "..."` line in alef.toml so it
/// matches the running CLI. No-op if the file already pins the same version.
pub fn write_alef_toml_version(config_path: &Path) -> Result<()> {
    let cli = cli_version();
    let content =
        std::fs::read_to_string(config_path).with_context(|| format!("failed to read {}", config_path.display()))?;

    let new_content = if VERSION_LINE_RE.is_match(&content) {
        VERSION_LINE_RE
            .replace(&content, format!(r#"version = "{cli}""#).as_str())
            .to_string()
    } else {
        // No top-level pin yet — prepend one. Keep a blank line between the
        // pin and whatever follows so existing structure is preserved.
        let separator = if content.starts_with('\n') { "" } else { "\n" };
        format!("version = \"{cli}\"\n{separator}{content}")
    };

    if new_content != content {
        std::fs::write(config_path, new_content)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
        tracing::info!("Updated {} version pin to {cli}", config_path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cfg_with_version(v: Option<&str>) -> AlefConfig {
        let mut toml = String::new();
        if let Some(version) = v {
            toml.push_str(&format!("version = \"{version}\"\n"));
        }
        toml.push_str(
            r#"
languages = []

[crate]
name = "stub"
sources = ["src/lib.rs"]
version_from = "Cargo.toml"
"#,
        );
        toml::from_str(&toml).expect("valid stub config")
    }

    #[test]
    fn missing_pin_is_compatible() {
        let config = cfg_with_version(None);
        assert!(check_alef_toml_version(&config).is_ok());
    }

    #[test]
    fn pin_equal_to_cli_passes() {
        let config = cfg_with_version(Some(cli_version()));
        assert!(check_alef_toml_version(&config).is_ok());
    }

    #[test]
    fn pin_lower_than_cli_passes() {
        let config = cfg_with_version(Some("0.0.1"));
        assert!(check_alef_toml_version(&config).is_ok());
    }

    #[test]
    fn pin_higher_than_cli_errors() {
        // Bump the major past anything the CLI could plausibly be.
        let config = cfg_with_version(Some("999.0.0"));
        let err = check_alef_toml_version(&config).expect_err("higher pin must reject");
        let msg = format!("{err}");
        assert!(msg.contains("999.0.0"), "error must mention the offending pin: {msg}");
        assert!(msg.contains(cli_version()), "error must mention the CLI version: {msg}");
    }

    #[test]
    fn pin_invalid_semver_errors() {
        let config = cfg_with_version(Some("not-a-version"));
        assert!(check_alef_toml_version(&config).is_err());
    }

    #[test]
    fn write_replaces_existing_top_level_version() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        fs::write(
            &path,
            "version = \"0.0.1\"\nlanguages = []\n\n[crate]\nname = \"x\"\nsources = []\nversion_from = \"Cargo.toml\"\n",
        )
        .unwrap();

        write_alef_toml_version(&path).expect("write ok");
        let updated = fs::read_to_string(&path).unwrap();
        assert!(
            updated.contains(&format!("version = \"{}\"", cli_version())),
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
            "languages = []\n\n[crate]\nname = \"x\"\nsources = []\nversion_from = \"Cargo.toml\"\n",
        )
        .unwrap();

        write_alef_toml_version(&path).expect("write ok");
        let updated = fs::read_to_string(&path).unwrap();
        assert!(
            updated.starts_with(&format!("version = \"{}\"", cli_version())),
            "pin must be prepended to alef.toml: {updated}"
        );
        // Existing structure must remain parseable.
        let _: AlefConfig = toml::from_str(&updated).expect("post-write config still parses");
    }

    #[test]
    fn write_does_not_clobber_dependency_version_specs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        // A dependency-style `version = "1.2.3"` indented inside an inline table
        // would NOT be at start-of-line, so the line-anchored regex must skip it.
        fs::write(
            &path,
            "version = \"0.0.1\"\nlanguages = []\n\n[crate]\nname = \"x\"\nsources = []\nversion_from = \"Cargo.toml\"\n\n[dependencies.something]\n  version = \"1.2.3\"\n",
        )
        .unwrap();

        write_alef_toml_version(&path).expect("write ok");
        let updated = fs::read_to_string(&path).unwrap();
        assert!(
            updated.contains("version = \"1.2.3\""),
            "indented dependency version must be untouched: {updated}"
        );
    }
}
