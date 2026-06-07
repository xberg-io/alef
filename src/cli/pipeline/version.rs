use crate::core::config::{Language, ResolvedCrateConfig};
use anyhow::Context as _;
use std::sync::LazyLock;
use tracing::{debug, info, warn};

use super::helpers::{run_command, run_optional};
use super::version_regen::{regenerate_readmes, regenerate_scaffold_after_sync, regenerate_test_apps_after_sync};
use super::version_registry::sync_registry_package_versions;
use super::version_swift::precompute_swift_checksum;
use super::version_text::{
    read_workspace_license, render_citation_cff, replace_citation_version, replace_gradle_project_version,
    replace_version_pattern, restore_gleam_dep_ranges, sync_cargo_lock_path_versions, sync_docs_version_badges,
    sync_e2e_dart_pubspec_lock, sync_e2e_go_mod, sync_e2e_java_pom, sync_gemfile_lock,
};
use crate::core::version::to_r_version;

/// Regex for matching version field in Cargo.toml format files.
static CARGO_VERSION_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r#"(?m)^(version\s*=\s*)"[^"]*""#).expect("valid regex"));

/// Regex for matching semantic version strings.
static SEMVER_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\d+\.\d+\.\d+(-[a-zA-Z0-9._]+)*").expect("valid regex"));

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
fn bump_version(version: &str, component: &str) -> anyhow::Result<String> {
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
fn write_version_to_cargo_toml(cargo_toml_path: &str, new_version: &str) -> anyhow::Result<()> {
    let content =
        std::fs::read_to_string(cargo_toml_path).with_context(|| format!("Failed to read {cargo_toml_path}"))?;

    // Match `version = "..."` as a standalone line (covers both [package] and [workspace.package])
    let new_content = CARGO_VERSION_RE
        .replace(&content, format!(r#"version = "{new_version}""#).as_str())
        .to_string();

    if new_content == content {
        anyhow::bail!("Could not find a `version = \"...\"` field to update in {cargo_toml_path}");
    }

    std::fs::write(cargo_toml_path, new_content)
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
pub(super) fn to_pep440(version: &str) -> String {
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

use crate::core::version::to_rubygems_prerelease;

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

    // Patch a single dep-table item in-place. Returns true when any version was
    // updated. `dep_table` must be a `&mut Item` pointing at an inline or
    // regular TOML table of `{ dep-name = { version = "...", ... } }` entries.
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
            // Only touch deps whose name is a workspace member.
            if !workspace_members.contains(key.get()) {
                continue;
            }
            // The dep value can be an inline table `{ path = "...", version = "X" }`.
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

    // Walk standard top-level dep tables.
    for table_key in &["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(item) = doc.get_mut(table_key) {
            if patch_dep_table(item, new_version, workspace_members) {
                changed = true;
            }
        }
    }

    // Walk [workspace.dependencies] (root manifest only).
    // We use a two-level path so we don't accidentally touch
    // `[workspace.package]` or other sibling keys.
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
            // Collect the keys first to avoid borrow conflicts.
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

/// Verify that all package manifest versions match the Cargo.toml source of truth.
/// Returns a list of mismatches (empty = all consistent).
pub fn verify_versions(config: &ResolvedCrateConfig) -> anyhow::Result<Vec<String>> {
    let expected = read_version(&config.version_from)?;
    let expected_pep440 = to_pep440(&expected);
    let expected_rubygems = to_rubygems_prerelease(&expected);
    let mut mismatches = Vec::new();

    // Cache compiled regexes across calls within this verify pass — the same
    // ~15 patterns get reused on every invocation, and `Regex::new` is the
    // dominant cost when the function is called from a tight loop.
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

    // Python (PEP 440 format)
    if let Some(found) = extract_version("packages/python/pyproject.toml", r#"version\s*=\s*"([^"]*)""#) {
        if found != expected_pep440 {
            mismatches.push(format!(
                "packages/python/pyproject.toml: found {found}, expected {expected_pep440}"
            ));
        }
    }

    // Node
    if let Some(found) = extract_version("packages/typescript/package.json", r#""version"\s*:\s*"([^"]*)""#) {
        if found != expected {
            mismatches.push(format!(
                "packages/typescript/package.json: found {found}, expected {expected}"
            ));
        }
    }

    // Java
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

    // Ruby gemspec (compare normalized form)
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

    // Ruby version.rb files (packages/ruby/{lib/*/,ext/*/src/*/,ext/*/native/src/*/}version.rb) (compare normalized form)
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

    // C# csproj
    if let Some(found) = extract_version(
        "packages/csharp/SampleCrawler/SampleCrawler.csproj",
        r"<Version>([^<]*)</Version>",
    ) {
        if found != expected {
            mismatches.push(format!("packages/csharp: found {found}, expected {expected}"));
        }
    }

    // PHP composer.json
    if let Some(found) = extract_version("packages/php/composer.json", r#""version"\s*:\s*"([^"]*)""#) {
        if found != expected {
            mismatches.push(format!(
                "packages/php/composer.json: found {found}, expected {expected}"
            ));
        }
    }

    // Dart pubspec.yaml — `version: X.Y.Z`
    if let Some(found) = extract_version("packages/dart/pubspec.yaml", r"(?m)^version:\s*([^\s#\n]+)") {
        if found != expected {
            mismatches.push(format!(
                "packages/dart/pubspec.yaml: found {found}, expected {expected}"
            ));
        }
    }

    // Zig build.zig.zon — `.version = "X.Y.Z"`. The `(?m)^\s*\.version\b`
    // anchor is required so the `.minimum_zig_version = "..."` line on the
    // same file is not picked up by the looser `.version` substring match.
    if let Some(found) = extract_version("packages/zig/build.zig.zon", r#"(?m)^\s*\.version\s*=\s*"([^"]*)""#) {
        if found != expected {
            mismatches.push(format!(
                "packages/zig/build.zig.zon: found {found}, expected {expected}"
            ));
        }
    }

    // Swift Package.swift binary release URL, when the root package opts into binary distribution.
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

/// Sync version from Cargo.toml to all package manifest files.
///
/// When `no_regen` is `false` (the default for direct CLI invocations), this
/// function automatically regenerates `test_apps/` scaffold files after updating
/// `[crates.e2e.registry.packages.*].version` in `alef.toml`, so the version
/// pins in generated files (pyproject.toml, mix.exs, build.zig.zon, Package.swift,
/// etc.) always match the workspace version atomically.
///
/// Pass `no_regen = true` to opt out of the automatic regeneration — useful when
/// this function is called from within another codegen pass that owns `test_apps/`
/// itself (e.g. `alef generate`).
pub fn sync_versions(
    config: &ResolvedCrateConfig,
    config_path: &std::path::Path,
    bump: Option<&str>,
    no_regen: bool,
    skip_swift_checksum: bool,
) -> anyhow::Result<()> {
    // If bump is requested, read current version, bump it, and write it back to Cargo.toml.
    if let Some(component) = bump {
        let current = read_version(&config.version_from)?;
        let bumped = bump_version(&current, component)?;
        info!("Bumping version {current} -> {bumped} ({component})");
        write_version_to_cargo_toml(&config.version_from, &bumped).context("failed to sync versions")?;
        info!("Updated {} with bumped version {bumped}", config.version_from);
    }

    let version = read_version(&config.version_from)?;

    // Always do the manifest scan. The previous warm-path short-circuit
    // checked `.alef/last_synced_version` and returned early when the
    // canonical version matched, which silently masked real drift:
    // a manifest hand-edited to the wrong version, a newly-added manifest
    // file (e.g. `e2e/rust/Cargo.toml` introduced after the last sync), or
    // a stale `alef:hash:` line all looked the same as "already synced"
    // because the cache key was only the version string. CI runs without
    // the cache, so it produced a different result and the alef-sync-versions
    // hook failed for downstream consumers. The scan is fast (sub-second
    // on sample_core-sized repos) and the work is idempotent when nothing
    // is actually stale.
    let last_path = std::path::Path::new(".alef").join("last_synced_version");
    info!("Syncing version {version}");

    let mut updated = vec![];
    // Track which ecosystems had manifests rewritten, so we can refresh
    // their lockfiles after all updates complete (BLK-11).
    let mut any_node_pkg_modified = false;
    let mut any_cargo_toml_modified = false;
    let mut any_composer_json_modified = false;
    let mut any_mix_exs_modified = false;
    // All paths matched by [[workspace.sync.text_replacements]] globs, whether
    // or not the version substitution actually changed their content.  Used to
    // ensure finalize_hashes is called on every sync target so that stale
    // alef:hash: lines are refreshed even when the version was already correct.
    let mut text_replacement_paths: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();

    // Workspace Cargo.toml files: sync [package] version in both members and excluded crates.
    // After updating [package] version, also patch intra-workspace dep version pins so that
    // entries like `sample_core = { path = "...", version = "X.Y.Z" }` get bumped to match.
    if let Ok(root_content) = std::fs::read_to_string("Cargo.toml") {
        if let Ok(root_toml) = root_content.parse::<toml::Table>() {
            let empty_vec = vec![];
            let members = root_toml
                .get("workspace")
                .and_then(|w| w.get("members"))
                .and_then(|m| m.as_array())
                .unwrap_or(&empty_vec);
            let excludes = root_toml
                .get("workspace")
                .and_then(|w| w.get("exclude"))
                .and_then(|m| m.as_array())
                .unwrap_or(&empty_vec);

            // Collect all workspace-member crate names so we can identify
            // which dep entries to bump. A crate name is the `name` field in
            // its [package] table. Delegated to the shared discovery helper
            // (globs the same members + exclude patterns relative to the
            // workspace root, which is the current working directory here).
            let workspace_member_names: std::collections::HashSet<String> =
                crate::publish::workspace::workspace_member_crates(std::path::Path::new("."))
                    .map(|m| m.names.into_iter().collect())
                    .unwrap_or_default();

            // Collect all matching Cargo.toml paths for the member/exclude update pass.
            let mut cargo_toml_paths: Vec<String> = vec![];
            for pattern_val in members.iter().chain(excludes.iter()) {
                if let Some(pattern) = pattern_val.as_str() {
                    if let Ok(paths) = glob::glob(&format!("{pattern}/Cargo.toml")) {
                        for entry in paths.flatten() {
                            cargo_toml_paths.push(entry.to_string_lossy().to_string());
                        }
                    }
                }
            }

            // Also include non-workspace Cargo crates under packages/*/rust/Cargo.toml
            // (e.g. packages/swift/rust/Cargo.toml, packages/dart/rust/Cargo.toml).
            // These live outside the [workspace] members list but still need version syncing.
            for entry in glob::glob("packages/*/rust/Cargo.toml").into_iter().flatten().flatten() {
                let path_str = entry.to_string_lossy().to_string();
                if !cargo_toml_paths.contains(&path_str) {
                    cargo_toml_paths.push(path_str);
                }
            }

            for path_str in &cargo_toml_paths {
                // Update [package] version (regex-anchored to start-of-line).
                // Skip crates that use workspace version inheritance or have no version.
                if write_version_to_cargo_toml(path_str, &version).is_ok() && !updated.contains(path_str) {
                    updated.push(path_str.clone());
                    any_cargo_toml_modified = true;
                }
                // Also patch intra-workspace dep version pins in all dep tables.
                if !workspace_member_names.is_empty() {
                    match patch_workspace_dep_versions(path_str, &version, &workspace_member_names) {
                        Ok(true) => {
                            if !updated.contains(path_str) {
                                updated.push(path_str.clone());
                                any_cargo_toml_modified = true;
                            }
                        }
                        Ok(false) => {}
                        Err(e) => {
                            debug!("Could not patch dep versions in {path_str}: {e}");
                        }
                    }
                }
            }

            // Patch [workspace.dependencies] in the root Cargo.toml.
            if !workspace_member_names.is_empty() {
                match patch_workspace_dep_versions("Cargo.toml", &version, &workspace_member_names) {
                    Ok(true) => {
                        if !updated.contains(&"Cargo.toml".to_string()) {
                            updated.push("Cargo.toml".to_string());
                            any_cargo_toml_modified = true;
                        }
                    }
                    Ok(false) => {}
                    Err(e) => {
                        debug!("Could not patch workspace dep versions in root Cargo.toml: {e}");
                    }
                }
            }
        }
    }

    // Python: pyproject.toml — convert semver pre-release to PEP 440 format
    // e.g., "0.1.0-rc.1" → "0.1.0rc1", "0.1.0-alpha.2" → "0.1.0a2", "0.1.0-beta.3" → "0.1.0b3"
    //
    // Three candidate paths are checked, deduplicated by canonicalized path:
    //   1. "packages/python/pyproject.toml" — legacy default kept for back-compat.
    //   2. "{config.package_dir(Language::Python)}/pyproject.toml" — configurable
    //      distribution manifest (e.g. when [crates.output] python = "packages/mypkg/").
    //   3. "{config.output_for("python")}/pyproject.toml" — the maturin-build
    //      pyproject that lives alongside the PyO3 source crate (e.g.
    //      "crates/{lib}-py/src/pyproject.toml").  This was the missed case that
    //      caused version drift in prerelease downstream packages.
    let python_version = to_pep440(&version);
    {
        let pkg_dir = config.package_dir(Language::Python);
        let mut python_paths: Vec<String> = vec![
            "packages/python/pyproject.toml".to_string(),
            std::path::Path::new(&pkg_dir)
                .join("pyproject.toml")
                .to_string_lossy()
                .into_owned(),
        ];
        if let Some(output_dir) = config.output_for("python") {
            python_paths.push(output_dir.join("pyproject.toml").to_string_lossy().into_owned());
        }
        // Dedup by canonicalized path so we don't write the same file twice when
        // multiple candidates resolve to the same location on disk.
        let mut seen_canonical: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
        for python_path in python_paths {
            // Soft-skip missing files — not every repo uses every layout.
            let canonical = match std::fs::canonicalize(&python_path) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if !seen_canonical.insert(canonical) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&python_path) {
                if let Some(new_content) = replace_version_pattern(&content, r#"version = "[^"]*""#, &python_version) {
                    std::fs::write(&python_path, &new_content)
                        .with_context(|| format!("failed to write {python_path}"))?;
                    updated.push(python_path);
                }
            }
        }
    }

    // Node: package.json — use the configured Node package_dir, falling back
    // to "packages/node" (the modern default) and "packages/typescript" (legacy
    // path retained so older repos that still use the old default keep syncing).
    let node_pkg_dir = config.package_dir(Language::Node);
    let mut node_paths: Vec<String> = vec![format!("{node_pkg_dir}/package.json")];
    if node_pkg_dir != "packages/typescript" {
        node_paths.push("packages/typescript/package.json".to_string());
    }
    for node_path in node_paths {
        if let Ok(content) = std::fs::read_to_string(&node_path) {
            if let Some(new_content) = replace_version_pattern(&content, r#""version": "[^"]*""#, &version) {
                std::fs::write(&node_path, &new_content).with_context(|| format!("failed to write {node_path}"))?;
                updated.push(node_path);
                any_node_pkg_modified = true;
            }
        }
    }

    // Ruby: *.gemspec (convert to RubyGems prerelease format)
    let ruby_version = to_rubygems_prerelease(&version);
    if let Ok(entries) = std::fs::read_dir("packages/ruby") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gemspec") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(new_content) =
                        replace_version_pattern(&content, r#"spec\.version\s*=\s*['"][^'"]*['"]"#, &ruby_version)
                    {
                        std::fs::write(&path, &new_content)?;
                        updated.push(path.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // Ruby: {lib/*/,ext/*/src/*/,ext/*/native/src/*/}version.rb (convert to RubyGems prerelease format)
    for pattern in &[
        "packages/ruby/lib/*/version.rb",
        "packages/ruby/ext/*/src/*/version.rb",
        "packages/ruby/ext/*/native/src/*/version.rb",
    ] {
        for entry in glob::glob(pattern).into_iter().flatten().flatten() {
            if let Ok(content) = std::fs::read_to_string(&entry) {
                if let Some(new_content) =
                    replace_version_pattern(&content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, &ruby_version)
                {
                    std::fs::write(&entry, &new_content)?;
                    updated.push(entry.to_string_lossy().to_string());
                }
            }
        }
    }

    // Ruby: Gemfile.lock — update the path-gem version entries so bundler does not
    // reject the lockfile with "frozen mode" errors on the next CI run.
    // The lockfile contains the gem version in two places:
    //   1. Under PATH > specs: `    <name> (<version>)` (4-space indent)
    //   2. Under CHECKSUMS:    `  <name> (<version>)` (2-space indent, no sha256)
    // We replace both textually, reusing the already-computed ruby_version.
    let gemfile_lock_path = std::path::Path::new("packages/ruby/Gemfile.lock");
    if gemfile_lock_path.exists() {
        if let Ok(content) = std::fs::read_to_string(gemfile_lock_path) {
            if let Some(new_content) = sync_gemfile_lock(&content, &ruby_version) {
                std::fs::write(gemfile_lock_path, &new_content)
                    .context("failed to write packages/ruby/Gemfile.lock")?;
                updated.push("packages/ruby/Gemfile.lock".to_string());
            }
        }
    }

    // PHP: composer.json
    if let Ok(content) = std::fs::read_to_string("packages/php/composer.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version": "[^"]*""#, &version) {
            std::fs::write("packages/php/composer.json", &new_content)?;
            updated.push("packages/php/composer.json".to_string());
            any_composer_json_modified = true;
        }
    }

    // Elixir: mix.exs — handle both `version: "X.Y.Z"` and `@version "X.Y.Z"` patterns
    if let Ok(content) = std::fs::read_to_string("packages/elixir/mix.exs") {
        if let Some(new_content) = replace_version_pattern(&content, r#"version: "[^"]*""#, &version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            updated.push("packages/elixir/mix.exs".to_string());
            any_mix_exs_modified = true;
        } else if let Some(new_content) = replace_version_pattern(&content, r#"@version "[^"]*""#, &version) {
            std::fs::write("packages/elixir/mix.exs", &new_content)?;
            updated.push("packages/elixir/mix.exs".to_string());
            any_mix_exs_modified = true;
        }
    }

    // Elixir NIF crate Cargo.lock: a Rustler NIF crate lives under the Elixir
    // package's `native/<nif>/` directory and ships a committed `Cargo.lock`
    // inside the Hex source tarball. The NIF `Cargo.toml` is bumped elsewhere
    // (workspace member pass or a `[sync].extra_paths` entry), but its committed
    // lockfile keeps the OLD version on every local/path-source entry — the
    // consumer's own crates plus the NIF crate itself — which makes `cargo build`
    // from the published tarball fail with a lock/manifest version mismatch.
    // Glob the lockfiles under the Elixir package's `native/` tree (the NIF crate
    // name is consumer-specific) and rewrite only their sourceless entries.
    {
        let elixir_pkg = config.package_dir(Language::Elixir);
        let nif_lock_glob = format!("{elixir_pkg}/native/*/Cargo.lock");
        for entry in glob::glob(&nif_lock_glob).into_iter().flatten().flatten() {
            if let Ok(content) = std::fs::read_to_string(&entry) {
                if let Some(new_content) = sync_cargo_lock_path_versions(&content, &version) {
                    std::fs::write(&entry, &new_content)
                        .with_context(|| format!("failed to write {}", entry.display()))?;
                    updated.push(entry.to_string_lossy().to_string());
                }
            }
        }
    }

    // Go: go.mod (no version field, skip)

    // Java: pom.xml
    if let Ok(content) = std::fs::read_to_string("packages/java/pom.xml") {
        if let Some(new_content) = replace_version_pattern(&content, r#"<version>[^<]*</version>"#, &version) {
            std::fs::write("packages/java/pom.xml", &new_content)?;
            updated.push("packages/java/pom.xml".to_string());
        }
    }

    // C#: *.csproj (recursive under packages/csharp)
    for entry in glob::glob("packages/csharp/**/*.csproj")
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(content) = std::fs::read_to_string(&entry) {
            if let Some(new_content) = replace_version_pattern(&content, r#"<Version>[^<]*</Version>"#, &version) {
                std::fs::write(&entry, &new_content)?;
                updated.push(entry.to_string_lossy().to_string());
            }
        }
    }

    // Kotlin (JVM/Multiplatform): packages/kotlin/build.gradle.kts carries a
    // top-level `version = "..."` (Gradle `Project.version`) used by the
    // maven-publish task. It is distinct from the e2e Gradle build (handled via
    // the e2e codegen path) and from plugin/extension `version` constructs in the
    // same file, which `replace_gradle_project_version` deliberately skips.
    let kotlin_gradle = std::path::Path::new(&config.package_dir(Language::Kotlin)).join("build.gradle.kts");
    if let Ok(content) = std::fs::read_to_string(&kotlin_gradle) {
        if let Some(new_content) = replace_gradle_project_version(&content, &version) {
            std::fs::write(&kotlin_gradle, &new_content)
                .with_context(|| format!("failed to write {}", kotlin_gradle.display()))?;
            updated.push(kotlin_gradle.to_string_lossy().to_string());
        }
    }

    // Kotlin Android: packages/kotlin-android/build.gradle.kts carries the
    // library version in a `coordinates(... version = "...")` block. The same
    // `replace_gradle_project_version` function is safe to use here — it anchors
    // to the first start-of-line `version = "..."` assignment, which in the
    // Android build file is the `coordinates` version, not a plugin declaration.
    let kotlin_android_gradle =
        std::path::Path::new(&config.package_dir(Language::KotlinAndroid)).join("build.gradle.kts");
    if let Ok(content) = std::fs::read_to_string(&kotlin_android_gradle) {
        if let Some(new_content) = replace_gradle_project_version(&content, &version) {
            std::fs::write(&kotlin_android_gradle, &new_content)
                .with_context(|| format!("failed to write {}", kotlin_android_gradle.display()))?;
            updated.push(kotlin_android_gradle.to_string_lossy().to_string());
        }
    }

    // WASM: package.json
    for wasm_pkg in glob::glob("crates/*-wasm/package.json").into_iter().flatten().flatten() {
        if let Ok(content) = std::fs::read_to_string(&wasm_pkg) {
            if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
                std::fs::write(&wasm_pkg, &new_content)?;
                updated.push(wasm_pkg.to_string_lossy().to_string());
            }
        }
    }

    // Node binding crate manifest: crates/*-node/package.json. Some repos keep
    // a `package.json` next to the NAPI-RS binding crate (alongside the Cargo
    // manifest) so `npm publish --workspace` can resolve the prebuilt binary.
    // `validate-versions` already checks this file; sync must write to it too.
    //
    // Two version surfaces live in this manifest and both must be bumped:
    //   1. the top-level `"version"` (the parent NAPI package version), and
    //   2. every `optionalDependencies` entry pointing at a sibling NAPI
    //      platform package (e.g. `"@scope/foo-linux-x64-gnu": "X.Y.Z"`).
    // Leaving (2) stale makes `pnpm install --frozen-lockfile` fail with
    // `ERR_PNPM_OUTDATED_LOCKFILE` because the lockfile's recorded specifiers
    // diverge from the manifest. The platform deps are emitted by the scaffold
    // with the parent's version (see `scaffold_node`), so they are always
    // in lock-step with the parent and can be rewritten unconditionally.
    for node_pkg in glob::glob("crates/*-node/package.json").into_iter().flatten().flatten() {
        if let Ok(content) = std::fs::read_to_string(&node_pkg) {
            let mut working = content.clone();
            if let Some(rewritten) = replace_version_pattern(&working, r#""version":\s*"[^"]*""#, &version) {
                working = rewritten;
            }
            // Rewrite sibling NAPI platform-package version pins. Source the
            // parent package name from the manifest itself so this stays
            // generic across consumers (no hardcoded scope/prefix).
            if let Ok(pkg_json) = serde_json::from_str::<serde_json::Value>(&working) {
                if let Some(parent_name) = pkg_json.get("name").and_then(|v| v.as_str()) {
                    let pattern = format!(r#""({}-[^"]+)":\s*"[^"]*""#, regex::escape(parent_name));
                    if let Ok(re) = regex::Regex::new(&pattern) {
                        let replacement = format!(r#""$1": "{version}""#);
                        working = re.replace_all(&working, replacement.as_str()).to_string();
                    }
                }
            }
            if working != content {
                std::fs::write(&node_pkg, &working)?;
                updated.push(node_pkg.to_string_lossy().to_string());
                any_node_pkg_modified = true;
            }
        }
    }

    // Pre-staged NAPI platform manifests: crates/*-node/npm/<platform>/package.json.
    // alef pre-stages these at scaffold time so `napi prepublish` does not have to
    // `napi create-npm-dirs` during release; each carries its own top-level
    // `"version"` that must follow the parent package version on every bump.
    for platform_pkg in glob::glob("crates/*-node/npm/*/package.json")
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(content) = std::fs::read_to_string(&platform_pkg) {
            if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
                std::fs::write(&platform_pkg, &new_content)?;
                updated.push(platform_pkg.to_string_lossy().to_string());
                any_node_pkg_modified = true;
            }
        }
    }

    // Root package.json (if present): typically a private "root" manifest
    // bookkeeping pnpm workspaces alongside the published bindings. Without
    // this, `validate-versions` flags a mismatch every release because the
    // root manifest carries its own `"version"` that nothing else writes to.
    if let Ok(content) = std::fs::read_to_string("package.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
            std::fs::write("package.json", &new_content)?;
            updated.push("package.json".to_string());
            any_node_pkg_modified = true;
        }
    }

    // Root composer.json (if present)
    if let Ok(content) = std::fs::read_to_string("composer.json") {
        if let Some(new_content) = replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version) {
            std::fs::write("composer.json", &new_content)?;
            updated.push("composer.json".to_string());
            any_composer_json_modified = true;
        }
    }

    // R: DESCRIPTION file — CRAN rejects SemVer dash prereleases.
    if let Ok(content) = std::fs::read_to_string("packages/r/DESCRIPTION") {
        let r_version = to_r_version(&version);
        if let Some(new_content) = replace_version_pattern(&content, r"Version:\s*[^\n]*", &r_version) {
            std::fs::write("packages/r/DESCRIPTION", &new_content)?;
            updated.push("packages/r/DESCRIPTION".to_string());
        }
    }

    // Dart: pubspec.yaml — uses `version: X.Y.Z` YAML syntax (unquoted, no quotes)
    if let Ok(content) = std::fs::read_to_string("packages/dart/pubspec.yaml") {
        static PUBSPEC_VERSION_RE: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"(?m)^version:\s*[^\s#\n]+").expect("valid regex"));
        let new_content = PUBSPEC_VERSION_RE
            .replace(&content, format!("version: {version}").as_str())
            .into_owned();
        if new_content != content {
            std::fs::write("packages/dart/pubspec.yaml", &new_content)?;
            updated.push("packages/dart/pubspec.yaml".to_string());
        }
    }

    // Zig: build.zig.zon — `.version = "X.Y.Z"`. The anchor `(?m)^(\s*)\.version`
    // captures the leading indent so the rewrite preserves it, and prevents the
    // `.minimum_zig_version = "..."` line on the same file from being touched
    // (it starts with `.minimum_zig_version`, not `.version`).
    if let Ok(content) = std::fs::read_to_string("packages/zig/build.zig.zon") {
        static ZON_VERSION_RE: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r#"(?m)^(\s*)\.version\s*=\s*"[^"]*""#).expect("valid regex"));
        let new_content = ZON_VERSION_RE
            .replace(&content, format!(r#"$1.version = "{version}""#).as_str())
            .into_owned();
        if new_content != content {
            std::fs::write("packages/zig/build.zig.zon", &new_content)?;
            updated.push("packages/zig/build.zig.zon".to_string());
        }
    }

    // Python: __init__.py — may sit at packages/python/__init__.py (flat layout)
    // or packages/python/<module>/__init__.py (src layout with named package).
    // Walk both shapes with a glob so consumers that use either convention are
    // covered by a single sync pass.
    for py_init in glob::glob("packages/python/**/__init__.py")
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(content) = std::fs::read_to_string(&py_init) {
            if let Some(new_content) = replace_version_pattern(&content, r#"__version__\s*=\s*"[^"]*""#, &version) {
                std::fs::write(&py_init, &new_content)
                    .with_context(|| format!("failed to write {}", py_init.display()))?;
                updated.push(py_init.to_string_lossy().to_string());
            }
        }
    }

    // Go: ffi_loader.go
    if let Ok(content) = std::fs::read_to_string("packages/go/ffi_loader.go") {
        if let Some(new_content) = replace_version_pattern(&content, r#"defaultFFIVersion\s*=\s*"[^"]*""#, &version) {
            std::fs::write("packages/go/ffi_loader.go", &new_content)?;
            updated.push("packages/go/ffi_loader.go".to_string());
        }
    }

    // Go: cmd/download_ffi/main.go — `moduleVersion` constant is interpolated
    // by the Go backend at binding-generation time and therefore not covered by
    // the regular `alef generate` / `alef all` flow when only `sync-versions` is
    // run. Without this, consumers of a freshly released version will pull the
    // prior release's FFI binary because `moduleVersion` still points at the old tag.
    for entry in glob::glob("packages/go/cmd/download_ffi/main.go")
        .into_iter()
        .flatten()
        .flatten()
    {
        if let Ok(content) = std::fs::read_to_string(&entry) {
            if let Some(new_content) = replace_version_pattern(&content, r#"moduleVersion\s*=\s*"[^"]*""#, &version) {
                std::fs::write(&entry, &new_content).with_context(|| format!("failed to write {}", entry.display()))?;
                updated.push(entry.to_string_lossy().to_string());
            }
        }
    }

    // Swift Package.swift files: root + test_apps + e2e.
    // Root Package.swift (seed file) uses `url: "...v__ALEF_SWIFT_VERSION__..."` placeholder.
    // Generated test_apps and e2e entries use `from: "X.Y.Z"` version bounds.
    // Without bumping these, `swift package resolve` fetches the prior release when
    // the app is run against a freshly cut tag — causing 404s or wrong-version failures.
    // The glob crate (0.3.x) does not support brace alternatives, so we run separate passes.

    // Root Package.swift (seed file with binary URL placeholder)
    if let Ok(content) = std::fs::read_to_string("Package.swift") {
        let new_content = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{version}"));
        if new_content != content {
            std::fs::write("Package.swift", &new_content)?;
            updated.push("Package.swift".to_string());
        }
    }

    // test_apps/*/Package.swift and e2e/*/Package.swift (generated entries with `from:` bounds)
    for swift_pkg_pattern in &["test_apps/*/Package.swift", "e2e/*/Package.swift"] {
        for swift_pkg in glob::glob(swift_pkg_pattern).into_iter().flatten().flatten() {
            if let Ok(content) = std::fs::read_to_string(&swift_pkg) {
                if let Some(new_content) = replace_version_pattern(&content, r#"from:\s*"[^"]*""#, &version) {
                    std::fs::write(&swift_pkg, &new_content)
                        .with_context(|| format!("failed to write {}", swift_pkg.display()))?;
                    updated.push(swift_pkg.to_string_lossy().to_string());
                }
            }
        }
    }

    // C FFI download_ffi.sh: the generated shell helper declares `VERSION="X.Y.Z"`
    // (no spaces around `=`) at the top of the script so it can construct the
    // correct GitHub-Releases tarball URL for the prebuilt FFI binary. Both the
    // e2e and test_apps copies must be bumped in lock-step with the workspace version.
    // The glob crate (0.3.x) does not support brace alternatives; run two passes.
    for sh_pattern in &["e2e/c/download_ffi.sh", "test_apps/c/download_ffi.sh"] {
        for sh_script in glob::glob(sh_pattern).into_iter().flatten().flatten() {
            if let Ok(content) = std::fs::read_to_string(&sh_script) {
                if let Some(new_content) = replace_version_pattern(&content, r#"VERSION="[^"]*""#, &version) {
                    std::fs::write(&sh_script, &new_content)
                        .with_context(|| format!("failed to write {}", sh_script.display()))?;
                    updated.push(sh_script.to_string_lossy().to_string());
                }
            }
        }
    }

    // E2e manifests: the generated integration test trees under e2e/<lang>/ use
    // local-source references to the library packages but still embed a hardcoded
    // version string in language-native manifests (pom.xml, Gemfile.lock, go.mod,
    // pubspec.lock). Without syncing these, CI's frozen-lockfile / dependency-
    // resolution modes reject the mismatched version on the next run.
    //
    // Rules:
    //   • Java  — e2e/java/pom.xml: <version> inside system-scope dep + <systemPath>
    //   • Ruby  — e2e/ruby/Gemfile.lock: path-gem version (reuses sync_gemfile_lock)
    //   • Go    — e2e/go/go.mod: require line version for the library module
    //   • Dart  — e2e/dart/pubspec.lock: `version:` under path-source package entry
    //
    // These paths are alef-generated and therefore always present when the language
    // backend is active; soft-skip (read_to_string returning Err) handles repos
    // that do not yet generate them.

    // Java e2e pom.xml
    let e2e_java_pom = std::path::Path::new("e2e/java/pom.xml");
    if let Ok(content) = std::fs::read_to_string(e2e_java_pom) {
        if let Some(new_content) = sync_e2e_java_pom(&content, &version) {
            std::fs::write(e2e_java_pom, &new_content).context("failed to write e2e/java/pom.xml")?;
            updated.push("e2e/java/pom.xml".to_string());
        }
    }

    // Ruby e2e Gemfile.lock
    let e2e_ruby_lock = std::path::Path::new("e2e/ruby/Gemfile.lock");
    if e2e_ruby_lock.exists() {
        if let Ok(content) = std::fs::read_to_string(e2e_ruby_lock) {
            if let Some(new_content) = sync_gemfile_lock(&content, &ruby_version) {
                std::fs::write(e2e_ruby_lock, &new_content).context("failed to write e2e/ruby/Gemfile.lock")?;
                updated.push("e2e/ruby/Gemfile.lock".to_string());
            }
        }
    }

    // Go e2e go.mod — discover the module path fragment from the file itself
    // so this logic works for any consumer repo.
    // We look for a `require` line whose module path ends with `/packages/go`
    // and update its version.
    for entry in glob::glob("e2e/go/go.mod").into_iter().flatten().flatten() {
        if let Ok(content) = std::fs::read_to_string(&entry) {
            // Find the module path fragment: any require entry ending in /packages/go
            // that pairs with a local `replace` directive.
            static GO_MOD_REQUIRE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r"(?m)^\s+([\w./\-]+/packages/go)\s+v[\w.\-]+").expect("valid regex")
            });
            if let Some(caps) = GO_MOD_REQUIRE_RE.captures(&content) {
                let fragment = caps[1].to_string();
                if let Some(new_content) = sync_e2e_go_mod(&content, &fragment, &version) {
                    std::fs::write(&entry, &new_content)
                        .with_context(|| format!("failed to write {}", entry.display()))?;
                    updated.push(entry.to_string_lossy().to_string());
                }
            }
        }
    }

    // Dart e2e pubspec.lock
    let e2e_dart_lock = std::path::Path::new("e2e/dart/pubspec.lock");
    if e2e_dart_lock.exists() {
        if let Ok(content) = std::fs::read_to_string(e2e_dart_lock) {
            if let Some(new_content) = sync_e2e_dart_pubspec_lock(&content, &version) {
                std::fs::write(e2e_dart_lock, &new_content).context("failed to write e2e/dart/pubspec.lock")?;
                updated.push("e2e/dart/pubspec.lock".to_string());
            }
        }
    }

    // CITATION.cff (Citation File Format) — YAML at repo root.
    //
    // Two modes:
    //   1. `[workspace.citation]` block present in alef.toml: render the whole
    //      file from config + canonical version. Idempotent — file is only
    //      rewritten when the rendered content differs from disk.
    //   2. No `[workspace.citation]` block but a hand-authored CITATION.cff
    //      exists at the repo root: leave content alone and only update the
    //      top-level `version:` scalar.
    if let Some(citation_config) = config.citation.as_ref() {
        let fallback_license = read_workspace_license(&config.version_from);
        let rendered = render_citation_cff(citation_config, &version, fallback_license.as_deref());
        let needs_write = match std::fs::read_to_string("CITATION.cff") {
            Ok(current) => current != rendered,
            Err(_) => true,
        };
        if needs_write {
            std::fs::write("CITATION.cff", &rendered)?;
            updated.push("CITATION.cff".to_string());
        }
    } else if let Ok(content) = std::fs::read_to_string("CITATION.cff") {
        if let Some(new_content) = replace_citation_version(&content, &version) {
            std::fs::write("CITATION.cff", &new_content)?;
            updated.push("CITATION.cff".to_string());
        }
    }

    // Process extra_paths from config [sync] section (glob patterns)
    if let Some(sync_config) = &config.sync {
        for pattern in &sync_config.extra_paths {
            match glob::glob(pattern) {
                Ok(paths) => {
                    for entry in paths {
                        match entry {
                            Ok(path) => {
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    let file_name = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
                                    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                                    if file_name == "package.json" {
                                        // For package.json files, only update the top-level
                                        // "version" field to avoid clobbering dependency versions.
                                        if let Some(new_content) =
                                            replace_version_pattern(&content, r#""version":\s*"[^"]*""#, &version)
                                        {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else if file_name == "Cargo.toml" {
                                        // Cargo.toml: only update [package] version (line-anchored).
                                        // Never use replace_all — it corrupts dependency version specs.
                                        let path_str = path.to_string_lossy().to_string();
                                        if write_version_to_cargo_toml(&path_str, &version).is_ok() {
                                            updated.push(path_str);
                                        }
                                    } else if file_name == "pyproject.toml" {
                                        // pyproject.toml: only update the `version = "..."` field.
                                        // Never do blanket regex replace — it corrupts requires-python
                                        // and dependency version specifiers.
                                        let py_ver = to_pep440(&version);
                                        if let Some(new_content) =
                                            replace_version_pattern(&content, r#"version = "[^"]*""#, &py_ver)
                                        {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else if file_name == "version.rb" {
                                        // Ruby version.rb: gem-formatted, replace VERSION constant only.
                                        // Never use SEMVER_RE — `0.3.0` in `0.3.0.pre.rc.2` would re-acquire
                                        // a dash-form prerelease, corrupting the gem version.
                                        let rb_ver = to_rubygems_prerelease(&version);
                                        if let Some(new_content) = replace_version_pattern(
                                            &content,
                                            r#"VERSION\s*=\s*['"][^'"]*['"]"#,
                                            &rb_ver,
                                        ) {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else if extension == "gemspec" {
                                        // gemspec: gem-formatted, replace spec.version only.
                                        let rb_ver = to_rubygems_prerelease(&version);
                                        if let Some(new_content) = replace_version_pattern(
                                            &content,
                                            r#"spec\.version\s*=\s*['"][^'"]*['"]"#,
                                            &rb_ver,
                                        ) {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else if file_name == "gleam.toml" {
                                        // gleam.toml: update the package version field AND restore
                                        // canonical dependency version ranges. The restore is a
                                        // self-healing safeguard — earlier alef releases routed
                                        // `gleam.toml` through the SEMVER_RE catch-all path, which
                                        // rewrote `gleam_stdlib = ">= 0.34.0 and < 2.0.0"` into
                                        // `>= {workspace_version} and < {workspace_version}` (an
                                        // empty range gleam refuses to resolve). Without the
                                        // dep-range restore, any package that still has the
                                        // corrupted shape on disk stays broken until a contributor
                                        // notices.
                                        let mut new_content = content.clone();
                                        if let Some(updated_version) =
                                            replace_version_pattern(&new_content, r#"version = "[^"]*""#, &version)
                                        {
                                            new_content = updated_version;
                                        }
                                        new_content = restore_gleam_dep_ranges(&new_content);
                                        if new_content != content {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    } else {
                                        let new_content = SEMVER_RE.replace_all(&content, version.as_str()).to_string();
                                        if new_content != content {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("Glob entry error for pattern '{pattern}': {e}");
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("Invalid glob pattern '{pattern}': {e}");
                }
            }
        }

        // Process text_replacements from config [sync] section.
        // Collect every resolved path regardless of whether its content changes —
        // we need to run finalize_hashes on all of them so that a file whose
        // version string was already correct but whose alef:hash: is stale (e.g.
        // because the registry-e2e codegen changed between alef releases) still
        // gets its hash header refreshed by `alef generate`.
        for replacement in &sync_config.text_replacements {
            match glob::glob(&replacement.path) {
                Ok(paths) => {
                    for entry in paths {
                        match entry {
                            Ok(path) => {
                                // Always record the path so finalize_hashes can
                                // refresh a stale alef:hash: even when the version
                                // substitution is a no-op.
                                text_replacement_paths.insert(path.clone());
                                if let Ok(content) = std::fs::read_to_string(&path) {
                                    let pep440 = to_pep440(&version);
                                    let rubygems = to_rubygems_prerelease(&version);
                                    let r_ver = to_r_version(&version);
                                    let search = replacement
                                        .search
                                        .replace("{python_version}", &pep440)
                                        .replace("{ruby_version}", &rubygems)
                                        .replace("{r_version}", &r_ver)
                                        .replace("{version}", &version);
                                    let replace = replacement
                                        .replace
                                        .replace("{python_version}", &pep440)
                                        .replace("{ruby_version}", &rubygems)
                                        .replace("{r_version}", &r_ver)
                                        .replace("{version}", &version);
                                    if let Ok(re) = regex::Regex::new(&search) {
                                        let new_content = re.replace_all(&content, replace.as_str()).to_string();
                                        if new_content != content {
                                            if let Err(e) = std::fs::write(&path, &new_content) {
                                                debug!("Could not write {}: {e}", path.display());
                                            } else {
                                                updated.push(path.to_string_lossy().to_string());
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("Glob entry error for pattern '{}': {e}", replacement.path);
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("Invalid glob pattern '{}': {e}", replacement.path);
                }
            }
        }
    }

    // Docs API-reference version badges: `alef docs` injects the workspace
    // version into the `<span class="version-badge">v…</span>` heading marker,
    // but consumers bump via `alef sync-versions`, which regenerates READMEs and
    // not the docs tree. Without this, the badge stays pinned at the previous
    // version after a sync-only bump. The default docs output directory mirrors
    // the `alef docs` default (`docs/reference`). Updated files are added to
    // `updated`, so finalize_hashes below refreshes their alef:hash headers.
    for badge_file in sync_docs_version_badges(std::path::Path::new("docs/reference"), &version) {
        updated.push(badge_file);
    }

    // Refresh lockfiles for any ecosystem whose manifests were rewritten.
    // This ensures CI's frozen-lockfile mode won't reject mismatched lockfiles.
    // Each command is idempotent and run_optional gracefully handles absent binaries.
    // See BLK-11 for context.
    if any_node_pkg_modified {
        run_optional("pnpm", &["install", "--no-frozen-lockfile", "--ignore-scripts", "-w"]);
    }
    if any_cargo_toml_modified {
        run_optional("cargo", &["update", "--workspace", "--offline"]);
    }
    if any_composer_json_modified {
        run_optional("composer", &["update", "--lock", "--no-interaction"]);
    }
    if any_mix_exs_modified {
        run_optional("mix", &["deps.get"]);
    }

    // Finalize alef:hash lines in every file that carries the alef header and
    // was either rewritten by this sync OR is a text_replacement target whose
    // embedded hash may be stale even when the version string was already correct.
    // Without including all text_replacement targets, a file generated by a
    // different alef sub-command (e.g. `alef e2e generate --registry`) can end up
    // with a hash that was valid for the old codegen output but is no longer valid
    // after an alef version bump — and `alef generate` would silently leave it stale.
    let mut finalize_paths: std::collections::HashSet<std::path::PathBuf> =
        updated.iter().map(std::path::PathBuf::from).collect();
    finalize_paths.extend(text_replacement_paths);
    if !finalize_paths.is_empty() {
        let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
        match super::super::cache::sources_hash(&config.sources) {
            Ok(sources_hash) => {
                match super::generate::finalize_hashes(&finalize_paths, &sources_hash, &alef_toml_bytes) {
                    Ok(n) if n > 0 => {
                        debug!("  Finalized alef:hash in {n} file(s)");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!("Could not finalize hashes after version sync: {e}");
                    }
                }
            }
            Err(e) => {
                warn!("Could not compute sources hash for finalize_hashes: {e}");
            }
        }
    }

    for file in &updated {
        info!("  Updated: {file}");
    }

    // Rebuild FFI to refresh C headers (cbindgen) if FFI language is configured
    // AND something actually changed. Skip when versions were already in sync —
    // a warm rerun should not invoke cargo at all.
    if !updated.is_empty() && config.languages.contains(&Language::Ffi) {
        let ffi_crate = config
            .explicit_output
            .ffi
            .as_ref()
            .and_then(|p| {
                // Output path is like "crates/sample-markdown-ffi/src/" — get the crate dir name
                let p = p.to_string_lossy();
                let trimmed = p.trim_end_matches('/');
                let trimmed = trimmed.strip_suffix("/src").unwrap_or(trimmed);
                trimmed.rsplit('/').next().map(|s| s.to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("{}-ffi", config.core_crate_dir()));
        info!("Rebuilding FFI ({ffi_crate}) to refresh C headers...");
        let _ = run_command(&format!("cargo build -p {ffi_crate}"));
    }

    // Stamp the last-synced version so the next warm run can skip the entire
    // glob+regex pass without re-stat'ing every manifest.
    let _ = std::fs::create_dir_all(".alef");
    let _ = std::fs::write(&last_path, &version);

    // Sync [crates.e2e.registry.packages.*].version fields in alef.toml so that
    // registry-mode e2e test apps always reference the current workspace version.
    // This runs unconditionally (even when no consumer manifest changed) because
    // the registry entries in alef.toml may be stale independently of the
    // language binding manifests.
    match sync_registry_package_versions(config_path, &version) {
        Ok(true) => {
            info!("Updated registry package versions in {}", config_path.display());
        }
        Ok(false) => {}
        Err(e) => {
            warn!(
                "Could not sync registry package versions in {}: {e}",
                config_path.display()
            );
        }
    }

    // Regenerate test_apps/ scaffold files so version pins in generated files
    // (pyproject.toml, mix.exs, build.zig.zon, Package.swift, etc.) are
    // atomically in sync with the updated registry package versions in alef.toml.
    //
    // This is the fix for the rc.13 incident: sync-versions updated alef.toml
    // registry entries but left stale version strings in previously-generated
    // test_apps/ files, causing 4 of 15 test_apps to fail with version mismatches.
    //
    // Skipped when:
    //   (a) --no-regen was passed by the caller, OR
    //   (b) no [e2e] block is configured (nothing to regenerate)
    if !no_regen {
        if let Some(e2e_config) = config.e2e.as_ref() {
            match regenerate_test_apps_after_sync(config, e2e_config, config_path) {
                Ok(count) if count > 0 => {
                    info!("  Regenerated {count} test_apps file(s) with updated version pins");
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Could not regenerate test_apps after version sync: {e}");
                }
            }
        }

        // Regenerate scaffold files so version fields embedded at scaffold-generation
        // time (gemspec spec.version, pubspec.yaml version:, R DESCRIPTION Version:,
        // binding-crate Cargo.toml [package] version, etc.) reflect the bumped
        // workspace version atomically with the sync.
        //
        // This closes the gap where `alef all` was run against Cargo.toml@rc.N,
        // then `sync-versions` bumped to rc.(N+1) and only updated test_apps —
        // leaving scaffold output with the stale rc.N version baked in.
        //
        // Always runs when `no_regen=false`, regardless of whether an [e2e] block
        // is configured, since scaffold emission does not depend on e2e config.
        match regenerate_scaffold_after_sync(config, config_path) {
            Ok(count) if count > 0 => {
                info!("  Regenerated {count} scaffold file(s) with updated version pins");
            }
            Ok(_) => {}
            Err(e) => {
                warn!("Could not regenerate scaffold after version sync: {e}");
            }
        }

        // Re-apply the root `Package.swift` URL placeholder substitution after
        // scaffold regen, because `scaffold_swift` emits the manifest with
        // `v__ALEF_SWIFT_VERSION__` (so the file under VCS stays stable across
        // version bumps) and `regenerate_scaffold_after_sync` overwrites the
        // substituted file with the placeholder form. Without this second pass,
        // every `alef sync-versions` run leaves Package.swift pointing at a
        // literal `v__ALEF_SWIFT_VERSION__` GitHub release URL, breaking
        // SwiftPM resolution for downstream consumers.
        //
        // The checksum placeholder `__ALEF_SWIFT_CHECKSUM__` is now substituted
        // by `precompute_swift_checksum` below (when `--skip-swift-checksum` is
        // not passed) rather than deferring to the publish flow. This means the
        // main version tag's Package.swift contains the real sha256 from day one,
        // and SwiftPM consumers using `from: "X.Y.Z"` get the correct checksum
        // without needing a separate `swift-X.Y.Z` namespace tag.
        if let Ok(content) = std::fs::read_to_string("Package.swift") {
            let new_content = content.replace("v__ALEF_SWIFT_VERSION__", &format!("v{version}"));
            if new_content != content {
                std::fs::write("Package.swift", &new_content)?;
                if !updated.iter().any(|p| p == "Package.swift") {
                    updated.push("Package.swift".to_string());
                }
            }
        }

        // Precompute the artifactbundle checksum and substitute `__ALEF_SWIFT_CHECKSUM__`
        // in root Package.swift so the version tag tree has a real sha256 baked in.
        // Skipped when `--skip-swift-checksum` is passed, when swift is not configured,
        // when the swift binding crate is absent, or when no pre-built bundle is available
        // and the build prerequisites (Xcode / Apple targets) are not on this host.
        if !skip_swift_checksum {
            match precompute_swift_checksum(config) {
                Ok(Some(checksum)) => {
                    info!("Swift artifactbundle checksum precomputed: {checksum}");
                    if !updated.iter().any(|p| p == "Package.swift") {
                        updated.push("Package.swift".to_string());
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn!("Swift checksum precompute failed: {e} — Package.swift retains placeholder");
                }
            }
        }
    }

    // If no manifest actually changed, nothing else needs refreshing — the
    // generated README/docs/binding hashes still match. This is the warm-path
    // fast exit: hundreds of files were already on disk with the right
    // version, so we skip the cache wipe + README regeneration entirely.
    if updated.is_empty() {
        debug!("Versions already in sync — skipping README regeneration");
        return Ok(());
    }

    // Selective cache invalidation: only README (and stage caches that embed
    // version strings) are stale after a sync. Leave the IR cache and the
    // per-language binding hashes in place so the next `alef generate` does
    // not have to re-extract or re-emit unchanged backends.
    let hashes_dir = std::path::Path::new(".alef").join("hashes");
    for stem in ["readme", "docs", "scaffold"] {
        for ext in [".hash", ".manifest", ".output_hashes"] {
            let p = hashes_dir.join(format!("{stem}{ext}"));
            if p.exists() {
                let _ = std::fs::remove_file(&p);
            }
        }
    }

    // Regenerate READMEs with the new version.
    info!("Regenerating READMEs with updated version");
    match regenerate_readmes(config, config_path) {
        Ok(count) => {
            if count > 0 {
                info!("  Regenerated {count} README(s)");
            } else {
                debug!("  No READMEs updated");
            }
        }
        Err(e) => {
            warn!("Could not regenerate READMEs: {e}");
        }
    }

    Ok(())
}

/// Render the version string for a registry package entry in `alef.toml`.
///
/// The existing `version` value carries both a constraint prefix (e.g., `>=`,

#[cfg(test)]
#[path = "version_tests.rs"]
mod tests;
