//! Synchronise the alef version pin in `alef.toml` and the `.pre-commit-config.yaml`
//! alef hook `rev:` with the running alef CLI.
//!
//! Every alef.toml may carry a `[workspace] alef_version = "X.Y.Z"` field that
//! records the alef CLI version a project was last generated with. The generation
//! commands (`generate`, `all`, `scaffold`) make the running CLI the source of
//! truth:
//!
//! 1. [`check_alef_toml_version`] compares the pin to the running CLI and logs an
//!    INFO on upgrade (running newer) or a WARN on downgrade (running older). It
//!    never errors — regenerating with an older binary is allowed, just flagged.
//! 2. [`write_alef_toml_version`] rewrites the pin to the CLI version so install-alef
//!    and downstream consumers know exactly which alef produced the on-disk output.
//! 3. [`sync_precommit_alef_rev`] bumps the alef hook `rev:` in the consumer's
//!    `.pre-commit-config.yaml` to `v{cli}` so the pre-commit hook tracks the same
//!    version.

use crate::core::config::WorkspaceConfig;
use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{DocumentMut, Item, value};

/// CLI version baked in at compile time.
pub fn cli_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Compare `workspace.alef_version` against the running CLI and log the direction
/// of any change. Never errors: a downgrade is warned, an upgrade is info-logged,
/// an equal/missing/unparseable pin is silent (the pin is reconciled later by
/// [`write_alef_toml_version`]).
pub fn check_alef_toml_version(workspace: &WorkspaceConfig) -> Result<()> {
    let Some(pin) = workspace.alef_version.as_deref() else {
        return Ok(());
    };
    let cli = cli_version();
    let (Ok(pin_v), Ok(cli_v)) = (semver::Version::parse(pin), semver::Version::parse(cli)) else {
        tracing::warn!(
            "alef.toml `[workspace] alef_version = \"{pin}\"` is not valid semver; it will be reset to the running CLI version {cli}"
        );
        return Ok(());
    };

    match cli_v.cmp(&pin_v) {
        std::cmp::Ordering::Greater => {
            tracing::info!("Upgrading alef pin {pin} → {cli} (running a newer alef)");
        }
        std::cmp::Ordering::Less => {
            tracing::warn!(
                "Running alef {cli} is older than the pinned alef_version {pin} in alef.toml; \
                 the pin will be lowered to {cli}"
            );
        }
        std::cmp::Ordering::Equal => {}
    }
    Ok(())
}

/// Rewrite (or insert) `[workspace] alef_version = "..."` in alef.toml so it
/// matches the running CLI. No-op if the field already holds the CLI version.
///
/// The new-schema field lives under `[workspace]`. A top-level `version = "..."`
/// would be flagged as legacy by [`crate::core::config::detect_legacy_keys`].
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

/// Heuristic for identifying the alef hooks repo on a `.pre-commit-config.yaml`
/// `- repo:` line. We match any URL whose path ends in `/alef` or `/alef.git`
/// (case-insensitive) so the detection works regardless of which organization
/// hosts the fork. Alef itself stays project-agnostic — downstream forks under
/// different orgs get the same self-sync behaviour for free.
fn is_alef_hooks_repo_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("- repo:") {
        return false;
    }
    let Some((_, url)) = trimmed.split_once(':') else {
        return false;
    };
    let url = url.trim().trim_end_matches('/').to_ascii_lowercase();
    let path = url.strip_suffix(".git").unwrap_or(&url);
    path.rsplit_once('/').is_some_and(|(_, tail)| tail == "alef")
}

/// Bump the alef hook `rev:` in `<repo_root>/.pre-commit-config.yaml` to `v{cli}`.
///
/// Surgical, format-preserving line edit: only the `rev:` line of the alef hook
/// block is rewritten; the rest of the file (other hooks, comments, spacing) is
/// untouched. No-ops silently when the file is absent, when the alef hooks are
/// `local` (no `rev:`), or when the rev already matches. Quote style of the
/// existing value is preserved.
pub fn sync_precommit_alef_rev(repo_root: &Path) -> Result<()> {
    let path = repo_root.join(".pre-commit-config.yaml");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Ok(()); // absent or unreadable — nothing to sync
    };
    let cli = cli_version();
    let new_rev = format!("v{cli}");

    let mut lines: Vec<String> = content.lines().map(str::to_owned).collect();

    // Locate the alef hooks repo block.
    let Some(repo_idx) = lines.iter().position(|l| is_alef_hooks_repo_line(l)) else {
        return Ok(()); // no remote alef hook block (e.g. local hooks) — nothing to do
    };

    // Find the block's `rev:` line, before the next `- repo:` entry.
    let mut rev_idx = None;
    for (idx, line) in lines.iter().enumerate().skip(repo_idx + 1) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- repo:") {
            break;
        }
        if trimmed.starts_with("rev:") {
            rev_idx = Some(idx);
            break;
        }
    }
    let Some(rev_idx) = rev_idx else {
        return Ok(()); // local hooks / no pinned rev
    };

    let line = &lines[rev_idx];
    let indent_len = line.len() - line.trim_start().len();
    let indent = line[..indent_len].to_owned();
    let raw_value = line.trim_start().strip_prefix("rev:").unwrap_or("").trim();
    let quoted = raw_value.starts_with('"');
    let old_rev = raw_value.trim_matches('"').to_owned();

    if old_rev == new_rev {
        return Ok(()); // already current
    }

    // Log the direction of the change (best-effort semver compare, sans `v`).
    let parse = |s: &str| semver::Version::parse(s.trim_start_matches('v')).ok();
    match (parse(&old_rev), parse(cli)) {
        (Some(old), Some(new)) if new > old => {
            tracing::info!(
                "Upgrading alef pre-commit hook rev {old_rev} → {new_rev} in {}",
                path.display()
            );
        }
        (Some(old), Some(new)) if new < old => {
            tracing::warn!(
                "Lowering alef pre-commit hook rev {old_rev} → {new_rev} in {} (running an older alef)",
                path.display()
            );
        }
        _ => {}
    }

    let new_value = if quoted { format!("\"{new_rev}\"") } else { new_rev };
    lines[rev_idx] = format!("{indent}rev: {new_value}");

    let mut out = lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&path, out).with_context(|| format!("failed to write {}", path.display()))?;
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
    fn pin_higher_than_cli_warns_not_errors() {
        // A pin newer than the running CLI (downgrade) is allowed — warned, not rejected.
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

    #[test]
    fn precommit_sync_bumps_alef_rev_and_leaves_other_repos_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".pre-commit-config.yaml");
        fs::write(
            &path,
            "repos:\n  - repo: https://github.com/astral-sh/ruff-pre-commit\n    rev: v0.14.0\n    hooks:\n      - id: ruff\n  - repo: https://github.com/sample_crate-dev/alef\n    rev: v0.0.1\n    hooks:\n      - id: alef-verify\n      - id: alef-sync-versions\n",
        )
        .unwrap();

        sync_precommit_alef_rev(dir.path()).expect("sync ok");
        let updated = fs::read_to_string(&path).unwrap();

        assert!(
            updated.contains(&format!("rev: v{}", cli_version())),
            "alef hook rev must be bumped to v<cli>: {updated}"
        );
        assert!(!updated.contains("rev: v0.0.1"), "old alef rev must be gone: {updated}");
        assert!(
            updated.contains("rev: v0.14.0"),
            "the ruff hook's rev must be untouched: {updated}"
        );
    }

    #[test]
    fn precommit_sync_preserves_quoted_rev_style() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".pre-commit-config.yaml");
        fs::write(
            &path,
            "repos:\n  - repo: https://github.com/sample_crate-dev/alef\n    rev: \"v0.0.1\"\n    hooks:\n      - id: alef-verify\n",
        )
        .unwrap();

        sync_precommit_alef_rev(dir.path()).expect("sync ok");
        let updated = fs::read_to_string(&path).unwrap();
        assert!(
            updated.contains(&format!("rev: \"v{}\"", cli_version())),
            "quoted rev style must be preserved: {updated}"
        );
    }

    #[test]
    fn precommit_sync_noops_when_absent_or_local() {
        // Absent file: no error.
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(sync_precommit_alef_rev(dir.path()).is_ok());

        // Local alef hooks (no remote repo / no rev): file left unchanged.
        let path = dir.path().join(".pre-commit-config.yaml");
        let local = "repos:\n  - repo: local\n    hooks:\n      - id: alef-verify\n        entry: alef verify\n        language: system\n";
        fs::write(&path, local).unwrap();
        sync_precommit_alef_rev(dir.path()).expect("sync ok");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            local,
            "local-hook config must be untouched"
        );
    }
}
