use crate::core::config::ResolvedCrateConfig;
use crate::core::version::to_rubygems_prerelease;
use anyhow::Context as _;
use tracing::info;

/// Read the version from a Cargo.toml file (workspace or regular package).
pub(crate) fn read_version(version_from: &str) -> anyhow::Result<String> {
    let content =
        std::fs::read_to_string(version_from).with_context(|| format!("failed to read version file {version_from}"))?;
    let value: toml::Value =
        toml::from_str(&content).with_context(|| format!("failed to parse TOML in {version_from}"))?;
    if let Some(v) = value
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(v.to_string());
    }
    if let Some(v) = value
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
    {
        return Ok(v.to_string());
    }
    anyhow::bail!("Could not find version in {version_from}")
}

/// Bump a semver version string by the given component (major, minor, patch).
pub(super) fn bump_version(version: &str, component: &str) -> anyhow::Result<String> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        anyhow::bail!("Invalid semver version: {version}");
    }
    let mut major: u64 = parts[0]
        .parse()
        .with_context(|| format!("Invalid major version component: {}", parts[0]))?;
    let mut minor: u64 = parts[1]
        .parse()
        .with_context(|| format!("Invalid minor version component: {}", parts[1]))?;
    let mut patch: u64 = parts[2]
        .parse()
        .with_context(|| format!("Invalid patch version component: {}", parts[2]))?;

    match component {
        "major" => {
            major += 1;
            minor = 0;
            patch = 0;
        }
        "minor" => {
            minor += 1;
            patch = 0;
        }
        "patch" => {
            patch += 1;
        }
        other => anyhow::bail!("Unknown bump component '{other}': expected major, minor, or patch"),
    }

    Ok(format!("{major}.{minor}.{patch}"))
}

/// Write a bumped version back into a Cargo.toml (workspace or regular package).
pub(super) fn write_version_to_cargo_toml(cargo_toml_path: &str, new_version: &str) -> anyhow::Result<()> {
    use toml_edit::DocumentMut;

    let content =
        std::fs::read_to_string(cargo_toml_path).with_context(|| format!("Failed to read {cargo_toml_path}"))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("Failed to parse TOML in {cargo_toml_path}"))?;

    // dependency (e.g. `[target.'cfg(...)'.dependencies.hf-hub]`), and the regex
    let mut changed = false;

    if let Some(ws_version) = doc
        .get_mut("workspace")
        .and_then(|w| w.as_table_like_mut())
        .and_then(|t| t.get_mut("package"))
        .and_then(|p| p.as_table_like_mut())
        .and_then(|t| t.get_mut("version"))
    {
        if ws_version.is_str() && ws_version.as_str() != Some(new_version) {
            *ws_version = toml_edit::value(new_version);
            changed = true;
        }
    }

    if let Some(pkg_version) = doc
        .get_mut("package")
        .and_then(|p| p.as_table_like_mut())
        .and_then(|t| t.get_mut("version"))
    {
        if pkg_version.is_str() && pkg_version.as_str() != Some(new_version) {
            *pkg_version = toml_edit::value(new_version);
            changed = true;
        }
    }

    if !changed {
        anyhow::bail!(
            "Could not find a `[package]`/`[workspace.package]` version field to update in {cargo_toml_path}"
        );
    }

    std::fs::write(cargo_toml_path, doc.to_string())
        .with_context(|| format!("Failed to write updated version to {cargo_toml_path}"))?;

    Ok(())
}

/// Convert a semver pre-release version to PEP 440 format for Python/PyPI.
/// e.g., "0.1.0-rc.1" → "0.1.0rc1", "0.1.0-alpha.2" → "0.1.0a2", "0.1.0-beta.3" → "0.1.0b3"
/// Non-pre-release versions are returned unchanged.
///
/// Single-pass implementation: builds the result into one pre-allocated
/// `String` instead of chaining five `.replace()` calls (each of which
/// allocates a new intermediate `String`).
pub(crate) fn to_pep440(version: &str) -> String {
    let Some((base, pre)) = version.split_once('-') else {
        return version.to_string();
    };
    let mut out = String::with_capacity(base.len() + pre.len());
    out.push_str(base);
    let pre_norm = if let Some(rest) = pre.strip_prefix("alpha.").or_else(|| pre.strip_prefix("alpha")) {
        out.push('a');
        rest
    } else if let Some(rest) = pre.strip_prefix("beta.").or_else(|| pre.strip_prefix("beta")) {
        out.push('b');
        rest
    } else if let Some(rest) = pre.strip_prefix("rc.").or_else(|| pre.strip_prefix("rc")) {
        out.push_str("rc");
        rest
    } else {
        pre
    };
    for c in pre_norm.chars() {
        if c != '.' {
            out.push(c);
        }
    }
    out
}

/// Patch intra-workspace `version = "..."` pins inside a Cargo.toml dep table,
/// preserving all formatting and comments via `toml_edit`.
///
/// Only dep entries whose key is in `workspace_members` are touched. External
/// crates (e.g. `serde`, `tokio`) are left intact.
///
/// Handles these dep-table shapes:
/// - `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`
/// - `[target.'cfg(...)'.dependencies]` and the dev/build variants
/// - `[workspace.dependencies]` (root manifest only, included when present)
///
/// Returns `true` when at least one version pin was updated.
pub(crate) fn patch_workspace_dep_versions(
    cargo_toml_path: &str,
    new_version: &str,
    workspace_members: &std::collections::HashSet<String>,
) -> anyhow::Result<bool> {
    use toml_edit::{DocumentMut, Item};

    let content =
        std::fs::read_to_string(cargo_toml_path).with_context(|| format!("failed to read {cargo_toml_path}"))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse TOML in {cargo_toml_path}"))?;

    let mut changed = false;

    fn patch_dep_table(
        dep_table: &mut Item,
        new_version: &str,
        workspace_members: &std::collections::HashSet<String>,
    ) -> bool {
        let Some(table) = dep_table.as_table_like_mut() else {
            return false;
        };
        let mut any = false;
        for (key, item) in table.iter_mut() {
            let is_member = workspace_members.contains(key.get())
                || item
                    .as_table_like()
                    .and_then(|t| t.get("package"))
                    .and_then(|v| v.as_str())
                    .is_some_and(|pkg| workspace_members.contains(pkg));
            if !is_member {
                continue;
            }
            if let Some(inline) = item.as_table_like_mut() {
                if let Some(ver_item) = inline.get_mut("version") {
                    if ver_item.as_str() != Some(new_version) {
                        *ver_item = toml_edit::value(new_version);
                        any = true;
                    }
                }
            }
        }
        any
    }

    for table_key in &["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(item) = doc.get_mut(table_key) {
            if patch_dep_table(item, new_version, workspace_members) {
                changed = true;
            }
        }
    }

    if let Some(workspace) = doc.get_mut("workspace") {
        if let Some(ws_table) = workspace.as_table_like_mut() {
            if let Some(deps) = ws_table.get_mut("dependencies") {
                if patch_dep_table(deps, new_version, workspace_members) {
                    changed = true;
                }
            }
        }
    }

    // Walk [target.'cfg(...)'.{dependencies,dev-dependencies,build-dependencies}].
    if let Some(target_item) = doc.get_mut("target") {
        if let Some(target_table) = target_item.as_table_like_mut() {
            let cfg_keys: Vec<String> = target_table.iter().map(|(k, _)| k.to_string()).collect();
            for cfg_key in cfg_keys {
                if let Some(cfg_item) = target_table.get_mut(&cfg_key) {
                    if let Some(cfg_table) = cfg_item.as_table_like_mut() {
                        for dep_key in &["dependencies", "dev-dependencies", "build-dependencies"] {
                            if let Some(dep_item) = cfg_table.get_mut(dep_key) {
                                if patch_dep_table(dep_item, new_version, workspace_members) {
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if changed {
        std::fs::write(cargo_toml_path, doc.to_string())
            .with_context(|| format!("failed to write updated dep versions to {cargo_toml_path}"))?;
    }

    Ok(changed)
}

/// Patch the `version = "..."` field inside a `[patch.crates-io]` entry in a
/// root `Cargo.toml`, when the entry belongs to the named crate.
///
/// Only entries that already carry a `version =` key are touched — path-only
/// entries (e.g. `sample_lib = { path = "crates/sample-lib" }`) are left intact.
///
/// Returns `true` when the version was updated, `false` when it was already
/// correct or no matching entry was found.
pub(crate) fn patch_cargo_crates_io_version(
    cargo_toml_path: &str,
    crate_name: &str,
    new_version: &str,
) -> anyhow::Result<bool> {
    use toml_edit::DocumentMut;

    let content =
        std::fs::read_to_string(cargo_toml_path).with_context(|| format!("failed to read {cargo_toml_path}"))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse TOML in {cargo_toml_path}"))?;

    let Some(patch) = doc.get_mut("patch") else {
        return Ok(false);
    };
    let Some(patch_table) = patch.as_table_like_mut() else {
        return Ok(false);
    };
    let Some(crates_io) = patch_table.get_mut("crates-io") else {
        return Ok(false);
    };
    let Some(crates_io_table) = crates_io.as_table_like_mut() else {
        return Ok(false);
    };
    let Some(entry) = crates_io_table.get_mut(crate_name) else {
        return Ok(false);
    };
    let Some(entry_table) = entry.as_table_like_mut() else {
        return Ok(false);
    };
    let Some(ver_item) = entry_table.get_mut("version") else {
        return Ok(false);
    };
    if ver_item.as_str() == Some(new_version) {
        return Ok(false);
    }
    *ver_item = toml_edit::value(new_version);
    std::fs::write(cargo_toml_path, doc.to_string())
        .with_context(|| format!("failed to write updated patch version to {cargo_toml_path}"))?;
    Ok(true)
}

/// Verify that all package manifest versions match the Cargo.toml source of truth.
/// Returns a list of mismatches (empty = all consistent).
pub fn verify_versions(config: &ResolvedCrateConfig) -> anyhow::Result<Vec<String>> {
    let expected = read_version(&config.version_from)?;
    let expected_pep440 = to_pep440(&expected);
    let expected_rubygems = to_rubygems_prerelease(&expected);
    let mut mismatches = Vec::new();

    fn extract_version(path: &str, pattern: &str) -> Option<String> {
        use std::collections::HashMap;
        use std::sync::Mutex;
        use std::sync::OnceLock;
        static CACHE: OnceLock<Mutex<HashMap<String, regex::Regex>>> = OnceLock::new();
        let content = std::fs::read_to_string(path).ok()?;
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = cache.lock().ok()?;
        let re = match guard.get(pattern) {
            Some(re) => re.clone(),
            None => {
                let re = regex::Regex::new(pattern).ok()?;
                guard.insert(pattern.to_string(), re.clone());
                re
            }
        };
        drop(guard);
        re.captures(&content)?.get(1).map(|m| m.as_str().to_string())
    }

    if let Some(found) = extract_version("packages/python/pyproject.toml", r#"version\s*=\s*"([^"]*)""#) {
        if found != expected_pep440 {
            mismatches.push(format!(
                "packages/python/pyproject.toml: found {found}, expected {expected_pep440}"
            ));
        }
    }

    if let Some(found) = extract_version("packages/node/package.json", r#""version"\s*:\s*"([^"]*)""#) {
        if found != expected {
            mismatches.push(format!(
                "packages/node/package.json: found {found}, expected {expected}"
            ));
        }
    }

    if let Some(found) = extract_version("packages/java/pom.xml", r"<version>([^<]*)</version>") {
        if found != expected {
            mismatches.push(format!("packages/java/pom.xml: found {found}, expected {expected}"));
        }
    }

    // Elixir — check both `version: "X.Y.Z"` and `@version "X.Y.Z"` patterns
    if let Some(found) = extract_version("packages/elixir/mix.exs", r#"version:\s*"([^"]*)""#)
        .or_else(|| extract_version("packages/elixir/mix.exs", r#"@version\s*"([^"]*)""#))
    {
        if found != expected {
            mismatches.push(format!("packages/elixir/mix.exs: found {found}, expected {expected}"));
        }
    }

    if let Ok(entries) = std::fs::read_dir("packages/ruby") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gemspec") {
                if let Some(found) = extract_version(
                    &path.to_string_lossy(),
                    r"spec\.version\s*=\s*['\x22]([^'\x22]*)['\x22]",
                ) {
                    if found != expected_rubygems {
                        mismatches.push(format!(
                            "{}: found {found}, expected {expected_rubygems}",
                            path.display()
                        ));
                    }
                }
            }
        }
    }

    for pattern in &[
        "packages/ruby/lib/*/version.rb",
        "packages/ruby/ext/*/src/*/version.rb",
        "packages/ruby/ext/*/native/src/*/version.rb",
    ] {
        if let Ok(entries) = glob::glob(pattern) {
            for entry in entries.flatten() {
                if let Some(found) = extract_version(&entry.to_string_lossy(), r#"VERSION\s*=\s*["']([^"']*)["']"#) {
                    if found != expected_rubygems {
                        mismatches.push(format!(
                            "{}: found {found}, expected {expected_rubygems}",
                            entry.display()
                        ));
                    }
                }
            }
        }
    }

    if let Some(found) = extract_version(
        "packages/csharp/SampleCrawler/SampleCrawler.csproj",
        r"<Version>([^<]*)</Version>",
    ) {
        if found != expected {
            mismatches.push(format!("packages/csharp: found {found}, expected {expected}"));
        }
    }

    if let Some(found) = extract_version("packages/php/composer.json", r#""version"\s*:\s*"([^"]*)""#) {
        if found != expected {
            mismatches.push(format!(
                "packages/php/composer.json: found {found}, expected {expected}"
            ));
        }
    }

    if let Some(found) = extract_version("packages/dart/pubspec.yaml", r"(?m)^version:\s*([^\s#\n]+)") {
        if found != expected {
            mismatches.push(format!(
                "packages/dart/pubspec.yaml: found {found}, expected {expected}"
            ));
        }
    }

    if let Some(found) = extract_version("packages/zig/build.zig.zon", r#"(?m)^\s*\.version\s*=\s*"([^"]*)""#) {
        if found != expected {
            mismatches.push(format!(
                "packages/zig/build.zig.zon: found {found}, expected {expected}"
            ));
        }
    }

    if let Some(found) = extract_version(
        "Package.swift",
        r#"releases/download/v(\d+\.\d+\.\d+(?:-[a-zA-Z0-9._]+)*)/"#,
    ) {
        if found != expected {
            mismatches.push(format!("Package.swift: found {found}, expected {expected}"));
        }
    }

    Ok(mismatches)
}

/// Set an explicit version in the Cargo.toml (supports pre-release versions like 0.1.0-rc.1).
pub fn set_version(config: &ResolvedCrateConfig, version: &str) -> anyhow::Result<()> {
    write_version_to_cargo_toml(&config.version_from, version)
        .with_context(|| format!("failed to set version to {version}"))?;
    info!("Set version to {version} in {}", config.version_from);
    Ok(())
}
