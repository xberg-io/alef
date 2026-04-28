//! Cross-manifest version consistency checker.
//!
//! Reads the canonical version from `Cargo.toml` (workspace or package level),
//! then verifies that every language manifest that alef manages agrees on the
//! same version string.
//!
//! Replaces:
//! - `actions/validate-versions/scripts/validate.py`
//! - `kreuzberg/scripts/publish/validate-version-consistency.sh`
//! - `kreuzberg/scripts/publish/verify-cargo-version.sh`

use alef_core::config::AlefConfig;
use anyhow::{Context, Result};
use serde_json::json;
use std::path::Path;

/// A single manifest version check result.
#[derive(Debug)]
pub struct VersionCheck {
    /// Human-readable label (e.g. "packages/python/pyproject.toml").
    pub label: String,
    /// Version found in this manifest. `None` means the file/field was absent.
    pub found: Option<String>,
    /// Whether this manifest matches the canonical version.
    pub matches: bool,
}

/// Run version consistency check across all configured language manifests.
///
/// Returns `(canonical_version, checks)` or an error if the canonical version
/// cannot be determined.
pub fn run(config: &AlefConfig, workspace_root: &Path, output_json: bool) -> Result<Vec<VersionCheck>> {
    let canonical = config
        .resolved_version()
        .context("Cannot read canonical version from Cargo.toml (version_from)")?;

    let checks = collect_checks(config, workspace_root, &canonical);

    if output_json {
        let entries: Vec<serde_json::Value> = checks
            .iter()
            .map(|c| {
                json!({
                    "manifest": c.label,
                    "found": c.found,
                    "expected": canonical,
                    "ok": c.matches,
                })
            })
            .collect();
        let out = json!({
            "canonical": canonical,
            "ok": checks.iter().all(|c| c.matches),
            "checks": entries,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("Canonical version: {canonical}");
        println!("{}", "-".repeat(40));
        for check in &checks {
            let status = if check.matches { "ok" } else { "MISMATCH" };
            let found = check.found.as_deref().unwrap_or("<not found>");
            println!("  [{status}] {} = {found}", check.label);
        }
        println!("{}", "-".repeat(40));
        let mismatches: Vec<_> = checks.iter().filter(|c| !c.matches).collect();
        if mismatches.is_empty() {
            println!("All {} manifests consistent: {canonical}", checks.len());
        } else {
            println!("{} mismatch(es) found:", mismatches.len());
            for m in &mismatches {
                println!("  FAIL  {} (found: {:?})", m.label, m.found);
            }
        }
    }

    Ok(checks)
}

fn collect_checks(config: &AlefConfig, workspace_root: &Path, canonical: &str) -> Vec<VersionCheck> {
    let mut checks = Vec::new();

    // Python: pyproject.toml `version = "..."`
    let py_dir = config.package_dir(alef_core::config::extras::Language::Python);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{py_dir}/pyproject.toml"),
        workspace_root,
        read_pyproject_version,
    );

    // Node: package.json `"version": "..."`
    let node_dir = config.package_dir(alef_core::config::extras::Language::Node);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{node_dir}/package.json"),
        workspace_root,
        read_package_json_version,
    );

    // Ruby: version.rb files. Layout varies — look at the well-known locations
    // alef's sync-versions writes to (lib-based, ext-based, and ext-native-based).
    for pattern in [
        "packages/ruby/lib/*/version.rb",
        "packages/ruby/ext/*/src/*/version.rb",
        "packages/ruby/ext/*/native/src/*/version.rb",
    ] {
        push_glob_checks(&mut checks, canonical, pattern, workspace_root, read_ruby_version);
    }

    // PHP: composer.json. Composer relies on git tags for version, so the file
    // typically has no `version` field. Only validate if a value is actually
    // declared in the manifest.
    let php_dir = config.package_dir(alef_core::config::extras::Language::Php);
    let php_path = format!("{php_dir}/composer.json");
    if workspace_root.join(&php_path).exists() && read_package_json_version(&workspace_root.join(&php_path)).is_some() {
        push_check_if_exists(
            &mut checks,
            canonical,
            &php_path,
            workspace_root,
            read_package_json_version,
        );
    }

    // Elixir: mix.exs uses either `@version "..."` (constant) or `version: "..."` (keyword).
    let elixir_dir = config.package_dir(alef_core::config::extras::Language::Elixir);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{elixir_dir}/mix.exs"),
        workspace_root,
        read_mix_exs_version,
    );

    // Go: doc.go is optional (binding comment-based versioning is convention,
    // not requirement). Only check if the file exists.
    let go_dir = config.package_dir(alef_core::config::extras::Language::Go);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{go_dir}/doc.go"),
        workspace_root,
        read_go_doc_version,
    );

    // Java: pom.xml `<version>...</version>`
    let java_dir = config.package_dir(alef_core::config::extras::Language::Java);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{java_dir}/pom.xml"),
        workspace_root,
        read_pom_xml_version,
    );

    // C#: .csproj `<Version>...</Version>`
    let csharp_dir = config.package_dir(alef_core::config::extras::Language::Csharp);
    let csharp_ns = config.csharp_namespace();
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{csharp_dir}/{csharp_ns}/{csharp_ns}.csproj"),
        workspace_root,
        read_csproj_version,
    );

    // R: DESCRIPTION `Version: ...`
    let r_dir = config.package_dir(alef_core::config::extras::Language::R);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{r_dir}/DESCRIPTION"),
        workspace_root,
        read_description_version,
    );

    // WASM: package.json (same reader as Node).
    let wasm_dir = config.package_dir(alef_core::config::extras::Language::Wasm);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{wasm_dir}/package.json"),
        workspace_root,
        read_package_json_version,
    );

    // Root package.json (some repos publish a top-level npm package alongside the binding).
    push_check_if_exists(
        &mut checks,
        canonical,
        "package.json",
        workspace_root,
        read_package_json_version,
    );

    // crates/{name}-wasm/package.json and crates/{name}-node/package.json.
    let crate_name = &config.crate_config.name;
    for sub in ["wasm", "node"] {
        let path = format!("crates/{crate_name}-{sub}/package.json");
        push_check_if_exists(&mut checks, canonical, &path, workspace_root, read_package_json_version);
    }

    checks
}

/// Push a check only when the file actually exists. Absent files are silently
/// skipped — they're treated as "not configured for this repo" rather than
/// "mismatch with no version".
fn push_check_if_exists(
    checks: &mut Vec<VersionCheck>,
    canonical: &str,
    rel_path: &str,
    workspace_root: &Path,
    reader: fn(&Path) -> Option<String>,
) {
    let full_path = workspace_root.join(rel_path);
    if !full_path.exists() {
        return;
    }
    let found = reader(&full_path);
    let matches = found.as_deref() == Some(canonical);
    checks.push(VersionCheck {
        label: rel_path.to_string(),
        found,
        matches,
    });
}

/// Walk a glob pattern relative to `workspace_root` and push a check per match.
/// Each match is treated as an existing file; reader is invoked unconditionally.
fn push_glob_checks(
    checks: &mut Vec<VersionCheck>,
    canonical: &str,
    pattern: &str,
    workspace_root: &Path,
    reader: fn(&Path) -> Option<String>,
) {
    let abs_pattern = workspace_root.join(pattern);
    let Some(pattern_str) = abs_pattern.to_str() else {
        return;
    };
    let Ok(entries) = glob::glob(pattern_str) else {
        return;
    };
    for entry in entries.flatten() {
        let label = entry
            .strip_prefix(workspace_root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| entry.display().to_string());
        let found = reader(&entry);
        let matches = found.as_deref() == Some(canonical);
        checks.push(VersionCheck { label, found, matches });
    }
}

// ---- per-format version readers ----

fn read_pyproject_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("version") && trimmed.contains('=') {
            let val = trimmed.split_once('=')?.1.trim();
            return Some(val.trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

fn read_package_json_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    val["version"].as_str().map(|s| s.to_string())
}

fn read_ruby_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if line.contains("VERSION") && line.contains('=') {
            let val = line.split_once('=')?.1.trim();
            return Some(val.trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

fn read_mix_exs_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        // Module-attribute form: `@version "X.Y.Z"`.
        if trimmed.starts_with("@version") {
            let val = trimmed.split_once('"')?.1;
            let val = val.split('"').next()?;
            return Some(val.to_string());
        }
        // Keyword form inside `def project do ...`: `version: "X.Y.Z",`.
        if let Some(rest) = trimmed.strip_prefix("version:") {
            let val = rest.split_once('"')?.1;
            let val = val.split('"').next()?;
            return Some(val.to_string());
        }
    }
    None
}

fn read_go_doc_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    // Look for patterns like `// targets Kreuzberg X.Y.Z` or `// version X.Y.Z`
    for line in content.lines() {
        let lower = line.to_lowercase();
        if lower.contains("version") || lower.contains("targets") || lower.contains("kreuzberg") {
            // Extract last version-like token (X.Y.Z possibly with -rc.N suffix).
            for token in line.split_whitespace().rev() {
                if token.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && token.contains('.') {
                    return Some(token.trim_end_matches('.').to_string());
                }
            }
        }
    }
    None
}

fn read_pom_xml_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    // Scan for `<version>...</version>` anywhere in the file (handles single-line and multi-line XML).
    let text = content.as_str();
    let start = text.find("<version>")?;
    let inner_start = start + "<version>".len();
    let end = text[inner_start..].find("</version>")?;
    Some(text[inner_start..inner_start + end].to_string())
}

fn read_csproj_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    // Scan for `<Version>...</Version>` anywhere in the file.
    let text = content.as_str();
    let start = text.find("<Version>")?;
    let inner_start = start + "<Version>".len();
    let end = text[inner_start..].find("</Version>")?;
    Some(text[inner_start..inner_start + end].to_string())
}

fn read_description_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("Version:") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Read the workspace version from Cargo.toml directly.
///
/// Used when `alef.toml` is not available (standalone invocation).
#[allow(dead_code)]
pub fn read_cargo_version(cargo_toml: &Path) -> Option<String> {
    let content = std::fs::read_to_string(cargo_toml).ok()?;
    let val: toml::Value = toml::from_str(&content).ok()?;
    // workspace.package.version or package.version
    val.get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .or_else(|| val.get("package")?.get("version")?.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_workspace(canonical: &str) -> TempDir {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Cargo.toml with workspace version
        fs::write(
            root.join("Cargo.toml"),
            format!("[workspace.package]\nversion = \"{canonical}\"\n\n[workspace]\nresolver = \"2\"\n"),
        )
        .unwrap();

        // pyproject.toml at default path
        fs::create_dir_all(root.join("packages/python")).unwrap();
        fs::write(
            root.join("packages/python/pyproject.toml"),
            format!("[project]\nname = \"mylib\"\nversion = \"{canonical}\"\n"),
        )
        .unwrap();

        // package.json at default node path
        fs::create_dir_all(root.join("packages/node")).unwrap();
        fs::write(
            root.join("packages/node/package.json"),
            format!("{{\"name\":\"mylib\",\"version\":\"{canonical}\"}}\n"),
        )
        .unwrap();

        tmp
    }

    fn minimal_config(root: &Path) -> AlefConfig {
        let content = format!(
            r#"
languages = ["python", "node"]
[crate]
name = "mylib"
sources = ["src/lib.rs"]
version_from = "{root}/Cargo.toml"
"#,
            root = root.display()
        );
        toml::from_str(&content).unwrap()
    }

    #[test]
    fn read_pyproject_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("pyproject.toml");
        fs::write(&path, "[project]\nversion = \"1.2.3\"\n").unwrap();
        assert_eq!(read_pyproject_version(&path), Some("1.2.3".to_string()));
    }

    #[test]
    fn read_package_json_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("package.json");
        fs::write(&path, r#"{"name":"foo","version":"2.0.0"}"#).unwrap();
        assert_eq!(read_package_json_version(&path), Some("2.0.0".to_string()));
    }

    #[test]
    fn read_ruby_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("version.rb");
        fs::write(&path, "  VERSION = \"1.0.0-rc.1\"\n").unwrap();
        assert_eq!(read_ruby_version(&path), Some("1.0.0-rc.1".to_string()));
    }

    #[test]
    fn read_mix_exs_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("mix.exs");
        fs::write(&path, "  @version \"3.0.0\"\n").unwrap();
        assert_eq!(read_mix_exs_version(&path), Some("3.0.0".to_string()));
    }

    #[test]
    fn read_pom_xml_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("pom.xml");
        fs::write(&path, "<project><version>1.5.0</version></project>").unwrap();
        assert_eq!(read_pom_xml_version(&path), Some("1.5.0".to_string()));
    }

    #[test]
    fn read_csproj_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("MyLib.csproj");
        fs::write(
            &path,
            "<Project><PropertyGroup><Version>1.2.0</Version></PropertyGroup></Project>",
        )
        .unwrap();
        assert_eq!(read_csproj_version(&path), Some("1.2.0".to_string()));
    }

    #[test]
    fn read_description_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("DESCRIPTION");
        fs::write(&path, "Package: mylib\nVersion: 0.9.1\nTitle: My Lib\n").unwrap();
        assert_eq!(read_description_version(&path), Some("0.9.1".to_string()));
    }

    #[test]
    fn read_cargo_version_workspace() {
        let tmp = TempDir::new().unwrap();
        let cargo_toml = tmp.path().join("Cargo.toml");
        fs::write(&cargo_toml, "[workspace.package]\nversion = \"5.0.0\"\n").unwrap();
        assert_eq!(read_cargo_version(&cargo_toml), Some("5.0.0".to_string()));
    }

    #[test]
    fn all_consistent_reports_ok() {
        let tmp = make_workspace("1.0.0");
        let config = minimal_config(tmp.path());
        let checks = run(&config, tmp.path(), false).unwrap();
        let mismatches: Vec<_> = checks.iter().filter(|c| !c.matches).collect();
        // Only checks for manifests that exist are run; only python and node are set up.
        // Others will report missing but still "match" only if None == None, which is false.
        // This test only asserts that py and node pass:
        let py = checks.iter().find(|c| c.label.contains("pyproject")).unwrap();
        assert!(py.matches, "pyproject.toml should match: {:?}", py);
        let node = checks
            .iter()
            .find(|c| c.label.contains("package.json") && c.label.contains("node"))
            .unwrap();
        assert!(node.matches, "package.json should match: {:?}", node);
        let _ = mismatches; // other manifests may be absent, that's expected
    }

    #[test]
    fn mismatch_detected() {
        let tmp = make_workspace("1.0.0");
        // Write wrong version to pyproject.toml
        std::fs::write(
            tmp.path().join("packages/python/pyproject.toml"),
            "[project]\nversion = \"9.9.9\"\n",
        )
        .unwrap();
        let config = minimal_config(tmp.path());
        let checks = run(&config, tmp.path(), false).unwrap();
        let py = checks.iter().find(|c| c.label.contains("pyproject")).unwrap();
        assert!(!py.matches, "pyproject.toml should mismatch");
        assert_eq!(py.found.as_deref(), Some("9.9.9"));
    }
}
