use crate::core::config::{CitationAuthor, CitationConfig, Language, ResolvedCrateConfig};
use anyhow::Context as _;
use std::sync::LazyLock;
use tracing::{debug, info, warn};

use super::helpers::{run_command, run_optional};
use super::{extract, readme};

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
fn to_pep440(version: &str) -> String {
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

use crate::core::version::{to_r_version, to_rubygems_prerelease};

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
/// `~>`, `^`, `v`) and the version number.  This function:
///
/// 1. Strips the known prefix from `existing_version`.
/// 2. Re-renders the bare version using the appropriate per-language formatter
///    (`to_pep440` for Python, `to_rubygems_prerelease` for Ruby,
///    `to_r_version` for R, identity for everything else).
/// 3. Re-attaches the original prefix.
///
/// Returns `None` when the rendered version is already current (no write needed).
pub(crate) fn render_registry_version(lang: &str, workspace_version: &str, existing_version: &str) -> Option<String> {
    // Extract the prefix: any leading non-alphanumeric, non-dot characters
    // (e.g., ">=", "~> ", "^", "v") that precede the semver digits.
    let prefix_len = existing_version.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
    let prefix = &existing_version[..prefix_len];

    // Render the bare version core using the per-language formatter.
    let rendered_core: String = match lang {
        "python" => to_pep440(workspace_version),
        "ruby" => to_rubygems_prerelease(workspace_version),
        "r" => to_r_version(workspace_version),
        _ => workspace_version.to_string(),
    };

    let new_version = format!("{prefix}{rendered_core}");
    if new_version == existing_version {
        None
    } else {
        Some(new_version)
    }
}

/// Update a zig package hash by substituting the version component.
/// Zig hashes have the format: `<pkg-name>-<version>-<base64sha>`.
/// When the version changes, we substitute just the version part, leaving the
/// base64 sha unchanged (marked as stale until the zig publish step refreshes it
/// via `zig fetch --save`).
///
/// Returns `Some(new_hash)` if the version component changed, `None` otherwise.
fn update_zig_package_hash(existing_hash: &str, old_version: &str, new_version: &str) -> Option<String> {
    // Zig hash format: `<name>-<version>-<base64sha>`, e.g.
    // `sample_pkg-1.4.0-rc.50-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9`
    // We need to find the version component and replace it.
    // The version is sandwiched between the second-to-last and last dash (before the base64).
    //
    // Strategy: split by `-`, find the old version in the parts, and replace it.
    let parts: Vec<&str> = existing_hash.split('-').collect();
    if parts.len() < 3 {
        return None; // Malformed hash
    }

    // The base64 part (last segment) is always non-semver and doesn't contain dashes.
    // Find the position of the old version within the split parts.
    // For a hash like `sample_pkg-1.4.0-rc.50-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9`,
    // after split: ["sample_pkg", "1.4.0", "rc.50", "Jfgk_..."]
    // We need to identify which parts compose the version.

    // A semver version may contain dots and dashes (e.g., "1.4.0-rc.50").
    // When split by "-", it becomes ["1.4.0", "rc", "50"].
    // The base64 part at the end is a single token with underscores and alphanumerics.

    // Heuristic: the base64 part is the last segment and contains underscores or
    // starts with an uppercase letter (typical base64url). All preceding parts
    // (after the pkg name) form the version.
    let base64_part = parts[parts.len() - 1];
    let is_base64 = base64_part.contains('_') || base64_part.chars().next().is_some_and(|c| c.is_ascii_uppercase());

    if !is_base64 {
        return None; // Couldn't identify base64 part
    }

    // Join the parts that make up the version by searching for old_version.
    // We'll try to find old_version as a substring of the joined middle parts.
    let middle_parts = &parts[1..parts.len() - 1]; // Everything except name and base64
    let joined_middle = middle_parts.join("-");

    if joined_middle.contains(old_version) {
        let new_middle = joined_middle.replace(old_version, new_version);
        let new_hash = format!("{}-{}-{}", parts[0], new_middle, base64_part);
        if new_hash != existing_hash {
            return Some(new_hash);
        }
    }

    None
}

/// Rewrite `version` fields under `[crates.<name>.e2e.registry.packages.<lang>]`
/// in `alef.toml` to track the current workspace version.
///
/// Uses `toml_edit` for format-preserving surgery: comments, blank lines, and
/// key ordering are all preserved.  Only entries that already have a `version`
/// field are touched — this function never inserts a new `version` field.
///
/// Returns `true` when at least one field was rewritten.
pub(crate) fn sync_registry_package_versions(
    config_path: &std::path::Path,
    workspace_version: &str,
) -> anyhow::Result<bool> {
    use toml_edit::{DocumentMut, Item};

    let content =
        std::fs::read_to_string(config_path).with_context(|| format!("failed to read {}", config_path.display()))?;
    let mut doc: DocumentMut = content
        .parse()
        .with_context(|| format!("failed to parse {} as TOML", config_path.display()))?;

    let mut changed = false;

    // Walk `[[crates]]` array (new-style) and `[crates]` table (old-style).
    // Both shapes may carry `.e2e.registry.packages` sub-tables.
    let crate_keys: Vec<String> = doc.iter().map(|(k, _)| k.to_string()).collect();
    for key in &crate_keys {
        if key != "crates" {
            continue;
        }
        let crates_item = match doc.get_mut(key.as_str()) {
            Some(item) => item,
            None => continue,
        };

        // Helper closure: given a mutable reference to a single crate table,
        // walk its `.e2e.registry.packages.*` and update `version` and `hash` fields.
        fn patch_crate_table(crate_table: &mut dyn toml_edit::TableLike, workspace_version: &str) -> bool {
            let e2e = match crate_table.get_mut("e2e").and_then(|i| i.as_table_like_mut()) {
                Some(t) => t,
                None => return false,
            };
            let registry = match e2e.get_mut("registry").and_then(|i| i.as_table_like_mut()) {
                Some(t) => t,
                None => return false,
            };
            let packages = match registry.get_mut("packages").and_then(|i| i.as_table_like_mut()) {
                Some(t) => t,
                None => return false,
            };
            let lang_keys: Vec<String> = packages.iter().map(|(k, _)| k.to_string()).collect();
            let mut any = false;
            for lang in &lang_keys {
                let pkg = match packages.get_mut(lang.as_str()).and_then(|i| i.as_table_like_mut()) {
                    Some(t) => t,
                    None => continue,
                };
                let existing_version = match pkg.get("version").and_then(|i| i.as_str()) {
                    Some(v) => v.to_string(),
                    None => continue, // no version field — skip (don't insert)
                };
                if let Some(new_ver) = render_registry_version(lang, workspace_version, &existing_version) {
                    if let Some(ver_item) = pkg.get_mut("version") {
                        *ver_item = toml_edit::value(new_ver.clone());
                        any = true;
                    }

                    // For zig, also update the hash field when the version changes.
                    // Hash format: `<pkg-name>-<version>-<base64sha>`.
                    // We inline-substitute the version part, leaving the base64 sha unchanged.
                    // Note: The sha will be stale until the zig publish workflow
                    // re-runs `zig fetch --save URL` post-release.
                    if lang == "zig" {
                        if let Some(hash_item) = pkg.get_mut("hash") {
                            if let Some(existing_hash) = hash_item.as_str() {
                                if let Some(new_hash) =
                                    update_zig_package_hash(existing_hash, &existing_version, &new_ver)
                                {
                                    *hash_item = toml_edit::value(new_hash);
                                }
                            }
                        }
                    }
                }
            }
            any
        }

        // `[[crates]]` is an array of tables.
        if let Some(arr) = crates_item.as_array_of_tables_mut() {
            for crate_table in arr.iter_mut() {
                if patch_crate_table(crate_table, workspace_version) {
                    changed = true;
                }
            }
        }
        // `[crates]` is a plain table (single-crate config style).
        else if let Item::Table(tbl) = crates_item {
            if patch_crate_table(tbl as &mut dyn toml_edit::TableLike, workspace_version) {
                changed = true;
            }
        }
    }

    if changed {
        let new_content = doc.to_string();
        std::fs::write(config_path, &new_content)
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }

    Ok(changed)
}

/// Build the swift artifactbundle for the current crate, compute its sha256,
/// substitute `__ALEF_SWIFT_CHECKSUM__` in root `Package.swift`, and write a
/// sidecar file at `target/alef-swift-checksum.txt` so the publish workflow can
/// reuse the checksum without rebuilding.
///
/// # Steps
///
/// 1. Detect whether the workspace has a swift binding crate (`{name}-swift`).
///    Skip with a warning if not found.
/// 2. Check whether `Package.swift` still contains `__ALEF_SWIFT_CHECKSUM__`.
///    Return early if already substituted (idempotent).
/// 3. Look for a pre-built `.artifactbundle.zip` under `dist/swift-artifactbundle/`.
///    If none exists, shell out to `cargo build -p {crate}-swift --release` and
///    the alef-bundled build script to produce one.  Skips gracefully when the
///    build prerequisites (Xcode / Apple targets) are absent.
/// 4. Compute the checksum with `swift package compute-checksum {zip}` (falls back
///    to a SHA-256 hex digest computed in-process if `swift` is not on PATH).
/// 5. Substitute the checksum in `Package.swift` and write the sidecar file.
///
/// Returns `Ok(Some(checksum))` when substitution succeeds, `Ok(None)` when skipped.
fn precompute_swift_checksum(config: &ResolvedCrateConfig) -> anyhow::Result<Option<String>> {
    use super::helpers::run_command_captured;

    // Guard: Package.swift must exist and still contain the placeholder.
    let pkg_swift_path = std::path::Path::new("Package.swift");
    let pkg_content = match std::fs::read_to_string(pkg_swift_path) {
        Ok(c) => c,
        Err(_) => {
            debug!("Package.swift not found — skipping swift checksum precompute");
            return Ok(None);
        }
    };
    if !pkg_content.contains("__ALEF_SWIFT_CHECKSUM__") {
        debug!("Package.swift already has a real checksum — skipping precompute");
        return Ok(None);
    }

    // Guard: swift must be in the configured languages.
    if !config.languages.contains(&Language::Swift) {
        debug!("Swift not configured — skipping swift checksum precompute");
        return Ok(None);
    }

    // Guard: the swift binding crate must exist. Some consumers put it under
    // `crates/{name}-swift/` (alef default), others under `packages/swift/rust/`.
    // Probe both before giving up.
    let swift_crate = format!("{}-swift", config.name);
    let candidate_manifests = [
        format!("crates/{swift_crate}/Cargo.toml"),
        "packages/swift/rust/Cargo.toml".to_string(),
    ];
    let swift_manifest = candidate_manifests
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .cloned();
    let Some(swift_manifest) = swift_manifest else {
        warn!(
            "Swift binding crate `{swift_crate}` not found under any of {:?} — \
             skipping checksum precompute. Run with --skip-swift-checksum to suppress.",
            candidate_manifests
        );
        return Ok(None);
    };
    debug!("Using swift manifest: {swift_manifest}");

    // Look for a pre-built artifactbundle zip under dist/swift-artifactbundle/.
    // The build action outputs `{ArtifactName}.artifactbundle.zip` there.
    let bundle_dir = std::path::Path::new("dist/swift-artifactbundle");
    let existing_zip = if bundle_dir.exists() {
        std::fs::read_dir(bundle_dir).ok().and_then(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.extension().and_then(|e| e.to_str()) == Some("zip"))
        })
    } else {
        None
    };

    let zip_path = match existing_zip {
        Some(p) => {
            info!("Using pre-built artifactbundle: {}", p.display());
            p
        }
        None => {
            // No pre-built zip found — attempt to build.
            info!("Building swift artifactbundle for `{swift_crate}`…");
            let build_cmd = format!("cargo build -p {swift_crate} --release --target aarch64-apple-darwin");
            match run_command_captured(&build_cmd) {
                Ok(_) => {}
                Err(e) => {
                    warn!(
                        "Swift artifactbundle build failed (missing Xcode / Apple targets?): {e}\n\
                         Re-run with --skip-swift-checksum to skip this step."
                    );
                    return Ok(None);
                }
            }
            // After cargo build, look again.
            std::fs::create_dir_all(bundle_dir).ok();
            match std::fs::read_dir(bundle_dir).ok().and_then(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .find(|p| p.extension().and_then(|e| e.to_str()) == Some("zip"))
            }) {
                Some(p) => p,
                None => {
                    warn!(
                        "No .zip found in `dist/swift-artifactbundle/` after build — \
                         skipping checksum substitution."
                    );
                    return Ok(None);
                }
            }
        }
    };

    // Compute checksum: prefer `swift package compute-checksum` (canonical tool),
    // fall back to an in-process SHA-256.
    let checksum_cmd = format!("swift package compute-checksum {}", zip_path.display());
    let checksum = match run_command_captured(&checksum_cmd) {
        Ok((stdout, _)) => stdout.trim().to_string(),
        Err(_) => {
            // Fallback: compute SHA-256 in-process.
            info!("`swift` not found — computing SHA-256 in-process");
            let bytes = std::fs::read(&zip_path).with_context(|| format!("failed to read {}", zip_path.display()))?;
            compute_sha256_hex(&bytes)
        }
    };

    if checksum.is_empty() {
        warn!("Computed empty checksum — skipping substitution");
        return Ok(None);
    }

    // Substitute in Package.swift.
    let new_content = pkg_content.replace("__ALEF_SWIFT_CHECKSUM__", &checksum);
    std::fs::write(pkg_swift_path, &new_content).context("writing Package.swift with checksum")?;
    info!("Substituted __ALEF_SWIFT_CHECKSUM__ → {checksum} in Package.swift");

    // Write sidecar so publish.yaml can reuse the hash without rebuilding.
    std::fs::create_dir_all("target").ok();
    std::fs::write("target/alef-swift-checksum.txt", &checksum).context("writing target/alef-swift-checksum.txt")?;

    Ok(Some(checksum))
}

/// Compute a lowercase hex SHA-256 digest of `bytes` without shelling out.
///
/// Used as a fallback when `swift package compute-checksum` is not available.
fn compute_sha256_hex(bytes: &[u8]) -> String {
    // sha2 is pulled in transitively (ring → sha2 in some configurations).
    // Use a manual implementation to avoid adding a direct dependency.
    use std::num::Wrapping;

    // SHA-256 round constants K.
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    // Initial hash values H.
    let mut h: [Wrapping<u32>; 8] = [
        Wrapping(0x6a09e667),
        Wrapping(0xbb67ae85),
        Wrapping(0x3c6ef372),
        Wrapping(0xa54ff53a),
        Wrapping(0x510e527f),
        Wrapping(0x9b05688c),
        Wrapping(0x1f83d9ab),
        Wrapping(0x5be0cd19),
    ];

    // Pre-processing: add padding.
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut msg = bytes.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0x00);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    // Process each 512-bit (64-byte) chunk.
    for chunk in msg.chunks_exact(64) {
        let mut w = [Wrapping(0u32); 64];
        for i in 0..16 {
            w[i] = Wrapping(u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]));
        }
        for i in 16..64 {
            let s0 = w[i - 15].0.rotate_right(7) ^ w[i - 15].0.rotate_right(18) ^ (w[i - 15].0 >> 3);
            let s1 = w[i - 2].0.rotate_right(17) ^ w[i - 2].0.rotate_right(19) ^ (w[i - 2].0 >> 10);
            w[i] = w[i - 16] + Wrapping(s0) + w[i - 7] + Wrapping(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.0.rotate_right(6) ^ e.0.rotate_right(11) ^ e.0.rotate_right(25);
            let ch = (e.0 & f.0) ^ ((!e.0) & g.0);
            let temp1 = hh + Wrapping(s1) + Wrapping(ch) + Wrapping(K[i]) + w[i];
            let s0 = a.0.rotate_right(2) ^ a.0.rotate_right(13) ^ a.0.rotate_right(22);
            let maj = (a.0 & b.0) ^ (a.0 & c.0) ^ (b.0 & c.0);
            let temp2 = Wrapping(s0) + Wrapping(maj);
            hh = g;
            g = f;
            f = e;
            e = d + temp1;
            d = c;
            c = b;
            b = a;
            a = temp1 + temp2;
        }
        h[0] += a;
        h[1] += b;
        h[2] += c;
        h[3] += d;
        h[4] += e;
        h[5] += f;
        h[6] += g;
        h[7] += hh;
    }

    format!(
        "{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        h[0].0, h[1].0, h[2].0, h[3].0, h[4].0, h[5].0, h[6].0, h[7].0
    )
}

/// Regenerate registry-mode test_apps scaffold files after a version sync so
/// that version pins in generated files (e.g. pyproject.toml, mix.exs,
/// build.zig.zon, Package.swift) reflect the updated workspace version.
///
/// Mirrors the `TestApps::Generate` dispatch in `main.rs` but runs inside the
/// `sync_versions` pipeline so the update is atomic with the alef.toml mutation
/// performed by `sync_registry_package_versions`.
///
/// The config is reloaded from `config_path` (which was just updated by
/// `sync_registry_package_versions`) so that the regenerated scaffold files
/// pick up the new registry package version values, not the stale in-memory
/// values from the config that was loaded before `sync_versions` ran.
///
/// Returns the number of files written (0 when everything was already current).
fn regenerate_test_apps_after_sync(
    config: &ResolvedCrateConfig,
    _e2e_config: &crate::core::config::e2e::E2eConfig,
    config_path: &std::path::Path,
) -> anyhow::Result<usize> {
    use crate::core::config::NewAlefConfig;
    use crate::core::config::e2e::DependencyMode;

    // Reload alef.toml from disk so the in-memory config reflects the
    // registry package version that `sync_registry_package_versions` just wrote.
    // The stale in-memory `config.e2e` would produce pyproject.toml / mix.exs /
    // build.zig.zon with the old version pins — exactly the rc.13 bug this
    // function is designed to prevent.
    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {} for test_apps regen", config_path.display()))?;
    let new_alef_cfg: NewAlefConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse {} for test_apps regen", config_path.display()))?;
    let mut resolved_crates = new_alef_cfg
        .resolve()
        .with_context(|| format!("failed to resolve {} for test_apps regen", config_path.display()))?;

    // Find the matching crate by name. Fall back to the first crate with an
    // [e2e] block when the name doesn't match (e.g. single-crate repos).
    let fresh_config = resolved_crates
        .iter()
        .position(|c| c.name == config.name && c.e2e.is_some())
        .or_else(|| resolved_crates.iter().position(|c| c.e2e.is_some()))
        .map(|idx| resolved_crates.swap_remove(idx))
        .ok_or_else(|| anyhow::anyhow!("no crate with [e2e] block found in reloaded config"))?;

    let e2e_config = fresh_config
        .e2e
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("reloaded crate has no [e2e] block"))?;

    // Build a registry-mode clone so `generate_e2e` uses published-package
    // coordinates rather than local path dependencies.
    let mut registry_config = e2e_config.clone();
    registry_config.dep_mode = DependencyMode::Registry;
    let e2e_ref = &registry_config;

    // Extract IR (empty for repos with no sources configured — the scaffold
    // files like pyproject.toml do not require IR content).
    let api = extract(&fresh_config, config_path, false)?;

    // Generate test_apps/ scaffold files for all configured e2e languages.
    let files = crate::e2e::generate_e2e(&fresh_config, e2e_ref, None, &api.types, &api.enums)?;
    if files.is_empty() {
        return Ok(0);
    }

    let base_dir = std::path::PathBuf::from(".");
    let count = super::generate::write_scaffold_files_with_overwrite(&files, &base_dir, true)?;

    let sources_hash = super::super::cache::sources_hash(&fresh_config.sources)?;
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    let path_set: std::collections::HashSet<std::path::PathBuf> =
        files.iter().map(|f| base_dir.join(&f.path)).collect();
    super::generate::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

    Ok(count)
}

/// Regenerate scaffold files (pyproject.toml, package.json, gemspec, pubspec.yaml,
/// Cargo.toml in binding crates, etc.) after a version sync so that version fields
/// embedded at scaffold-generation time reflect the updated workspace version.
///
/// The scaffold generator reads `api.version` from the IR, which in turn reflects
/// the current `Cargo.toml` workspace version. Reloading the config from
/// `config_path` after `sync_versions` has written the bumped version ensures the
/// IR carries the fresh version string.
///
/// Scaffold files with `generated_header: true` are always overwritten (they are
/// fully alef-managed, e.g. `.cargo/config.toml`). Scaffold files with
/// `generated_header: false` (seeds — Cargo.toml templates, gemspec, pubspec.yaml)
/// are also overwritten here so version strings stay in sync atomically with the
/// workspace bump. This mirrors what `alef all --clean` would do.
///
/// Returns the number of scaffold files written (0 when all were already current).
fn regenerate_scaffold_after_sync(
    config: &ResolvedCrateConfig,
    config_path: &std::path::Path,
) -> anyhow::Result<usize> {
    use crate::core::config::NewAlefConfig;

    // Reload alef.toml so the in-memory config reflects the bumped version that
    // `sync_versions` just wrote to Cargo.toml (version_from). The stale
    // in-memory `api.version` would produce scaffold files with the old version
    // string — identical to the rc.13 bug for test_apps but on the scaffold side.
    let raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {} for scaffold regen", config_path.display()))?;
    let new_alef_cfg: NewAlefConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse {} for scaffold regen", config_path.display()))?;
    let mut resolved_crates = new_alef_cfg
        .resolve()
        .with_context(|| format!("failed to resolve {} for scaffold regen", config_path.display()))?;

    // Match by name; fall back to first crate (single-crate repos).
    let fresh_config = resolved_crates
        .iter()
        .position(|c| c.name == config.name)
        .or(Some(0))
        .and_then(|idx| {
            if idx < resolved_crates.len() {
                Some(resolved_crates.swap_remove(idx))
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("no crate found in reloaded config for scaffold regen"))?;

    // Extract IR — scaffold generators use api.version (from Cargo.toml) and
    // api.types/enums. Sources may be empty for pure-scaffold repos; extract
    // tolerates that.
    let api = extract(&fresh_config, config_path, false)?;
    let languages = fresh_config.languages.clone();

    let scaffold_files = super::scaffold(&api, &fresh_config, &languages)?;
    if scaffold_files.is_empty() {
        return Ok(0);
    }

    let base_dir = std::path::PathBuf::from(".");
    // Always overwrite: scaffold seed files (gemspec, pubspec.yaml, Cargo.toml)
    // must reflect the bumped version even when they already exist on disk.
    let count = super::generate::write_scaffold_files_with_overwrite(&scaffold_files, &base_dir, true)?;

    let sources_hash = super::super::cache::sources_hash(&fresh_config.sources)?;
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    let path_set: std::collections::HashSet<std::path::PathBuf> =
        scaffold_files.iter().map(|f| base_dir.join(&f.path)).collect();
    super::generate::finalize_hashes(&path_set, &sources_hash, &alef_toml_bytes)?;

    Ok(count)
}

/// Internal helper to regenerate READMEs after a version sync.
/// Extracts IR, computes README files, and writes them to disk.
fn regenerate_readmes(config: &ResolvedCrateConfig, config_path: &std::path::Path) -> anyhow::Result<usize> {
    let api = extract(config, config_path, false)?;
    let languages = config.languages.clone();
    let readme_files = readme(&api, config, &languages)?;
    let base_dir = std::path::PathBuf::from(".");
    let sources_hash = super::super::cache::sources_hash(&config.sources)?;
    let alef_toml_bytes = super::super::cache::read_alef_toml_bytes(config_path);
    let count = super::generate::write_scaffold_files_with_overwrite(&readme_files, &base_dir, true)?;
    let paths: std::collections::HashSet<std::path::PathBuf> =
        readme_files.iter().map(|f| base_dir.join(&f.path)).collect();
    super::generate::finalize_hashes(&paths, &sources_hash, &alef_toml_bytes)?;
    Ok(count)
}

/// Update all `<gem-name> (<old-version>)` entries in a Gemfile.lock to `new_ruby_version`.
///
/// Gemfile.lock records the path-gem version in two places:
///
/// 1. Under `PATH > specs:` — four-space indent, may include dependency lines below it.
/// 2. Under `CHECKSUMS` — two-space indent, no sha256 suffix (path gems are not downloaded).
///
/// Both patterns look like `  <name> (<version>)` with varying indentation. We replace
/// every occurrence of `<name> (<old>)` with `<name> (<new>)` regardless of indent, so
/// the function handles any future Gemfile.lock layout changes automatically.
///
/// Returns `Some(new_content)` when at least one substitution was made, `None` when the
/// lockfile already contains the target version everywhere (idempotent).
fn sync_gemfile_lock(content: &str, new_ruby_version: &str) -> Option<String> {
    // Build a regex that matches `<gem-name> (<any-version>)` on a word boundary
    // so we never accidentally match a gem whose name is a prefix of another.
    // The gem name is captured from the first occurrence we find in the file
    // (the PATH > specs block always appears first).
    use std::sync::LazyLock;
    static GEM_VERSION_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        // Matches: optional leading whitespace + gem-name + space + (version)
        // Capture group 1 = gem name, group 2 = version inside parens.
        regex::Regex::new(r"(?m)^([ \t]*)([A-Za-z0-9_-]+) \(([^)]+)\)$").expect("valid regex")
    });

    // Collect the set of gem names that appear in the PATH block (path gems).
    // PATH block starts with "^PATH" and ends at the next blank line or new section.
    let path_gem_names: std::collections::HashSet<String> = {
        let mut names = std::collections::HashSet::new();
        let mut in_specs = false;
        for line in content.lines() {
            if line.trim_start().starts_with("specs:") {
                // Only enter specs-tracking mode when we are in a PATH block, which
                // always appears before GEM. A simple heuristic: the PATH section
                // starts with "^PATH" (no indent). Track whether we saw PATH before
                // seeing GEM.
            }
            if line == "PATH" {
                in_specs = true;
                continue;
            }
            if in_specs && line.starts_with("  specs:") {
                continue;
            }
            if in_specs && line.starts_with("    ") {
                // Four-space indent — these are gem entries in the PATH specs block.
                if let Some(caps) = GEM_VERSION_RE.captures(line) {
                    let indent = &caps[1];
                    let name = &caps[2];
                    if indent.len() == 4 {
                        names.insert(name.to_string());
                    }
                }
                continue;
            }
            // A line without four-space indent ends the PATH > specs block.
            if in_specs
                && !line.starts_with("    ")
                && !line.trim().is_empty()
                && line != "PATH"
                && !line.starts_with("  ")
            {
                // Top-level section header — PATH block is done.
                in_specs = false;
            }
        }
        names
    };

    if path_gem_names.is_empty() {
        return None;
    }

    let mut changed = false;
    let new_content = content
        .lines()
        .map(|line| {
            if let Some(caps) = GEM_VERSION_RE.captures(line) {
                let gem_name = &caps[2];
                let current_version = &caps[3];
                if path_gem_names.contains(gem_name) && current_version != new_ruby_version {
                    changed = true;
                    // Reconstruct the line with the new version, preserving indent.
                    let indent = &caps[1];
                    return format!("{indent}{gem_name} ({new_ruby_version})");
                }
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Preserve trailing newline if the original had one.
    let new_content = if content.ends_with('\n') {
        format!("{new_content}\n")
    } else {
        new_content
    };

    if changed { Some(new_content) } else { None }
}

/// Rewrite the dependency `<version>` and `<systemPath>` in an e2e `pom.xml`
/// for a path-scope system dependency on the library JAR.
///
/// The e2e `pom.xml` carries a `<dependency>` block like:
/// ```xml
/// <dependency>
///   <groupId>dev.sample_core.sample_widget</groupId>
///   <artifactId>sample-widget</artifactId>
///   <version>0.3.0-rc.27</version>
///   <scope>system</scope>
///   <systemPath>.../sample-widget-0.3.0-rc.27.jar</systemPath>
/// </dependency>
/// ```
/// Unlike `packages/java/pom.xml`, this file has a *separate* `<version>0.1.0</version>`
/// for the e2e project itself at the top — we must not touch that one.
///
/// Strategy: two passes.
///
/// 1. Collect the byte-ranges of every `<dependency>...</dependency>` block
///    that contains a `<systemPath>` element.
/// 2. Within those ranges, rewrite `<version>X</version>` and the version
///    fragment inside `<systemPath>`.
///
/// All other `<version>` tags are left untouched.
///
/// Returns `Some(new_content)` when a replacement was made, `None` otherwise.
fn sync_e2e_java_pom(content: &str, new_version: &str) -> Option<String> {
    use std::sync::LazyLock;

    static DEP_BLOCK_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?s)<dependency>(.*?)</dependency>").expect("valid regex"));
    static VERSION_TAG_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"<version>([^<]*)</version>").expect("valid regex"));
    static SYSTEM_PATH_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(<systemPath>[^<]*?-)(\d+\.\d+\.\d+(?:-[A-Za-z0-9._]+)*)(\.[a-zA-Z]+</systemPath>)")
            .expect("valid regex")
    });

    let mut result = content.to_string();
    let mut changed = false;

    // Collect ranges of <dependency> blocks that contain <systemPath>.
    // We iterate over matches in the ORIGINAL content to get stable offsets,
    // then apply replacements from back to front so earlier offsets stay valid.
    let dep_matches: Vec<(usize, usize, String)> = DEP_BLOCK_RE
        .find_iter(content)
        .filter_map(|m| {
            let block = m.as_str();
            if !block.contains("<systemPath>") {
                return None;
            }
            // Rewrite <version> and <systemPath> within this block.
            let new_block = VERSION_TAG_RE
                .replace(block, |caps: &regex::Captures<'_>| {
                    let ver = &caps[1];
                    if ver != new_version && !ver.contains('$') && !ver.contains('.') && ver.parse::<u64>().is_err() {
                        // Only rewrite if it looks like a semver (has dots).
                        // The check below handles that properly.
                        format!("<version>{ver}</version>")
                    } else if ver != new_version && ver.contains('.') && !ver.contains('$') {
                        format!("<version>{new_version}</version>")
                    } else {
                        format!("<version>{ver}</version>")
                    }
                })
                .into_owned();
            let new_block = SYSTEM_PATH_RE
                .replace(&new_block, |caps: &regex::Captures<'_>| {
                    format!("{}{}{}", &caps[1], new_version, &caps[3])
                })
                .into_owned();
            if new_block != block {
                Some((m.start(), m.end(), new_block))
            } else {
                None
            }
        })
        .collect();

    // Apply from back to front so offsets remain valid.
    for (start, end, new_block) in dep_matches.into_iter().rev() {
        result.replace_range(start..end, &new_block);
        changed = true;
    }

    if changed { Some(result) } else { None }
}

/// Rewrite the version for a module in a `go.mod` `require` block.
///
/// The e2e `go.mod` has a line like:
/// ```text
/// github.com/sample-core-dev/sample-widget/packages/go v0.3.0-rc.27
/// ```
/// We want to update ONLY lines whose module path matches `module_path_fragment`
/// — a substring that uniquely identifies the library module (e.g.
/// `"sample-core-dev/sample-widget/packages/go"`). All other `require` entries are
/// left untouched.
///
/// Returns `Some(new_content)` when a replacement was made, `None` otherwise.
fn sync_e2e_go_mod(content: &str, module_path_fragment: &str, new_version: &str) -> Option<String> {
    let mut changed = false;
    let lines: Vec<String> = content
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            // Match lines of the form `<module-path> v<version>` inside a require block.
            if trimmed.starts_with(module_path_fragment) || line.trim_start().starts_with(module_path_fragment) {
                // The line is `\t<module> v<version>` or `    <module> v<version>`.
                // Split on the version token (starts with 'v' followed by a digit).
                if let Some(pos) = trimmed.rfind(" v") {
                    let current_ver = &trimmed[pos + 2..]; // strip " v"
                    if current_ver != new_version {
                        changed = true;
                        let indent = &line[..line.len() - line.trim_start().len()];
                        let module_path = &trimmed[..pos];
                        return format!("{indent}{module_path} v{new_version}");
                    }
                }
            }
            line.to_string()
        })
        .collect();

    if !changed {
        return None;
    }
    let new_content = lines.join("\n");
    let new_content = if content.ends_with('\n') {
        format!("{new_content}\n")
    } else {
        new_content
    };
    Some(new_content)
}

/// Rewrite the `version:` field for a path-source package in a Dart `pubspec.lock`.
///
/// Dart's pub lockfile has entries like:
/// ```yaml
///   sample-widget:
///     dependency: "direct main"
///     description:
///       path: "../../packages/dart"
///       relative: true
///     source: path
///     version: "0.3.0-rc.23"
/// ```
/// We match the package name, confirm it is a `source: path` entry, and rewrite
/// only its `version:` scalar. Registry (hosted) packages are left untouched.
///
/// Returns `Some(new_content)` when a replacement was made, `None` otherwise.
fn sync_e2e_dart_pubspec_lock(content: &str, new_version: &str) -> Option<String> {
    // State machine: look for `  <name>:\n` (two-space indent, no further indent),
    // then confirm `    source: path` within that block, then rewrite `    version:`.
    let lines: Vec<&str> = content.lines().collect();
    let n = lines.len();
    let mut result: Vec<String> = Vec::with_capacity(n);
    let mut changed = false;
    let mut i = 0;

    while i < n {
        let line = lines[i];
        // Detect a top-level package entry: exactly 2-space-indented key ending with `:`.
        if line.starts_with("  ") && !line.starts_with("   ") && line.trim_end().ends_with(':') {
            // Collect the block for this package entry (all lines with deeper indent).
            let block_start = i;
            i += 1;
            while i < n && (lines[i].starts_with("    ") || lines[i].trim().is_empty()) {
                i += 1;
            }
            let block = &lines[block_start..i];

            // Check if this block is a path-source package.
            let is_path_source = block.iter().any(|l| l.trim() == "source: path");
            if is_path_source {
                // Rewrite the `    version: "..."` line in this block.
                for &bline in block {
                    let trimmed = bline.trim();
                    if trimmed.starts_with("version:") {
                        // Extract current version (may be quoted or unquoted).
                        let val = trimmed.trim_start_matches("version:").trim().trim_matches('"');
                        if val != new_version {
                            changed = true;
                            let indent = &bline[..bline.len() - bline.trim_start().len()];
                            result.push(format!("{indent}version: \"{new_version}\""));
                        } else {
                            result.push(bline.to_string());
                        }
                    } else {
                        result.push(bline.to_string());
                    }
                }
            } else {
                for &bline in block {
                    result.push(bline.to_string());
                }
            }
        } else {
            result.push(line.to_string());
            i += 1;
        }
    }

    if !changed {
        return None;
    }
    let new_content = result.join("\n");
    let new_content = if content.ends_with('\n') {
        format!("{new_content}\n")
    } else {
        new_content
    };
    Some(new_content)
}

/// Read the workspace license string (`[workspace.package].license`) from a
/// Cargo.toml path. Used as the fallback `license:` value for CITATION.cff
/// when the `[workspace.citation]` block omits it. Returns `None` on any
/// read/parse failure or when the field is absent — caller decides what to do.
fn read_workspace_license(version_from: &str) -> Option<String> {
    let content = std::fs::read_to_string(version_from).ok()?;
    let value: toml::Value = toml::from_str(&content).ok()?;
    value
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("license"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            value
                .get("package")
                .and_then(|p| p.get("license"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

/// Render a full `CITATION.cff` YAML document from config + canonical version.
///
/// Emits fields in the canonical CFF order (`cff-version`, `message`, `title`,
/// `abstract`, `authors`, `repository-code`, `url`, `license`, `version`,
/// `date-released`, `doi`). Author entries are emitted as either person-form
/// (`family-names` + `given-names`) or entity-form (`name`) depending on which
/// fields are populated; if both styles are set on a single author the person
/// form wins. Strings containing characters that need escaping (`:`, `#`, `\`,
/// `"`) are emitted double-quoted; otherwise the renderer uses bare scalars.
fn render_citation_cff(citation: &CitationConfig, version: &str, fallback_license: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("# This file is generated by alef sync-versions; do not edit by hand.\n");
    out.push_str("# Source: [workspace.citation] in alef.toml + workspace version in Cargo.toml.\n");
    out.push_str("cff-version: 1.2.0\n");
    out.push_str(&format!("message: {}\n", yaml_scalar(&citation.message)));
    out.push_str(&format!("title: {}\n", yaml_scalar(&citation.title)));
    out.push_str(&format!("abstract: {}\n", yaml_scalar(&citation.abstract_)));
    out.push_str("authors:\n");
    for author in &citation.authors {
        out.push_str(&render_citation_author(author));
    }
    out.push_str(&format!(
        "repository-code: {}\n",
        yaml_scalar(&citation.repository_code)
    ));
    if let Some(url) = &citation.url {
        out.push_str(&format!("url: {}\n", yaml_scalar(url)));
    }
    let license = citation.license.as_deref().or(fallback_license);
    if let Some(license) = license {
        out.push_str(&format!("license: {}\n", yaml_scalar(license)));
    }
    out.push_str(&format!("version: {version}\n"));
    if let Some(date) = &citation.date_released {
        out.push_str(&format!("date-released: {}\n", yaml_scalar(date)));
    }
    if let Some(doi) = &citation.doi {
        out.push_str(&format!("doi: {}\n", yaml_scalar(doi)));
    }
    out
}

/// Render a single `authors:` list entry. Two-space indent (`  - key: value`)
/// matches the canonical CITATION.cff layout produced by `cffinit`.
fn render_citation_author(author: &CitationAuthor) -> String {
    let mut entry = String::new();
    let person_form = author.family_names.is_some() || author.given_names.is_some();
    if person_form {
        if let Some(family) = &author.family_names {
            entry.push_str(&format!("  - family-names: {}\n", yaml_scalar(family)));
            if let Some(given) = &author.given_names {
                entry.push_str(&format!("    given-names: {}\n", yaml_scalar(given)));
            }
        } else if let Some(given) = &author.given_names {
            entry.push_str(&format!("  - given-names: {}\n", yaml_scalar(given)));
        }
        if let Some(email) = &author.email {
            entry.push_str(&format!("    email: {}\n", yaml_scalar(email)));
        }
        if let Some(orcid) = &author.orcid {
            entry.push_str(&format!("    orcid: {}\n", yaml_scalar(orcid)));
        }
    } else if let Some(name) = &author.name {
        entry.push_str(&format!("  - name: {}\n", yaml_scalar(name)));
        if let Some(email) = &author.email {
            entry.push_str(&format!("    email: {}\n", yaml_scalar(email)));
        }
        if let Some(orcid) = &author.orcid {
            entry.push_str(&format!("    orcid: {}\n", yaml_scalar(orcid)));
        }
    }
    entry
}

/// Emit a YAML scalar — double-quoted with escaping when the value contains
/// characters that would change YAML parsing semantics (`:`, `#`, leading
/// special chars, embedded quotes), bare otherwise. Tuned for the limited set
/// of strings that appear in CITATION.cff (titles, names, URLs, abstracts).
fn yaml_scalar(value: &str) -> String {
    let needs_quoting = value.is_empty()
        || value.contains(':')
        || value.contains('#')
        || value.contains('"')
        || value.contains('\\')
        || value.contains('\n')
        || value.contains('\t')
        || value.contains(' ')
        || value.contains('\'')
        || value.contains('@')
        || value.starts_with(['!', '&', '*', '?', '|', '>', '"', '%', '`', '[', ']', '{', '}', ',']);
    if needs_quoting {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

/// Regex for the top-level `version:` key in a CITATION.cff YAML file.
/// Anchored to start-of-line so nested `version:` keys inside `references:` /
/// `preferred-citation:` blocks (which are indented) are not touched.
/// The Rust `regex` crate has no backreferences, so each quote style is its
/// own alternation arm and the matching arm tells us which to emit back.
/// Capture groups:
///   1. literal `version:` + spacing
///   2. value when double-quoted
///   3. value when single-quoted
///   4. value when unquoted (bare scalar)
static CITATION_VERSION_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?m)^(version:[ \t]+)(?:"([^"\n]*)"|'([^'\n]*)'|([^\s#'"]+))[ \t]*(?:#[^\n]*)?$"#)
        .expect("valid regex")
});

/// Update the top-level `version:` scalar in a CITATION.cff. Preserves the
/// original quote style (unquoted, single-, or double-quoted). Returns
/// `Some(new_content)` only when the value actually changes — guards against
/// idempotent re-writes that would dirty the working tree on every sync.
fn replace_citation_version(content: &str, new_version: &str) -> Option<String> {
    let captures = CITATION_VERSION_RE.captures(content)?;
    let (current, replacement) = if let Some(value) = captures.get(2) {
        (value.as_str(), format!("{}\"{new_version}\"", &captures[1]))
    } else if let Some(value) = captures.get(3) {
        (value.as_str(), format!("{}'{new_version}'", &captures[1]))
    } else if let Some(value) = captures.get(4) {
        (value.as_str(), format!("{}{new_version}", &captures[1]))
    } else {
        return None;
    };
    if current == new_version {
        return None;
    }
    let new_content = CITATION_VERSION_RE.replace(content, replacement.as_str()).into_owned();
    if new_content == content {
        return None;
    }
    Some(new_content)
}

/// Replace version pattern in content. Returns `Some(new_content)` only when
/// the regex match exists *and* the captured version string actually differs
/// from the target. This is the idempotency guard against:
///   1. backend codegen that emits a manifest with the right value but in a
///      slightly different syntactic form (e.g. Magnus emits `VERSION =
///      "4.10.0.pre.rc.9"` while the regex's replacement template uses
///      single-quotes); without this guard the two paths ping-pong and every
///      warm `alef generate` rewrites the manifest, triggers README regen,
///      and looks like real drift to downstream tooling.
///   2. trivial round-trips where new content == old content despite the
///      regex matching.
fn replace_version_pattern(content: &str, pattern: &str, version: &str) -> Option<String> {
    let regex = regex::Regex::new(pattern).ok()?;
    let captures = regex.captures(content)?;
    let matched = captures.get(0)?.as_str();
    // Extract the version literal (text between the first pair of quotes or
    // angle/colon delimiters) and short-circuit when it already equals the
    // target. This way `VERSION = "x"` and `VERSION = 'x'` both count as
    // "already in sync" when x matches, regardless of quote style.
    if matched_version_equals(matched, version) {
        return None;
    }

    let replacement = match pattern {
        p if p.contains("version =") && !p.contains("spec") && !p.contains("VERSION") => {
            format!(r#"version = "{version}""#)
        }
        p if p.contains("\"version\"") && p.contains("\"") => format!(r#""version": "{version}""#),
        p if p.contains("spec") => format!("spec.version = \"{version}\""),
        p if p.contains("<version>") => format!("<version>{version}</version>"),
        p if p.contains("<Version>") => format!("<Version>{version}</Version>"),
        p if p.contains("@version") => format!(r#"@version "{version}""#),
        p if p.contains("version:") && p.contains(":") => format!(r#"version: "{version}""#),
        p if p.contains("__version__") => format!(r#"__version__ = "{version}""#),
        p if p.contains("defaultFFIVersion") => format!(r#"defaultFFIVersion = "{version}""#),
        p if p.contains("moduleVersion") => format!(r#"moduleVersion = "{version}""#),
        p if p.contains("Version:") => format!("Version: {version}"),
        // Swift Package.swift `.package(url:..., from: "X.Y.Z")` — keep the key,
        // replace only the quoted version literal.
        p if p.contains("from:") => format!(r#"from: "{version}""#),
        // Bash `VERSION="X.Y.Z"` (no spaces around `=`). Must come before the
        // generic `VERSION` arm below so the no-space form is preserved verbatim.
        p if p.contains("VERSION=\"") => format!(r#"VERSION="{version}""#),
        p if p.contains("VERSION") => format!("VERSION = \"{version}\""),
        _ => return None,
    };

    let new_content = regex.replace(content, replacement.as_str()).to_string();
    if new_content == content {
        return None;
    }
    Some(new_content)
}

/// Extract the version-literal substring from a regex match string and decide
/// whether it already equals `target`. The match string is something like
/// `VERSION = "1.2.3"`, `version = "1.2.3"`, `<version>1.2.3</version>`,
/// `Version: 1.2.3`. We look for the first chunk after the delimiter and
/// compare it to `target`; quote style is irrelevant.
fn matched_version_equals(matched: &str, target: &str) -> bool {
    extract_version_literal(matched).is_some_and(|v| v == target)
}

/// Restore canonical hex dependency version ranges in `gleam.toml`.
///
/// Earlier alef releases sometimes routed `gleam.toml` through the catch-all
/// `SEMVER_RE.replace_all` path, which rewrote every `\d+\.\d+\.\d+` literal
/// in the file with the workspace version — turning
/// `gleam_stdlib = ">= 0.34.0 and < 2.0.0"` into
/// `gleam_stdlib = ">= 5.0.0-rc.1 and < 5.0.0-rc.1"` (an empty version range
/// that gleam refuses to resolve).
///
/// This helper deterministically restores the canonical ranges from
/// `template_versions::hex` whenever it sees a `gleam_stdlib` or `gleeunit`
/// dependency line, so a single `alef sync-versions` heals affected
/// manifests without manual intervention.
fn restore_gleam_dep_ranges(content: &str) -> String {
    use crate::core::template_versions::hex;
    static GLEAM_DEP_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        // Match lines like:  `gleam_stdlib = "..."`  or  `gleeunit = "..."`
        // Captures: 1=name, 2=value (between quotes).
        regex::Regex::new(r#"(?m)^(gleam_stdlib|gleeunit)\s*=\s*"([^"]*)""#).expect("valid regex")
    });

    GLEAM_DEP_RE
        .replace_all(content, |caps: &regex::Captures<'_>| {
            let name = &caps[1];
            let canonical = match name {
                "gleam_stdlib" => hex::GLEAM_STDLIB_VERSION_RANGE,
                "gleeunit" => hex::GLEEUNIT_VERSION_RANGE,
                _ => return caps[0].to_string(),
            };
            format!("{name} = \"{canonical}\"")
        })
        .into_owned()
}

fn extract_version_literal(matched: &str) -> Option<&str> {
    // Try paired-quote form first ("..." or '...').
    if let Some(start) = matched.find(['"', '\'']) {
        let quote = matched.as_bytes()[start];
        let rest = &matched[start + 1..];
        if let Some(end) = rest.find(quote as char) {
            return Some(&rest[..end]);
        }
    }
    // Try angle-bracket form (<version>...</version> or <Version>...</Version>).
    if let Some(close) = matched.find('>') {
        let rest = &matched[close + 1..];
        if let Some(end) = rest.find('<') {
            return Some(&rest[..end]);
        }
    }
    // Try colon-delimited form (`Version: 1.2.3`).
    if let Some(colon) = matched.find(':') {
        return Some(matched[colon + 1..].trim());
    }
    // Try `=` delimited unquoted form.
    if let Some(eq) = matched.find('=') {
        return Some(matched[eq + 1..].trim());
    }
    None
}

/// Bump the top-level project `version = "..."` assignment in a Gradle Kotlin
/// DSL build file (`build.gradle.kts`).
///
/// Gradle build files embed several version-bearing constructs that must NOT be
/// touched:
///   - plugin declarations:  `kotlin("jvm") version "2.3.21"`,
///     `id("org.jlleitschuh.gradle.ktlint") version "1.0.0"`
///   - extension config:      `version.set("1.0.0")` (e.g. the ktlint block)
///   - dependency coordinates: `api("net.java.dev.jna:jna:5.14.0")`
///
/// Only the project version is expressed as a start-of-line `version = "..."`
/// assignment (Gradle Kotlin DSL `Project.version`). The regex anchors to the
/// line start (after optional leading whitespace) and requires the `=`
/// assignment form, so the plugin/extension/coordinate shapes above — which use
/// a space-delimited `version "..."`, a `version.set(...)` call, or no `version`
/// token at all — are left intact.
///
/// Returns the rewritten content when the project version changed, or `None`
/// when the file has no such line or it already matches `new_version`.
fn replace_gradle_project_version(content: &str, new_version: &str) -> Option<String> {
    static GRADLE_VERSION_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"(?m)^(\s*)version\s*=\s*"[^"]*""#).expect("valid regex"));
    let captures = GRADLE_VERSION_RE.captures(content)?;
    let matched = captures.get(0)?.as_str();
    if matched_version_equals(matched, new_version) {
        return None;
    }
    let indent = captures.get(1).map(|m| m.as_str()).unwrap_or("");
    let replacement = format!(r#"{indent}version = "{new_version}""#);
    let new_content = GRADLE_VERSION_RE.replace(content, replacement.as_str()).into_owned();
    if new_content == content {
        return None;
    }
    Some(new_content)
}

/// Rewrite the `version = "..."` field of every local/path-source `[[package]]`
/// entry in a committed `Cargo.lock` so it matches the freshly-bumped manifests.
///
/// A binding that ships a committed `Cargo.lock` inside its source tarball (e.g.
/// a Rustler NIF crate packaged into a Hex release) must keep that lockfile in
/// step with the workspace version, otherwise `cargo build` from the published
/// tarball fails with a lock/manifest version mismatch.
///
/// Registry dependencies carry a `source = "registry+..."` (or `git+...`) key
/// and an upstream-pinned version that must never be rewritten. Local crates —
/// the consumer's own workspace members and the NIF crate itself — have NO
/// `source` key and share the workspace version. We bump only those, leaving
/// every registry/git entry untouched.
///
/// The lockfile is line-oriented and `cargo` rewrites it deterministically, so a
/// targeted line rewrite (rather than a full TOML re-serialize) preserves the
/// canonical formatting and avoids reordering. Returns the rewritten content
/// when at least one local entry changed, else `None`.
fn sync_cargo_lock_path_versions(content: &str, new_version: &str) -> Option<String> {
    let mut out = String::with_capacity(content.len());
    let mut changed = false;

    // Split into `[[package]]` blocks while preserving any preamble (the lock
    // header + `version = 3`/`version = 4` format line) verbatim. We collect
    // each block's lines, decide whether it is a local (sourceless) package, and
    // only then rewrite its `version = "..."` line.
    let mut block: Vec<&str> = Vec::new();
    let mut in_package_block = false;

    // Flush the buffered block to `out`, rewriting the version line only when the
    // block is a `[[package]]` entry with no `source` key.
    let flush = |block: &mut Vec<&str>, out: &mut String, changed: &mut bool| {
        if block.is_empty() {
            return;
        }
        let is_package = block.first().is_some_and(|l| l.trim() == "[[package]]");
        let has_source = block.iter().any(|l| l.trim_start().starts_with("source = "));
        for line in block.iter() {
            if is_package && !has_source && line.trim_start().starts_with("version = ") {
                let indent_len = line.len() - line.trim_start().len();
                let indent = &line[..indent_len];
                let rewritten = format!(r#"{indent}version = "{new_version}""#);
                if rewritten != *line {
                    *changed = true;
                }
                out.push_str(&rewritten);
            } else {
                out.push_str(line);
            }
            out.push('\n');
        }
        block.clear();
    };

    for line in content.lines() {
        if line.trim() == "[[package]]" {
            // Starting a new package block: flush whatever came before (preamble
            // or the previous block).
            flush(&mut block, &mut out, &mut changed);
            in_package_block = true;
            block.push(line);
        } else if in_package_block {
            block.push(line);
        } else {
            // Preamble before the first `[[package]]`: emit verbatim.
            out.push_str(line);
            out.push('\n');
        }
    }
    flush(&mut block, &mut out, &mut changed);

    if !changed {
        return None;
    }
    // Preserve the original trailing-newline shape: `str::lines()` drops the
    // final newline, and we re-add one per line above. If the source did not end
    // in a newline, trim the extra one we appended.
    if !content.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

/// Bump the `version-badge` span in generated docs API-reference pages.
///
/// `alef docs` injects the workspace version into the `<span class="version-badge">v…</span>`
/// marker when it regenerates each `api-{lang}.md` heading. A `sync-versions`-only
/// bump (the path consumers take on every release) regenerates READMEs but not
/// the docs tree, so without this the badge stays pinned at the previous version.
/// This rewrites the badge text in-place across all `{docs_reference_dir}/api-*.md`
/// pages so a plain `alef sync-versions` leaves a fully version-consistent tree.
///
/// The match is anchored to the literal `version-badge` span class and the `v`
/// prefix the docs template emits, so unrelated `v…` text in prose is untouched.
/// Returns the list of files whose badge was rewritten.
fn sync_docs_version_badges(docs_reference_dir: &std::path::Path, new_version: &str) -> Vec<String> {
    static BADGE_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"(<span class="version-badge">v)[^<]*(</span>)"#).expect("valid regex"));
    let mut updated = Vec::new();
    let pattern = docs_reference_dir.join("api-*.md");
    let Some(pattern_str) = pattern.to_str() else {
        return updated;
    };
    for entry in glob::glob(pattern_str).into_iter().flatten().flatten() {
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        let replacement = format!("${{1}}{new_version}${{2}}");
        let new_content = BADGE_RE.replace_all(&content, replacement.as_str()).into_owned();
        if new_content != content {
            if let Err(e) = std::fs::write(&entry, &new_content) {
                debug!("Could not write {}: {e}", entry.display());
            } else {
                updated.push(entry.to_string_lossy().to_string());
            }
        }
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::pipeline::generate;

    #[test]
    fn to_pep440_no_prerelease_passthrough() {
        assert_eq!(to_pep440("1.2.3"), "1.2.3");
        assert_eq!(to_pep440("0.1.0"), "0.1.0");
    }

    #[test]
    fn to_pep440_rc_prerelease() {
        assert_eq!(to_pep440("0.1.0-rc.1"), "0.1.0rc1");
        assert_eq!(to_pep440("4.10.0-rc.9"), "4.10.0rc9");
    }

    #[test]
    fn to_pep440_alpha_beta_prerelease() {
        assert_eq!(to_pep440("1.0.0-alpha.2"), "1.0.0a2");
        assert_eq!(to_pep440("1.0.0-beta.3"), "1.0.0b3");
    }

    #[test]
    fn to_pep440_strips_internal_dots() {
        assert_eq!(to_pep440("0.1.0-rc.1.2"), "0.1.0rc12");
    }

    #[test]
    fn zon_version_regex_anchors_to_dot_version_only() {
        // The regex used by validate_versions + sync_all_manifests to rewrite
        // `.version = "X.Y.Z"` in build.zig.zon. Must NOT match
        // `.minimum_zig_version = "..."` which sits on the same file.
        let re = regex::Regex::new(r#"(?m)^\s*\.version\s*=\s*"([^"]*)""#).expect("valid regex");
        let zon = r#".{
    .name = .my_pkg,
    .version = "1.9.0-rc.1",
    .fingerprint = 0x6f52c41163f42c8c,
    .minimum_zig_version = "0.16.0",
}
"#;
        let captures: Vec<_> = re.captures_iter(zon).collect();
        assert_eq!(
            captures.len(),
            1,
            "regex must match exactly one line, not .minimum_zig_version"
        );
        assert_eq!(&captures[0][1], "1.9.0-rc.1");
    }

    #[test]
    fn matched_version_equals_treats_quote_style_uniformly() {
        assert!(matched_version_equals("VERSION = '1.0.0'", "1.0.0"));
        assert!(matched_version_equals("VERSION = \"1.0.0\"", "1.0.0"));
        assert!(!matched_version_equals("VERSION = '1.0.0'", "2.0.0"));
        assert!(matched_version_equals("<version>1.0.0</version>", "1.0.0"));
        assert!(matched_version_equals("Version: 1.0.0", "1.0.0"));
    }

    fn citation_author_person() -> CitationAuthor {
        // TODO(alef-generic-cleanup): Replace sample_crate.dev/sample-markdown citation fixtures with neutral data.
        CitationAuthor {
            family_names: Some("Hirschfeld".to_string()),
            given_names: Some("Na'aman".to_string()),
            name: None,
            email: Some("naaman@sample_crate.dev".to_string()),
            orcid: Some("https://orcid.org/0009-0000-2247-5072".to_string()),
        }
    }

    fn citation_author_entity() -> CitationAuthor {
        CitationAuthor {
            family_names: None,
            given_names: None,
            name: Some("SampleCrate, Inc.".to_string()),
            email: None,
            orcid: None,
        }
    }

    fn citation_config_mit() -> CitationConfig {
        CitationConfig {
            title: "sample-markdown".to_string(),
            abstract_: "Fast markup conversion converter.".to_string(),
            authors: vec![citation_author_person()],
            message: "If you use this software, please cite it using the metadata below.".to_string(),
            repository_code: "https://github.com/sample_crate-dev/sample-markdown".to_string(),
            url: Some("https://sample_crate.dev".to_string()),
            license: Some("MIT".to_string()),
            date_released: Some("2026-05-17".to_string()),
            doi: None,
        }
    }

    #[test]
    fn render_citation_cff_mit_full_round_trip() {
        let rendered = render_citation_cff(&citation_config_mit(), "3.5.0", None);
        let expected = r#"# This file is generated by alef sync-versions; do not edit by hand.
# Source: [workspace.citation] in alef.toml + workspace version in Cargo.toml.
cff-version: 1.2.0
message: "If you use this software, please cite it using the metadata below."
title: sample-markdown
abstract: "Fast markup conversion converter."
authors:
  - family-names: Hirschfeld
    given-names: "Na'aman"
    email: "naaman@sample_crate.dev"
    orcid: "https://orcid.org/0009-0000-2247-5072"
repository-code: "https://github.com/sample_crate-dev/sample-markdown"
url: "https://sample_crate.dev"
license: MIT
version: 3.5.0
date-released: 2026-05-17
"#;
        assert_eq!(rendered, expected);
    }

    #[test]
    fn render_citation_cff_elv2_with_entity_author() {
        let mut config = citation_config_mit();
        config.title = "sample_crate".to_string();
        config.repository_code = "https://github.com/sample_crate-dev/sample_crate".to_string();
        config.license = Some("Elastic-2.0".to_string());
        config.authors = vec![citation_author_person(), citation_author_entity()];
        let rendered = render_citation_cff(&config, "5.0.0-rc.1", None);
        assert!(rendered.contains("  - family-names: Hirschfeld\n    given-names: \"Na'aman\""));
        assert!(rendered.contains("  - name: \"SampleCrate, Inc.\"\n"));
        assert!(rendered.contains("license: Elastic-2.0\n"));
        assert!(rendered.contains("version: 5.0.0-rc.1\n"));
    }

    #[test]
    fn render_citation_cff_falls_back_to_cargo_license() {
        let mut config = citation_config_mit();
        config.license = None;
        let rendered = render_citation_cff(&config, "1.0.0", Some("Apache-2.0"));
        assert!(rendered.contains("license: Apache-2.0\n"));
    }

    #[test]
    fn render_citation_cff_omits_optional_fields_when_unset() {
        let config = CitationConfig {
            title: "tiny".to_string(),
            abstract_: "Tiny library.".to_string(),
            authors: vec![citation_author_person()],
            message: "Cite me.".to_string(),
            repository_code: "https://example.com/tiny".to_string(),
            url: None,
            license: None,
            date_released: None,
            doi: None,
        };
        let rendered = render_citation_cff(&config, "0.1.0", None);
        assert!(!rendered.contains("url:"));
        assert!(!rendered.contains("license:"));
        assert!(!rendered.contains("date-released:"));
        assert!(!rendered.contains("doi:"));
    }

    #[test]
    fn render_citation_cff_idempotent_for_unchanged_version() {
        let config = citation_config_mit();
        let first = render_citation_cff(&config, "3.5.0", None);
        let second = render_citation_cff(&config, "3.5.0", None);
        assert_eq!(first, second);
    }

    #[test]
    fn replace_citation_version_unquoted_scalar() {
        let content = "cff-version: 1.2.0\ntitle: example\nversion: 1.0.0\n";
        let new = replace_citation_version(content, "2.0.0").expect("regex matched");
        assert!(new.contains("version: 2.0.0\n"));
        assert!(new.contains("title: example\n"));
        assert!(!new.contains("1.0.0"));
    }

    #[test]
    fn replace_citation_version_double_quoted_preserves_quotes() {
        let content = "version: \"1.0.0\"\n";
        let new = replace_citation_version(content, "2.0.0").expect("regex matched");
        assert_eq!(new, "version: \"2.0.0\"\n");
    }

    #[test]
    fn replace_citation_version_single_quoted_preserves_quotes() {
        let content = "version: '1.0.0'\n";
        let new = replace_citation_version(content, "2.0.0").expect("regex matched");
        assert_eq!(new, "version: '2.0.0'\n");
    }

    #[test]
    fn replace_citation_version_rc_suffix_passes_through() {
        let content = "version: 5.0.0-rc.1\n";
        let new = replace_citation_version(content, "5.0.0-rc.2").expect("regex matched");
        assert_eq!(new, "version: 5.0.0-rc.2\n");
    }

    #[test]
    fn replace_citation_version_no_op_when_already_current() {
        let content = "version: 1.0.0\n";
        assert!(replace_citation_version(content, "1.0.0").is_none());
    }

    #[test]
    fn replace_citation_version_ignores_nested_version_keys() {
        // CFF allows nested `version:` keys inside references blocks (indented).
        // Only the top-level one must change.
        let content = "version: 1.0.0\nreferences:\n  - type: software\n    version: 9.9.9\n";
        let new = replace_citation_version(content, "2.0.0").expect("regex matched");
        assert!(new.starts_with("version: 2.0.0\n"));
        assert!(new.contains("    version: 9.9.9\n"));
    }

    #[test]
    fn test_replace_version_pattern_ruby_version() {
        let content = r#"# This file is auto-generated by alef
module SampleCrate
  VERSION = "1.0.0"
end
"#;

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "2.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        assert_eq!(
            new_content,
            r#"# This file is auto-generated by alef
module SampleCrate
  VERSION = "2.0.0"
end
"#
        );
    }

    #[test]
    fn test_replace_version_pattern_ruby_version_single_quotes() {
        let content = "VERSION = '1.5.2'";

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "2.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        // rubocop Style/StringLiterals: output normalised to double quotes (rubocop default).
        assert_eq!(new_content, "VERSION = \"2.0.0\"");
    }

    #[test]
    fn test_replace_version_pattern_ruby_version_double_quotes() {
        let content = "VERSION = \"1.5.2\"";

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "3.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        // rubocop Style/StringLiterals: output normalised to double quotes regardless of input.
        assert_eq!(new_content, "VERSION = \"3.0.0\"");
    }

    #[test]
    fn test_replace_version_pattern_ruby_in_module() {
        let content = r#"module MyGem
  VERSION = "0.5.0"
end"#;

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "1.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        assert!(new_content.contains("VERSION = \"1.0.0\""));
        assert!(!new_content.contains("0.5.0"));
    }

    #[test]
    fn test_replace_version_pattern_no_match() {
        let content = "NOTHING = \"1.0.0\"";

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "2.0.0");
        assert!(result.is_none());
    }

    #[test]
    fn test_replace_version_pattern_preserves_other_content() {
        let content = r#"# frozen_string_literal: true
module SampleCrate
  VERSION = "1.0.0"
  # Other stuff
  CONST = "something"
end"#;

        let result = replace_version_pattern(content, r#"VERSION\s*=\s*['"][^'"]*['"]"#, "2.0.0");
        assert!(result.is_some());

        let new_content = result.unwrap();
        assert!(new_content.contains("# frozen_string_literal: true"));
        assert!(new_content.contains("CONST = \"something\""));
        assert!(new_content.contains("VERSION = \"2.0.0\""));
    }

    /// Verify that `finalize_hashes` updates the alef:hash line in a file that
    /// carries the alef header marker. This exercises the mechanism used by
    /// `sync_versions` to refresh stale hashes after rewriting version strings.
    #[test]
    fn test_finalize_hashes_updates_alef_hash_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("version.rb");

        // Simulate a version.rb that was written by alef with an alef header
        // but no hash line yet (as written by write_scaffold_files_with_overwrite
        // before finalize_hashes runs).
        let content = "# This file is auto-generated by alef — do not edit manually.\n# frozen_string_literal: true\n\nmodule MyGem\n  VERSION = '2.0.0'\nend\n";
        std::fs::write(&path, content).expect("write");

        let paths: std::collections::HashSet<std::path::PathBuf> = std::iter::once(path.clone()).collect();
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"ruby\"]\n";
        let n = generate::finalize_hashes(&paths, "test-sources-hash", alef_toml_bytes).expect("finalize ok");
        assert_eq!(n, 1, "finalize_hashes must update the file with the alef:hash line");

        let updated = std::fs::read_to_string(&path).expect("read");
        assert!(
            updated.contains("alef:hash:"),
            "file must contain alef:hash: after finalize_hashes, got:\n{updated}"
        );
    }

    /// Verify that `finalize_hashes` is idempotent: running it twice on the same
    /// file must not change the hash line on the second run.
    #[test]
    fn test_finalize_hashes_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("version.rb");

        let content =
            "# This file is auto-generated by alef — do not edit manually.\n\nmodule MyGem\n  VERSION = '2.0.0'\nend\n";
        std::fs::write(&path, content).expect("write");

        let paths: std::collections::HashSet<std::path::PathBuf> = std::iter::once(path.clone()).collect();
        let alef_toml_bytes = b"[workspace]\nlanguages = [\"ruby\"]\n";

        let _ = generate::finalize_hashes(&paths, "sources", alef_toml_bytes).expect("first finalize");
        let after_first = std::fs::read_to_string(&path).expect("read after first");

        let n2 = generate::finalize_hashes(&paths, "sources", alef_toml_bytes).expect("second finalize");
        assert_eq!(n2, 0, "second finalize_hashes must be a no-op (same inputs hash)");

        let after_second = std::fs::read_to_string(&path).expect("read after second");
        assert_eq!(after_first, after_second, "content must not change on second finalize");
    }

    const GEMFILE_LOCK_SAMPLE: &str = "\
PATH
  remote: .
  specs:
    sample_crate (4.10.0.pre.rc.13)
      rb_sys (~> 0.9)

GEM
  remote: https://rubygems.org/
  specs:
    rake (13.4.2)

PLATFORMS
  ruby

DEPENDENCIES
  sample_crate!

CHECKSUMS
  sample_crate (4.10.0.pre.rc.13)
  rake (13.4.2) sha256=abcdef

BUNDLED WITH
  4.0.7
";

    #[test]
    fn sync_gemfile_lock_updates_both_occurrences() {
        let result = sync_gemfile_lock(GEMFILE_LOCK_SAMPLE, "4.10.0.pre.rc.14");
        assert!(result.is_some(), "expected Some when version changes");
        let new = result.unwrap();
        // PATH > specs entry updated
        assert!(
            new.contains("    sample_crate (4.10.0.pre.rc.14)"),
            "PATH specs entry not updated:\n{new}"
        );
        // CHECKSUMS entry updated
        assert!(
            new.contains("  sample_crate (4.10.0.pre.rc.14)"),
            "CHECKSUMS entry not updated:\n{new}"
        );
        // Other gem versions are unchanged
        assert!(
            new.contains("rake (13.4.2)"),
            "non-path gem version must not change:\n{new}"
        );
        // Old version must be gone
        assert!(!new.contains("4.10.0.pre.rc.13"), "old version must be removed:\n{new}");
    }

    #[test]
    fn sync_gemfile_lock_is_idempotent() {
        let first = sync_gemfile_lock(GEMFILE_LOCK_SAMPLE, "4.10.0.pre.rc.14").unwrap();
        let second = sync_gemfile_lock(&first, "4.10.0.pre.rc.14");
        assert!(
            second.is_none(),
            "second call with same version must return None (already in sync)"
        );
    }

    #[test]
    fn sync_gemfile_lock_preserves_trailing_newline() {
        let with_newline = format!("{GEMFILE_LOCK_SAMPLE}\n");
        let result = sync_gemfile_lock(&with_newline, "4.10.0.pre.rc.99").unwrap();
        assert!(result.ends_with('\n'), "trailing newline must be preserved");
    }

    #[test]
    fn sync_gemfile_lock_no_path_gem_returns_none() {
        // A Gemfile.lock with no PATH block — nothing to sync.
        let content = "GEM\n  remote: https://rubygems.org/\n  specs:\n    rake (13.4.2)\n";
        let result = sync_gemfile_lock(content, "1.0.0");
        assert!(result.is_none(), "no PATH gem means nothing to update");
    }

    #[test]
    fn restore_gleam_dep_ranges_repairs_corrupted_workspace_version_ranges() {
        let corrupted = "name = \"sample_crate\"\nversion = \"5.0.0-rc.1\"\ntarget = \"erlang\"\n\n[dependencies]\ngleam_stdlib = \">= 5.0.0-rc.1 and < 5.0.0-rc.1\"\n\n[dev-dependencies]\ngleeunit = \">= 5.0.0-rc.1 and < 5.0.0-rc.1\"\n";
        let healed = restore_gleam_dep_ranges(corrupted);
        assert!(
            healed.contains("gleam_stdlib = \">= 0.34.0 and < 2.0.0\""),
            "gleam_stdlib should be restored to canonical range, got:\n{healed}"
        );
        assert!(
            healed.contains("gleeunit = \">= 1.0.0 and < 2.0.0\""),
            "gleeunit should be restored to canonical range, got:\n{healed}"
        );
        // The package version line itself must not be touched.
        assert!(
            healed.contains("version = \"5.0.0-rc.1\""),
            "package version must not be rewritten, got:\n{healed}"
        );
    }

    #[test]
    fn restore_gleam_dep_ranges_is_idempotent_on_healthy_input() {
        let healthy = "name = \"sample_crate\"\nversion = \"5.0.0-rc.1\"\n\n[dependencies]\ngleam_stdlib = \">= 0.34.0 and < 2.0.0\"\n\n[dev-dependencies]\ngleeunit = \">= 1.0.0 and < 2.0.0\"\n";
        let healed = restore_gleam_dep_ranges(healthy);
        assert_eq!(healed, healthy, "healthy gleam.toml must not be rewritten");
    }

    /// Root `package.json` is a private "root" pnpm-workspace bookkeeping
    /// manifest. It carries its own top-level `"version"` that must track the
    /// canonical Cargo.toml version so `validate-versions` does not flag a
    /// drift on every release. The replacement must not touch nested
    /// `"version"` fields inside `devDependencies` / `pnpm.overrides` / etc.
    #[test]
    fn test_replace_version_pattern_root_package_json_only_top_level() {
        let content = r#"{
  "name": "sample_crate-root",
  "version": "4.9.5",
  "private": true,
  "devDependencies": {
    "@vitest/coverage-v8": "^4.1.5",
    "tsx": "^4.21.0",
    "typescript": "^6.0.3"
  },
  "pnpm": {
    "overrides": {
      "glob": "10.5.0"
    }
  }
}
"#;
        let new_content = replace_version_pattern(content, r#""version":\s*"[^"]*""#, "5.0.0-rc.1")
            .expect("root package.json version must update");
        assert!(
            new_content.contains(r#""version": "5.0.0-rc.1""#),
            "top-level version must be rewritten, got:\n{new_content}"
        );
        assert!(
            !new_content.contains(r#""version": "4.9.5""#),
            "old version must be removed, got:\n{new_content}"
        );
        // Nested dependency versions and pnpm overrides must remain intact.
        assert!(
            new_content.contains("\"@vitest/coverage-v8\": \"^4.1.5\""),
            "devDependency version specs must not be touched, got:\n{new_content}"
        );
        assert!(
            new_content.contains("\"glob\": \"10.5.0\""),
            "pnpm overrides must not be touched, got:\n{new_content}"
        );
    }

    /// Serialize tests that mutate process-global CWD. `std::env::set_current_dir`
    /// is shared across the test binary, so concurrent tempdir-based `sync_versions`
    /// tests would race without this guard.
    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// End-to-end: `sync_versions` must rewrite both `package.json` (root) and
    /// every `crates/*-node/package.json` file alongside the existing manifests.
    /// Regression test for the sample_core publish.yaml dry-run failure where the
    /// root manifest stayed at 4.9.5 while Cargo.toml jumped to 5.0.0-rc.1.
    #[test]
    fn sync_versions_writes_root_and_node_crate_package_json() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Minimal workspace: Cargo.toml at canonical "1.0.0", root package.json
        // and crates/mylib-node/package.json both stale at "0.9.0".
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");
        std::fs::write(
            root.join("package.json"),
            "{\n  \"name\": \"mylib-root\",\n  \"version\": \"0.9.0\",\n  \"private\": true\n}\n",
        )
        .expect("write root package.json");
        std::fs::create_dir_all(root.join("crates/mylib-node")).expect("mkdir crates/mylib-node");
        std::fs::write(
            root.join("crates/mylib-node/package.json"),
            "{\n  \"name\": \"mylib\",\n  \"version\": \"0.9.0\"\n}\n",
        )
        .expect("write crates/mylib-node/package.json");

        // Drop a minimal alef.toml so we can resolve a config.
        // Normalize backslashes to / so the path is a valid TOML basic string on Windows.
        let alef_toml = format!(
            "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        // Switch into the tempdir for the duration of the call — sync_versions
        // resolves relative paths against CWD.
        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        // Always restore the CWD before unwrapping, so a panic doesn't leave
        // the test runner in a broken directory.
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let root_pkg = std::fs::read_to_string(root.join("package.json")).expect("read root package.json");
        assert!(
            root_pkg.contains(r#""version": "1.0.0""#),
            "root package.json must be bumped to canonical version, got:\n{root_pkg}"
        );
        assert!(
            !root_pkg.contains("0.9.0"),
            "old version must be gone from root package.json, got:\n{root_pkg}"
        );

        let node_pkg = std::fs::read_to_string(root.join("crates/mylib-node/package.json"))
            .expect("read crates/mylib-node/package.json");
        assert!(
            node_pkg.contains(r#""version": "1.0.0""#),
            "crates/*-node/package.json must be bumped to canonical version, got:\n{node_pkg}"
        );
    }

    /// `sync_versions` must rewrite `optionalDependencies` pins to sibling NAPI
    /// platform packages and the pre-staged platform manifests under
    /// `crates/*-node/npm/<platform>/package.json`. Leaving these stale makes
    /// `pnpm install --frozen-lockfile` fail with `ERR_PNPM_OUTDATED_LOCKFILE`
    /// because the lockfile and manifest disagree on the platform-package
    /// version. Regression test for the sample_crawler rc.34 Build Node bindings
    /// failure where the top-level version was rc.34 but the `optionalDependencies`
    /// and platform manifests stayed at rc.33.
    #[test]
    fn sync_versions_bumps_napi_platform_pins_and_manifests() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        // crate-level manifest with optionalDependencies at the OLD version
        std::fs::create_dir_all(root.join("crates/mylib-node")).expect("mkdir crates/mylib-node");
        std::fs::write(
            root.join("crates/mylib-node/package.json"),
            "{\n  \"name\": \"@scope/mylib\",\n  \"version\": \"0.9.0\",\n  \"optionalDependencies\": {\n    \"@scope/mylib-linux-x64-gnu\": \"0.9.0\",\n    \"@scope/mylib-darwin-arm64\": \"0.9.0\",\n    \"@scope/mylib-win32-x64-msvc\": \"0.9.0\"\n  }\n}\n",
        )
        .expect("write crates/mylib-node/package.json");

        // Pre-staged platform manifests at the OLD version
        for platform in &["linux-x64-gnu", "darwin-arm64", "win32-x64-msvc"] {
            let dir = root.join(format!("crates/mylib-node/npm/{platform}"));
            std::fs::create_dir_all(&dir).expect("mkdir platform dir");
            std::fs::write(
                dir.join("package.json"),
                format!("{{\n  \"name\": \"@scope/mylib-{platform}\",\n  \"version\": \"0.9.0\"\n}}\n"),
            )
            .expect("write platform package.json");
        }

        let alef_toml = format!(
            "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let crate_pkg = std::fs::read_to_string(root.join("crates/mylib-node/package.json"))
            .expect("read crates/mylib-node/package.json");
        assert!(
            !crate_pkg.contains("0.9.0"),
            "old version must be gone from crates/mylib-node/package.json (including optionalDependencies), got:\n{crate_pkg}"
        );
        assert!(
            crate_pkg.contains(r#""@scope/mylib-linux-x64-gnu": "1.0.0""#),
            "optionalDependencies pin to linux-x64-gnu must be bumped, got:\n{crate_pkg}"
        );
        assert!(
            crate_pkg.contains(r#""@scope/mylib-darwin-arm64": "1.0.0""#),
            "optionalDependencies pin to darwin-arm64 must be bumped, got:\n{crate_pkg}"
        );
        assert!(
            crate_pkg.contains(r#""@scope/mylib-win32-x64-msvc": "1.0.0""#),
            "optionalDependencies pin to win32-x64-msvc must be bumped, got:\n{crate_pkg}"
        );

        for platform in &["linux-x64-gnu", "darwin-arm64", "win32-x64-msvc"] {
            let manifest = std::fs::read_to_string(root.join(format!("crates/mylib-node/npm/{platform}/package.json")))
                .expect("read platform package.json");
            assert!(
                manifest.contains(r#""version": "1.0.0""#),
                "platform manifest {platform} must be bumped, got:\n{manifest}"
            );
            assert!(
                !manifest.contains("0.9.0"),
                "old version must be gone from platform manifest {platform}, got:\n{manifest}"
            );
        }
    }

    /// `sync_versions` must bump BOTH the consumer pyproject
    /// (`packages/python/pyproject.toml`) and the source-template pyproject that
    /// lives alongside the PyO3 crate (`crates/{lib}-py/src/pyproject.toml`,
    /// selected via `[crates.output] python`) to the PEP 440 normalised
    /// prerelease form. Regression test for the source-template version drift
    /// that made `alef validate versions` fail on a tagged prerelease.
    #[test]
    fn sync_versions_bumps_both_python_pyprojects_to_pep440_prerelease() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.15.6-rc.2\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        // Consumer publish manifest, stale.
        std::fs::create_dir_all(root.join("packages/python")).expect("mkdir packages/python");
        std::fs::write(
            root.join("packages/python/pyproject.toml"),
            "[project]\nname = \"mylib\"\nversion = \"0.15.5\"\n",
        )
        .expect("write packages/python/pyproject.toml");

        // Source-template manifest alongside the PyO3 crate, stale.
        std::fs::create_dir_all(root.join("crates/mylib-py/src")).expect("mkdir crates/mylib-py/src");
        std::fs::write(
            root.join("crates/mylib-py/src/pyproject.toml"),
            "[project]\nname = \"mylib\"\nversion = \"0.15.5\"\n",
        )
        .expect("write crates/mylib-py/src/pyproject.toml");

        let alef_toml = format!(
            "[workspace]\nlanguages = [\"python\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n[crates.output]\npython = \"crates/mylib-py/src/\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let consumer =
            std::fs::read_to_string(root.join("packages/python/pyproject.toml")).expect("read consumer pyproject");
        assert!(
            consumer.contains(r#"version = "0.15.6rc2""#),
            "consumer pyproject must be PEP 440 normalised, got:\n{consumer}"
        );

        let source = std::fs::read_to_string(root.join("crates/mylib-py/src/pyproject.toml"))
            .expect("read source-template pyproject");
        assert!(
            source.contains(r#"version = "0.15.6rc2""#),
            "source-template pyproject must be PEP 440 normalised, got:\n{source}"
        );
        assert!(
            !source.contains("0.15.5") && !source.contains("0.15.6-rc.2"),
            "source-template must hold only the normalised version, got:\n{source}"
        );
    }

    // -----------------------------------------------------------------------
    // patch_workspace_dep_versions unit tests
    // -----------------------------------------------------------------------

    /// patch_workspace_dep_versions updates [dependencies], [dev-dependencies],
    /// [build-dependencies], [target.*.dependencies], and [workspace.dependencies]
    /// but leaves external crate pins intact.
    #[test]
    fn patch_workspace_dep_versions_all_dep_table_shapes() {
        use std::collections::HashSet;

        let dir = tempfile::tempdir().expect("tempdir");

        let cargo_toml = r#"[package]
name = "crate-a"
version = "5.0.0-rc.1"

[dependencies]
crate-b = { path = "../crate-b", version = "5.0.0-rc.1", optional = true }
serde = "1.0"

[dev-dependencies]
crate-c = { path = "../crate-c", version = "5.0.0-rc.1" }
tempfile = "3"

[build-dependencies]
crate-b = { path = "../crate-b", version = "5.0.0-rc.1" }

[target.'cfg(unix)'.dependencies]
crate-b = { path = "../crate-b", version = "5.0.0-rc.1", optional = true }
libc = "0.2"

[workspace.dependencies]
crate-c = { path = "../crate-c", version = "5.0.0-rc.1", default-features = false }
tokio = { version = "1.0", features = ["full"] }
"#;

        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, cargo_toml).expect("write");

        let members: HashSet<String> = ["crate-b", "crate-c"].iter().map(|s| s.to_string()).collect();

        let changed = patch_workspace_dep_versions(path.to_str().unwrap(), "5.0.0-rc.2", &members).expect("patch ok");

        assert!(changed, "at least one version pin must have been updated");

        let result = std::fs::read_to_string(&path).expect("read");

        // [package] version is NOT touched by patch_workspace_dep_versions — only dep tables.
        // All workspace member dep-table pins must be bumped to rc.2.
        // crate-b appears in [dependencies], [build-dependencies], and [target.*.dependencies].
        let crate_b_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.contains("crate-b") && l.contains("version"))
            .collect();
        assert!(
            !crate_b_lines.is_empty(),
            "expected crate-b dep lines with version=:\n{result}"
        );
        for line in &crate_b_lines {
            assert!(
                line.contains("5.0.0-rc.2"),
                "crate-b pin not bumped:\n  {line}\nfull:\n{result}"
            );
        }
        // crate-c appears in [dev-dependencies] and [workspace.dependencies].
        let crate_c_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.contains("crate-c") && l.contains("version"))
            .collect();
        assert!(
            !crate_c_lines.is_empty(),
            "expected crate-c dep lines with version=:\n{result}"
        );
        for line in &crate_c_lines {
            assert!(
                line.contains("5.0.0-rc.2"),
                "crate-c pin not bumped:\n  {line}\nfull:\n{result}"
            );
        }

        // External crates must be untouched.
        assert!(
            result.contains(r#"serde = "1.0""#),
            "serde must not be touched:\n{result}"
        );
        assert!(
            result.contains(r#"tempfile = "3""#),
            "tempfile must not be touched:\n{result}"
        );
        assert!(
            result.contains(r#"libc = "0.2""#),
            "libc must not be touched:\n{result}"
        );
        assert!(
            result.contains(r#"tokio = { version = "1.0", features = ["full"] }"#),
            "tokio must not be touched:\n{result}"
        );
    }

    /// patch_workspace_dep_versions is idempotent: calling it twice with the
    /// same target version returns false and does not rewrite the file.
    #[test]
    fn patch_workspace_dep_versions_is_idempotent() {
        use std::collections::HashSet;

        let dir = tempfile::tempdir().expect("tempdir");

        let cargo_toml = "[package]\nname = \"crate-a\"\nversion = \"5.0.0-rc.2\"\n\n[dependencies]\ncrate-b = { path = \"../crate-b\", version = \"5.0.0-rc.2\" }\n";

        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, cargo_toml).expect("write");

        let members: HashSet<String> = std::iter::once("crate-b".to_string()).collect();

        let changed = patch_workspace_dep_versions(path.to_str().unwrap(), "5.0.0-rc.2", &members).expect("patch ok");
        assert!(!changed, "no change expected when already at target version");
    }

    /// patch_workspace_dep_versions does not touch path-only deps (no version= key).
    #[test]
    fn patch_workspace_dep_versions_skips_path_only_deps() {
        use std::collections::HashSet;

        let dir = tempfile::tempdir().expect("tempdir");

        let cargo_toml = "[package]\nname = \"crate-a\"\nversion = \"1.0.0\"\n\n[dependencies]\ncrate-b = { path = \"../crate-b\" }\n";

        let path = dir.path().join("Cargo.toml");
        std::fs::write(&path, cargo_toml).expect("write");

        let members: HashSet<String> = std::iter::once("crate-b".to_string()).collect();

        let changed = patch_workspace_dep_versions(path.to_str().unwrap(), "2.0.0", &members).expect("patch ok");
        assert!(!changed, "path-only deps without version= must not be touched");
    }

    // -----------------------------------------------------------------------
    // sync_versions dep-table end-to-end test
    // -----------------------------------------------------------------------

    /// Full workspace e2e: after sync_versions the version bump propagates from
    /// [workspace.package] into [workspace.dependencies] and all dep-table shapes
    /// in member crates. External pins must be untouched.
    #[test]
    fn sync_versions_patches_dep_tables_on_version_change() {
        use crate::core::config::NewAlefConfig;

        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        fn write_file(dir: &std::path::Path, rel: &str, content: &str) {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("mkdir");
            }
            std::fs::write(path, content).expect("write");
        }

        // Root Cargo.toml: canonical version already at rc.2 (simulates task version:set).
        write_file(
            root,
            "Cargo.toml",
            "[workspace.package]\nversion = \"5.0.0-rc.2\"\n\n[workspace]\nresolver = \"2\"\nmembers = [\"crates/alpha\", \"crates/beta\"]\n\n[workspace.dependencies]\nalpha = { path = \"crates/alpha\", version = \"5.0.0-rc.1\", default-features = false }\nserde = \"1.0\"\n",
        );

        // crates/alpha: upstream crate, no intra-workspace deps.
        write_file(
            root,
            "crates/alpha/Cargo.toml",
            "[package]\nname = \"alpha\"\nversion = \"5.0.0-rc.1\"\n\n[dependencies]\nserde = \"1.0\"\n",
        );

        // crates/beta: all four dep-table shapes referencing alpha.
        write_file(
            root,
            "crates/beta/Cargo.toml",
            "[package]\nname = \"beta\"\nversion = \"5.0.0-rc.1\"\n\n[dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\", optional = true }\nserde = \"1.0\"\n\n[dev-dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\" }\ntempfile = \"3\"\n\n[build-dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\" }\n\n[target.'cfg(unix)'.dependencies]\nalpha = { path = \"../alpha\", version = \"5.0.0-rc.1\", features = [\"unix\"] }\nlibc = \"0.2\"\n",
        );

        // Normalize backslashes to / so the path is a valid TOML basic string on Windows.
        let alef_toml_content = format!(
            "[workspace]\nlanguages = [\"node\"]\n[[crates]]\nname = \"alpha\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        write_file(root, "alef.toml", &alef_toml_content);
        let alef_toml_path = root.join("alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml_content).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        // Root [workspace.dependencies] alpha pin must be bumped to rc.2.
        let root_cargo = std::fs::read_to_string(root.join("Cargo.toml")).expect("read root");
        assert!(
            root_cargo.contains(r#"alpha = { path = "crates/alpha", version = "5.0.0-rc.2""#),
            "root [workspace.dependencies] alpha must be bumped to rc.2:\n{root_cargo}"
        );
        assert!(
            root_cargo.contains(r#"serde = "1.0""#),
            "root serde must be untouched:\n{root_cargo}"
        );

        // crates/alpha [package] version must be rc.2.
        let alpha_cargo = std::fs::read_to_string(root.join("crates/alpha/Cargo.toml")).expect("read alpha");
        assert!(
            alpha_cargo.contains("version = \"5.0.0-rc.2\""),
            "alpha [package] must be bumped:\n{alpha_cargo}"
        );

        // crates/beta: all four dep-table shapes must reference rc.2.
        let beta_cargo = std::fs::read_to_string(root.join("crates/beta/Cargo.toml")).expect("read beta");
        let alpha_version_lines: Vec<&str> = beta_cargo
            .lines()
            .filter(|l| l.contains("alpha") && l.contains("version"))
            .collect();
        assert!(
            !alpha_version_lines.is_empty(),
            "expected alpha dep lines with version= in beta:\n{beta_cargo}"
        );
        for line in &alpha_version_lines {
            assert!(
                line.contains("5.0.0-rc.2"),
                "alpha pin not bumped to rc.2 in beta:\n  {line}\nfull:\n{beta_cargo}"
            );
        }
        assert!(
            !beta_cargo.contains("5.0.0-rc.1"),
            "old rc.1 must be gone from beta:\n{beta_cargo}"
        );

        // External deps in beta must be untouched.
        assert!(
            beta_cargo.contains(r#"serde = "1.0""#),
            "serde must not be touched:\n{beta_cargo}"
        );
        assert!(
            beta_cargo.contains(r#"tempfile = "3""#),
            "tempfile must not be touched:\n{beta_cargo}"
        );
        assert!(
            beta_cargo.contains(r#"libc = "0.2""#),
            "libc must not be touched:\n{beta_cargo}"
        );
    }

    #[test]
    fn run_optional_logs_but_does_not_fail_on_missing_binary() {
        // Verify that run_optional gracefully handles a binary that doesn't exist.
        // This test just invokes the function and verifies it doesn't panic.
        // The actual command execution would fail, but run_optional logs and returns.
        super::super::helpers::run_optional("nonexistent_binary_12345", &["arg1", "arg2"]);
        // If we reach here without panicking, the test passes.
    }

    #[test]
    fn run_optional_succeeds_for_simple_command() {
        // Verify that run_optional can run a simple builtin command (echo) successfully.
        super::super::helpers::run_optional("echo", &["test"]);
        // If we reach here without panicking, the test passes.
    }

    // --- Kotlin Gradle project version ------------------------------------

    const GRADLE_BUILD_SAMPLE: &str = r#"import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
  `java-library`
  kotlin("jvm") version "2.3.21"
  `maven-publish`
  id("org.jlleitschuh.gradle.ktlint") version "12.1.0"
}

group = "dev.example"
version = "0.15.6-rc.2"

repositories {
  mavenCentral()
}

dependencies {
  api("net.java.dev.jna:jna:5.14.0")
}

ktlint {
  version.set("1.0.1")
}
"#;

    #[test]
    fn replace_gradle_project_version_bumps_only_project_version() {
        let out = replace_gradle_project_version(GRADLE_BUILD_SAMPLE, "0.15.6-rc.3").expect("project version bumped");
        assert!(
            out.contains("version = \"0.15.6-rc.3\""),
            "project version must be bumped:\n{out}"
        );
        // Plugin version strings must be untouched.
        assert!(
            out.contains(r#"kotlin("jvm") version "2.3.21""#),
            "kotlin plugin version must not change:\n{out}"
        );
        assert!(
            out.contains(r#"id("org.jlleitschuh.gradle.ktlint") version "12.1.0""#),
            "ktlint plugin version must not change:\n{out}"
        );
        // ktlint extension version must be untouched.
        assert!(
            out.contains(r#"version.set("1.0.1")"#),
            "ktlint extension version must not change:\n{out}"
        );
        // Dependency coordinate must be untouched.
        assert!(
            out.contains(r#"api("net.java.dev.jna:jna:5.14.0")"#),
            "jna coordinate must not change:\n{out}"
        );
        assert!(!out.contains("0.15.6-rc.2"), "old version must be gone:\n{out}");
    }

    #[test]
    fn replace_gradle_project_version_is_idempotent() {
        let first = replace_gradle_project_version(GRADLE_BUILD_SAMPLE, "0.15.6-rc.3").unwrap();
        assert!(
            replace_gradle_project_version(&first, "0.15.6-rc.3").is_none(),
            "second call with same version must return None"
        );
    }

    #[test]
    fn replace_gradle_project_version_no_project_version_returns_none() {
        // A build file with only plugin/extension version constructs — no
        // top-level `version = "..."` assignment to bump.
        let content = "plugins {\n  kotlin(\"jvm\") version \"2.3.21\"\n}\n";
        assert!(replace_gradle_project_version(content, "1.0.0").is_none());
    }

    // --- NIF Cargo.lock path-source versions ------------------------------

    const NIF_CARGO_LOCK_SAMPLE: &str = r#"# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 4

[[package]]
name = "example_core"
version = "0.15.6-rc.2"
dependencies = [
 "serde",
]

[[package]]
name = "example_nif"
version = "0.15.6-rc.2"
dependencies = [
 "example_core",
 "rustler",
]

[[package]]
name = "serde"
version = "1.0.219"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "5f0e2c6ed6606019b4e29e69dbaba95b11854410e5347d525002456dbbb786b6"
"#;

    #[test]
    fn sync_cargo_lock_path_versions_bumps_only_sourceless_entries() {
        let out = sync_cargo_lock_path_versions(NIF_CARGO_LOCK_SAMPLE, "0.15.6-rc.3").expect("lock updated");
        // The lock-format preamble must be preserved verbatim.
        assert!(
            out.contains("version = 4"),
            "lock format version line must be preserved:\n{out}"
        );
        // Local/path crates (no source key) get bumped.
        assert_eq!(
            out.matches("version = \"0.15.6-rc.3\"").count(),
            2,
            "both local crates must be bumped:\n{out}"
        );
        assert!(!out.contains("0.15.6-rc.2"), "old version must be gone:\n{out}");
        // Registry dependency must be untouched.
        assert!(
            out.contains("version = \"1.0.219\""),
            "registry dep version must not change:\n{out}"
        );
        assert!(
            out.contains("source = \"registry+https://github.com/rust-lang/crates.io-index\""),
            "registry source line must be preserved:\n{out}"
        );
    }

    #[test]
    fn sync_cargo_lock_path_versions_is_idempotent() {
        let first = sync_cargo_lock_path_versions(NIF_CARGO_LOCK_SAMPLE, "0.15.6-rc.3").unwrap();
        assert!(
            sync_cargo_lock_path_versions(&first, "0.15.6-rc.3").is_none(),
            "second call with same version must return None"
        );
    }

    #[test]
    fn sync_cargo_lock_path_versions_preserves_no_trailing_newline() {
        let no_newline = NIF_CARGO_LOCK_SAMPLE.trim_end_matches('\n');
        let out = sync_cargo_lock_path_versions(no_newline, "0.15.6-rc.3").unwrap();
        assert!(
            !out.ends_with('\n'),
            "absence of trailing newline must be preserved:\n{out:?}"
        );
    }

    #[test]
    fn sync_cargo_lock_path_versions_all_registry_returns_none() {
        // Lockfile where every package has a source — nothing local to bump.
        let content = "version = 4\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.219\"\nsource = \"registry+x\"\n";
        assert!(sync_cargo_lock_path_versions(content, "9.9.9").is_none());
    }

    // --- Docs version badge -----------------------------------------------

    #[test]
    fn sync_docs_version_badges_updates_api_files_only() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        std::fs::write(
            dir.join("api-rust.md"),
            "## Rust API Reference <span class=\"version-badge\">v0.15.6-rc.2</span>\n\nbody\n",
        )
        .expect("write api-rust.md");
        std::fs::write(
            dir.join("api-python.md"),
            "## Python API Reference <span class=\"version-badge\">v0.15.6-rc.2</span>\n",
        )
        .expect("write api-python.md");
        // A non-api doc must not be touched even if it has a badge.
        std::fs::write(
            dir.join("configuration.md"),
            "## Configuration <span class=\"version-badge\">v0.15.6-rc.2</span>\n",
        )
        .expect("write configuration.md");

        let updated = sync_docs_version_badges(dir, "0.15.6-rc.3");
        assert_eq!(
            updated.len(),
            2,
            "only the two api-*.md files must be updated: {updated:?}"
        );

        let rust = std::fs::read_to_string(dir.join("api-rust.md")).unwrap();
        assert!(
            rust.contains("<span class=\"version-badge\">v0.15.6-rc.3</span>"),
            "rust badge must be bumped:\n{rust}"
        );
        let config = std::fs::read_to_string(dir.join("configuration.md")).unwrap();
        assert!(
            config.contains("v0.15.6-rc.2"),
            "non-api doc must not be touched:\n{config}"
        );
    }

    #[test]
    fn sync_docs_version_badges_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        std::fs::write(
            dir.join("api-go.md"),
            "## Go API Reference <span class=\"version-badge\">v1.0.0</span>\n",
        )
        .expect("write api-go.md");
        let _ = sync_docs_version_badges(dir, "1.0.0");
        let second = sync_docs_version_badges(dir, "1.0.0");
        assert!(second.is_empty(), "second call with same version must be a no-op");
    }

    /// End-to-end: a `sync_versions` bump must update the Kotlin package gradle
    /// project version, the NIF crate's committed Cargo.lock path entries, and
    /// the docs API-reference version badges — the three coverage gaps.
    #[test]
    fn sync_versions_bumps_kotlin_gradle_nif_lock_and_docs_badges() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.15.6-rc.3\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        // Kotlin package build.gradle.kts, stale.
        std::fs::create_dir_all(root.join("packages/kotlin")).expect("mkdir packages/kotlin");
        std::fs::write(root.join("packages/kotlin/build.gradle.kts"), GRADLE_BUILD_SAMPLE)
            .expect("write build.gradle.kts");

        // Elixir NIF crate committed Cargo.lock, stale.
        std::fs::create_dir_all(root.join("packages/elixir/native/example_nif")).expect("mkdir native");
        std::fs::write(
            root.join("packages/elixir/native/example_nif/Cargo.lock"),
            NIF_CARGO_LOCK_SAMPLE,
        )
        .expect("write Cargo.lock");

        // Docs API-reference page, stale badge.
        std::fs::create_dir_all(root.join("docs/reference")).expect("mkdir docs/reference");
        std::fs::write(
            root.join("docs/reference/api-elixir.md"),
            "## Elixir API Reference <span class=\"version-badge\">v0.15.6-rc.2</span>\n",
        )
        .expect("write api-elixir.md");

        let alef_toml = format!(
            "[workspace]\nlanguages = [\"kotlin\", \"elixir\"]\n[[crates]]\nname = \"example\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let gradle = std::fs::read_to_string(root.join("packages/kotlin/build.gradle.kts")).expect("read gradle");
        assert!(
            gradle.contains("version = \"0.15.6-rc.3\""),
            "kotlin gradle project version must be bumped:\n{gradle}"
        );
        assert!(
            gradle.contains(r#"kotlin("jvm") version "2.3.21""#),
            "kotlin plugin version must not change:\n{gradle}"
        );

        let lock =
            std::fs::read_to_string(root.join("packages/elixir/native/example_nif/Cargo.lock")).expect("read lock");
        assert_eq!(
            lock.matches("version = \"0.15.6-rc.3\"").count(),
            2,
            "both local NIF lock entries must be bumped:\n{lock}"
        );
        assert!(
            lock.contains("version = \"1.0.219\""),
            "registry dep in lock must not change:\n{lock}"
        );

        let badge = std::fs::read_to_string(root.join("docs/reference/api-elixir.md")).expect("read api-elixir.md");
        assert!(
            badge.contains("<span class=\"version-badge\">v0.15.6-rc.3</span>"),
            "docs version badge must be bumped:\n{badge}"
        );
    }

    // -----------------------------------------------------------------------
    // render_registry_version unit tests
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // update_zig_package_hash unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn update_zig_package_hash_rc_prerelease() {
        // Zig hash format: `<name>-<version>-<base64sha>`
        // When syncing from rc.50 to rc.53, only the version part should change.
        let existing = "sample_pkg-1.4.0-rc.50-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9";
        let result = update_zig_package_hash(existing, "1.4.0-rc.50", "1.4.0-rc.53");
        assert_eq!(
            result,
            Some("sample_pkg-1.4.0-rc.53-Jfgk_HsxAQAl3_LX7NCs1l27EHcYVF9dieEDCVAwUxK9".to_string()),
            "rc prerelease version must be substituted in hash"
        );
    }

    #[test]
    fn update_zig_package_hash_release_version() {
        // Zig hash from rc.53 to stable 1.4.0
        let existing = "sample_pkg-1.4.0-rc.53-AbCd_XyZ123456789";
        let result = update_zig_package_hash(existing, "1.4.0-rc.53", "1.4.0");
        assert_eq!(
            result,
            Some("sample_pkg-1.4.0-AbCd_XyZ123456789".to_string()),
            "release version must substitute prerelease"
        );
    }

    #[test]
    fn update_zig_package_hash_same_version_is_none() {
        // No change → return None
        let existing = "mylib-0.1.0-rc.1-SomeBase64Hash";
        let result = update_zig_package_hash(existing, "0.1.0-rc.1", "0.1.0-rc.1");
        assert_eq!(result, None, "same version must return None");
    }

    #[test]
    fn update_zig_package_hash_malformed_hash_is_none() {
        // Malformed hash (too few dashes) → return None gracefully
        let existing = "notenoughparts";
        let result = update_zig_package_hash(existing, "0.1.0", "0.2.0");
        assert_eq!(result, None, "malformed hash must return None");
    }

    // -----------------------------------------------------------------------
    // render_registry_version unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn render_registry_version_python_pep440_rc_prerelease() {
        // PEP 440: ">=0.1.0rc9" → ">=0.3.0rc28"
        let result = render_registry_version("python", "0.3.0-rc.28", ">=0.1.0rc9");
        assert_eq!(result, Some(">=0.3.0rc28".to_string()));
    }

    #[test]
    fn render_registry_version_python_pep440_release() {
        let result = render_registry_version("python", "1.0.0", ">=0.9.0");
        assert_eq!(result, Some(">=1.0.0".to_string()));
    }

    #[test]
    fn render_registry_version_python_already_current_is_none() {
        let result = render_registry_version("python", "0.3.0-rc.28", ">=0.3.0rc28");
        assert_eq!(result, None);
    }

    #[test]
    fn render_registry_version_node_semver_rc() {
        // npm: "^0.1.0-rc.9" → "^0.3.0-rc.28"
        let result = render_registry_version("node", "0.3.0-rc.28", "^0.1.0-rc.9");
        assert_eq!(result, Some("^0.3.0-rc.28".to_string()));
    }

    #[test]
    fn render_registry_version_elixir_hex_constraint() {
        // Hex: "~> 0.1.0-rc.9" → "~> 0.3.0-rc.28"
        let result = render_registry_version("elixir", "0.3.0-rc.28", "~> 0.1.0-rc.9");
        assert_eq!(result, Some("~> 0.3.0-rc.28".to_string()));
    }

    #[test]
    fn render_registry_version_ruby_rubygems_prerelease() {
        // RubyGems: ">= 0.1.0.pre.rc.9" → ">= 0.3.0.pre.rc.28"
        let result = render_registry_version("ruby", "0.3.0-rc.28", ">= 0.1.0.pre.rc.9");
        assert_eq!(result, Some(">= 0.3.0.pre.rc.28".to_string()));
    }

    #[test]
    fn render_registry_version_ruby_already_current_is_none() {
        let result = render_registry_version("ruby", "0.3.0-rc.28", ">= 0.3.0.pre.rc.28");
        assert_eq!(result, None);
    }

    #[test]
    fn render_registry_version_go_module_version() {
        // Go: "v0.1.0-rc.9" → "v0.3.0-rc.28"
        let result = render_registry_version("go", "0.3.0-rc.28", "v0.1.0-rc.9");
        assert_eq!(result, Some("v0.3.0-rc.28".to_string()));
    }

    #[test]
    fn render_registry_version_rust_bare_semver() {
        // crates.io: "0.1.0-rc.9" → "0.3.0-rc.28"
        let result = render_registry_version("rust", "0.3.0-rc.28", "0.1.0-rc.9");
        assert_eq!(result, Some("0.3.0-rc.28".to_string()));
    }

    #[test]
    fn render_registry_version_php_composer_range() {
        // Composer: ">=0.1.0-rc.9" → ">=0.3.0-rc.28"
        let result = render_registry_version("php", "0.3.0-rc.28", ">=0.1.0-rc.9");
        assert_eq!(result, Some(">=0.3.0-rc.28".to_string()));
    }

    // -----------------------------------------------------------------------
    // sync_registry_package_versions integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn sync_registry_package_versions_rewrites_all_language_entries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let alef_toml_path = tmp.path().join("alef.toml");
        std::fs::write(
            &alef_toml_path,
            concat!(
                "[workspace]\nalef_version = \"0.19.0\"\nlanguages = []\n\n",
                "[[crates]]\nname = \"mylib\"\nsources = []\n\n",
                "[crates.e2e.registry]\noutput = \"test_apps\"\n\n",
                "[crates.e2e.registry.packages.python]\n",
                "name = \"mylib\"\n",
                "version = \">=0.1.0rc9\"\n\n",
                "[crates.e2e.registry.packages.node]\n",
                "name = \"@myorg/mylib\"\n",
                "version = \"^0.1.0-rc.9\"\n\n",
                "[crates.e2e.registry.packages.elixir]\n",
                "name = \"mylib\"\n",
                "version = \"~> 0.1.0-rc.9\"\n\n",
                "[crates.e2e.registry.packages.ruby]\n",
                "name = \"mylib\"\n",
                "version = \">= 0.1.0.pre.rc.9\"\n",
            ),
        )
        .expect("write alef.toml");

        let changed = sync_registry_package_versions(&alef_toml_path, "0.3.0-rc.28").expect("sync ok");
        assert!(changed, "must report at least one change");

        let updated = std::fs::read_to_string(&alef_toml_path).expect("read alef.toml");

        assert!(
            updated.contains("version = \">=0.3.0rc28\""),
            "python version must be PEP 440 formatted: {updated}"
        );
        assert!(
            updated.contains("version = \"^0.3.0-rc.28\""),
            "node version must preserve ^ prefix: {updated}"
        );
        assert!(
            updated.contains("version = \"~> 0.3.0-rc.28\""),
            "elixir version must preserve ~> prefix: {updated}"
        );
        assert!(
            updated.contains("version = \">= 0.3.0.pre.rc.28\""),
            "ruby version must be RubyGems formatted: {updated}"
        );
        // Names must be untouched.
        assert!(updated.contains("name = \"mylib\""), "package names must be preserved");
        assert!(
            updated.contains("name = \"@myorg/mylib\""),
            "node name must be preserved"
        );
    }

    #[test]
    fn sync_registry_package_versions_skips_entries_without_version_field() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let alef_toml_path = tmp.path().join("alef.toml");
        let original = concat!(
            "[workspace]\nlanguages = []\n\n",
            "[[crates]]\nname = \"mylib\"\nsources = []\n\n",
            "[crates.e2e.registry.packages.go]\n",
            "module = \"github.com/myorg/mylib\"\n",
            // No version field here — must not be inserted.
        );
        std::fs::write(&alef_toml_path, original).expect("write");

        let changed = sync_registry_package_versions(&alef_toml_path, "0.3.0-rc.28").expect("sync ok");
        assert!(!changed, "no version field → no change");

        let content = std::fs::read_to_string(&alef_toml_path).expect("read");
        assert!(!content.contains("version"), "version must not be inserted: {content}");
    }

    #[test]
    fn sync_registry_package_versions_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let alef_toml_path = tmp.path().join("alef.toml");
        std::fs::write(
            &alef_toml_path,
            concat!(
                "[workspace]\nlanguages = []\n\n",
                "[[crates]]\nname = \"mylib\"\nsources = []\n\n",
                "[crates.e2e.registry.packages.python]\n",
                "version = \">=0.3.0rc28\"\n",
            ),
        )
        .expect("write");

        // First call: already current → no change.
        let changed = sync_registry_package_versions(&alef_toml_path, "0.3.0-rc.28").expect("sync ok");
        assert!(!changed, "already-current version must be a no-op");
    }

    #[test]
    fn sync_registry_package_versions_preserves_toml_comments_and_order() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let alef_toml_path = tmp.path().join("alef.toml");
        let original = concat!(
            "# Top-level comment\n",
            "[workspace]\nlanguages = []\n\n",
            "[[crates]]\nname = \"mylib\"\nsources = []\n\n",
            "# Registry section comment\n",
            "[crates.e2e.registry.packages.python]\n",
            "name = \"mylib\"\n",
            "version = \">=0.1.0rc9\"\n",
        );
        std::fs::write(&alef_toml_path, original).expect("write");

        sync_registry_package_versions(&alef_toml_path, "0.3.0-rc.28").expect("sync ok");

        let updated = std::fs::read_to_string(&alef_toml_path).expect("read");
        assert!(
            updated.contains("# Top-level comment"),
            "top-level comment must be preserved: {updated}"
        );
        assert!(
            updated.contains("# Registry section comment"),
            "registry section comment must be preserved: {updated}"
        );
        // name must appear before version (key order preserved).
        let name_pos = updated.find("name = ").expect("name field present");
        let ver_pos = updated.find("version = ").expect("version field present");
        assert!(name_pos < ver_pos, "name must appear before version in output");
    }

    /// `sync_registry_package_versions` must update all language entries with
    /// both a plain prefix-less version and a prefixed constraint in a single call,
    /// covering a production alef.toml shape with mixed version constraints.
    #[test]
    fn sync_registry_package_versions_handles_go_and_bare_semver_langs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let alef_toml_path = tmp.path().join("alef.toml");
        std::fs::write(
            &alef_toml_path,
            concat!(
                "[workspace]\nlanguages = []\n\n",
                "[[crates]]\nname = \"mylib\"\nsources = []\n\n",
                "[crates.e2e.registry.packages.go]\n",
                "module = \"github.com/myorg/mylib\"\n",
                "version = \"v0.1.0-rc.9\"\n\n",
                "[crates.e2e.registry.packages.rust]\n",
                "name = \"mylib\"\n",
                "version = \"0.1.0-rc.9\"\n\n",
                "[crates.e2e.registry.packages.php]\n",
                "name = \"myorg/mylib\"\n",
                "version = \">=0.1.0-rc.9\"\n",
            ),
        )
        .expect("write alef.toml");

        let changed = sync_registry_package_versions(&alef_toml_path, "0.3.0-rc.28").expect("sync ok");
        assert!(changed, "must report at least one change");

        let updated = std::fs::read_to_string(&alef_toml_path).expect("read alef.toml");
        assert!(
            updated.contains("version = \"v0.3.0-rc.28\""),
            "go version must have v prefix: {updated}"
        );
        assert!(
            updated.contains("version = \"0.3.0-rc.28\""),
            "rust bare semver must be updated: {updated}"
        );
        assert!(
            updated.contains("version = \">=0.3.0-rc.28\""),
            "php composer constraint must be updated: {updated}"
        );
    }

    // -----------------------------------------------------------------------
    // sync_e2e_java_pom tests
    // -----------------------------------------------------------------------

    const JAVA_E2E_POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
    <groupId>dev.sample_crate.sample_crawler</groupId>
    <artifactId>sample_crawler-e2e-java</artifactId>
    <version>0.1.0</version>

    <dependencies>
        <dependency>
            <groupId>dev.sample_crate.sample_crawler</groupId>
            <artifactId>sample_crawler</artifactId>
            <version>0.3.0-rc.27</version>
            <scope>system</scope>
            <systemPath>${project.basedir}/../../packages/java/target/sample_crawler-0.3.0-rc.27.jar</systemPath>
        </dependency>
        <dependency>
            <groupId>org.junit.jupiter</groupId>
            <artifactId>junit-jupiter</artifactId>
            <version>${junit.version}</version>
            <scope>test</scope>
        </dependency>
    </dependencies>
</project>
"#;

    #[test]
    fn sync_e2e_java_pom_updates_dependency_version_and_system_path() {
        let result = sync_e2e_java_pom(JAVA_E2E_POM, "0.3.0-rc.28");
        assert!(result.is_some(), "expected Some when version changes");
        let new = result.unwrap();
        // Dependency <version> updated
        assert!(
            new.contains("<version>0.3.0-rc.28</version>"),
            "dependency version must be updated:\n{new}"
        );
        // systemPath version fragment updated
        assert!(
            new.contains("sample_crawler-0.3.0-rc.28.jar"),
            "systemPath must be updated:\n{new}"
        );
        // Project-level <version>0.1.0</version> must NOT be touched
        assert!(
            new.contains("<version>0.1.0</version>"),
            "project version must be unchanged:\n{new}"
        );
        // JUnit variable reference must not be touched
        assert!(
            new.contains("<version>${junit.version}</version>"),
            "junit version placeholder must be unchanged:\n{new}"
        );
        // Old version string gone
        assert!(!new.contains("0.3.0-rc.27"), "old version must be removed:\n{new}");
    }

    #[test]
    fn sync_e2e_java_pom_is_idempotent() {
        let first = sync_e2e_java_pom(JAVA_E2E_POM, "0.3.0-rc.28").unwrap();
        let second = sync_e2e_java_pom(&first, "0.3.0-rc.28");
        assert!(second.is_none(), "second call with same version must be a no-op");
    }

    #[test]
    fn sync_e2e_java_pom_no_system_scope_returns_none() {
        let content = "<?xml version=\"1.0\"?>\n<project><version>0.1.0</version></project>\n";
        assert!(
            sync_e2e_java_pom(content, "1.0.0").is_none(),
            "no system-scope dep means nothing to update"
        );
    }

    // -----------------------------------------------------------------------
    // sync_e2e_go_mod tests
    // -----------------------------------------------------------------------

    const GO_MOD_E2E: &str = "\
module e2e_go

go 1.26

require (
\tgithub.com/sample_crate-dev/sample_crawler/packages/go v0.3.0-rc.27
\tgithub.com/stretchr/testify v1.11.1
)

replace github.com/sample_crate-dev/sample_crawler/packages/go => ../../packages/go
";

    #[test]
    fn sync_e2e_go_mod_updates_library_require_line() {
        let fragment = "github.com/sample_crate-dev/sample_crawler/packages/go";
        let result = sync_e2e_go_mod(GO_MOD_E2E, fragment, "0.3.0-rc.28");
        assert!(result.is_some(), "expected Some when version changes");
        let new = result.unwrap();
        assert!(
            new.contains("github.com/sample_crate-dev/sample_crawler/packages/go v0.3.0-rc.28"),
            "library require line must be updated:\n{new}"
        );
        // Third-party dep untouched
        assert!(
            new.contains("github.com/stretchr/testify v1.11.1"),
            "testify version must be unchanged:\n{new}"
        );
        assert!(!new.contains("v0.3.0-rc.27"), "old version must be gone:\n{new}");
    }

    #[test]
    fn sync_e2e_go_mod_is_idempotent() {
        let fragment = "github.com/sample_crate-dev/sample_crawler/packages/go";
        let first = sync_e2e_go_mod(GO_MOD_E2E, fragment, "0.3.0-rc.28").unwrap();
        let second = sync_e2e_go_mod(&first, fragment, "0.3.0-rc.28");
        assert!(second.is_none(), "second call with same version must be a no-op");
    }

    // -----------------------------------------------------------------------
    // sync_e2e_dart_pubspec_lock tests
    // -----------------------------------------------------------------------

    const DART_PUBSPEC_LOCK: &str = "\
# Generated by pub
packages:
  async:
    dependency: transitive
    description:
      name: async
      sha256: abc123
      url: \"https://pub.dev\"
    source: hosted
    version: \"1.19.1\"
  sample_crawler:
    dependency: \"direct main\"
    description:
      path: \"../../packages/dart\"
      relative: true
    source: path
    version: \"0.3.0-rc.23\"
  logging:
    dependency: transitive
    description:
      name: logging
      sha256: def456
      url: \"https://pub.dev\"
    source: hosted
    version: \"1.2.0\"
";

    #[test]
    fn sync_e2e_dart_pubspec_lock_updates_path_source_version() {
        let result = sync_e2e_dart_pubspec_lock(DART_PUBSPEC_LOCK, "0.3.0-rc.28");
        assert!(result.is_some(), "expected Some when version changes");
        let new = result.unwrap();
        assert!(
            new.contains("version: \"0.3.0-rc.28\""),
            "path-source version must be updated:\n{new}"
        );
        // Hosted packages untouched
        assert!(
            new.contains("version: \"1.19.1\""),
            "hosted async version must be unchanged:\n{new}"
        );
        assert!(
            new.contains("version: \"1.2.0\""),
            "hosted logging version must be unchanged:\n{new}"
        );
        assert!(!new.contains("0.3.0-rc.23"), "old version must be gone:\n{new}");
    }

    #[test]
    fn sync_e2e_dart_pubspec_lock_is_idempotent() {
        let first = sync_e2e_dart_pubspec_lock(DART_PUBSPEC_LOCK, "0.3.0-rc.28").unwrap();
        let second = sync_e2e_dart_pubspec_lock(&first, "0.3.0-rc.28");
        assert!(second.is_none(), "second call with same version must be a no-op");
    }

    #[test]
    fn sync_e2e_dart_pubspec_lock_no_path_source_returns_none() {
        // A pubspec.lock with only hosted packages — nothing to sync.
        let content = "packages:\n  async:\n    dependency: transitive\n    description:\n      name: async\n      url: \"https://pub.dev\"\n    source: hosted\n    version: \"1.19.1\"\n";
        assert!(
            sync_e2e_dart_pubspec_lock(content, "0.3.0-rc.28").is_none(),
            "no path-source means nothing to update"
        );
    }

    // -----------------------------------------------------------------------
    // sync_versions auto test_apps regen
    // -----------------------------------------------------------------------

    /// Regression test for the rc.13 incident: after `sync-versions` updates
    /// `[crates.e2e.registry.packages.python].version` in alef.toml, the
    /// generated `test_apps/python/pyproject.toml` must contain the new version
    /// string rather than the stale prior version.
    ///
    /// This test exercises `sync_versions` with `no_regen=false` (the default
    /// for direct CLI invocations). The alef.toml has a minimal `[e2e]` block
    /// with an empty fixtures directory so `generate_e2e` runs scaffold-only
    /// (no IR extraction needed for pyproject.toml generation).
    #[test]
    fn sync_versions_regenerates_test_apps_pins() {
        use crate::core::config::NewAlefConfig;

        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Workspace Cargo.toml at the target version (1.2.3).
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.2.3\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        // Empty fixtures directory (generate_e2e accepts empty fixture sets —
        // scaffold files like pyproject.toml are always emitted).
        std::fs::create_dir_all(root.join("fixtures")).expect("mkdir fixtures");

        // alef.toml: minimal [e2e] block with registry python package pinned at
        // the stale version "0.0.0". sync_versions must update this AND regenerate
        // test_apps/python/pyproject.toml with the bumped version.
        //
        // [crates.e2e.call] is required by the schema; module/function both
        // default to empty string via #[serde(default)].
        let alef_toml = format!(
            concat!(
                "[workspace]\n",
                "languages = [\"python\"]\n\n",
                "[[crates]]\n",
                "name = \"mylib\"\n",
                "sources = []\n",
                "version_from = \"{cargo_toml}\"\n\n",
                "[crates.e2e]\n",
                "fixtures = \"fixtures\"\n",
                "languages = [\"python\"]\n\n",
                "[crates.e2e.call]\n",
                "module = \"mylib\"\n",
                "function = \"parse\"\n\n",
                "[crates.e2e.registry.packages.python]\n",
                "name = \"mylib\"\n",
                "version = \"0.0.0\"\n",
            ),
            cargo_toml = root.join("Cargo.toml").display().to_string().replace('\\', "/"),
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        // no_regen=false: auto-regen must fire and update test_apps/.
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, false, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        // alef.toml registry version must be bumped.
        let updated_toml = std::fs::read_to_string(&alef_toml_path).expect("read alef.toml");
        assert!(
            updated_toml.contains("version = \"1.2.3\""),
            "alef.toml registry package version must be updated to 1.2.3:\n{updated_toml}"
        );
        assert!(
            !updated_toml.contains("version = \"0.0.0\""),
            "stale 0.0.0 must be gone from alef.toml:\n{updated_toml}"
        );

        // test_apps/python/pyproject.toml must reference the new version.
        let pyproject_path = root.join("test_apps/python/pyproject.toml");
        assert!(
            pyproject_path.exists(),
            "test_apps/python/pyproject.toml must be generated by auto-regen"
        );
        let pyproject = std::fs::read_to_string(&pyproject_path).expect("read pyproject.toml");
        // The dependency line must reference the new registry version.
        // The e2e project's own `version = "0.0.0"` header is intentional and
        // unrelated to the registry pin — assert on the dependency entry specifically.
        assert!(
            pyproject.contains("mylib==1.2.3"),
            "test_apps/python/pyproject.toml must pin the new registry version 1.2.3:\n{pyproject}"
        );
        assert!(
            !pyproject.contains("mylib==0.0.0"),
            "stale registry pin mylib==0.0.0 must be gone from test_apps/python/pyproject.toml:\n{pyproject}"
        );
    }

    // -----------------------------------------------------------------------
    // moduleVersion (Go download_ffi) text replacement
    // -----------------------------------------------------------------------

    /// Regression test for the rc.13/rc.14 incident: `sync-versions` must rewrite
    /// `moduleVersion = "..."` in `packages/go/cmd/download_ffi/main.go` so that
    /// Go module consumers of a freshly released version pull the correct FFI binary.
    ///
    /// Without this fix, the developer's `alef all` (run before the version bump)
    /// bakes the old version into main.go; the subsequent `sync-versions` bump
    /// updated Cargo.toml and other manifests but left main.go stale, causing
    /// Go consumers to pull the previous release's FFI binary.
    #[test]
    fn sync_versions_updates_go_module_version_in_download_ffi() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Workspace Cargo.toml at the target version (1.9.0-rc.14).
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.9.0-rc.14\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        // Simulate packages/go/cmd/download_ffi/main.go with the stale rc.13 version.
        let download_ffi_dir = root.join("packages/go/cmd/download_ffi");
        std::fs::create_dir_all(&download_ffi_dir).expect("mkdir download_ffi");
        let stale_main_go = concat!(
            "// Tool to download platform-specific FFI libraries from GitHub releases.\n",
            "package main\n\nconst (\n",
            "\tmoduleVersion = \"1.9.0-rc.13\"\n",
            "\trepoURL       = \"https://github.com/example/mylib\"\n",
            ")\n",
        );
        std::fs::write(download_ffi_dir.join("main.go"), stale_main_go).expect("write main.go");

        let alef_toml = format!(
            concat!(
                "[workspace]\nlanguages = [\"go\"]\n\n",
                "[[crates]]\nname = \"mylib\"\nsources = []\n",
                "version_from = \"{cargo_toml}\"\n",
            ),
            cargo_toml = root.join("Cargo.toml").display().to_string().replace('\\', "/"),
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: crate::core::config::NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        // no_regen=true to skip scaffold/test_apps regen — testing text replacement only.
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let updated_main = std::fs::read_to_string(download_ffi_dir.join("main.go")).expect("read main.go");
        assert!(
            updated_main.contains("moduleVersion = \"1.9.0-rc.14\""),
            "moduleVersion must be updated to 1.9.0-rc.14:\n{updated_main}"
        );
        assert!(
            !updated_main.contains("1.9.0-rc.13"),
            "stale rc.13 moduleVersion must be gone from main.go:\n{updated_main}"
        );
        // Surrounding code must be untouched.
        assert!(
            updated_main.contains("repoURL"),
            "other constants must be preserved:\n{updated_main}"
        );
    }

    // -----------------------------------------------------------------------
    // sync_versions scaffold regen
    // -----------------------------------------------------------------------

    /// Regression test: after `sync-versions` bumps the workspace version, the
    /// scaffold generator must be re-run so that scaffold files embedding the
    /// version (R DESCRIPTION, Dart pubspec.yaml, Ruby gemspec, etc.) reflect
    /// the new version atomically with the workspace bump.
    ///
    /// This test uses the R backend because `packages/r/DESCRIPTION` embeds
    /// `Version: X.Y.Z` at scaffold time and is therefore the canonical
    /// scaffold-side version surface not covered by the existing text-replacement
    /// pass in `sync_versions`.
    #[test]
    fn sync_versions_regenerates_scaffold_version_fields() {
        use crate::core::config::NewAlefConfig;

        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Workspace Cargo.toml at the NEW version (1.2.3).
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.2.3\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        // Pre-populate packages/r/DESCRIPTION with the STALE version (0.0.0)
        // so we can verify the scaffold regen overwrites it.
        std::fs::create_dir_all(root.join("packages/r")).expect("mkdir packages/r");
        let stale_description = concat!(
            "Package: mylib\nTitle: My Library\nVersion: 0.0.0\nDescription: A library.\n",
            "License: MIT\nEncoding: UTF-8\nRoxygenNote: 7.3.1\n",
        );
        std::fs::write(root.join("packages/r/DESCRIPTION"), stale_description).expect("write DESCRIPTION");

        // alef.toml: R language enabled, no e2e block.
        let alef_toml = format!(
            concat!(
                "[workspace]\n",
                "languages = [\"r\"]\n\n",
                "[[crates]]\n",
                "name = \"mylib\"\n",
                "sources = []\n",
                "version_from = \"{cargo_toml}\"\n",
            ),
            cargo_toml = root.join("Cargo.toml").display().to_string().replace('\\', "/"),
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        // no_regen=false: scaffold regen must fire and update packages/r/DESCRIPTION.
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, false, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let description_path = root.join("packages/r/DESCRIPTION");
        assert!(
            description_path.exists(),
            "packages/r/DESCRIPTION must exist after scaffold regen"
        );
        let description = std::fs::read_to_string(&description_path).expect("read DESCRIPTION");
        assert!(
            description.contains("Version: 1.2"),
            "DESCRIPTION must contain the new version 1.2.x after scaffold regen:\n{description}"
        );
        assert!(
            !description.contains("Version: 0.0.0"),
            "stale Version: 0.0.0 must be gone from DESCRIPTION:\n{description}"
        );
    }

    // -----------------------------------------------------------------------
    // Four-pattern gap regression tests (kotlin-android, python __version__,
    // swift from:, C download_ffi.sh VERSION=)
    // -----------------------------------------------------------------------

    /// `sync_versions` must bump `version = "..."` inside `packages/kotlin-android/build.gradle.kts`
    /// (the `coordinates()` block version), leaving plugin `version "..."` declarations intact.
    #[test]
    fn sync_versions_bumps_kotlin_android_gradle_coordinates_version() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        let gradle_content = concat!(
            "plugins {\n",
            "    id(\"com.android.library\") version \"8.13.0\"\n",
            "    kotlin(\"android\") version \"2.3.21\"\n",
            "}\n",
            "\n",
            "mavenPublishing {\n",
            "    coordinates(\n",
            "        groupId = \"dev.example\",\n",
            "        artifactId = \"mylib-android\",\n",
            "        version = \"1.9.0-rc.16\",\n",
            "    )\n",
            "}\n",
        );
        std::fs::create_dir_all(root.join("packages/kotlin-android")).expect("mkdir");
        std::fs::write(root.join("packages/kotlin-android/build.gradle.kts"), gradle_content)
            .expect("write build.gradle.kts");

        let alef_toml = format!(
            "[workspace]\nlanguages = [\"kotlin_android\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let gradle = std::fs::read_to_string(root.join("packages/kotlin-android/build.gradle.kts"))
            .expect("read build.gradle.kts");
        assert!(
            gradle.contains("version = \"1.9.0-rc.17\""),
            "kotlin-android coordinates version must be bumped:\n{gradle}"
        );
        assert!(
            gradle.contains(r#"kotlin("android") version "2.3.21""#),
            "kotlin plugin version must not change:\n{gradle}"
        );
        assert!(
            gradle.contains(r#"id("com.android.library") version "8.13.0""#),
            "android plugin version must not change:\n{gradle}"
        );
        assert!(
            !gradle.contains("1.9.0-rc.16"),
            "stale rc.16 version must be gone:\n{gradle}"
        );
    }

    /// `sync_versions` must find `__version__ = "..."` in a nested module `__init__.py`
    /// under `packages/python/<module>/` (src layout), not just a flat `packages/python/__init__.py`.
    #[test]
    fn sync_versions_bumps_nested_python_init_version() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        // Nested src-layout module path: packages/python/mylib/__init__.py
        let py_module_dir = root.join("packages/python/mylib");
        std::fs::create_dir_all(&py_module_dir).expect("mkdir");
        std::fs::write(
            py_module_dir.join("__init__.py"),
            "\"\"\"mylib public API.\"\"\"\n\n__version__ = \"1.9.0-rc.16\"\n",
        )
        .expect("write __init__.py");

        let alef_toml = format!(
            "[workspace]\nlanguages = [\"python\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let content = std::fs::read_to_string(py_module_dir.join("__init__.py")).expect("read __init__.py");
        assert!(
            content.contains("__version__ = \"1.9.0-rc.17\""),
            "nested __version__ must be bumped:\n{content}"
        );
        assert!(
            !content.contains("1.9.0-rc.16"),
            "stale rc.16 __version__ must be gone:\n{content}"
        );
    }

    /// `sync_versions` must bump the `from: "X.Y.Z"` version pin in
    /// `test_apps/swift/Package.swift` without touching the rest of the file.
    #[test]
    fn sync_versions_bumps_swift_package_from_version() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        let swift_pkg_content = concat!(
            "// swift-tools-version: 6.0\n",
            "import PackageDescription\n",
            "\n",
            "let package = Package(\n",
            "    name: \"TestApp\",\n",
            "    dependencies: [\n",
            "        .package(url: \"https://example.com/alef-sample/mylib.git\", from: \"1.9.0-rc.16\"),\n",
            "    ],\n",
            "    targets: []\n",
            ")\n",
        );
        let swift_dir = root.join("test_apps/swift");
        std::fs::create_dir_all(&swift_dir).expect("mkdir");
        std::fs::write(swift_dir.join("Package.swift"), swift_pkg_content).expect("write Package.swift");

        let alef_toml = format!(
            "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let swift_pkg = std::fs::read_to_string(swift_dir.join("Package.swift")).expect("read Package.swift");
        assert!(
            swift_pkg.contains("from: \"1.9.0-rc.17\""),
            "swift from: version must be bumped:\n{swift_pkg}"
        );
        assert!(
            !swift_pkg.contains("from: \"1.9.0-rc.16\""),
            "stale rc.16 from: version must be gone:\n{swift_pkg}"
        );
        assert!(
            swift_pkg.contains("https://example.com/alef-sample/mylib.git"),
            "repo URL must be preserved:\n{swift_pkg}"
        );
    }

    /// `sync_versions` must substitute `v__ALEF_SWIFT_VERSION__` in the root
    /// `Package.swift` and the substitution must SURVIVE the in-band scaffold
    /// regen pass that runs immediately after the text-replacement loop.
    ///
    /// Regression: the binaryTarget root manifest emitter writes the file with
    /// `v__ALEF_SWIFT_VERSION__` as a placeholder so the in-VCS file stays
    /// stable across version bumps. `regenerate_scaffold_after_sync` then
    /// overwrites the substituted manifest with the placeholder form. Without
    /// a second-pass substitution at the end of `sync_versions`, the on-disk
    /// `Package.swift` permanently points at the literal
    /// `…/releases/download/v__ALEF_SWIFT_VERSION__/…` URL and SwiftPM
    /// resolution fails for downstream consumers.
    #[test]
    fn sync_versions_root_package_swift_placeholder_survives_scaffold_regen() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        // Root Package.swift uses the binaryTarget placeholder shape that
        // `scaffold_swift` emits today.
        let root_pkg_content = concat!(
            "// swift-tools-version: 6.0\n",
            "import PackageDescription\n",
            "let package = Package(name: \"MyLib\", targets: [\n",
            "  .binaryTarget(\n",
            "    name: \"RustBridge\",\n",
            "    url: \"https://example.com/alef-sample/mylib/releases/download/v__ALEF_SWIFT_VERSION__/MyLib-rs.artifactbundle.zip\",\n",
            "    checksum: \"__ALEF_SWIFT_CHECKSUM__\"\n",
            "  ),\n",
            "])\n",
        );
        std::fs::write(root.join("Package.swift"), root_pkg_content).expect("write root Package.swift");

        let alef_toml = format!(
            "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        // no_regen=false → scaffold regen runs and would clobber the substitution
        // without the second-pass fix.
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, false, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        let root_pkg = std::fs::read_to_string(root.join("Package.swift")).expect("read root Package.swift");
        assert!(
            !root_pkg.contains("v__ALEF_SWIFT_VERSION__"),
            "root Package.swift must not retain the version placeholder after sync_versions, got:\n{root_pkg}"
        );
        assert!(
            root_pkg.contains("/releases/download/v1.9.0-rc.17/"),
            "root Package.swift URL must point at substituted version v1.9.0-rc.17, got:\n{root_pkg}"
        );
        // When skip_swift_checksum=true (the value used in this test), the checksum
        // placeholder is NOT substituted by sync-versions — it is only filled in
        // when --skip-swift-checksum is not passed AND the swift binding crate + a
        // pre-built artifactbundle zip are available on the current host.
        assert!(
            root_pkg.contains("__ALEF_SWIFT_CHECKSUM__"),
            "root Package.swift must retain the checksum placeholder when skip_swift_checksum=true, got:\n{root_pkg}"
        );
    }

    /// `sync_versions` must bump `VERSION="X.Y.Z"` (no spaces around `=`) in
    /// both `e2e/c/download_ffi.sh` and `test_apps/c/download_ffi.sh`.
    #[test]
    fn sync_versions_bumps_c_download_ffi_sh_version() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.9.0-rc.17\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        let sh_content = concat!(
            "#!/usr/bin/env bash\n",
            "set -euo pipefail\n",
            "\n",
            "REPO_URL=\"https://example.com/alef-sample/mylib\"\n",
            "VERSION=\"1.9.0-rc.16\"\n",
            "FFI_PKG_NAME=\"mylib-ffi\"\n",
        );

        let e2e_c_dir = root.join("e2e/c");
        std::fs::create_dir_all(&e2e_c_dir).expect("mkdir e2e/c");
        std::fs::write(e2e_c_dir.join("download_ffi.sh"), sh_content).expect("write e2e download_ffi.sh");

        let test_apps_c_dir = root.join("test_apps/c");
        std::fs::create_dir_all(&test_apps_c_dir).expect("mkdir test_apps/c");
        std::fs::write(test_apps_c_dir.join("download_ffi.sh"), sh_content).expect("write test_apps download_ffi.sh");

        let alef_toml = format!(
            "[workspace]\nlanguages = [\"c\"]\n[[crates]]\nname = \"mylib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve config");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("set_current_dir");
        let sync_result = sync_versions(&resolved_cfg, &alef_toml_path, None, true, true);
        let _ = std::env::set_current_dir(&original_cwd);
        sync_result.expect("sync_versions ok");

        for (label, dir) in [("e2e", &e2e_c_dir), ("test_apps", &test_apps_c_dir)] {
            let content = std::fs::read_to_string(dir.join("download_ffi.sh"))
                .unwrap_or_else(|_| panic!("read {label}/c/download_ffi.sh"));
            assert!(
                content.contains("VERSION=\"1.9.0-rc.17\""),
                "{label}/c/download_ffi.sh VERSION must be bumped:\n{content}"
            );
            assert!(
                !content.contains("VERSION=\"1.9.0-rc.16\""),
                "{label}/c/download_ffi.sh stale rc.16 must be gone:\n{content}"
            );
            // Ensure we only replaced VERSION, not REPO_URL or FFI_PKG_NAME.
            assert!(
                content.contains("REPO_URL="),
                "{label}/c/download_ffi.sh REPO_URL must be preserved:\n{content}"
            );
        }
    }

    /// `compute_sha256_hex` must return the correct SHA-256 digest for a known
    /// input. The expected value was computed independently with:
    ///
    /// ```sh
    /// printf '' | shasum -a 256  # → e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    /// printf 'abc' | shasum -a 256  # → ba7816bf8f01cfea414140de5dae2ec73b00361bbef0469f26f5816a7fef1500
    /// ```
    #[test]
    fn compute_sha256_hex_empty_input() {
        let hex = compute_sha256_hex(b"");
        assert_eq!(
            hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "SHA-256 of empty input must match reference"
        );
    }

    #[test]
    fn compute_sha256_hex_abc() {
        let hex = compute_sha256_hex(b"abc");
        assert_eq!(
            hex, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "SHA-256 of 'abc' must match reference"
        );
    }

    /// `precompute_swift_checksum` must substitute `__ALEF_SWIFT_CHECKSUM__` in
    /// `Package.swift` when a pre-built `.artifactbundle.zip` exists in
    /// `dist/swift-artifactbundle/` and the current config has swift configured.
    ///
    /// This test does not shell out to `swift package compute-checksum`; it uses
    /// the in-process SHA-256 fallback because `swift` may not be on PATH in CI.
    #[test]
    fn precompute_swift_checksum_substitutes_when_zip_present() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Write Cargo.toml for the workspace.
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"2.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        // Write a root Package.swift with both placeholders.
        let pkg_content = concat!(
            "// swift-tools-version: 6.0\n",
            "import PackageDescription\n",
            "let package = Package(name: \"TestLib\", targets: [\n",
            "  .binaryTarget(\n",
            "    name: \"RustBridge\",\n",
            "    url: \"https://example.com/testlib/releases/download/v2.0.0/TestLib-rs.artifactbundle.zip\",\n",
            "    checksum: \"__ALEF_SWIFT_CHECKSUM__\"\n",
            "  ),\n",
            "])\n",
        );
        std::fs::write(root.join("Package.swift"), pkg_content).expect("write Package.swift");

        // Create the swift binding crate directory so the guard passes.
        let swift_crate_dir = root.join("crates/testlib-swift");
        std::fs::create_dir_all(&swift_crate_dir).expect("mkdir swift crate");
        std::fs::write(
            swift_crate_dir.join("Cargo.toml"),
            "[package]\nname = \"testlib-swift\"\nversion = \"2.0.0\"\n",
        )
        .expect("write swift Cargo.toml");

        // Create a minimal fake zip in dist/swift-artifactbundle/.
        let bundle_dir = root.join("dist/swift-artifactbundle");
        std::fs::create_dir_all(&bundle_dir).expect("mkdir bundle dir");
        let zip_content = b"fake-artifactbundle-zip-content-for-testing";
        std::fs::write(bundle_dir.join("TestLib-rs.artifactbundle.zip"), zip_content).expect("write fake zip");

        // Compute the expected checksum in-process.
        let expected_checksum = compute_sha256_hex(zip_content);

        // Write alef.toml with swift configured.
        let alef_toml = format!(
            "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"testlib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("chdir");
        let result = precompute_swift_checksum(&resolved_cfg);
        let _ = std::env::set_current_dir(&original_cwd);

        let checksum = result
            .expect("precompute_swift_checksum must succeed")
            .expect("must return Some(checksum) when zip is present");

        // The checksum must match the in-process SHA-256.
        assert_eq!(
            checksum, expected_checksum,
            "returned checksum must equal in-process SHA-256 of the fake zip"
        );

        // Package.swift must have the placeholder replaced.
        let pkg_result = std::fs::read_to_string(root.join("Package.swift")).expect("read");
        assert!(
            !pkg_result.contains("__ALEF_SWIFT_CHECKSUM__"),
            "Package.swift must not retain the placeholder after precompute, got:\n{pkg_result}"
        );
        assert!(
            pkg_result.contains(&expected_checksum),
            "Package.swift must contain the computed checksum, got:\n{pkg_result}"
        );

        // Sidecar file must be written.
        let sidecar =
            std::fs::read_to_string(root.join("target/alef-swift-checksum.txt")).expect("sidecar file must exist");
        assert_eq!(
            sidecar.trim(),
            expected_checksum,
            "sidecar must contain the computed checksum"
        );
    }

    /// `precompute_swift_checksum` must skip gracefully when no zip is found and
    /// the cargo build fails (missing Apple targets on non-macOS CI).
    #[test]
    fn precompute_swift_checksum_skips_when_no_zip_and_build_fails() {
        use crate::core::config::NewAlefConfig;
        let _guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_cwd = std::env::current_dir().expect("cwd");

        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"2.0.0\"\n\n[workspace]\nresolver = \"2\"\nmembers = []\n",
        )
        .expect("write Cargo.toml");

        let pkg_content = concat!(
            "// swift-tools-version: 6.0\n",
            "let package = Package(name: \"TestLib\", targets: [\n",
            "  .binaryTarget(name: \"RustBridge\",\n",
            "    url: \"https://example.com/v2.0.0/TestLib-rs.artifactbundle.zip\",\n",
            "    checksum: \"__ALEF_SWIFT_CHECKSUM__\"\n",
            "  ),\n",
            "])\n",
        );
        std::fs::write(root.join("Package.swift"), pkg_content).expect("write Package.swift");

        // Create the swift binding crate directory so that guard passes.
        let swift_crate_dir = root.join("crates/testlib-swift");
        std::fs::create_dir_all(&swift_crate_dir).expect("mkdir swift crate");
        std::fs::write(
            swift_crate_dir.join("Cargo.toml"),
            // Intentionally reference a nonexistent crate to guarantee build failure.
            "[package]\nname = \"testlib-swift\"\nversion = \"2.0.0\"\n[lib]\nname = \"nonexistent_guaranteed_fail\"\n",
        )
        .expect("write swift Cargo.toml");

        // No zip in dist/ — triggers the build path which will fail.
        let alef_toml = format!(
            "[workspace]\nlanguages = [\"swift\"]\n[[crates]]\nname = \"testlib\"\nsources = []\nversion_from = \"{}\"\n",
            root.join("Cargo.toml").display().to_string().replace('\\', "/")
        );
        let alef_toml_path = root.join("alef.toml");
        std::fs::write(&alef_toml_path, &alef_toml).expect("write alef.toml");

        let cfg: NewAlefConfig = toml::from_str(&alef_toml).expect("parse alef.toml");
        let mut resolved = cfg.resolve().expect("resolve");
        let resolved_cfg = resolved.remove(0);

        std::env::set_current_dir(root).expect("chdir");
        let result = precompute_swift_checksum(&resolved_cfg);
        let _ = std::env::set_current_dir(&original_cwd);

        // Must return Ok(None) — not an error — so sync_versions can continue.
        assert!(
            result.is_ok(),
            "precompute_swift_checksum must not propagate build errors, got: {:?}",
            result
        );
        assert!(result.unwrap().is_none(), "must return None when build fails");

        // Package.swift must still have the placeholder.
        let pkg_result = std::fs::read_to_string(root.join("Package.swift")).expect("read");
        assert!(
            pkg_result.contains("__ALEF_SWIFT_CHECKSUM__"),
            "Package.swift must retain placeholder when build fails, got:\n{pkg_result}"
        );
    }
}
