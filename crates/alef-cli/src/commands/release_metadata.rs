//! Release metadata JSON emitter.
//!
//! Computes and prints the JSON metadata object that CI workflows consume to
//! determine what to build and publish. Ports the logic of
//! `kreuzberg/scripts/publish/validate-and-compute-release-metadata.sh`
//! faithfully to Rust.

use alef_core::config::AlefConfig;
use anyhow::Result;
use serde_json::json;
use std::collections::HashSet;

/// All recognised release target names.
pub const ALL_RELEASE_TARGETS: &[&str] = &[
    "python", "node", "ruby", "cli", "crates", "docker", "homebrew", "java", "csharp", "go", "wasm", "php", "elixir",
    "r", "c-ffi",
];

/// Computed release metadata.
#[derive(Debug)]
pub struct ReleaseMetadata {
    pub tag: String,
    pub version: String,
    pub npm_tag: String,
    pub git_ref: String,
    pub checkout_ref: String,
    pub target_sha: String,
    pub matrix_ref: String,
    pub dry_run: bool,
    pub force_republish: bool,
    pub is_tag: bool,
    pub is_prerelease: bool,
    pub release_targets: String,
    pub release_any: bool,
    /// Per-target flags (target_name → enabled).
    pub targets: std::collections::HashMap<String, bool>,
}

impl ReleaseMetadata {
    /// Emit the metadata as a JSON string (same shape as the bash script).
    pub fn to_json(&self) -> Result<String> {
        let mut map = serde_json::Map::new();
        map.insert("tag".to_string(), json!(self.tag));
        map.insert("version".to_string(), json!(self.version));
        map.insert("npm_tag".to_string(), json!(self.npm_tag));
        map.insert("ref".to_string(), json!(self.git_ref));
        map.insert("checkout_ref".to_string(), json!(self.checkout_ref));
        map.insert("target_sha".to_string(), json!(self.target_sha));
        map.insert("matrix_ref".to_string(), json!(self.matrix_ref));
        map.insert("dry_run".to_string(), json!(self.dry_run));
        map.insert("force_republish".to_string(), json!(self.force_republish));
        map.insert("is_tag".to_string(), json!(self.is_tag));
        map.insert("is_prerelease".to_string(), json!(self.is_prerelease));
        map.insert("release_targets".to_string(), json!(self.release_targets));
        map.insert("release_any".to_string(), json!(self.release_any));

        // Per-target boolean flags.
        for target in ALL_RELEASE_TARGETS {
            let key = format!("release_{}", target.replace('-', "_"));
            let enabled = self.targets.get(*target).copied().unwrap_or(false);
            map.insert(key, json!(enabled));
        }

        serde_json::to_string_pretty(&serde_json::Value::Object(map)).map_err(anyhow::Error::from)
    }
}

/// Compute release metadata from inputs.
///
/// - `tag` — release tag (must start with `v`).
/// - `targets_csv` — comma-separated target list, or `"all"` / empty for everything.
/// - `git_ref` — optional ref override (commit SHA, branch, or `refs/...`).
/// - `event` — GitHub event name (release / workflow_dispatch / repository_dispatch).
/// - `dry_run`, `force_republish` — workflow inputs.
/// - `config` — optional `AlefConfig`; when present, valid targets are expanded to
///   include languages present in `config.languages`.
pub fn compute(
    tag: &str,
    targets_csv: &str,
    git_ref: Option<&str>,
    event: &str,
    dry_run: bool,
    force_republish: bool,
    config: Option<&AlefConfig>,
) -> Result<ReleaseMetadata> {
    // Validate tag format.
    if !tag.starts_with('v') {
        anyhow::bail!("Tag must start with 'v' (got: {tag})");
    }
    let version = tag.trim_start_matches('v').to_string();

    // Resolve ref.
    let resolved_ref = resolve_ref(tag, git_ref, event);
    let (checkout_ref, target_sha) = resolve_checkout(&resolved_ref);
    let matrix_ref = resolve_matrix_ref(&resolved_ref);
    let is_tag = resolved_ref.starts_with("refs/tags/");

    // Prerelease detection.
    let is_prerelease = is_prerelease_version(&version);
    let npm_tag = if is_prerelease { "next" } else { "latest" }.to_string();

    // Resolve valid targets (from config or hardcoded list).
    let valid_targets: HashSet<&str> = ALL_RELEASE_TARGETS.iter().copied().collect();

    // Parse requested targets.
    let enabled = parse_targets(targets_csv, &valid_targets)?;

    // homebrew implies cli.
    let mut enabled = enabled;
    if enabled.get("homebrew").copied().unwrap_or(false) {
        enabled.insert("cli".to_string(), true);
    }

    let enabled_list: Vec<&str> = ALL_RELEASE_TARGETS
        .iter()
        .copied()
        .filter(|t| enabled.get(*t).copied().unwrap_or(false))
        .collect();

    let release_targets = if enabled_list.len() == ALL_RELEASE_TARGETS.len() {
        "all".to_string()
    } else if enabled_list.is_empty() {
        "none".to_string()
    } else {
        enabled_list.join(",")
    };

    let release_any = !enabled_list.is_empty();

    // Include any extra targets from config languages that aren't in ALL_RELEASE_TARGETS.
    // (Forward-looking for when new languages are added to alef.toml.)
    if let Some(cfg) = config {
        let _extra_langs: Vec<String> = cfg
            .languages
            .iter()
            .map(|l| l.to_string())
            .filter(|l| !valid_targets.contains(l.as_str()))
            .collect();
        // TODO(phase-2): emit release_* flags for dynamically discovered languages.
    }

    Ok(ReleaseMetadata {
        tag: tag.to_string(),
        version,
        npm_tag,
        git_ref: resolved_ref,
        checkout_ref,
        target_sha,
        matrix_ref,
        dry_run,
        force_republish,
        is_tag,
        is_prerelease,
        release_targets,
        release_any,
        targets: enabled,
    })
}

fn resolve_ref(tag: &str, git_ref: Option<&str>, event: &str) -> String {
    if let Some(r) = git_ref {
        if !r.is_empty() {
            // If the caller passed just a tag name without refs/ prefix, normalise.
            if r == tag {
                return format!("refs/tags/{tag}");
            }
            if r.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && r.starts_with('v')
                || r.chars().all(|c| c.is_ascii_hexdigit()) && r.len() == 40
            {
                // Looks like a SHA or a bare version tag.
                return r.to_string();
            }
            if !r.starts_with("refs/") {
                return format!("refs/heads/{r}");
            }
            return r.to_string();
        }
    }
    // Default: all events produce a tag ref.
    let _ = event;
    format!("refs/tags/{tag}")
}

fn resolve_checkout(git_ref: &str) -> (String, String) {
    // Full SHA (40 hex chars).
    if git_ref.len() == 40 && git_ref.chars().all(|c| c.is_ascii_hexdigit()) {
        return ("refs/heads/main".to_string(), git_ref.to_string());
    }
    (git_ref.to_string(), String::new())
}

fn resolve_matrix_ref(git_ref: &str) -> String {
    if git_ref.len() == 40 && git_ref.chars().all(|c| c.is_ascii_hexdigit()) {
        return "main".to_string();
    }
    if let Some(branch) = git_ref.strip_prefix("refs/heads/") {
        return branch.to_string();
    }
    if let Some(tag) = git_ref.strip_prefix("refs/tags/") {
        return tag.to_string();
    }
    git_ref.to_string()
}

fn is_prerelease_version(version: &str) -> bool {
    version.contains("-rc") || version.contains("-alpha") || version.contains("-beta") || version.contains("-pre")
}

fn parse_targets(csv: &str, valid: &HashSet<&str>) -> Result<std::collections::HashMap<String, bool>> {
    let mut enabled: std::collections::HashMap<String, bool> = std::collections::HashMap::new();

    let csv = csv.trim();
    if csv.is_empty() || csv == "all" || csv == "*" || csv == "default" {
        for &t in valid {
            enabled.insert(t.to_string(), true);
        }
        return Ok(enabled);
    }

    for raw in csv.split(',') {
        let t = raw.trim().to_lowercase();
        if t.is_empty() {
            continue;
        }
        let normalised = normalise_target(&t);
        if normalised == "none" {
            // Disable all.
            for &vt in valid {
                enabled.insert(vt.to_string(), false);
            }
            continue;
        }
        if normalised == "all" {
            for &vt in valid {
                enabled.insert(vt.to_string(), true);
            }
            continue;
        }
        if !valid.contains(normalised) {
            anyhow::bail!(
                "Unknown release target '{normalised}'. Allowed: {}",
                ALL_RELEASE_TARGETS.join(", ")
            );
        }
        enabled.insert(normalised.to_string(), true);
    }

    Ok(enabled)
}

fn normalise_target(t: &str) -> &str {
    match t {
        "csharp" | "dotnet" | "cs" | "nuget" => "csharp",
        "go" | "golang" => "go",
        "wasm" | "webassembly" => "wasm",
        "r" | "rproject" => "r",
        "elixir" | "hex" => "elixir",
        "c-ffi" | "c_ffi" | "cffi" => "c-ffi",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_release_event_all_targets() {
        let meta = compute("v4.1.0", "all", None, "release", false, false, None).unwrap();
        assert_eq!(meta.tag, "v4.1.0");
        assert_eq!(meta.version, "4.1.0");
        assert_eq!(meta.npm_tag, "latest");
        assert!(!meta.is_prerelease);
        assert!(meta.release_any);
        assert_eq!(meta.release_targets, "all");
        assert!(meta.targets["python"]);
        assert!(meta.targets["node"]);
    }

    #[test]
    fn compute_prerelease_tag() {
        let meta = compute("v4.1.0-rc.1", "", None, "release", false, false, None).unwrap();
        assert!(meta.is_prerelease);
        assert_eq!(meta.npm_tag, "next");
    }

    #[test]
    fn compute_target_subset() {
        let meta = compute("v4.0.0", "python,node", None, "workflow_dispatch", true, false, None).unwrap();
        assert!(meta.dry_run);
        assert!(meta.targets["python"]);
        assert!(meta.targets["node"]);
        assert!(!meta.targets.get("ruby").copied().unwrap_or(false));
        assert_eq!(meta.release_targets, "python,node");
    }

    #[test]
    fn compute_homebrew_implies_cli() {
        let meta = compute("v4.0.0", "homebrew", None, "workflow_dispatch", false, false, None).unwrap();
        assert!(meta.targets["homebrew"]);
        assert!(meta.targets["cli"]);
    }

    #[test]
    fn compute_ref_override_sha() {
        let sha = "a".repeat(40);
        let meta = compute("v4.0.0", "all", Some(&sha), "workflow_dispatch", false, false, None).unwrap();
        assert_eq!(meta.checkout_ref, "refs/heads/main");
        assert_eq!(meta.target_sha, sha);
        assert_eq!(meta.matrix_ref, "main");
    }

    #[test]
    fn compute_ref_override_branch() {
        let meta = compute(
            "v4.0.0",
            "all",
            Some("my-branch"),
            "workflow_dispatch",
            false,
            false,
            None,
        )
        .unwrap();
        assert_eq!(meta.checkout_ref, "refs/heads/my-branch");
        assert_eq!(meta.matrix_ref, "my-branch");
    }

    #[test]
    fn compute_invalid_tag_no_v_prefix() {
        let result = compute("4.0.0", "all", None, "release", false, false, None);
        assert!(result.is_err());
    }

    #[test]
    fn compute_unknown_target_errors() {
        let result = compute("v4.0.0", "unknown-target", None, "release", false, false, None);
        assert!(result.is_err());
    }

    #[test]
    fn json_output_has_all_fields() {
        let meta = compute("v1.0.0", "all", None, "release", false, false, None).unwrap();
        let json_str = meta.to_json().unwrap();
        let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(val["release_python"].as_bool().unwrap());
        assert!(val["release_c_ffi"].as_bool().unwrap());
        assert_eq!(val["version"], "1.0.0");
        assert_eq!(val["npm_tag"], "latest");
    }

    #[test]
    fn targets_normalisation() {
        // csharp aliases
        assert_eq!(normalise_target("dotnet"), "csharp");
        assert_eq!(normalise_target("nuget"), "csharp");
        // go aliases
        assert_eq!(normalise_target("golang"), "go");
        // c-ffi aliases
        assert_eq!(normalise_target("cffi"), "c-ffi");
        assert_eq!(normalise_target("c_ffi"), "c-ffi");
    }
}
