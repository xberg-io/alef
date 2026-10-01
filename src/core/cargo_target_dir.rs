//! Resolve cargo's effective build-output directory instead of assuming `<workspace>/target`.
//!
//! Cargo picks that directory in a fixed order: the `--target-dir` flag, then the
//! `CARGO_TARGET_DIR` environment variable, then `build.target-dir` from the nearest
//! `.cargo/config.toml`, then `<workspace root>/target`. Only the last of those is what a
//! hand-written `workspace_root.join("target")` finds, so every alef call site that reads build
//! output off disk used to go blind the moment a host redirected the output elsewhere -- and
//! going blind is silent: the directory scan simply matches nothing, which is indistinguishable
//! from "the build produced nothing" (alef #477, where `alef generate --lang swift` under an
//! external `CARGO_TARGET_DIR` reported a successful re-materialization and left the committed
//! Swift bridge files stale).
//!
//! `cargo metadata`'s `target_directory` field is the authoritative answer -- it is cargo itself
//! reporting the winner of that precedence race -- so [`resolve`] asks cargo rather than
//! re-deriving the rules here.
//!
//! Two things about the lookup are easy to get wrong and are load-bearing for callers:
//!
//! - Cargo discovers `.cargo/config.toml` by walking up from the **current working directory**,
//!   not from `--manifest-path`. [`resolve`] therefore pins the metadata child to the manifest's
//!   directory instead of inheriting ambient process state.
//! - `CARGO_TARGET_DIR` reaches the child through the inherited environment, so the ambient value
//!   is honoured for free; [`resolve`]'s explicit parameter exists for callers that ran a build
//!   under a target dir they never exported process-wide. ~keep

use anyhow::Context as _;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Ask cargo for the effective target directory of the workspace owning `manifest_path`.
///
/// `cargo_target_dir` is injected into the `cargo metadata` child as `CARGO_TARGET_DIR` when
/// present, so a caller that ran a build under a redirected target dir never has to mutate the
/// process environment to make the resolution follow it. Pass `None` to let the ambient
/// environment and `.cargo/config.toml` decide.
///
/// # Errors
///
/// Returns an error when `cargo metadata` fails to run or exits non-zero, or when its JSON output
/// cannot be parsed (see [`from_metadata`]).
pub fn resolve(manifest_path: &Path, cargo_target_dir: Option<&OsStr>) -> anyhow::Result<PathBuf> {
    let manifest_path = std::path::absolute(manifest_path).with_context(|| {
        format!(
            "failed to resolve absolute manifest path for {}",
            manifest_path.display()
        )
    })?;
    let manifest_dir = manifest_path
        .parent()
        .with_context(|| format!("manifest path has no parent: {}", manifest_path.display()))?;
    let mut command = std::process::Command::new("cargo");
    command
        .args(["metadata", "--format-version", "1", "--no-deps", "--manifest-path"])
        .arg(&manifest_path)
        .current_dir(manifest_dir);
    if let Some(target_dir) = cargo_target_dir {
        command.env("CARGO_TARGET_DIR", target_dir);
    }
    let output = command.output().with_context(|| {
        format!(
            "failed to run `cargo metadata --manifest-path {}`",
            manifest_path.display()
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "`cargo metadata --manifest-path {}` failed: {}",
            manifest_path.display(),
            stderr.trim()
        );
    }

    let json = String::from_utf8_lossy(&output.stdout);
    from_metadata(&json).with_context(|| format!("resolving the target directory for {}", manifest_path.display()))
}

/// Pure function: read `target_directory` out of `cargo metadata --format-version 1` JSON.
///
/// Kept separate from [`resolve`] so the parse is unit-testable without running cargo.
///
/// # Errors
///
/// Returns an error when the JSON does not parse or carries no `target_directory` field.
pub fn from_metadata(metadata_json: &str) -> anyhow::Result<PathBuf> {
    let metadata: serde_json::Value =
        serde_json::from_str(metadata_json).context("failed to parse cargo metadata JSON")?;
    let target_directory = metadata["target_directory"]
        .as_str()
        .context("cargo metadata JSON missing `target_directory`")?;
    Ok(PathBuf::from(target_directory))
}

/// Best-effort target directory for a caller that only knows a workspace root.
///
/// Asks cargo first. When cargo cannot answer -- it is not installed, or the root carries a
/// `Cargo.lock` but no readable `Cargo.toml`, which is exactly the shape the swift-bridge
/// ancestor walks anchor on -- this falls back to `CARGO_TARGET_DIR` and then to the historical
/// `<workspace root>/target`, so a host without cargo behaves no worse than before this module
/// existed. The fallback deliberately does not parse `.cargo/config.toml`: reimplementing cargo's
/// upward search and profile merging is the hand-rolled resolution this module exists to replace,
/// and a wrong answer there would be just as silent as the bug it is meant to fix. ~keep
pub fn for_workspace_root(workspace_root: &Path) -> PathBuf {
    match resolve(&workspace_root.join("Cargo.toml"), None) {
        Ok(target_dir) => target_dir,
        Err(error) => {
            let fallback = std::env::var_os("CARGO_TARGET_DIR")
                .filter(|value| !value.is_empty())
                .map_or_else(|| workspace_root.join("target"), |value| workspace_root.join(value));
            tracing::debug!(
                workspace_root = %workspace_root.display(),
                fallback = %fallback.display(),
                "cargo metadata could not report a target directory ({error:#}); falling back"
            );
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_external_target_directory_out_of_metadata_json() {
        let json = r#"{"target_directory": "/var/tmp/some-external-target"}"#;
        assert_eq!(
            from_metadata(json).expect("parse metadata"),
            PathBuf::from("/var/tmp/some-external-target")
        );
    }

    #[test]
    fn errors_when_target_directory_is_missing() {
        let err = from_metadata(r#"{"packages": []}"#).unwrap_err();
        assert!(
            format!("{err:#}").contains("missing `target_directory`"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn errors_on_invalid_json() {
        let err = from_metadata("not json").unwrap_err();
        assert!(
            format!("{err:#}").contains("failed to parse cargo metadata JSON"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn for_workspace_root_follows_cargo_config_without_ambient_cwd() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().canonicalize().unwrap_or_else(|_| temp.path().to_path_buf());
        std::fs::create_dir_all(root.join("src")).expect("create src");
        std::fs::create_dir_all(root.join(".cargo")).expect("create .cargo");
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"target-dir-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write Cargo.toml");
        std::fs::write(root.join("src/lib.rs"), "pub fn noop() {}\n").expect("write lib.rs");
        let external = root.join("external-target");
        std::fs::write(
            root.join(".cargo/config.toml"),
            format!("[build]\ntarget-dir = \"{}\"\n", external.display()),
        )
        .expect("write .cargo/config.toml");

        let resolved = {
            let _target_dir_guard = crate::test_support::ClearedCargoTargetDirGuard::clear();
            for_workspace_root(&root)
        };

        assert_eq!(
            resolved,
            external,
            "the resolver must report the `.cargo/config.toml` target-dir, not {}",
            root.join("target").display()
        );
    }
}
