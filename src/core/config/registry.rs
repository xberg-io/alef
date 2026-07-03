//! Registry-based source resolution for `[[crates.source_crates]]` with `from_registry = true`.
//!
//! When a `SourceCrate` has `from_registry = true`, alef runs `cargo metadata` against
//! the consumer workspace to locate the crate's source directory in `~/.cargo/registry/src/…`
//! (or wherever cargo placed it) and rebases the relative `sources` paths against it.
//! This makes regen hermetic — no sibling checkout of the dependency is required.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Run `cargo metadata` for the workspace at `workspace_root` and return the source
/// directory of the package named `crate_name`.
///
/// Normalises hyphen/underscore differences in both the query name and each package
/// name before matching.
///
/// # Errors
///
/// Returns a human-readable `String` error when:
/// - `cargo metadata` fails to run or exits non-zero
/// - `crate_name` is not present in the resolved dependency graph
/// - The package's `manifest_path` is missing or has no parent
pub fn resolve_crate_source_dir(workspace_root: &Path, crate_name: &str) -> Result<PathBuf, String> {
    let manifest = workspace_root.join("Cargo.toml");
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&manifest)
        .output()
        .map_err(|e| format!("failed to run `cargo metadata`: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`cargo metadata --manifest-path {}` failed: {}",
            manifest.display(),
            stderr.trim()
        ));
    }

    let json = String::from_utf8_lossy(&output.stdout);
    resolve_crate_source_dir_from_metadata(&json, crate_name)
        .map_err(|e| format!("{e} (workspace: `{}`)", workspace_root.display()))
}

/// Parse the JSON output of `cargo metadata --format-version 1` and return the source
/// directory of the package named `crate_name`.
///
/// This is a pure function and is unit-testable without running cargo.
pub fn resolve_crate_source_dir_from_metadata(metadata_json: &str, crate_name: &str) -> Result<PathBuf, String> {
    let metadata: serde_json::Value =
        serde_json::from_str(metadata_json).map_err(|e| format!("failed to parse cargo metadata JSON: {e}"))?;

    let packages = metadata["packages"]
        .as_array()
        .ok_or_else(|| "cargo metadata JSON missing `packages` array".to_string())?;

    let normalized_query = normalize_crate_name(crate_name);

    let pkg = packages
        .iter()
        .find(|pkg| {
            pkg["name"]
                .as_str()
                .map(|n| normalize_crate_name(n) == normalized_query)
                .unwrap_or(false)
        })
        .ok_or_else(|| {
            let available: Vec<&str> = packages.iter().filter_map(|pkg| pkg["name"].as_str()).collect();
            format!(
                "crate `{crate_name}` not found in cargo metadata; \
                 available packages: {}",
                available.join(", ")
            )
        })?;

    let manifest_path_str = pkg["manifest_path"]
        .as_str()
        .ok_or_else(|| format!("package `{crate_name}` has no `manifest_path` in cargo metadata"))?;

    let crate_root = Path::new(manifest_path_str)
        .parent()
        .ok_or_else(|| format!("manifest_path `{manifest_path_str}` has no parent directory"))?;

    Ok(crate_root.to_path_buf())
}

fn normalize_crate_name(name: &str) -> String {
    name.replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_metadata_json(packages: &[(&str, &str)]) -> String {
        let pkg_entries: Vec<String> = packages
            .iter()
            .map(|(name, manifest_path)| format!(r#"{{"name": "{name}", "manifest_path": "{manifest_path}"}}"#))
            .collect();
        format!(r#"{{"packages": [{}]}}"#, pkg_entries.join(","))
    }

    #[test]
    fn resolves_registry_path_from_metadata() {
        let json = fake_metadata_json(&[(
            "sample-crate",
            "/home/user/.cargo/registry/src/index.crates.io-abc/sample-crate-0.1.0/Cargo.toml",
        )]);

        let dir = resolve_crate_source_dir_from_metadata(&json, "sample-crate").unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/home/user/.cargo/registry/src/index.crates.io-abc/sample-crate-0.1.0")
        );
    }

    #[test]
    fn rebases_multiple_sources() {
        let json = fake_metadata_json(&[(
            "sample-crate",
            "/home/user/.cargo/registry/src/index.crates.io-abc/sample-crate-0.1.0/Cargo.toml",
        )]);

        let crate_dir = resolve_crate_source_dir_from_metadata(&json, "sample-crate").unwrap();
        let rel_sources = [
            PathBuf::from("src/types/config.rs"),
            PathBuf::from("src/types/discovery.rs"),
            PathBuf::from("src/net/ssrf.rs"),
        ];
        let rebased: Vec<PathBuf> = rel_sources.iter().map(|p| crate_dir.join(p)).collect();

        let base = "/home/user/.cargo/registry/src/index.crates.io-abc/sample-crate-0.1.0";
        assert_eq!(rebased[0], PathBuf::from(format!("{base}/src/types/config.rs")));
        assert_eq!(rebased[1], PathBuf::from(format!("{base}/src/types/discovery.rs")));
        assert_eq!(rebased[2], PathBuf::from(format!("{base}/src/net/ssrf.rs")));
    }

    #[test]
    fn normalizes_hyphen_query_against_hyphen_package() {
        // query with underscore, package name with hyphen
        let json = fake_metadata_json(&[(
            "my-crate",
            "/home/user/.cargo/registry/src/index.crates.io-abc/my-crate-1.0.0/Cargo.toml",
        )]);
        let dir = resolve_crate_source_dir_from_metadata(&json, "my_crate").unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/home/user/.cargo/registry/src/index.crates.io-abc/my-crate-1.0.0")
        );
    }

    #[test]
    fn normalizes_underscore_query_against_underscore_package() {
        // query with hyphen, package name with underscore
        let json = fake_metadata_json(&[(
            "my_crate",
            "/home/user/.cargo/registry/src/index.crates.io-abc/my_crate-1.0.0/Cargo.toml",
        )]);
        let dir = resolve_crate_source_dir_from_metadata(&json, "my-crate").unwrap();
        assert_eq!(
            dir,
            PathBuf::from("/home/user/.cargo/registry/src/index.crates.io-abc/my_crate-1.0.0")
        );
    }

    #[test]
    fn error_when_crate_not_found() {
        let json = fake_metadata_json(&[("other-crate", "/some/path/Cargo.toml")]);
        let err = resolve_crate_source_dir_from_metadata(&json, "sample-crate").unwrap_err();
        assert!(err.contains("crate `sample-crate` not found"), "got: {err}");
        assert!(err.contains("other-crate"), "got: {err}");
    }

    #[test]
    fn error_on_invalid_json() {
        let err = resolve_crate_source_dir_from_metadata("not json", "sample-crate").unwrap_err();
        assert!(err.contains("failed to parse cargo metadata JSON"), "got: {err}");
    }

    #[test]
    fn error_when_packages_missing() {
        let err = resolve_crate_source_dir_from_metadata("{}", "sample-crate").unwrap_err();
        assert!(err.contains("missing `packages` array"), "got: {err}");
    }
}
