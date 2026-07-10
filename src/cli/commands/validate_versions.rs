//! Cross-manifest version consistency checker.
//!
//! Reads the canonical version from `Cargo.toml` (workspace or package level),
//! then verifies that every language manifest that alef manages agrees on the
//! same version string.
//!
//! Replaces:
//! - `actions/validate-versions/scripts/validate.py`
//! - `sample_core/scripts/publish/validate-version-consistency.sh`
//! - `sample_core/scripts/publish/verify-cargo-version.sh`

use crate::core::config::ResolvedCrateConfig;
use crate::core::version::{to_r_version, to_rubygems_prerelease};
use anyhow::{Context, Result};
use serde_json::json;
use std::path::Path;

/// Convert a semver pre-release to PEP 440 form for comparison against
/// `pyproject.toml` versions (e.g. "0.1.0-rc.1" → "0.1.0rc1").
fn to_pep440(version: &str) -> String {
    let Some((base, pre)) = version.split_once('-') else {
        return version.to_string();
    };
    let pep = pre
        .replace("alpha.", "a")
        .replace("alpha", "a")
        .replace("beta.", "b")
        .replace("beta", "b")
        .replace("rc.", "rc")
        .replace('.', "");
    format!("{base}{pep}")
}

fn identity(s: &str) -> String {
    s.to_string()
}

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
pub fn run(config: &ResolvedCrateConfig, workspace_root: &Path, output_json: bool) -> Result<Vec<VersionCheck>> {
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

fn collect_checks(config: &ResolvedCrateConfig, workspace_root: &Path, canonical: &str) -> Vec<VersionCheck> {
    let mut checks = Vec::new();

    let py_dir = config.package_dir(crate::core::config::extras::Language::Python);
    let py_path = join_manifest(&py_dir, "pyproject.toml");
    push_normalized_check(
        &mut checks,
        canonical,
        &py_path,
        workspace_root,
        read_pyproject_version,
        to_pep440,
    );
    if let Some(output_dir) = config.output_for("python") {
        let output_path = join_manifest(&output_dir.to_string_lossy(), "pyproject.toml");
        if output_path != py_path {
            push_normalized_check(
                &mut checks,
                canonical,
                &output_path,
                workspace_root,
                read_pyproject_version,
                to_pep440,
            );
        }
    }

    let node_dir = config.package_dir(crate::core::config::extras::Language::Node);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{node_dir}/package.json"),
        workspace_root,
        read_package_json_version,
    );

    for pattern in [
        "packages/ruby/lib/*/version.rb",
        "packages/ruby/ext/*/src/*/version.rb",
        "packages/ruby/ext/*/native/src/*/version.rb",
    ] {
        push_glob_checks_with_transform(
            &mut checks,
            canonical,
            pattern,
            workspace_root,
            read_ruby_version,
            to_rubygems_prerelease,
        );
    }

    let php_dir = config.package_dir(crate::core::config::extras::Language::Php);
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
    let elixir_dir = config.package_dir(crate::core::config::extras::Language::Elixir);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{elixir_dir}/mix.exs"),
        workspace_root,
        read_mix_exs_version,
    );

    let go_dir = config.package_dir(crate::core::config::extras::Language::Go);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{go_dir}/doc.go"),
        workspace_root,
        read_go_doc_version,
    );

    let java_dir = config.package_dir(crate::core::config::extras::Language::Java);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{java_dir}/pom.xml"),
        workspace_root,
        read_pom_xml_version,
    );

    let csharp_dir = config.package_dir(crate::core::config::extras::Language::Csharp);
    let csharp_ns = config.csharp_namespace();
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{csharp_dir}/{csharp_ns}/{csharp_ns}.csproj"),
        workspace_root,
        read_csproj_version,
    );

    let r_dir = config.package_dir(crate::core::config::extras::Language::R);
    push_check_with_transform(
        &mut checks,
        canonical,
        &format!("{r_dir}/DESCRIPTION"),
        workspace_root,
        read_description_version,
        to_r_version,
    );

    let wasm_dir = config.package_dir(crate::core::config::extras::Language::Wasm);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{wasm_dir}/package.json"),
        workspace_root,
        read_package_json_version,
    );

    push_check_if_exists(
        &mut checks,
        canonical,
        "package.json",
        workspace_root,
        read_package_json_version,
    );

    let crate_name = &config.name;
    for sub in ["wasm", "node"] {
        let path = format!("crates/{crate_name}-{sub}/package.json");
        push_check_if_exists(&mut checks, canonical, &path, workspace_root, read_package_json_version);
    }

    checks
}

/// Push a check only when the file actually exists and exposes a version
/// field. Absent files / absent version fields are silently skipped —
/// they're treated as "not configured for this repo" rather than
/// "mismatch with no version".
fn push_check_if_exists(
    checks: &mut Vec<VersionCheck>,
    canonical: &str,
    rel_path: &str,
    workspace_root: &Path,
    reader: fn(&Path) -> Option<String>,
) {
    push_check_with_transform(checks, canonical, rel_path, workspace_root, reader, identity);
}

/// Same as [`push_check_if_exists`] but applies a per-format transform
/// (e.g. semver→PEP 440) to the canonical version before comparing. This
/// keeps the JSON output's `expected` field equal to `canonical` so the
/// reported mismatch shows raw values, while `matches` reflects the
/// format-aware equality the manifest actually requires.
fn push_check_with_transform(
    checks: &mut Vec<VersionCheck>,
    canonical: &str,
    rel_path: &str,
    workspace_root: &Path,
    reader: fn(&Path) -> Option<String>,
    transform: fn(&str) -> String,
) {
    let full_path = workspace_root.join(rel_path);
    if !full_path.exists() {
        return;
    }
    let found = reader(&full_path);
    let Some(ref found_value) = found else {
        return;
    };
    let expected_in_format = transform(canonical);
    let matches = found_value == &expected_in_format;
    checks.push(VersionCheck {
        label: rel_path.to_string(),
        found,
        matches,
    });
}

/// Join a (possibly trailing-slash) manifest directory with a file name without
/// producing a doubled separator. `package_dir` returns whatever path the
/// `[crates.output]` config declares, which may end in `/` (e.g.
/// `crates/{lib}-py/src/`); a naive `format!("{dir}/{file}")` would then yield
/// `crates/{lib}-py/src//pyproject.toml`. Trimming trailing separators first
/// keeps the relative label clean and the on-disk lookup correct.
fn join_manifest(dir: &str, file: &str) -> String {
    let trimmed = dir.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        file.to_string()
    } else {
        format!("{trimmed}/{file}")
    }
}

/// Variant of [`push_check_with_transform`] that applies `normalize` to BOTH
/// the canonical version and the value found in the manifest before comparing.
/// This makes two equivalent spellings of the same version (e.g. PEP 440
/// `0.15.6-rc.2` and `0.15.6rc2`) compare equal as long as `normalize` maps
/// them to the same canonical form and is idempotent. The reported `found`
/// value preserves the raw manifest string so diagnostics show what is actually
/// on disk.
fn push_normalized_check(
    checks: &mut Vec<VersionCheck>,
    canonical: &str,
    rel_path: &str,
    workspace_root: &Path,
    reader: fn(&Path) -> Option<String>,
    normalize: fn(&str) -> String,
) {
    let full_path = workspace_root.join(rel_path);
    if !full_path.exists() {
        return;
    }
    let Some(found_value) = reader(&full_path) else {
        return;
    };
    let matches = normalize(&found_value) == normalize(canonical);
    checks.push(VersionCheck {
        label: rel_path.to_string(),
        found: Some(found_value),
        matches,
    });
}

/// Glob variant of [`push_check_with_transform`]. Walks `pattern`
/// relative to `workspace_root`, applies `transform` to the canonical
/// version, and pushes one check per match. Used for Ruby version.rb
/// files where canonical semver must be normalised to the RubyGems
/// prerelease form before comparison.
fn push_glob_checks_with_transform(
    checks: &mut Vec<VersionCheck>,
    canonical: &str,
    pattern: &str,
    workspace_root: &Path,
    reader: fn(&Path) -> Option<String>,
    transform: fn(&str) -> String,
) {
    let abs_pattern = workspace_root.join(pattern);
    let Some(pattern_str) = abs_pattern.to_str() else {
        return;
    };
    let Ok(entries) = glob::glob(pattern_str) else {
        return;
    };
    let expected = transform(canonical);
    for entry in entries.flatten() {
        let label = entry
            .strip_prefix(workspace_root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| entry.display().to_string());
        let found = reader(&entry);
        let Some(ref found_value) = found else {
            continue;
        };
        let matches = found_value == &expected;
        checks.push(VersionCheck { label, found, matches });
    }
}

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
    for line in content.lines() {
        let lower = line.to_lowercase();
        if lower.contains("version") || lower.contains("targets") {
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
    let text = content.as_str();
    let start = text.find("<version>")?;
    let inner_start = start + "<version>".len();
    let end = text[inner_start..].find("</version>")?;
    Some(text[inner_start..inner_start + end].to_string())
}

fn read_csproj_version(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
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

        fs::write(
            root.join("Cargo.toml"),
            format!("[workspace.package]\nversion = \"{canonical}\"\n\n[workspace]\nresolver = \"2\"\n"),
        )
        .unwrap();

        fs::create_dir_all(root.join("packages/python")).unwrap();
        fs::write(
            root.join("packages/python/pyproject.toml"),
            format!("[project]\nname = \"mylib\"\nversion = \"{canonical}\"\n"),
        )
        .unwrap();

        fs::create_dir_all(root.join("crates/mylib-node")).unwrap();
        fs::write(
            root.join("crates/mylib-node/package.json"),
            format!("{{\"name\":\"mylib\",\"version\":\"{canonical}\"}}\n"),
        )
        .unwrap();

        tmp
    }

    fn minimal_config(root: &Path) -> ResolvedCrateConfig {
        let root_str = root.display().to_string().replace('\\', "/");
        let content = format!(
            r#"
[workspace]
languages = ["python", "node"]
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
version_from = "{root_str}/Cargo.toml"
"#,
        );
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(&content).unwrap();
        cfg.resolve().unwrap().remove(0)
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
        let py = checks.iter().find(|c| c.label.contains("pyproject")).unwrap();
        assert!(py.matches, "pyproject.toml should match: {:?}", py);
        let node = checks
            .iter()
            .find(|c| c.label.contains("package.json") && c.label.contains("node"))
            .unwrap();
        assert!(node.matches, "package.json should match: {:?}", node);
        let _ = mismatches;
    }

    #[test]
    fn mismatch_detected() {
        let tmp = make_workspace("1.0.0");
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

    /// Build a config whose Python output path is an explicit `[crates.output]`
    /// directory — used to exercise the source-template layout where the
    /// publish-adjacent pyproject lives under `crates/{lib}-py/src/`.
    fn config_with_python_output(root: &Path, python_output: &str) -> ResolvedCrateConfig {
        let root_str = root.display().to_string().replace('\\', "/");
        let content = format!(
            r#"
[workspace]
languages = ["python"]
[[crates]]
name = "mylib"
sources = ["src/lib.rs"]
version_from = "{root_str}/Cargo.toml"
[crates.output]
python = "{python_output}"
"#,
        );
        let cfg: crate::core::config::NewAlefConfig = toml::from_str(&content).unwrap();
        cfg.resolve().unwrap().remove(0)
    }

    #[test]
    fn join_manifest_strips_trailing_separator() {
        assert_eq!(
            join_manifest("crates/mylib-py/src/", "pyproject.toml"),
            "crates/mylib-py/src/pyproject.toml"
        );
        assert_eq!(
            join_manifest("crates/mylib-py/src", "pyproject.toml"),
            "crates/mylib-py/src/pyproject.toml"
        );
        assert_eq!(join_manifest("", "pyproject.toml"), "pyproject.toml");
    }

    #[test]
    fn pep440_canonical_and_normalized_prerelease_compare_equal() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.15.6-rc.2\"\n\n[workspace]\nresolver = \"2\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("packages/python")).unwrap();
        fs::write(
            root.join("packages/python/pyproject.toml"),
            "[project]\nname = \"mylib\"\nversion = \"0.15.6rc2\"\n",
        )
        .unwrap();

        let config = minimal_config(root);
        let checks = run(&config, root, false).unwrap();
        let py = checks.iter().find(|c| c.label.contains("pyproject")).unwrap();
        assert!(py.matches, "normalized rc form should match canonical: {py:?}");
        assert_eq!(py.found.as_deref(), Some("0.15.6rc2"));
    }

    #[test]
    fn pep440_manifest_in_canonical_prerelease_form_also_matches() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.15.6-rc.2\"\n\n[workspace]\nresolver = \"2\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("packages/python")).unwrap();
        fs::write(
            root.join("packages/python/pyproject.toml"),
            "[project]\nname = \"mylib\"\nversion = \"0.15.6-rc.2\"\n",
        )
        .unwrap();

        let config = minimal_config(root);
        let checks = run(&config, root, false).unwrap();
        let py = checks.iter().find(|c| c.label.contains("pyproject")).unwrap();
        assert!(py.matches, "canonical rc form in manifest should match: {py:?}");
    }

    #[test]
    fn source_template_pyproject_with_trailing_slash_output_has_no_doubled_slash() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.15.6-rc.2\"\n\n[workspace]\nresolver = \"2\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("crates/mylib-py/src")).unwrap();
        fs::write(
            root.join("crates/mylib-py/src/pyproject.toml"),
            "[project]\nname = \"mylib\"\nversion = \"0.15.6rc2\"\n",
        )
        .unwrap();

        let config = config_with_python_output(root, "crates/mylib-py/src/");
        let checks = run(&config, root, false).unwrap();
        let py = checks.iter().find(|c| c.label.contains("pyproject")).unwrap();
        assert_eq!(
            py.label, "crates/mylib-py/src/pyproject.toml",
            "label must not contain a doubled slash"
        );
        assert!(
            !py.label.contains("//"),
            "label must not contain a doubled slash: {}",
            py.label
        );
        assert!(py.matches, "source-template pyproject should match: {py:?}");
    }
}
