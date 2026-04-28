//! Go submodule tagging helper.
//!
//! Creates the two Go module tags required per release:
//! - `packages/go/v{major}/{tag}` — correct per Go module spec
//! - `packages/go/{tag}` — legacy format for backwards compatibility
//!
//! Both tags are pushed to the remote with `--force-with-lease` (or printed in
//! dry-run mode).
//!
//! Ports: `kreuzberg/scripts/publish/go/tag-and-push-go-module.sh`

use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use serde_json::json;

/// Parameters for the go-tag command.
pub struct GoTagParams<'a> {
    pub version: &'a str,
    pub remote: &'a str,
    pub dry_run: bool,
    pub output_json: bool,
    pub config: &'a AlefConfig,
    /// Working directory (repository root).
    pub workspace_root: &'a std::path::Path,
}

/// Create and push Go module tags for a release.
pub fn run(params: &GoTagParams<'_>) -> Result<Vec<String>> {
    let version = params.version.trim_start_matches('v');
    let tag = format!("v{version}");

    // Derive major version number.
    let major: u64 = version
        .split('.')
        .next()
        .context("cannot parse major version")?
        .parse()
        .context("major version is not a number")?;

    // Go submodule prefix from alef.toml output path for Go.
    let go_output = params.config.package_dir(alef_core::config::extras::Language::Go);
    // strip trailing slashes
    let go_base = go_output.trim_end_matches('/').to_string();

    // For v2+ modules, Go requires /v{major}/ in the module path.
    let go_module_path = if major >= 2 {
        format!("{go_base}/v{major}")
    } else {
        go_base.clone()
    };

    let module_tag = format!("{go_module_path}/{tag}");
    let legacy_tag = format!("{go_base}/{tag}"); // legacy non-major-versioned format

    // Collect tags to create.
    let tags = if major >= 2 {
        // For v2+ modules emit both the versioned path and the legacy path.
        vec![module_tag.clone(), legacy_tag.clone()]
    } else {
        vec![module_tag.clone()]
    };

    let mut created = Vec::new();

    for ref_tag in &tags {
        if params.dry_run {
            println!("[dry-run] Would create git tag: {ref_tag}");
            println!("[dry-run] Would push to {}: {ref_tag}", params.remote);
            created.push(ref_tag.clone());
        } else {
            create_and_push_tag(ref_tag, &tag, params.remote, params.workspace_root)?;
            created.push(ref_tag.clone());
        }
    }

    if params.output_json {
        let out = json!({
            "version": tag,
            "major": major,
            "tags_created": created,
            "remote": params.remote,
            "dry_run": params.dry_run,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if !params.dry_run {
        for t in &created {
            println!("Created and pushed tag: {t}");
        }
    }

    Ok(created)
}

fn create_and_push_tag(new_tag: &str, source_ref: &str, remote: &str, workspace_root: &std::path::Path) -> Result<()> {
    // Check if tag already exists locally.
    let local_check = std::process::Command::new("git")
        .args(["rev-parse", new_tag])
        .current_dir(workspace_root)
        .output();

    if local_check.is_ok_and(|o| o.status.success()) {
        eprintln!("  Tag {new_tag} already exists locally; skipping.");
        return Ok(());
    }

    // Check if tag already exists on remote.
    let remote_check = std::process::Command::new("git")
        .args(["ls-remote", "--tags", remote])
        .current_dir(workspace_root)
        .output()?;

    if String::from_utf8_lossy(&remote_check.stdout)
        .lines()
        .any(|l| l.contains(&format!("refs/tags/{new_tag}")))
    {
        eprintln!("  Tag {new_tag} already exists on remote; skipping.");
        return Ok(());
    }

    // Create annotated tag.
    let tag_status = std::process::Command::new("git")
        .args([
            "tag",
            "-a",
            new_tag,
            source_ref,
            "-m",
            &format!("Go module tag {new_tag}"),
        ])
        .current_dir(workspace_root)
        .status()
        .with_context(|| format!("git tag {new_tag}"))?;

    if !tag_status.success() {
        anyhow::bail!("git tag {new_tag} failed");
    }

    // Push the tag with --force-with-lease for safety.
    let push_status = std::process::Command::new("git")
        .args(["push", "--force-with-lease", remote, &format!("refs/tags/{new_tag}")])
        .current_dir(workspace_root)
        .status()
        .with_context(|| format!("git push tag {new_tag}"))?;

    if !push_status.success() {
        anyhow::bail!("git push for tag {new_tag} failed");
    }

    eprintln!("  Tag {new_tag} created and pushed.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_git_repo(dir: &std::path::Path) {
        Command::new("git").args(["init"]).current_dir(dir).output().unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .unwrap();
        std::fs::write(dir.join("README.md"), "test").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .output()
            .unwrap();
        // Create the release tag.
        Command::new("git")
            .args(["tag", "-a", "v4.1.0", "HEAD", "-m", "Release v4.1.0"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    fn minimal_config() -> AlefConfig {
        toml::from_str(
            r#"
languages = ["go"]
[crate]
name = "mylib"
sources = ["src/lib.rs"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn dry_run_prints_tags() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let config = minimal_config();
        let params = GoTagParams {
            version: "4.1.0",
            remote: "origin",
            dry_run: true,
            output_json: false,
            config: &config,
            workspace_root: tmp.path(),
        };
        let tags = run(&params).unwrap();
        assert!(!tags.is_empty());
        // Both module_tag and legacy_tag for major >= 2.
        assert!(tags.iter().any(|t| t.contains("packages/go/v4/v4.1.0")));
        assert!(tags.iter().any(|t| t.contains("packages/go/v4.1.0")));
    }

    #[test]
    fn major_version_extracted() {
        let v = "4.1.0";
        let major: u64 = v.split('.').next().unwrap().parse().unwrap();
        assert_eq!(major, 4);
    }

    #[test]
    fn version_with_v_prefix_stripped() {
        let version = "v4.1.0".trim_start_matches('v');
        assert_eq!(version, "4.1.0");
    }

    #[test]
    fn dry_run_json_output() {
        let tmp = TempDir::new().unwrap();
        init_git_repo(tmp.path());
        let config = minimal_config();
        let params = GoTagParams {
            version: "4.0.0",
            remote: "origin",
            dry_run: true,
            output_json: true,
            config: &config,
            workspace_root: tmp.path(),
        };
        // Should not error.
        let result = run(&params);
        assert!(result.is_ok());
    }
}
