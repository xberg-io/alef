//! Check the alef version pin in `alef.toml` against the running alef CLI, and
//! optionally rewrite it.
//!
//! Every alef.toml may carry a `[workspace] alef_version = "X.Y.Z"` field that
//! records the alef CLI version a project expects. Generation compares the pin to
//! the running CLI and warns on any mismatch, but version synchronization remains
//! an explicit release operation by default.
//!
//! [`maybe_update_alef_toml_version_pin`] is the opt-in exception: a caller that has
//! decided to allow it (an explicit flag or config toggle -- this module makes no
//! policy decision about which) can have the pin rewritten automatically, but only
//! when every one of its safety conditions holds. See that function's doc for why the
//! gate is this strict: `install-alef`'s `resolve.sh` reads the pin verbatim to fetch a
//! GitHub release asset, so an incorrectly rewritten pin does not fail loudly at
//! generation time -- it fails a consumer's CI at the install step, on every push,
//! until someone notices. ~keep

use crate::core::config::WorkspaceConfig;
use anyhow::{Context, Result};

/// CLI version baked in at compile time.
pub fn cli_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Compare `workspace.alef_version` against the running CLI and log the direction
/// of any change. Never errors: upgrades and downgrades are warned,
/// an equal or missing pin is silent.
pub fn check_alef_toml_version(workspace: &WorkspaceConfig) -> Result<()> {
    let Some(pin) = workspace.alef_version.as_deref() else {
        return Ok(());
    };
    let cli = cli_version();
    let (Ok(pin_v), Ok(cli_v)) = (semver::Version::parse(pin), semver::Version::parse(cli)) else {
        tracing::warn!(
            "alef.toml `[workspace] alef_version = \"{pin}\"` is not valid semver; running alef {cli}, \
             pin left unchanged"
        );
        return Ok(());
    };

    match cli_v.cmp(&pin_v) {
        std::cmp::Ordering::Greater => {
            // The pin drifting behind the running CLI is expected after every alef release
            // until a consumer bumps it; nothing here is actionable. ~keep
            tracing::info!(
                "alef {cli} is newer than alef.toml's alef_version pin {pin}; generation will not change the pin"
            );
        }
        std::cmp::Ordering::Less => {
            // Same as the newer-CLI branch above: an unbumped pin is expected, not actionable. ~keep
            tracing::info!(
                "alef {cli} is older than alef.toml's alef_version pin {pin}; generation will not change the pin"
            );
        }
        std::cmp::Ordering::Equal => {}
    }
    Ok(())
}

/// Rewrite `alef.toml`'s `[workspace] alef_version` pin to the running CLI's version,
/// but only when every one of the following holds. Returns `Ok(true)` iff the file was
/// rewritten, `Ok(false)` otherwise (nothing here ever errors just because a condition
/// was not met).
///
/// - `auto_update_enabled` is `true`. This function makes no policy decision of its
///   own about when auto-update should be allowed -- the caller passes the already-
///   resolved opt-in (e.g. an explicit CLI flag or a workspace config toggle).
/// - The pin is present and parses as semver.
/// - The running CLI is strictly *newer* than the pin. This never downgrades and never
///   rewrites an already-equal pin.
/// - `running_build_is_clean` is `true`. `install-alef`'s `resolve.sh` fetches a GitHub
///   release asset named after the pin verbatim, so writing a version built from a
///   dirty (or otherwise unreproducible) tree can point every consumer's next CI run at
///   a release asset that does not exist -- a failure that surfaces at the install step
///   of a consumer's pipeline, not here. The caller is expected to pass whether the
///   running binary's own build was stamped clean (see `bin_cli::build_info`); this
///   module deliberately takes that as a plain `bool` rather than reading build
///   provenance itself, so the gate is exercised the same way in a unit test as it is
///   at the CLI's call site. A clean tree is still not proof the exact commit was
///   released, which is why this stays behind `auto_update_enabled` too rather than
///   defaulting to on. ~keep
pub fn maybe_update_alef_toml_version_pin(
    workspace: &WorkspaceConfig,
    config_path: &std::path::Path,
    auto_update_enabled: bool,
    running_build_is_clean: bool,
) -> Result<bool> {
    if !auto_update_enabled {
        return Ok(false);
    }
    let Some(pin) = workspace.alef_version.as_deref() else {
        return Ok(false);
    };
    let cli = cli_version();
    let (Ok(pin_v), Ok(cli_v)) = (semver::Version::parse(pin), semver::Version::parse(cli)) else {
        return Ok(false);
    };
    if cli_v <= pin_v {
        return Ok(false);
    }
    if !running_build_is_clean {
        tracing::debug!(
            "alef {cli} is newer than the pinned alef_version {pin} in alef.toml, but this \
             build's working tree was not clean at compile time; leaving the pin unchanged"
        );
        return Ok(false);
    }
    write_alef_version_pin(config_path, cli)?;
    tracing::info!("Updated alef.toml `[workspace] alef_version` pin from {pin} to {cli}");
    Ok(true)
}

/// Surgically set `[workspace] alef_version` in the `alef.toml` at `config_path`,
/// leaving every other key, comment, and formatting choice in the file untouched.
fn write_alef_version_pin(config_path: &std::path::Path, version: &str) -> Result<()> {
    let content = std::fs::read_to_string(config_path)
        .with_context(|| format!("reading {} to update the alef_version pin", config_path.display()))?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {} to update the alef_version pin", config_path.display()))?;
    let workspace_item = doc.entry("workspace").or_insert(toml_edit::table());
    let Some(workspace_table) = workspace_item.as_table_mut() else {
        anyhow::bail!(
            "{} has a non-table [workspace] entry; cannot update alef_version",
            config_path.display()
        );
    };
    workspace_table["alef_version"] = toml_edit::value(version);
    std::fs::write(config_path, doc.to_string())
        .with_context(|| format!("writing {} after updating the alef_version pin", config_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

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
    #[traced_test]
    fn pin_lower_than_cli_reports_that_generation_preserves_pin() {
        let ws = workspace_with_version(Some("0.0.1"));
        assert!(check_alef_toml_version(&ws).is_ok());
        assert!(logs_contain("generation will not change the pin"));
    }

    #[test]
    fn pin_higher_than_cli_reports_and_does_not_error() {
        let ws = workspace_with_version(Some("999.0.0"));
        assert!(
            check_alef_toml_version(&ws).is_ok(),
            "a downgrade must warn, not hard-error"
        );
    }

    #[test]
    fn pin_invalid_semver_warns_not_errors() {
        let ws = workspace_with_version(Some("not-a-version"));
        assert!(
            check_alef_toml_version(&ws).is_ok(),
            "an unparseable pin must warn and continue, not error"
        );
    }

    #[test]
    fn version_check_does_not_rewrite_external_pin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        let config = "languages = []\nalef_version = \"0.0.1\"\n";
        std::fs::write(&path, config).expect("write fixture");
        let workspace: WorkspaceConfig = toml::from_str(config).expect("parse fixture");

        check_alef_toml_version(&workspace).expect("check pin");

        assert_eq!(std::fs::read_to_string(path).expect("read fixture"), config);
    }

    /// Fixture matching a real `alef.toml`'s `[workspace]` table shape (not the flat shape
    /// `WorkspaceConfig` itself deserializes from -- see `workspace_with_version`), since
    /// [`write_alef_version_pin`] edits the on-disk `[workspace]` table.
    fn write_fixture_alef_toml(dir: &std::path::Path, pin: &str) -> std::path::PathBuf {
        let path = dir.join("alef.toml");
        let config = format!("[workspace]\nalef_version = \"{pin}\"\nlanguages = []\n");
        std::fs::write(&path, config).expect("write fixture");
        path
    }

    /// REGRESSION (half one): a plain regen from a dev/dirty tree must never rewrite the pin,
    /// even when the caller has opted in and the running CLI is newer than the pin. Revert the
    /// `running_build_is_clean` gate in `maybe_update_alef_toml_version_pin` (e.g. delete the
    /// `if !running_build_is_clean` early return) and this test fails: the file is rewritten.
    #[test]
    fn auto_update_pin_does_not_write_from_a_dirty_build() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_fixture_alef_toml(dir.path(), "0.0.1");
        let workspace = workspace_with_version(Some("0.0.1"));
        let before = std::fs::read_to_string(&path).expect("read fixture");

        let wrote =
            maybe_update_alef_toml_version_pin(&workspace, &path, true, false).expect("dirty build must not error");

        assert!(!wrote, "a dirty build must report no write");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read fixture"),
            before,
            "a dirty build must never rewrite the pin"
        );
    }

    /// A plain regen also never opts in on its own: even on a clean build, `auto_update_enabled
    /// == false` (the default until a caller wires a flag or config toggle) must not write.
    #[test]
    fn auto_update_pin_does_not_write_when_not_opted_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_fixture_alef_toml(dir.path(), "0.0.1");
        let workspace = workspace_with_version(Some("0.0.1"));
        let before = std::fs::read_to_string(&path).expect("read fixture");

        let wrote = maybe_update_alef_toml_version_pin(&workspace, &path, false, true).expect("opt-out must not error");

        assert!(!wrote, "no opt-in must report no write");
        assert_eq!(std::fs::read_to_string(&path).expect("read fixture"), before);
    }

    /// REGRESSION (half two, the negative control): under every required condition -- opted in,
    /// clean build, running CLI strictly newer than the pin -- the pin DOES get rewritten. Without
    /// this test, half one could pass with the feature entirely inert (e.g. the function always
    /// returning `Ok(false)`). Revert `maybe_update_alef_toml_version_pin` to always return
    /// `Ok(false)`, or delete the call to `write_alef_version_pin`, and this test fails: `wrote`
    /// is `false` and the file's pin stays `0.0.1`.
    #[test]
    fn auto_update_pin_writes_when_opted_in_clean_and_newer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_fixture_alef_toml(dir.path(), "0.0.1");
        let workspace = workspace_with_version(Some("0.0.1"));

        let wrote =
            maybe_update_alef_toml_version_pin(&workspace, &path, true, true).expect("an eligible bump must not error");

        assert!(wrote, "an eligible bump must report that it wrote");
        let after = std::fs::read_to_string(&path).expect("read fixture");
        assert!(
            after.contains(&format!("alef_version = \"{}\"", cli_version())),
            "pin must be rewritten to the running CLI version:\n{after}"
        );
        assert!(
            after.contains("languages = []"),
            "unrelated keys must survive the surgical edit:\n{after}"
        );
    }

    /// Never downgrades and never rewrites an already-equal pin, even fully opted in and clean.
    #[test]
    fn auto_update_pin_does_not_write_when_pin_is_not_older_than_cli() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_fixture_alef_toml(dir.path(), cli_version());
        let workspace = workspace_with_version(Some(cli_version()));
        let before = std::fs::read_to_string(&path).expect("read fixture");

        let wrote =
            maybe_update_alef_toml_version_pin(&workspace, &path, true, true).expect("equal pin must not error");

        assert!(!wrote, "an equal pin must not be rewritten");
        assert_eq!(std::fs::read_to_string(&path).expect("read fixture"), before);
    }

    /// An unparseable pin is left alone rather than guessed at.
    #[test]
    fn auto_update_pin_does_not_write_for_invalid_semver() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_fixture_alef_toml(dir.path(), "not-a-version");
        let workspace = workspace_with_version(Some("not-a-version"));
        let before = std::fs::read_to_string(&path).expect("read fixture");

        let wrote =
            maybe_update_alef_toml_version_pin(&workspace, &path, true, true).expect("invalid semver must not error");

        assert!(!wrote);
        assert_eq!(std::fs::read_to_string(&path).expect("read fixture"), before);
    }

    /// No pin set at all: nothing to update.
    #[test]
    fn auto_update_pin_does_not_write_when_pin_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("alef.toml");
        std::fs::write(&path, "languages = []\n").expect("write fixture");
        let workspace = workspace_with_version(None);

        let wrote =
            maybe_update_alef_toml_version_pin(&workspace, &path, true, true).expect("missing pin must not error");

        assert!(!wrote);
    }
}
