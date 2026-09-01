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

/// Outcome of reading a version out of a manifest that is already known to exist.
///
/// ~keep The three states must stay distinct. The `push_*` helpers check `exists()`
/// first, so a reader that yields nothing has proven a failure, not an absence —
/// collapsing "could not read/parse" back into the same `None` as "declares no
/// version field" is what let `validate versions --exit-code` certify manifests it
/// never examined. `NoVersionField` stays a skip on purpose: `sync-versions`
/// (`pipeline::version`) also leaves versionless manifests alone, so the write side
/// and the gate agree on which manifests are managed.
#[derive(Debug, PartialEq, Eq)]
enum ReadOutcome {
    /// The manifest declares a version.
    Found(String),
    /// The manifest parsed but declares no version field — not managed here.
    NoVersionField,
    /// The manifest exists but could not be read or parsed; carries the reason.
    Unreadable(String),
}

type VersionReader = fn(&Path) -> ReadOutcome;

/// Read `path` to a string and run a line/text scanner over it, mapping an I/O
/// failure to [`ReadOutcome::Unreadable`] and a scanner miss to
/// [`ReadOutcome::NoVersionField`].
fn scan_text(path: &Path, scan: impl FnOnce(&str) -> Option<String>) -> ReadOutcome {
    match std::fs::read_to_string(path) {
        Ok(content) => scan(&content).map_or(ReadOutcome::NoVersionField, ReadOutcome::Found),
        Err(error) => ReadOutcome::Unreadable(format!("read failed: {error}")),
    }
}

/// Record a manifest that exists but whose version could not be read, as a
/// FAILING check so it reaches the `--exit-code` verdict in `release_commands`.
fn push_unreadable(checks: &mut Vec<VersionCheck>, label: String, reason: &str) {
    tracing::error!(
        manifest = %label,
        reason = %reason,
        "manifest exists but its version could not be read"
    );
    checks.push(VersionCheck {
        label,
        found: None,
        matches: false,
        blocked_on_publish: None,
    });
}

/// A single manifest version check result.
#[derive(Debug)]
pub struct VersionCheck {
    /// Human-readable label (e.g. "packages/python/pyproject.toml").
    pub label: String,
    /// Version found in this manifest. `None` means the manifest exists but could
    /// not be read or parsed, which is always a failing check — manifests that are
    /// merely absent or versionless produce no `VersionCheck` at all.
    pub found: Option<String>,
    /// Whether this manifest matches the canonical version.
    pub matches: bool,
    /// `name@version` that must be published before this mismatch can be resolved, when the
    /// manifest cannot be refreshed in-tree at all.
    ///
    /// ~keep Only ever set on a failing check. It DOES soften the verdict: [`checks_pass`]
    /// tolerates such a row, so it does not fail `--exit-code`. See that function for why —
    /// failing it makes the gate unsatisfiable by construction. It exists because the two kinds
    /// of mismatch call for opposite actions — plain drift is "run the sync and commit it", a
    /// blocked row is "publish the release, then refresh".
    pub blocked_on_publish: Option<String>,
}

/// Per-check status tag used in the report body.
fn status_label(check: &VersionCheck) -> &'static str {
    match (check.matches, check.found.is_some(), check.blocked_on_publish.is_some()) {
        (true, _, _) => "ok",
        (false, false, _) => "UNREADABLE",
        (false, true, true) => "UNPUBLISHED",
        (false, true, false) => "MISMATCH",
    }
}

/// Whether a set of checks should be treated as a passing gate.
///
/// ~keep Zero checks is a failure, not a pass: `all()` is vacuously true on an empty vector, so a
/// run that examined nothing used to report success. A `blocked_on_publish` check, by contrast,
/// does NOT count against the gate. Those are lockfile entries pinning the crate at the version
/// being released and resolved from the registry, so cargo cannot refresh them until that version
/// exists there -- which cannot happen until this gate passes. Failing them makes the gate
/// unsatisfiable by construction for any repo with a registry-depending test app. They are still
/// reported (`status_label` tags them UNPUBLISHED, and the summary counts them separately); they
/// just no longer fail a check nobody can act on.
pub(crate) fn checks_pass(checks: &[VersionCheck]) -> bool {
    !checks.is_empty() && checks.iter().all(|c| c.matches || c.blocked_on_publish.is_some())
}

/// One line of the trailing failure summary.
fn failure_line(check: &VersionCheck) -> String {
    match &check.blocked_on_publish {
        Some(pending) => format!(
            "  BLOCKED  {} (found: {:?}) - unresolvable until {pending} is published",
            check.label, check.found
        ),
        None => format!("  FAIL  {} (found: {:?})", check.label, check.found),
    }
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
                    "blocked_on_publish": c.blocked_on_publish,
                })
            })
            .collect();
        // ~keep `all()` is vacuously true on an empty vector, so a run that examined
        // nothing used to report `"ok": true`. Zero checks is a failure, not a pass.
        //
        let out = json!({
            "canonical": canonical,
            "ok": checks_pass(&checks),
            "checks": entries,
        });
        crate::bin_cli::output::payload(serde_json::to_string_pretty(&out)?);
    } else {
        crate::bin_cli::output::line(format!("Canonical version: {canonical}"));
        crate::bin_cli::output::line("-".repeat(40));
        for check in &checks {
            let status = status_label(check);
            let found = check.found.as_deref().unwrap_or("<unreadable>");
            crate::bin_cli::output::line(format!("  [{status}] {} = {found}", check.label));
        }
        crate::bin_cli::output::line("-".repeat(40));
        let mismatches: Vec<_> = checks.iter().filter(|c| !c.matches).collect();
        let blocked = mismatches.iter().filter(|c| c.blocked_on_publish.is_some()).count();
        if checks.is_empty() {
            crate::bin_cli::output::line("No manifests examined - nothing was verified.");
        } else if mismatches.is_empty() {
            crate::bin_cli::output::line(format!("All {} manifests consistent: {canonical}", checks.len()));
        } else {
            let summary = if blocked == 0 {
                format!("{} mismatch(es) found:", mismatches.len())
            } else {
                format!(
                    "{} mismatch(es) found, {blocked} of them unresolvable until the pending release is published:",
                    mismatches.len()
                )
            };
            crate::bin_cli::output::line(summary);
            for m in &mismatches {
                crate::bin_cli::output::line(failure_line(m));
            }
        }
    }

    // ~keep An empty check set is the gate examining nothing, which the caller's
    // `checks.iter().any(|c| !c.matches)` verdict cannot distinguish from success.
    // Fail here rather than downstream so the run is non-zero even without
    // `--exit-code`: "nothing to verify" is never a valid release-gate pass.
    if checks.is_empty() {
        anyhow::bail!(
            "version validation examined 0 manifests for crate `{}` (canonical {canonical}); check that \
             the configured package directories exist and are readable",
            config.name
        );
    }

    Ok(checks)
}

pub fn collect_checks(config: &ResolvedCrateConfig, workspace_root: &Path, canonical: &str) -> Vec<VersionCheck> {
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

    // ~keep composer.json routinely omits `version` (Packagist derives it from tags).
    // That used to need a pre-guard here; `push_check_if_exists` now skips
    // `NoVersionField` itself, and the pre-guard would have re-swallowed an
    // *unreadable* composer.json, so it is gone.
    let php_dir = config.package_dir(crate::core::config::extras::Language::Php);
    push_check_if_exists(
        &mut checks,
        canonical,
        &format!("{php_dir}/composer.json"),
        workspace_root,
        read_package_json_version,
    );

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

    checks.extend(super::version_manifests::collect(config, workspace_root, canonical));
    checks.extend(super::ruby_swift_versions::collect(config, workspace_root, canonical));
    checks
}

/// Push a check only when the file actually exists and exposes a version
/// field. Absent files and absent version fields are skipped — they're
/// "not configured for this repo". A file that exists but cannot be read or
/// parsed is NOT skipped: it becomes a failing check.
fn push_check_if_exists(
    checks: &mut Vec<VersionCheck>,
    canonical: &str,
    rel_path: &str,
    workspace_root: &Path,
    reader: VersionReader,
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
    reader: VersionReader,
    transform: fn(&str) -> String,
) {
    let full_path = workspace_root.join(rel_path);
    if !full_path.exists() {
        return;
    }
    match reader(&full_path) {
        ReadOutcome::Found(found_value) => {
            let matches = found_value == transform(canonical);
            checks.push(VersionCheck {
                label: rel_path.to_string(),
                found: Some(found_value),
                matches,
                blocked_on_publish: None,
            });
        }
        ReadOutcome::NoVersionField => {}
        ReadOutcome::Unreadable(reason) => push_unreadable(checks, rel_path.to_string(), &reason),
    }
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
    reader: VersionReader,
    normalize: fn(&str) -> String,
) {
    let full_path = workspace_root.join(rel_path);
    if !full_path.exists() {
        return;
    }
    match reader(&full_path) {
        ReadOutcome::Found(found_value) => {
            let matches = normalize(&found_value) == normalize(canonical);
            checks.push(VersionCheck {
                label: rel_path.to_string(),
                found: Some(found_value),
                matches,
                blocked_on_publish: None,
            });
        }
        ReadOutcome::NoVersionField => {}
        ReadOutcome::Unreadable(reason) => push_unreadable(checks, rel_path.to_string(), &reason),
    }
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
    reader: VersionReader,
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
        match reader(&entry) {
            ReadOutcome::Found(found_value) => {
                let matches = found_value == expected;
                checks.push(VersionCheck {
                    label,
                    found: Some(found_value),
                    matches,
                    blocked_on_publish: None,
                });
            }
            ReadOutcome::NoVersionField => {}
            ReadOutcome::Unreadable(reason) => push_unreadable(checks, label, &reason),
        }
    }
}

fn read_pyproject_version(path: &Path) -> ReadOutcome {
    scan_text(path, |content| {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("version") && trimmed.contains('=') {
                let val = trimmed.split_once('=')?.1.trim();
                return Some(val.trim_matches('"').trim_matches('\'').to_string());
            }
        }
        None
    })
}

fn read_package_json_version(path: &Path) -> ReadOutcome {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => return ReadOutcome::Unreadable(format!("read failed: {error}")),
    };
    // ~keep A JSON syntax error is a failed read, not a missing field: a truncated or
    // half-written package.json must fail the gate, never pass as "not configured".
    let val: serde_json::Value = match serde_json::from_str(&content) {
        Ok(val) => val,
        Err(error) => return ReadOutcome::Unreadable(format!("invalid JSON: {error}")),
    };
    val["version"]
        .as_str()
        .map_or(ReadOutcome::NoVersionField, |s| ReadOutcome::Found(s.to_string()))
}

fn read_ruby_version(path: &Path) -> ReadOutcome {
    scan_text(path, |content| {
        for line in content.lines() {
            if line.contains("VERSION") && line.contains('=') {
                let val = line.split_once('=')?.1.trim();
                return Some(val.trim_matches('"').trim_matches('\'').to_string());
            }
        }
        None
    })
}

fn read_mix_exs_version(path: &Path) -> ReadOutcome {
    scan_text(path, |content| {
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
    })
}

fn read_go_doc_version(path: &Path) -> ReadOutcome {
    scan_text(path, |content| {
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
    })
}

fn read_pom_xml_version(path: &Path) -> ReadOutcome {
    scan_text(path, |content| {
        let start = content.find("<version>")?;
        let inner_start = start + "<version>".len();
        let end = content[inner_start..].find("</version>")?;
        Some(content[inner_start..inner_start + end].to_string())
    })
}

fn read_description_version(path: &Path) -> ReadOutcome {
    scan_text(path, |content| {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("Version:") {
                return Some(rest.trim().to_string());
            }
        }
        None
    })
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

    fn found(version: &str) -> ReadOutcome {
        ReadOutcome::Found(version.to_string())
    }

    #[test]
    fn read_pyproject_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("pyproject.toml");
        fs::write(&path, "[project]\nversion = \"1.2.3\"\n").unwrap();
        assert_eq!(read_pyproject_version(&path), found("1.2.3"));
    }

    #[test]
    fn read_package_json_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("package.json");
        fs::write(&path, r#"{"name":"foo","version":"2.0.0"}"#).unwrap();
        assert_eq!(read_package_json_version(&path), found("2.0.0"));
    }

    #[test]
    fn read_ruby_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("version.rb");
        fs::write(&path, "  VERSION = \"1.0.0-rc.1\"\n").unwrap();
        assert_eq!(read_ruby_version(&path), found("1.0.0-rc.1"));
    }

    #[test]
    fn read_mix_exs_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("mix.exs");
        fs::write(&path, "  @version \"3.0.0\"\n").unwrap();
        assert_eq!(read_mix_exs_version(&path), found("3.0.0"));
    }

    #[test]
    fn read_pom_xml_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("pom.xml");
        fs::write(&path, "<project><version>1.5.0</version></project>").unwrap();
        assert_eq!(read_pom_xml_version(&path), found("1.5.0"));
    }

    #[test]
    fn read_description_version_ok() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("DESCRIPTION");
        fs::write(&path, "Package: mylib\nVersion: 0.9.1\nTitle: My Lib\n").unwrap();
        assert_eq!(read_description_version(&path), found("0.9.1"));
    }

    #[test]
    fn read_package_json_version_distinguishes_broken_from_versionless() {
        let tmp = TempDir::new().unwrap();
        let broken = tmp.path().join("broken.json");
        fs::write(&broken, r#"{"name":"foo","version":"#).unwrap();
        assert!(
            matches!(read_package_json_version(&broken), ReadOutcome::Unreadable(_)),
            "truncated JSON must report a failed read, not an absent field"
        );

        let versionless = tmp.path().join("versionless.json");
        fs::write(&versionless, r#"{"name":"foo","private":true}"#).unwrap();
        assert_eq!(
            read_package_json_version(&versionless),
            ReadOutcome::NoVersionField,
            "a well-formed manifest with no version field is unmanaged, not broken"
        );
    }

    /// ~keep The load-bearing assertion is the last one: `checks.iter().any(|c| !c.matches)`
    /// is verbatim the predicate `bin_cli::release_commands` uses to decide
    /// `process::exit(1)` for `validate versions --exit-code`. Asserting only on the
    /// printed message would still pass if the exit status never changed — which is
    /// exactly the defect this test exists to pin.
    #[test]
    fn unparseable_manifest_fails_validation_rather_than_being_skipped() {
        let tmp = make_workspace("1.0.0");
        fs::write(
            tmp.path().join("crates/mylib-node/package.json"),
            "{\"name\":\"mylib\",\"version\":",
        )
        .unwrap();
        let config = minimal_config(tmp.path());
        let checks = run(&config, tmp.path(), false).unwrap();

        let broken = checks
            .iter()
            .find(|c| c.label == "crates/mylib-node/package.json")
            .expect("an existing but unparseable manifest must still produce a check");
        assert!(!broken.matches, "unparseable manifest must not count as matching");
        assert_eq!(broken.found, None, "no version can be reported for a failed read");
        assert!(
            checks.iter().any(|c| !c.matches),
            "the --exit-code verdict predicate must see the failure"
        );
    }

    #[test]
    fn manifest_without_version_field_is_skipped_not_failed() {
        let tmp = make_workspace("1.0.0");
        fs::write(
            tmp.path().join("package.json"),
            "{\"name\":\"root\",\"private\":true}\n",
        )
        .unwrap();
        let config = minimal_config(tmp.path());
        let checks = run(&config, tmp.path(), false).unwrap();
        assert!(
            checks.iter().all(|c| c.label != "package.json"),
            "a versionless manifest is unmanaged (sync-versions skips it too), so it must not be checked"
        );
        assert!(
            checks.iter().all(|c| c.matches),
            "no failing check expected: {:?}",
            checks.iter().filter(|c| !c.matches).collect::<Vec<_>>()
        );
    }

    #[test]
    fn zero_checks_is_an_error_not_a_vacuous_pass() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.0.0\"\n\n[workspace]\nresolver = \"2\"\n",
        )
        .unwrap();
        let config = minimal_config(root);
        let error = run(&config, root, false)
            .expect_err("examining zero manifests must fail; it used to print 'All 0 manifests consistent'");
        assert!(
            error.to_string().contains("0 manifests"),
            "error should name the empty examination: {error}"
        );
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
    fn check(label: &str, found: Option<&str>, matches: bool, blocked_on_publish: Option<&str>) -> VersionCheck {
        VersionCheck {
            label: label.to_string(),
            found: found.map(str::to_string),
            matches,
            blocked_on_publish: blocked_on_publish.map(str::to_string),
        }
    }

    /// The crates.io leg of a release used to be blocked by a check that could only pass after
    /// that same release: a test app's lockfile pins the crate at the version being published and
    /// resolves it from the registry, so cargo cannot refresh it until the version is live.
    #[test]
    fn a_check_blocked_on_publish_does_not_fail_the_gate() {
        let checks = vec![
            check("packages/ruby/Cargo.toml", Some("3.11.2"), true, None),
            check(
                "test_apps/rust/Cargo.lock#app",
                Some("3.6.18"),
                false,
                Some("sample-crate-rs@3.11.2"),
            ),
        ];
        assert!(
            checks_pass(&checks),
            "a lockfile entry unresolvable until the pending release is published must not fail the gate"
        );
    }

    /// The other half: blocking must not become a blanket amnesty. A genuine mismatch the author
    /// can fix right now still fails, even when a blocked check sits beside it.
    #[test]
    fn a_real_mismatch_still_fails_even_alongside_a_blocked_check() {
        let checks = vec![
            check(
                "test_apps/rust/Cargo.lock#app",
                Some("3.6.18"),
                false,
                Some("sample-crate-rs@3.11.2"),
            ),
            check("packages/ruby/Cargo.toml", Some("3.11.1"), false, None),
        ];
        assert!(
            !checks_pass(&checks),
            "an actionable mismatch must still fail the gate regardless of any blocked check"
        );
    }

    /// A run that examined nothing verified nothing.
    #[test]
    fn an_empty_check_set_is_not_a_pass() {
        assert!(!checks_pass(&[]), "zero checks is a failure, not a vacuous pass");
    }

    /// The three failure modes have to be distinguishable in the report body, because they call
    /// for three different actions: fix the manifest, fix the file that cannot be read, or ship
    /// the release the lockfile is waiting on.
    #[test]
    fn status_label_separates_drift_from_a_publish_blocker() {
        assert_eq!(status_label(&check("a", Some("1.0.0"), true, None)), "ok");
        assert_eq!(status_label(&check("a", Some("0.9.0"), false, None)), "MISMATCH");
        assert_eq!(
            status_label(&check("a", Some("0.9.0"), false, Some("sample@1.0.0"))),
            "UNPUBLISHED"
        );
        assert_eq!(status_label(&check("a", None, false, None)), "UNREADABLE");
    }

    #[test]
    fn failure_line_names_the_release_a_blocked_row_is_waiting_on() {
        let blocked = failure_line(&check(
            "test_apps/rust/Cargo.lock#app",
            Some("1.14.3"),
            false,
            Some("sample@1.15.1"),
        ));
        assert!(
            blocked.contains("unresolvable until sample@1.15.1 is published"),
            "a blocked row must say what it waits on: {blocked}"
        );
        let drift = failure_line(&check("packages/python/pyproject.toml", Some("1.14.3"), false, None));
        assert!(
            !drift.contains("unresolvable"),
            "plain drift must not claim to be blocked: {drift}"
        );
        assert!(drift.contains("FAIL"), "plain drift keeps the FAIL prefix: {drift}");
    }
}
